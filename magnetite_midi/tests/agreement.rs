//! `Smf` and `scan` read the same bytes and must say the same thing about them. They share their definitions (the tempo map and key signatures are gathered by one type, the notes are paired
//! by one type), so this is not what keeps them in step, it is what shows they are: on hundreds of generated files, and on every real file in a folder if `MAGNETITE_MIDI_CORPUS` names one.

use magnetite_midi::scan::{scan, ScanOptions};
use magnetite_midi::smf::{Division, Event, EventKind, FileFormat, Smf, Track};
use magnetite_midi::MidiMessage;

/// A small deterministic generator (xorshift): the same files on every run and every machine.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn voice(m: MidiMessage, tick: u64) -> Event {
    Event {
        tick,
        kind: EventKind::Midi(m),
    }
}

/// A file with the awkward things in it: notes of one key that overlap, note-ons of velocity 0 as note-offs, notes that never end, tempo and time-signature changes in any track and on
/// the same tick, key signatures.
fn random_file(rng: &mut Rng) -> Vec<u8> {
    let tracks = 1 + rng.below(4) as usize;
    let division = [96u16, 480][rng.below(2) as usize];
    let mut out = Vec::new();
    for _ in 0..tracks {
        let mut events = Vec::new();
        for _ in 0..(5 + rng.below(80)) {
            let tick = rng.below(40) * (division as u64 / 4);
            let (channel, key) = (rng.below(3) as u8, 60 + rng.below(4) as u8); // few keys: notes of one key overlap
            match rng.below(10) {
                0..=3 => events.push(voice(
                    MidiMessage::NoteOn {
                        channel,
                        key,
                        velocity: 1 + rng.below(127) as u8,
                    },
                    tick,
                )),
                4 | 5 => events.push(voice(
                    MidiMessage::NoteOff {
                        channel,
                        key,
                        velocity: 0,
                    },
                    tick,
                )),
                6 => events.push(voice(
                    MidiMessage::NoteOn {
                        channel,
                        key,
                        velocity: 0,
                    },
                    tick,
                )),
                7 => events.push(Event {
                    tick,
                    kind: EventKind::Tempo(300_000 + rng.below(500_000) as u32),
                }),
                8 => events.push(Event {
                    tick,
                    kind: EventKind::TimeSignature {
                        numerator: 2 + rng.below(6) as u8,
                        denominator_pow2: 2 + rng.below(2) as u8,
                        clocks: 24,
                        thirty_seconds: 8,
                    },
                }),
                _ => events.push(Event {
                    tick,
                    kind: EventKind::KeySignature {
                        sharps: rng.below(15) as i8 - 7,
                        minor: rng.below(2) == 0,
                    },
                }),
            }
        }
        events.sort_by_key(|e| e.tick);
        out.push(Track { events });
    }
    let format = if tracks == 1 {
        FileFormat::Single
    } else {
        FileFormat::Parallel
    };
    Smf {
        format,
        division: Division::Ticks(division),
        tracks: out,
    }
    .to_bytes()
}

fn agree(name: &str, bytes: &[u8]) {
    let song = Smf::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let scanned = scan(bytes, ScanOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(song.tempo_map(), scanned.tempo_map, "{name}: the tempo map");
    assert_eq!(
        song.key_signatures(),
        scanned.key_signatures,
        "{name}: the key signatures"
    );
    assert_eq!(song.end_tick(), scanned.end_tick, "{name}: the end");
    let map = song.tempo_map();
    let from_song: Vec<_> = song
        .notes()
        .iter()
        .map(|n| {
            (
                map.secs_at(n.start_tick) as f32,
                map.secs_at(n.end_tick) as f32,
                n.track as u16,
                n.channel,
                n.key,
                n.velocity,
            )
        })
        .collect();
    let from_scan: Vec<_> = scanned
        .notes
        .as_slice()
        .iter()
        .map(|n| (n.start, n.end, n.track, n.channel, n.key, n.velocity))
        .collect();
    assert_eq!(from_song, from_scan, "{name}: the notes");
}

#[test]
fn generated_files_are_read_alike() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..300 {
        agree(&format!("generated file {i}"), &random_file(&mut rng));
    }
}

/// Opt-in: every `.mid` file in the folder `MAGNETITE_MIDI_CORPUS` names that `Smf` reads (a file is skipped if its size would make an owned `Smf` slow).
#[test]
fn real_files_are_read_alike() {
    let Some(dir) = std::env::var_os("MAGNETITE_MIDI_CORPUS") else {
        eprintln!("skipped: set MAGNETITE_MIDI_CORPUS to a folder of .mid files");
        return;
    };
    let mut compared = 0;
    for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mid"))
        {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        if bytes.len() > 20 << 20 {
            continue;
        }
        agree(&path.display().to_string(), &bytes);
        compared += 1;
    }
    eprintln!("{compared} real file(s) read alike by Smf and scan");
}
