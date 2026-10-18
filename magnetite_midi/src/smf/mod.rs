//! Standard MIDI Files: read into an owned song with absolute ticks, a [`TempoMap`] that converts between ticks, seconds, quarter
//! notes and bars, the notes paired from their on and off messages, and a writer. The bytes are `midly`'s work; this module is the
//! model a program wants (everything owned, absolute times, tolerant of the damaged files real collections are full of).

mod message;
mod playback;
mod shared;
mod tempo;
mod wire;

pub use self::message::MidiMessage;
pub(crate) use self::shared::{MetaEvents, NotePairer, PairedNote};
pub use self::tempo::{Bar, TempoMap};
pub use self::wire::decode_text;
use self::wire::{from_track_event, to_track_events};

use midly::num::u15;
use midly::{Format, Header, MetaMessage, Smf as RawSmf, Timing, TrackEvent, TrackEventKind};
use std::fmt;

/// What went wrong reading a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Error {}

/// What a tick of the file means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Division {
    /// Ticks per quarter note; the tempo events decide how long a tick lasts.
    Ticks(u16),
    /// Ticks of a fixed duration: `fps` frames per second (29.97 for drop frame), `subframe` ticks per frame. Tempo events do
    /// not change a tick's duration (and a file like this has no bars: see [`TempoMap::bars`]).
    Smpte { fps: f64, subframe: u8 },
}

/// How the tracks of a file relate (the three SMF formats).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// Format 0: one track.
    Single,
    /// Format 1: tracks played together (a tempo track and one track per part).
    Parallel,
    /// Format 2: independent sequences. [`Smf::parse`] puts them end to end.
    Sequential,
}

/// One thing that happens at a tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A channel message or a system-exclusive message.
    Midi(MidiMessage),
    /// Microseconds per quarter note.
    Tempo(u32),
    /// `numerator` / 2^`denominator_pow2` (3/4 is 3, 2). `clocks` and `thirty_seconds` are the file's metronome fields.
    TimeSignature {
        numerator: u8,
        denominator_pow2: u8,
        clocks: u8,
        thirty_seconds: u8,
    },
    /// Sharps (positive) or flats (negative), and major or minor.
    KeySignature {
        sharps: i8,
        minor: bool,
    },
    Marker(String),
    CuePoint(String),
    Text(String),
    Lyric(String),
    TrackName(String),
    InstrumentName(String),
    /// Where the track ends when that is later than its last other event: the file's own end-of-track event, kept so that the silence after the last sound is not lost (a track's
    /// length, and where a note still sounding ends, depend on it). An end-of-track that falls on the last event is not kept, and the writer adds one to every track.
    EndOfTrack,
    /// Any other meta event, kept so that a file written back loses nothing: the type byte and the data.
    Meta {
        kind: u8,
        data: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub tick: u64,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    /// In order of time.
    pub events: Vec<Event>,
}

impl Track {
    pub fn name(&self) -> Option<&str> {
        self.events.iter().find_map(|e| {
            if let EventKind::TrackName(n) = &e.kind {
                Some(n.as_str())
            } else {
                None
            }
        })
    }

    /// The tick of the last event, which is the track's end-of-track when it has silence after its last sound (0 for an empty track).
    pub fn end_tick(&self) -> u64 {
        self.events.last().map_or(0, |e| e.tick)
    }
}

/// A note: a note-on paired with the note-off that ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub track: usize,
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
    pub start_tick: u64,
    pub end_tick: u64,
}

/// A Standard MIDI File as an owned song.
#[derive(Clone, Debug, PartialEq)]
pub struct Smf {
    pub format: FileFormat,
    pub division: Division,
    pub tracks: Vec<Track>,
}

impl Smf {
    /// Read the bytes of a file. Tolerant on purpose: a missing end-of-track, a chunk cut short, an `RMID` wrapper and SMPTE time
    /// are all accepted (`midly` is, `midi-msg` is not: see `.plans/midi.md`). Text is decoded as UTF-8 with replacement
    /// characters, since files carry whatever their authoring program used. Format 2 sequences are put end to end.
    pub fn parse(bytes: &[u8]) -> Result<Smf, Error> {
        let smf = RawSmf::parse(bytes).map_err(|e| Error(format!("not a MIDI file: {e}")))?;
        let division = Division::from_timing(smf.header.timing);
        let format = match smf.header.format {
            Format::SingleTrack => FileFormat::Single,
            Format::Parallel => FileFormat::Parallel,
            Format::Sequential => FileFormat::Sequential,
        };
        let mut tracks = Vec::with_capacity(smf.tracks.len());
        let mut offset = 0u64; // format 2: where the next sequence begins
        for raw in &smf.tracks {
            let mut tick = offset;
            let mut events: Vec<Event> = Vec::with_capacity(raw.len());
            for e in raw {
                tick += e.delta.as_int() as u64;
                if matches!(e.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
                    // kept only when it marks silence after the last sound; on the last event it adds nothing
                    if events.last().map_or(tick > 0, |last| tick > last.tick) {
                        events.push(Event {
                            tick,
                            kind: EventKind::EndOfTrack,
                        });
                    }
                } else if let Some(kind) = from_track_event(&e.kind) {
                    events.push(Event { tick, kind });
                }
            }
            if format == FileFormat::Sequential {
                offset = tick;
            }
            tracks.push(Track { events });
        }
        Ok(Smf {
            format,
            division,
            tracks,
        })
    }

    /// The tick of the last event of any track.
    pub fn end_tick(&self) -> u64 {
        self.tracks.iter().map(Track::end_tick).max().unwrap_or(0)
    }

    /// The tempo map of the song (tempo and time-signature events of every track).
    pub fn tempo_map(&self) -> TempoMap {
        self.meta_events().tempo_map(self.division)
    }

    fn meta_events(&self) -> MetaEvents {
        let mut meta = MetaEvents::default();
        for event in self.tracks.iter().flat_map(|t| t.events.iter()) {
            meta.push_event(event);
        }
        meta
    }

    /// Length in seconds up to the last event.
    pub fn length_secs(&self) -> f64 {
        self.tempo_map().secs_at(self.end_tick())
    }

    /// Key signatures in order: `(tick, sharps, minor)`.
    pub fn key_signatures(&self) -> Vec<(u64, i8, bool)> {
        self.meta_events().key_signatures()
    }

    /// The notes of the song, in order of start. A note-on is ended by the next note-off of the same key and channel in its track;
    /// repeated note-ons of one key end in the order they began; one still sounding at the end of its track ends there. The sustain
    /// pedal is not applied.
    pub fn notes(&self) -> Vec<Note> {
        let mut out = Vec::new();
        for (track, t) in self.tracks.iter().enumerate() {
            let mut pairer = NotePairer::new();
            let note = |n: PairedNote| Note {
                track,
                channel: n.channel,
                key: n.key,
                velocity: n.velocity,
                start_tick: n.start_tick,
                end_tick: n.end_tick,
            };
            for e in &t.events {
                if let EventKind::Midi(m) = &e.kind {
                    out.extend(pairer.observe(e.tick, m).map(note));
                }
            }
            out.extend(pairer.finish(t.end_tick()).map(note));
        }
        out.sort_by_key(|n| (n.start_tick, n.track, n.channel, n.key));
        out
    }

    /// The bytes of a type-1 (or the song's own format) file. Events are written in order of tick; every track ends with an
    /// end-of-track event at its last tick. Format 2 is written as format 1: [`Smf::parse`] has already put its sequences end
    /// to end.
    pub fn to_bytes(&self) -> Vec<u8> {
        let timing = match self.division {
            Division::Ticks(t) => Timing::Metrical(u15::new(t & 0x7FFF)),
            Division::Smpte { fps, subframe } => {
                // 29.97 is the drop-frame rate: it must not be rounded to 30
                let fps = if (fps - 29.97).abs() < 0.01 {
                    midly::Fps::Fps29
                } else {
                    match fps.round() as u32 {
                        24 => midly::Fps::Fps24,
                        25 => midly::Fps::Fps25,
                        _ => midly::Fps::Fps30,
                    }
                };
                Timing::Timecode(fps, subframe)
            }
        };
        let format = match (self.format, self.tracks.len()) {
            (FileFormat::Single, 1) => Format::SingleTrack,
            (FileFormat::Single, _) | (FileFormat::Parallel, _) | (FileFormat::Sequential, _) => {
                Format::Parallel
            }
        };
        // the file format wants a sysex's data followed by F7, and midly borrows the slice, so the terminated copies live here
        let sysex: Vec<Vec<Option<Vec<u8>>>> = self
            .tracks
            .iter()
            .map(|t| {
                t.events
                    .iter()
                    .map(|e| match &e.kind {
                        EventKind::Midi(MidiMessage::SysEx(d)) => {
                            Some(d.iter().copied().chain([0xF7]).collect())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let orders: Vec<Vec<usize>> = self
            .tracks
            .iter()
            .map(|t| {
                let mut order: Vec<usize> = (0..t.events.len()).collect();
                order.sort_by_key(|&i| t.events[i].tick); // stable
                order
            })
            .collect();
        let tracks: Vec<Vec<TrackEvent>> = self
            .tracks
            .iter()
            .zip(&sysex)
            .zip(&orders)
            .map(|((t, s), o)| to_track_events(t, s, o))
            .collect();
        let smf = RawSmf {
            header: Header::new(format, timing),
            tracks,
        };
        let mut out = Vec::new();
        smf.write_std(&mut out)
            .expect("writing to a Vec does not fail");
        out
    }
}

/// Whether the first bytes of a file are a Standard MIDI File (`MThd`) or one in a RIFF wrapper (`RMID`), whatever the file is called.
pub fn looks_like_midi(head: &[u8]) -> bool {
    head.starts_with(b"MThd")
        || (head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"RMID")
}

/// [`looks_like_midi`] for a file on disk: false if it cannot be read.
pub fn file_looks_like_midi(path: impl AsRef<std::path::Path>) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .is_ok_and(|n| looks_like_midi(&head[..n]))
}

/// A key signature as a tonic (a pitch class, C = 0) and a mode: `sharps` counts up the circle of fifths from C major (or from A minor), negative for flats.
pub fn key_signature_tonic(sharps: i8, minor: bool) -> (u8, bool) {
    let major_tonic = (sharps as i32 * 7).rem_euclid(12);
    (
        if minor {
            (major_tonic + 9) % 12
        } else {
            major_tonic
        } as u8,
        minor,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_signature_names_its_tonic() {
        assert_eq!(key_signature_tonic(0, false), (0, false), "C major");
        assert_eq!(key_signature_tonic(0, true), (9, true), "A minor");
        assert_eq!(
            key_signature_tonic(2, false),
            (2, false),
            "two sharps: D major"
        );
        assert_eq!(
            key_signature_tonic(-3, false),
            (3, false),
            "three flats: E flat major"
        );
        assert_eq!(
            key_signature_tonic(-3, true),
            (0, true),
            "three flats, minor: C minor"
        );
        assert_eq!(
            key_signature_tonic(7, false),
            (1, false),
            "seven sharps: C sharp major"
        );
    }

    #[test]
    fn a_midi_file_is_known_by_its_first_bytes() {
        assert!(looks_like_midi(b"MThd\0\0\0\x06"));
        assert!(looks_like_midi(b"RIFF\x10\0\0\0RMIDdata"));
        assert!(!looks_like_midi(b"RIFF\x10\0\0\0WAVEfmt "));
        assert!(!looks_like_midi(b"MT"));
        assert!(!file_looks_like_midi("/there/is/no/such/file.mid"));
    }
}
