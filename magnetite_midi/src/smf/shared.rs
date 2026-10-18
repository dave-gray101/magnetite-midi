//! What `Smf` and [`scan`](crate::scan) share, so that two ways of reading the same bytes cannot disagree: what a tick means, how the tempo map and the key signatures are gathered,
//! and what a note is.

use super::{Division, Event, EventKind, MidiMessage, TempoMap};
use midly::Timing;

impl Division {
    /// What a tick means, from the header of a file as `midly` reads it.
    pub(crate) fn from_timing(timing: Timing) -> Division {
        match timing {
            Timing::Metrical(t) => Division::Ticks(t.as_int()),
            Timing::Timecode(fps, subframe) => Division::Smpte {
                fps: fps.as_f32() as f64,
                subframe,
            },
        }
    }
}

/// The events a tempo map and the key signatures are made from, gathered from any number of tracks: the one place that decides how they are ordered and combined, used by
/// [`Smf`] (which has every event) and by [`scan`](crate::scan) (which streams past them), so the two cannot disagree about the tempo map of a file.
#[derive(Default)]
pub(crate) struct MetaEvents {
    timing: Vec<Event>,
    keys: Vec<(u64, i8, bool)>,
}

impl MetaEvents {
    pub(crate) fn push_tempo(&mut self, tick: u64, micros_per_quarter: u32) {
        self.timing.push(Event {
            tick,
            kind: EventKind::Tempo(micros_per_quarter),
        });
    }

    pub(crate) fn push_time_signature(
        &mut self,
        tick: u64,
        numerator: u8,
        denominator_pow2: u8,
        clocks: u8,
        thirty_seconds: u8,
    ) {
        self.timing.push(Event {
            tick,
            kind: EventKind::TimeSignature {
                numerator,
                denominator_pow2,
                clocks,
                thirty_seconds,
            },
        });
    }

    pub(crate) fn push_key_signature(&mut self, tick: u64, sharps: i8, minor: bool) {
        self.keys.push((tick, sharps, minor));
    }

    /// Take what concerns the tempo map or the key from an event of an [`Smf`], and nothing else.
    pub(crate) fn push_event(&mut self, event: &Event) {
        match &event.kind {
            EventKind::Tempo(_) | EventKind::TimeSignature { .. } => {
                self.timing.push(event.clone())
            }
            EventKind::KeySignature { sharps, minor } => {
                self.keys.push((event.tick, *sharps, *minor))
            }
            _ => {}
        }
    }

    /// The tempo map. Stable: on one tick, the event of the earlier track (or the earlier in the stream) comes first.
    pub(crate) fn tempo_map(&self, division: Division) -> TempoMap {
        let mut all: Vec<&Event> = self.timing.iter().collect();
        all.sort_by_key(|e| e.tick);
        TempoMap::build(division, &all)
    }

    /// The key signatures in order of tick: `(tick, sharps, minor)`.
    pub(crate) fn key_signatures(&self) -> Vec<(u64, i8, bool)> {
        let mut keys = self.keys.clone();
        keys.sort_by_key(|k| k.0);
        keys
    }
}

/// A note-on and the note-off that ended it, in ticks.
pub(crate) struct PairedNote {
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
    pub start_tick: u64,
    pub end_tick: u64,
}

/// Pairs the note-ons of one track with their note-offs: the one definition of what a note is, used by [`Smf::notes`] and by [`scan`](crate::scan). A note-on is ended by the next
/// note-off (or note-on with velocity 0) of the same key and channel; repeated note-ons of one key end in the order they began; a note still sounding when the track ends is
/// ended by [`finish`](Self::finish) at the tick the caller says. The sustain pedal is not applied.
pub(crate) struct NotePairer {
    /// `(channel, key)` to the notes started and not yet ended, oldest first: 2048 queues that allocate only when used.
    open: Vec<std::collections::VecDeque<(u64, u8)>>,
    live: usize,
}

impl NotePairer {
    pub(crate) fn new() -> Self {
        NotePairer {
            open: (0..2048)
                .map(|_| std::collections::VecDeque::new())
                .collect(),
            live: 0,
        }
    }

    fn slot(channel: u8, key: u8) -> usize {
        (channel as usize & 15) * 128 + (key as usize & 127)
    }

    /// Show the pairer a message of the track at `tick`: the note that it ends, if it ends one.
    pub(crate) fn observe(&mut self, tick: u64, message: &MidiMessage) -> Option<PairedNote> {
        let (channel, key) = match *message {
            MidiMessage::NoteOn {
                channel,
                key,
                velocity,
            } if velocity > 0 => {
                self.open[Self::slot(channel, key)].push_back((tick, velocity));
                self.live += 1;
                return None;
            }
            MidiMessage::NoteOn { channel, key, .. }
            | MidiMessage::NoteOff { channel, key, .. } => (channel, key),
            _ => return None,
        };
        let (start_tick, velocity) = self.open[Self::slot(channel, key)].pop_front()?;
        self.live -= 1;
        Some(PairedNote {
            channel,
            key,
            velocity,
            start_tick,
            end_tick: tick,
        })
    }

    /// The notes still sounding, ended at `end_tick` (the end of the track), by channel and key and oldest first.
    pub(crate) fn finish(self, end_tick: u64) -> impl Iterator<Item = PairedNote> {
        let live = self.live;
        self.open
            .into_iter()
            .enumerate()
            .filter(move |_| live > 0)
            .flat_map(move |(slot, queue)| {
                queue
                    .into_iter()
                    .map(move |(start_tick, velocity)| PairedNote {
                        channel: (slot / 128) as u8,
                        key: (slot % 128) as u8,
                        velocity,
                        start_tick,
                        end_tick,
                    })
            })
    }
}
