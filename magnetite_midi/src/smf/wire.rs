//! Between this crate's events and `midly`'s, and the text a file carries.

use super::{EventKind, MidiMessage, Track};
use midly::num::{u24, u28};
use midly::{MetaMessage, TrackEvent, TrackEventKind};

fn text(bytes: &[u8]) -> String {
    decode_text(bytes)
}

/// Text of a meta event. Files carry whatever their authoring program used: valid UTF-8 is taken as it is; otherwise the bytes are read as Windows-1252, the
/// code page of most Western karaoke files (a Latin-1 superset), which never fails. Other code pages (Shift-JIS, GBK, EUC-KR) are not guessed and come out as Latin letters.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    // 0x80..=0x9F of Windows-1252 (the rest of 0x00..=0xFF is Latin-1); five bytes are undefined there and keep their control character
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| {
            if (0x80..0xA0).contains(&b) {
                HIGH[(b - 0x80) as usize]
            } else {
                b as char
            }
        })
        .collect()
}

pub(super) fn from_track_event(kind: &TrackEventKind) -> Option<EventKind> {
    Some(match kind {
        TrackEventKind::Midi { channel, message } => {
            EventKind::Midi(MidiMessage::from_voice(*channel, *message))
        }
        TrackEventKind::SysEx(data) => {
            let data = data.strip_suffix(&[0xF7]).unwrap_or(data);
            EventKind::Midi(MidiMessage::SysEx(data.to_vec()))
        }
        TrackEventKind::Escape(_) => return None,
        TrackEventKind::Meta(meta) => match meta {
            MetaMessage::EndOfTrack => return None,
            MetaMessage::Tempo(t) => EventKind::Tempo(t.as_int()),
            MetaMessage::TimeSignature(numerator, denominator_pow2, clocks, thirty_seconds) => {
                EventKind::TimeSignature {
                    numerator: *numerator,
                    denominator_pow2: *denominator_pow2,
                    clocks: *clocks,
                    thirty_seconds: *thirty_seconds,
                }
            }
            MetaMessage::KeySignature(sharps, minor) => EventKind::KeySignature {
                sharps: *sharps,
                minor: *minor,
            },
            MetaMessage::Marker(t) => EventKind::Marker(text(t)),
            MetaMessage::CuePoint(t) => EventKind::CuePoint(text(t)),
            MetaMessage::Text(t) => EventKind::Text(text(t)),
            MetaMessage::Lyric(t) => EventKind::Lyric(text(t)),
            MetaMessage::TrackName(t) => EventKind::TrackName(text(t)),
            MetaMessage::InstrumentName(t) => EventKind::InstrumentName(text(t)),
            MetaMessage::Copyright(t) => EventKind::Meta {
                kind: 0x02,
                data: t.to_vec(),
            },
            MetaMessage::ProgramName(t) => EventKind::Meta {
                kind: 0x08,
                data: t.to_vec(),
            },
            MetaMessage::DeviceName(t) => EventKind::Meta {
                kind: 0x09,
                data: t.to_vec(),
            },
            MetaMessage::MidiChannel(c) => EventKind::Meta {
                kind: 0x20,
                data: vec![c.as_int()],
            },
            MetaMessage::MidiPort(p) => EventKind::Meta {
                kind: 0x21,
                data: vec![p.as_int()],
            },
            MetaMessage::TrackNumber(n) => EventKind::Meta {
                kind: 0x00,
                data: n.map(|n| n.to_be_bytes().to_vec()).unwrap_or_default(),
            },
            MetaMessage::SmpteOffset(t) => EventKind::Meta {
                kind: 0x54,
                data: vec![t.hour(), t.minute(), t.second(), t.frame(), t.subframe()],
            },
            MetaMessage::SequencerSpecific(d) => EventKind::Meta {
                kind: 0x7F,
                data: d.to_vec(),
            },
            MetaMessage::Unknown(kind, d) => EventKind::Meta {
                kind: *kind,
                data: d.to_vec(),
            },
        },
    })
}

pub(super) fn to_track_events<'a>(
    track: &'a Track,
    sysex: &'a [Option<Vec<u8>>],
    order: &[usize],
) -> Vec<TrackEvent<'a>> {
    let mut out = Vec::with_capacity(order.len() + 1);
    let mut last = 0u64;
    let mut delta = |tick: u64| {
        let d = (tick - last).min(0x0FFF_FFFF) as u32;
        last = tick;
        u28::from_int_lossy(d)
    };
    for &i in order {
        let e = &track.events[i];
        let kind = match &e.kind {
            EventKind::Midi(m) => match m.to_voice() {
                Some((channel, message)) => TrackEventKind::Midi { channel, message },
                None => match &sysex[i] {
                    Some(data) => TrackEventKind::SysEx(data),
                    None => continue, // system messages other than sysex do not belong in a file
                },
            },
            EventKind::Tempo(t) => {
                TrackEventKind::Meta(MetaMessage::Tempo(u24::from_int_lossy(*t)))
            }
            EventKind::TimeSignature {
                numerator,
                denominator_pow2,
                clocks,
                thirty_seconds,
            } => TrackEventKind::Meta(MetaMessage::TimeSignature(
                *numerator,
                *denominator_pow2,
                *clocks,
                *thirty_seconds,
            )),
            EventKind::KeySignature { sharps, minor } => {
                TrackEventKind::Meta(MetaMessage::KeySignature(*sharps, *minor))
            }
            EventKind::Marker(t) => TrackEventKind::Meta(MetaMessage::Marker(t.as_bytes())),
            EventKind::CuePoint(t) => TrackEventKind::Meta(MetaMessage::CuePoint(t.as_bytes())),
            EventKind::Text(t) => TrackEventKind::Meta(MetaMessage::Text(t.as_bytes())),
            EventKind::Lyric(t) => TrackEventKind::Meta(MetaMessage::Lyric(t.as_bytes())),
            EventKind::TrackName(t) => TrackEventKind::Meta(MetaMessage::TrackName(t.as_bytes())),
            EventKind::InstrumentName(t) => {
                TrackEventKind::Meta(MetaMessage::InstrumentName(t.as_bytes()))
            }
            EventKind::Meta { kind, data } => {
                TrackEventKind::Meta(MetaMessage::Unknown(*kind, data))
            }
            EventKind::EndOfTrack => continue, // written once, at the end, whether or not the track has one
        };
        out.push(TrackEvent {
            delta: delta(e.tick),
            kind,
        });
    }
    let end = order.last().map_or(0, |&i| track.events[i].tick);
    out.push(TrackEvent {
        delta: delta(end),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}
