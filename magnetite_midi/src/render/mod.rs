//! Rendering a MIDI file to audio through a SoundFont, for files of any size: the notes are read from the bytes as the render goes (see [`crate::scan`]), so a "black MIDI" file of millions of
//! notes costs its own size and not hundreds of megabytes; the voices are shared between several synthesizers, each on its own thread, added as the notes need them; and the mix passes a
//! lookahead limiter, so that hundreds of voices summing past full scale are not heard as distortion.
//!
//! [`render`] takes the bytes of a file and a loaded [`SoundFont`] and hands interleaved stereo `f32` to a sink in chunks; the sink returns `false` to stop. It is deterministic: the same
//! file, font and options give the same samples. [`load_soundfont`] shares one copy of a font among everything that uses the file. [`tiny_soundfont`] makes a valid SoundFont of a few
//! kilobytes, so that tests need no real one.
//!
//! The sequencer of this crate ([`MidiFileSequencer`](crate::MidiFileSequencer)) plays in real time on one synthesizer, with speed and loops; this is for the other job, turning a whole file
//! into audio as fast as the machine allows.

use crate::{SoundFont, Synthesizer, SynthesizerSettings};
mod font;
mod limiter;
mod test_font;

pub use self::font::load_soundfont;
pub use self::test_font::tiny_soundfont;

use self::limiter::{Limiter, LOOKAHEAD};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;

/// What went wrong loading a font or rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError(pub String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for RenderError {}

/// How to render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    pub sample_rate: u32,
    /// The synthesizer's own reverb and chorus (about 1.5 times slower; the measured rate with it is still 90 times real time or more).
    pub reverb_and_chorus: bool,
    /// Seconds rendered after the last event, for releases and the reverb to die away.
    pub tail_secs: f64,
    /// Frames handed to the sink at a time (at most).
    pub chunk_frames: usize,
    /// The most voices sounding at once, in all; past it the synthesizers steal the cheapest (quietest, most released, then oldest). The default is effectively
    /// no limit: voices are made as notes need them, so a large number costs nothing until a file uses it, and what bounds a black MIDI file is the time to
    /// render it. Lower it on a machine that cannot keep up (a note that is not allowed to sound is cheap).
    pub max_polyphony: usize,
    /// The most synthesizers, each on its own thread. The render starts with one and adds one when the demand on the voices passes 64 for each synthesizer there is.
    pub synthesizers: usize,
    /// Start the render at this frame of the song: the sink gets the audio from there on (its first frame is this one). Notes that began earlier are not
    /// sounded, not even those still held; the program, controller, pedal and pitch-bend messages before it are applied, so what starts after it sounds as in a
    /// full render. For a seek to a time a slow render has not reached.
    pub start_frame: u64,
}

/// The default of `max_polyphony`: a million voices, which no file reaches in memory before it reaches the end of the time it takes.
pub const DEFAULT_MAX_POLYPHONY: usize = 1 << 20;

/// How long a released note is assumed to go on sounding (its release envelope), for counting the demand on the voices.
const RELEASE_SECS: f64 = 0.3;

/// The voices each synthesizer makes when it starts; more are made when notes need them (the render runs on its own threads, so making one costs nothing audible).
const INITIAL_VOICES: usize = 64;

/// A synthesizer is added when the demand passes this many voices for each one there is (the cost is the voices processed, so this spreads them over the cores).
const GROW_LOAD: usize = 64;

/// The synthesizers a render may use unless told otherwise: one for each core, at most 8.
pub fn default_synthesizers() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 8)
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            reverb_and_chorus: true,
            tail_secs: 2.5,
            chunk_frames: 4096,
            max_polyphony: DEFAULT_MAX_POLYPHONY,
            synthesizers: default_synthesizers(),
            start_frame: 0,
        }
    }
}

/// What a render produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSummary {
    /// Frames of audio (stereo frames) given to the sink.
    pub frames: u64,
    /// The length of the song itself in seconds (its last event), without the tail.
    pub song_secs: f64,
    /// The most voices the notes asked for at once, counting each note until a release time after its note-off (a lower bound: a note can take more than one voice).
    /// Past `max_polyphony` the synthesizers steal voices, so a peak above it is a render that lost notes.
    pub peak_demand: usize,
    /// How many synthesizers (threads) the render grew to.
    pub synthesizers: usize,
}

/// One window of the render for one synthesizer: its length in frames and the messages that fall in it, with their frame offsets into it.
struct Batch {
    frames: usize,
    messages: Vec<(u32, [i32; 4])>,
}

type Messages = Vec<(u32, [i32; 4])>;

/// Frames in a batch: long enough that the threads spend their time rendering and not waiting for each other, short enough to keep the first audio quick.
const BATCH_FRAMES: usize = 4096;

/// Batches in flight at once (each worker may be this far ahead of the sum).
const PIPELINE: usize = 3;

/// A synthesizer on its own thread: renders each batch it is sent, applying the batch's messages at their frames, and sends the audio back (interleaved stereo).
fn run_worker(mut synth: Synthesizer, batches: Receiver<Batch>, audio: SyncSender<Vec<f32>>) {
    let (mut left, mut right) = (vec![0f32; BATCH_FRAMES], vec![0f32; BATCH_FRAMES]);
    while let Ok(batch) = batches.recv() {
        let mut out = vec![0f32; batch.frames * 2];
        let mut at = 0usize;
        let mut render_to = |synth: &mut Synthesizer, at: &mut usize, to: usize| {
            let n = to - *at;
            if n > 0 {
                synth.render(&mut left[..n], &mut right[..n]);
                for i in 0..n {
                    out[2 * (*at + i)] = left[i];
                    out[2 * (*at + i) + 1] = right[i];
                }
                *at = to;
            }
        };
        for (offset, [channel, command, d1, d2]) in batch.messages {
            render_to(&mut synth, &mut at, offset as usize);
            synth.process_midi_message(channel, command, d1, d2);
        }
        render_to(&mut synth, &mut at, batch.frames);
        if audio.send(out).is_err() {
            return;
        }
    }
}

const STOPPED_THREAD: &str = "a synthesizer thread stopped";

/// Everything the render holds between batches: what is being filled, what is in flight, and the way out to the sink.
struct Mixer<'a> {
    to_workers: Vec<SyncSender<Batch>>,
    from_workers: Vec<Receiver<Vec<f32>>>,
    pending: Vec<Messages>,
    /// For each batch sent and not yet summed: its length and how many workers it went to.
    in_flight: VecDeque<(usize, usize)>,
    limiter: Limiter,
    release: f32,
    skip: usize,
    chunk: usize,
    sink: &'a mut dyn FnMut(&[f32]) -> bool,
}

impl Mixer<'_> {
    /// Hand every worker the messages gathered for the batch that ends here.
    fn send(&mut self, frames: usize) -> Result<(), RenderError> {
        for (tx, messages) in self.to_workers.iter().zip(self.pending.iter_mut()) {
            tx.send(Batch {
                frames,
                messages: std::mem::take(messages),
            })
            .map_err(|_| RenderError(STOPPED_THREAD.into()))?;
        }
        self.in_flight.push_back((frames, self.to_workers.len()));
        Ok(())
    }

    /// Add a worker; it takes part from the next batch sent.
    fn attach(&mut self, batches: SyncSender<Batch>, audio: Receiver<Vec<f32>>) {
        self.to_workers.push(batches);
        self.from_workers.push(audio);
        self.pending.push(Vec::new());
    }

    /// Sum the oldest batch from every worker, limit it and give it to the sink; `false` if the sink stopped.
    fn collect(&mut self) -> Result<bool, RenderError> {
        let Some((frames, workers)) = self.in_flight.pop_front() else {
            return Ok(true);
        };
        let mut mix = self.from_workers[0]
            .recv()
            .map_err(|_| RenderError(STOPPED_THREAD.into()))?;
        for rx in &self.from_workers[1..workers] {
            let part = rx.recv().map_err(|_| RenderError(STOPPED_THREAD.into()))?;
            for (m, p) in mix.iter_mut().zip(&part) {
                *m += p;
            }
        }
        self.limiter.process(&mut mix, self.release);
        let dropped = self.skip.min(frames);
        self.skip -= dropped;
        for piece in mix[dropped * 2..].chunks(self.chunk * 2) {
            if !(self.sink)(piece) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Keep no more than [`PIPELINE`] batches in flight; `false` if the sink stopped.
    fn keep_up(&mut self) -> Result<bool, RenderError> {
        while self.in_flight.len() > PIPELINE {
            if !self.collect()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Render a file through a font, straight from its bytes: the events are read lazily and merged by time (see `scan::channel_messages`), so the first audio comes
/// as soon as the tempo map is read and the memory is the file's own bytes, however many notes it has. `sink` receives interleaved stereo `f32` in chunks and returns `false` to stop (a cancelled source); the
/// render then ends with an error. Events are applied at the sample their tick falls on, to the synthesizer's block grid of 64 frames
/// (up to 1.45 ms later, measured).
///
/// A black MIDI file asks for thousands of voices at once, which one thread cannot process in time, so the render runs several synthesizers, each on its own
/// thread (up to `synthesizers`, added as the notes need them): every note-on goes to the next one in turn and its note-off to the same one, and everything else (programs, controllers, the pedal, pitch bend) to all of
/// them, so each is a complete player that holds its share of the notes. Their audio is summed in a fixed order, so the result does not depend on the threads' timing.
pub fn render(
    bytes: &[u8],
    font: &Arc<SoundFont>,
    opts: &RenderOptions,
    sink: &mut dyn FnMut(&[f32]) -> bool,
) -> Result<RenderSummary, RenderError> {
    // the voices allowed are shared out between the synthesizers there may be (a limit too small to share runs one)
    let max_count = opts.synthesizers.clamp(1, (opts.max_polyphony / 8).max(1));
    let voices_each = opts.max_polyphony.max(8).div_ceil(max_count);
    let mut settings = SynthesizerSettings::new(opts.sample_rate as i32);
    settings.enable_reverb_and_chorus = opts.reverb_and_chorus;
    settings.maximum_polyphony = voices_each;
    settings.initial_polyphony = Some(INITIAL_VOICES.min(voices_each)); // the rest are made as notes need them
    let new_synth = || {
        Synthesizer::new(font, &settings)
            .map_err(|e| RenderError(format!("cannot start the synthesizer: {e}")))
    };
    let first_synth = new_synth()?;

    let scan = crate::scan::scan(
        bytes,
        crate::scan::ScanOptions {
            notes: false,
            texts: false,
        },
    )
    .map_err(|e| RenderError(e.to_string()))?;
    let map = &scan.tempo_map;
    let sr = opts.sample_rate as f64;
    let song_secs = map.secs_at(scan.end_tick);
    let events = crate::scan::channel_messages(bytes, &scan.track_offsets)
        .map_err(|e| RenderError(e.to_string()))?;
    let release_frames = (RELEASE_SECS * sr) as u64;

    std::thread::scope(|scope| {
        let mut mixer = Mixer {
            to_workers: Vec::new(),
            from_workers: Vec::new(),
            pending: Vec::new(),
            in_flight: VecDeque::new(),
            limiter: Limiter::new(),
            release: 1.0 - (-1.0 / (0.1 * opts.sample_rate as f32)).exp(),
            skip: LOOKAHEAD, // frames of the limiter's delay still to drop from the start
            chunk: opts.chunk_frames.max(64),
            sink,
        };
        let start_worker = |mixer: &mut Mixer, synth: Synthesizer| {
            let (batch_tx, batch_rx) = sync_channel::<Batch>(PIPELINE + 1);
            let (audio_tx, audio_rx) = sync_channel::<Vec<f32>>(PIPELINE + 1);
            scope.spawn(move || run_worker(synth, batch_rx, audio_tx));
            mixer.attach(batch_tx, audio_rx);
        };
        start_worker(&mut mixer, first_synth);

        let first_frame = opts.start_frame;
        let mut start = first_frame; // the frame the batch being filled starts at
        let mut stopped = false;
        let mut holders: Vec<VecDeque<u8>> = vec![VecDeque::new(); 16 * 128]; // which synthesizer holds each (channel, key) note, oldest first
        let mut next_synth = 0usize;
        // The latest of every message that is not a note, in the order each first appeared, so that a synthesizer added late can be brought to the state of the others.
        let mut state: Vec<[i32; 4]> = Vec::new();
        let mut state_at: HashMap<(i32, i32, i32), usize> = HashMap::new();
        // The demand on the voices: notes held plus notes released within the last RELEASE_SECS (their frames, oldest first).
        let (mut held, mut peak_demand) = (0usize, 0usize);
        let mut released: VecDeque<u64> = VecDeque::new();

        let result = (|| -> Result<(), RenderError> {
            let mut last_frame = first_frame;
            for (tick, message) in events {
                let Some((channel, command, d1, d2)) = message.synth_args() else {
                    continue;
                };
                let mut frame = (map.secs_at(tick) * sr).round() as u64;
                if frame < first_frame {
                    if matches!(command, 0x80 | 0x90) {
                        continue;
                    }
                    frame = first_frame; // the state the song is in when the render starts
                }
                while frame >= start + BATCH_FRAMES as u64 {
                    mixer.send(BATCH_FRAMES)?;
                    start += BATCH_FRAMES as u64;
                    if !mixer.keep_up()? {
                        stopped = true;
                        return Ok(());
                    }
                }
                last_frame = frame;
                let item = ((frame - start) as u32, [channel, command, d1, d2]);
                let slot = (channel as usize & 15) * 128 + (d1 as usize & 127);
                match (command, d2) {
                    (0x90, 1..) => {
                        while released
                            .front()
                            .is_some_and(|f| *f + release_frames < frame)
                        {
                            released.pop_front();
                        }
                        held += 1;
                        let demand = held + released.len();
                        peak_demand = peak_demand.max(demand);
                        // grow the pool while the notes asked for would not fit in the voices there are (with a margin for notes that take more than one voice)
                        while mixer.to_workers.len() < max_count
                            && demand > mixer.to_workers.len() * GROW_LOAD
                        {
                            start_worker(&mut mixer, new_synth()?);
                            let latest = mixer.pending.len() - 1;
                            mixer.pending[latest].extend(state.iter().map(|m| (item.0, *m)));
                        }
                        let n = mixer.to_workers.len();
                        next_synth %= n;
                        holders[slot].push_back(next_synth as u8);
                        mixer.pending[next_synth].push(item);
                        next_synth = (next_synth + 1) % n;
                    }
                    (0x80, _) | (0x90, 0) => {
                        if let Some(w) = holders[slot].pop_front() {
                            mixer.pending[w as usize].push(item);
                            held -= 1;
                            released.push_back(frame);
                        }
                    }
                    _ => {
                        let key = (
                            channel,
                            command,
                            if command == 0xA0 || command == 0xB0 {
                                d1
                            } else {
                                0
                            },
                        );
                        let value = [channel, command, d1, d2];
                        match state_at.get(&key) {
                            Some(i) => state[*i] = value,
                            None => {
                                state_at.insert(key, state.len());
                                state.push(value);
                            }
                        }
                        mixer.pending.iter_mut().for_each(|p| p.push(item));
                    }
                }
            }
            // after the last event: the rest of the song and the tail (and the limiter's delay)
            let end = last_frame.max((song_secs * sr).round() as u64)
                + (opts.tail_secs * sr).round() as u64
                + LOOKAHEAD as u64;
            while start < end {
                let frames = ((end - start) as usize).min(BATCH_FRAMES);
                mixer.send(frames)?;
                start += frames as u64;
                if !mixer.keep_up()? {
                    stopped = true;
                    return Ok(());
                }
            }
            while !mixer.in_flight.is_empty() {
                if !mixer.collect()? {
                    stopped = true;
                    return Ok(());
                }
            }
            Ok(())
        })();
        let synthesizers = mixer.to_workers.len();
        drop(mixer); // the workers end when their batches do (at once, on a stop)
        result?;
        if stopped {
            return Err(RenderError("the render was stopped".into()));
        }
        Ok(RenderSummary {
            frames: start - first_frame - LOOKAHEAD as u64,
            song_secs,
            peak_demand,
            synthesizers,
        })
    })
}
