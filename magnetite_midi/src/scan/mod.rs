//! Reading a file for its **notes** and **text** without building the owned event list of [`Smf`](crate::smf::Smf).
//!
//! A "black MIDI" file has millions of notes: `tau2.5.9.mid` (50 MB) has 6.3 million, up to 816 000 in one second and 14 500 sounding at once.
//! Its [`Smf`](crate::smf::Smf) is 12.6 million 48-byte events (about 600 MB). [`scan`] walks `midly`'s lazy track iterators twice (once for
//! the tempo map, once for the notes) and keeps only a [`NoteTable`]: 16 bytes per note, in order of start, with times already in seconds. That is
//! 100 MB for the 6.3 million, built in about a second in a release build, and a window of it is found with two binary searches.

use crate::smf::MidiMessage;
use crate::smf::{decode_text, Division, Error, MetaEvents, NotePairer, PairedNote, TempoMap};
use midly::{MetaMessage, TrackEventKind};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// One note in a [`NoteTable`]: 16 bytes. Times are seconds in the file's own timeline (`f32` is good to 0.1 ms up to 15 minutes and 1 ms up to 4 hours).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackedNote {
    pub start: f32,
    pub end: f32,
    pub track: u16,
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
}

/// Every note of a file, in order of start (ties by track, channel, key, so the order is the same on every run).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteTable {
    notes: Vec<PackedNote>,
}

impl NoteTable {
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn as_slice(&self) -> &[PackedNote] {
        &self.notes
    }

    /// The index of the first note that starts at or after `secs`.
    pub fn first_at_or_after(&self, secs: f32) -> usize {
        self.notes.partition_point(|n| n.start < secs)
    }

    /// The notes that start in `[from, to)` seconds: two binary searches, whatever the density.
    pub fn starting_in(&self, from: f32, to: f32) -> &[PackedNote] {
        let (a, b) = (self.first_at_or_after(from), self.first_at_or_after(to));
        &self.notes[a..b.max(a)]
    }

    /// The most notes that start in any window of `window` seconds (how dense the file gets).
    pub fn densest(&self, window: f32) -> usize {
        let mut best = 0;
        let mut lo = 0;
        for (hi, n) in self.notes.iter().enumerate() {
            while self.notes[lo].start <= n.start - window {
                lo += 1;
            }
            best = best.max(hi + 1 - lo);
        }
        best
    }
}

/// What kind of text a [`TimedText`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Marker,
    CuePoint,
    Lyric,
    /// A plain text event (meta 0x01). Karaoke (`.kar`) files carry their syllables here, with `@` header lines; see [`crate::karaoke`].
    Text,
}

/// A marker, a cue point, a lyric syllable or a text event at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedText {
    pub time: f32,
    pub kind: TextKind,
    pub text: String,
    /// The track it was in.
    pub track: u16,
}

/// What [`scan`] found.
#[derive(Clone, Debug)]
pub struct Scan {
    pub notes: NoteTable,
    /// Markers, cue points and lyrics, in order of time.
    pub texts: Vec<TimedText>,
    pub tempo_map: TempoMap,
    /// Key signatures in order: `(tick, sharps, minor)`.
    pub key_signatures: Vec<(u64, i8, bool)>,
    /// The tick of the last event of any track (formats 2's sequences end to end).
    pub end_tick: u64,
    pub tracks: usize,
    /// Where each track starts, in ticks (all 0 except in format 2, whose sequences are put end to end).
    pub track_offsets: Vec<u64>,
    /// The first non-empty track name.
    pub title: Option<String>,
    /// Tracks that ended early because their bytes were damaged (the notes before the damage are kept).
    pub damaged_tracks: usize,
}

/// What [`scan`] keeps. The tempo map, the key signatures and the length are always read: they are cheap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    /// Keep the notes (the 16 bytes each). Without them the scan is one pass and holds almost nothing.
    pub notes: bool,
    /// Keep the markers, cue points and lyrics.
    pub texts: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            notes: true,
            texts: true,
        }
    }
}

/// Read the notes, markers, cue points and lyrics of a file, in memory proportional to the notes alone. Tolerant like [`Smf::parse`](crate::smf::Smf::parse):
/// a track whose bytes break off ends there and counts in [`Scan::damaged_tracks`].
pub fn scan(bytes: &[u8], options: ScanOptions) -> Result<Scan, Error> {
    let (header, tracks) =
        midly::parse(bytes).map_err(|e| Error(format!("not a MIDI file: {e}")))?;
    let division = Division::from_timing(header.timing);
    let sequential = header.format == midly::Format::Sequential;
    let tracks: Vec<_> = tracks.filter_map(Result::ok).collect();
    let mut damaged_tracks = 0;

    // pass 1: the tempo map, and the length of each track (format 2 puts the sequences end to end)
    let mut meta = MetaEvents::default();
    let mut end_tick = 0u64;
    let mut title: Option<String> = None;
    let mut offsets = Vec::with_capacity(tracks.len());
    let mut offset = 0u64;
    for track in &tracks {
        offsets.push(offset);
        let mut tick = offset;
        for event in track.clone() {
            let Ok(event) = event else {
                damaged_tracks += 1;
                break;
            };
            tick += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => meta.push_tempo(tick, t.as_int()),
                TrackEventKind::Meta(MetaMessage::TimeSignature(
                    numerator,
                    denominator_pow2,
                    clocks,
                    thirty_seconds,
                )) => meta.push_time_signature(
                    tick,
                    numerator,
                    denominator_pow2,
                    clocks,
                    thirty_seconds,
                ),
                TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)) => {
                    meta.push_key_signature(tick, sharps, minor)
                }
                TrackEventKind::Meta(MetaMessage::TrackName(t)) if title.is_none() => {
                    let name = decode_text(t).trim().to_string();
                    title = (!name.is_empty()).then_some(name);
                }
                _ => {}
            }
        }
        end_tick = end_tick.max(tick);
        if sequential {
            offset = tick;
        }
    }
    let tempo_map = meta.tempo_map(division);
    let key_signatures = meta.key_signatures();

    // pass 2: the notes (paired by the same `NotePairer` as `Smf::notes`) and the text
    let mut notes: Vec<PackedNote> = Vec::new();
    let mut texts: Vec<TimedText> = Vec::new();
    let secs = |tick: u64| tempo_map.secs_at(tick) as f32;
    for (index, track) in tracks
        .iter()
        .enumerate()
        .filter(|_| options.notes || options.texts)
    {
        let mut tick = offsets[index];
        let track_id = index.min(u16::MAX as usize) as u16;
        let mut pairer = NotePairer::new();
        let packed = |n: PairedNote| PackedNote {
            start: secs(n.start_tick),
            end: secs(n.end_tick),
            track: track_id,
            channel: n.channel,
            key: n.key,
            velocity: n.velocity,
        };
        for event in track.clone() {
            let Ok(event) = event else { break };
            tick += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Midi { channel, message } if options.notes => {
                    let message = MidiMessage::from_voice(channel, message);
                    notes.extend(pairer.observe(tick, &message).map(packed));
                }
                TrackEventKind::Meta(MetaMessage::Marker(t)) if options.texts => {
                    texts.push(TimedText {
                        time: secs(tick),
                        kind: TextKind::Marker,
                        text: decode_text(t),
                        track: track_id,
                    })
                }
                TrackEventKind::Meta(MetaMessage::CuePoint(t)) if options.texts => {
                    texts.push(TimedText {
                        time: secs(tick),
                        kind: TextKind::CuePoint,
                        text: decode_text(t),
                        track: track_id,
                    })
                }
                TrackEventKind::Meta(MetaMessage::Lyric(t)) if options.texts => {
                    texts.push(TimedText {
                        time: secs(tick),
                        kind: TextKind::Lyric,
                        text: decode_text(t),
                        track: track_id,
                    })
                }
                TrackEventKind::Meta(MetaMessage::Text(t)) if options.texts => {
                    texts.push(TimedText {
                        time: secs(tick),
                        kind: TextKind::Text,
                        text: decode_text(t),
                        track: track_id,
                    })
                }
                _ => {}
            }
        }
        // still sounding when the track ends: it ends there
        notes.extend(pairer.finish(tick).map(packed));
    }
    notes.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.track.cmp(&b.track))
            .then(a.channel.cmp(&b.channel))
            .then(a.key.cmp(&b.key))
    });
    texts.sort_by(|a, b| a.time.total_cmp(&b.time)); // stable
    Ok(Scan {
        notes: NoteTable { notes },
        texts,
        tempo_map,
        key_signatures,
        end_tick,
        tracks: tracks.len(),
        track_offsets: offsets,
        title,
        damaged_tracks,
    })
}

/// Every channel message of a file in order of time, read straight from the bytes: one lazy iterator per track, merged by tick (tracks keep their order within
/// a tick). Nothing is collected, so a 50 MB file costs its own bytes and a few hundred of the iterator's: the first message is ready as soon as the
/// tracks are opened, which is what lets a render start at once. `offsets` is [`Scan::track_offsets`]. A track whose bytes break off ends there.
pub fn channel_messages<'a>(
    bytes: &'a [u8],
    offsets: &[u64],
) -> Result<impl Iterator<Item = (u64, MidiMessage)> + 'a, Error> {
    let (_, tracks) = midly::parse(bytes).map_err(|e| Error(format!("not a MIDI file: {e}")))?;
    let mut tracks: Vec<_> = tracks.filter_map(Result::ok).collect();
    let mut ticks: Vec<u64> = (0..tracks.len())
        .map(|i| offsets.get(i).copied().unwrap_or(0))
        .collect();
    let mut pending: Vec<Option<MidiMessage>> = vec![None; tracks.len()];
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
    fn advance(track: &mut midly::EventIter<'_>, tick: &mut u64) -> Option<MidiMessage> {
        for event in track {
            let Ok(event) = event else { return None };
            *tick += event.delta.as_int() as u64;
            if let TrackEventKind::Midi { channel, message } = event.kind {
                return Some(MidiMessage::from_voice(channel, message));
            }
        }
        None
    }
    for i in 0..tracks.len() {
        pending[i] = advance(&mut tracks[i], &mut ticks[i]);
        if pending[i].is_some() {
            heap.push(Reverse((ticks[i], i)));
        }
    }
    Ok(std::iter::from_fn(move || {
        let Reverse((tick, i)) = heap.pop()?;
        let message = pending[i].take()?;
        pending[i] = advance(&mut tracks[i], &mut ticks[i]);
        if pending[i].is_some() {
            heap.push(Reverse((ticks[i], i)));
        }
        Some((tick, message))
    }))
}
