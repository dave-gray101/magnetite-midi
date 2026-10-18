//! The streaming scan must agree with the owned song it replaces for notes, and hold up on damaged and odd files.

use magnetite_midi::scan::{scan, ScanOptions, TextKind};
use magnetite_midi::smf::{Division, Event, EventKind, FileFormat, Smf, Track};
use magnetite_midi::MidiMessage;

fn ev(tick: u64, kind: EventKind) -> Event {
    Event { tick, kind }
}
fn on(tick: u64, channel: u8, key: u8, velocity: u8) -> Event {
    ev(
        tick,
        EventKind::Midi(MidiMessage::NoteOn {
            channel,
            key,
            velocity,
        }),
    )
}
fn off(tick: u64, channel: u8, key: u8) -> Event {
    ev(
        tick,
        EventKind::Midi(MidiMessage::NoteOff {
            channel,
            key,
            velocity: 0,
        }),
    )
}
fn song(format: FileFormat, tracks: Vec<Vec<Event>>) -> Smf {
    Smf {
        format,
        division: Division::Ticks(480),
        tracks: tracks.into_iter().map(|events| Track { events }).collect(),
    }
}

/// What `Smf::notes` says, as the table's rows.
fn reference(song: &Smf) -> Vec<(f32, f32, u16, u8, u8, u8)> {
    let map = song.tempo_map();
    song.notes()
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
        .collect()
}

fn rows(bytes: &[u8]) -> Vec<(f32, f32, u16, u8, u8, u8)> {
    scan(bytes, ScanOptions::default())
        .unwrap()
        .notes
        .as_slice()
        .iter()
        .map(|n| (n.start, n.end, n.track, n.channel, n.key, n.velocity))
        .collect()
}

#[test]
fn notes_agree_with_the_owned_song() {
    let tempo = vec![
        ev(0, EventKind::Tempo(500_000)),
        ev(960, EventKind::Tempo(400_000)),
    ];
    let a = vec![
        on(0, 0, 60, 90),
        on(240, 0, 64, 80),
        off(480, 0, 60),
        off(960, 0, 64),
        on(1000, 0, 60, 70),
        on(1100, 0, 60, 71),
        off(1200, 0, 60),
        off(1300, 0, 60),
    ];
    let b = vec![
        on(10, 9, 36, 100),
        on(20, 9, 36, 0),
        on(30, 1, 40, 50), /* never ended */
        off(5000, 1, 99),  /* never started */
    ];
    let s = song(FileFormat::Parallel, vec![tempo, a, b]);
    let bytes = s.to_bytes();
    let s = Smf::parse(&bytes).unwrap();
    assert_eq!(rows(&bytes), reference(&s));
    assert!(rows(&bytes).len() >= 5);
}

#[test]
fn a_note_still_sounding_ends_with_its_track() {
    let s = song(
        FileFormat::Single,
        vec![vec![
            on(0, 0, 60, 90),
            ev(960, EventKind::Marker("end".into())),
        ]],
    );
    let r = scan(&s.to_bytes(), ScanOptions::default()).unwrap();
    assert_eq!(r.notes.len(), 1);
    assert_eq!(r.notes.as_slice()[0].end, 1.0); // 960 ticks at 480 per quarter and 120 BPM
}

#[test]
fn format_two_sequences_are_put_end_to_end() {
    let s = song(
        FileFormat::Sequential,
        vec![
            vec![on(0, 0, 60, 90), off(480, 0, 60)],
            vec![on(0, 0, 62, 90), off(480, 0, 62)],
        ],
    );
    let bytes = s.to_bytes();
    let r = scan(&bytes, ScanOptions::default()).unwrap();
    let starts: Vec<f32> = r.notes.as_slice().iter().map(|n| n.start).collect();
    // to_bytes writes format 2 as format 1 (the tracks played together), as its doc says: the two start together
    assert_eq!(starts, vec![0.0, 0.0]);
    assert_eq!(rows(&bytes), reference(&Smf::parse(&bytes).unwrap()));
}

#[test]
fn texts_come_in_order_of_time_with_their_kind() {
    let s = song(
        FileFormat::Parallel,
        vec![
            vec![
                ev(480, EventKind::Marker("Chorus".into())),
                ev(0, EventKind::Marker("Intro".into())),
            ],
            vec![
                ev(240, EventKind::Lyric("la".into())),
                ev(480, EventKind::CuePoint("cue".into())),
            ],
        ],
    );
    let r = scan(&s.to_bytes(), ScanOptions::default()).unwrap();
    let got: Vec<(f32, TextKind, &str)> = r
        .texts
        .iter()
        .map(|t| (t.time, t.kind, t.text.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (0.0, TextKind::Marker, "Intro"),
            (0.25, TextKind::Lyric, "la"),
            (0.5, TextKind::Marker, "Chorus"),
            (0.5, TextKind::CuePoint, "cue")
        ]
    );
}

#[test]
fn the_options_leave_out_what_was_not_asked_for() {
    let s = song(
        FileFormat::Single,
        vec![vec![
            ev(
                0,
                EventKind::KeySignature {
                    sharps: 2,
                    minor: true,
                },
            ),
            on(0, 0, 60, 90),
            ev(10, EventKind::Marker("m".into())),
            off(480, 0, 60),
        ]],
    );
    let bytes = s.to_bytes();
    let none = scan(
        &bytes,
        ScanOptions {
            notes: false,
            texts: false,
        },
    )
    .unwrap();
    assert!(none.notes.is_empty() && none.texts.is_empty());
    assert_eq!(none.key_signatures, vec![(0, 2, true)]);
    assert_eq!(none.end_tick, 480);
    assert_eq!(
        scan(
            &bytes,
            ScanOptions {
                notes: true,
                texts: false
            }
        )
        .unwrap()
        .notes
        .len(),
        1
    );
    assert_eq!(
        scan(
            &bytes,
            ScanOptions {
                notes: false,
                texts: true
            }
        )
        .unwrap()
        .texts
        .len(),
        1
    );
}

#[test]
fn windows_are_found_by_binary_search() {
    let events: Vec<Event> = (0..1000u64)
        .flat_map(|i| [on(i * 120, 0, 60, 80), off(i * 120 + 10, 0, 60)])
        .collect(); // eight notes a second (0.125 s apart: exact in f32)
    let r = scan(
        &song(FileFormat::Single, vec![events]).to_bytes(),
        ScanOptions::default(),
    )
    .unwrap();
    assert_eq!(r.notes.len(), 1000);
    assert_eq!(r.notes.starting_in(1.0, 2.0).len(), 8);
    assert_eq!(r.notes.starting_in(1.0, 1.0).len(), 0);
    assert_eq!(r.notes.starting_in(2.0, 1.0).len(), 0); // an inverted window is empty, not a panic
    assert_eq!(r.notes.starting_in(-5.0, 1e9).len(), 1000);
    assert_eq!(r.notes.densest(1.0), 8);
}

#[test]
fn a_cut_off_track_keeps_what_came_before_the_damage() {
    let s = song(
        FileFormat::Single,
        vec![vec![
            on(0, 0, 60, 90),
            off(480, 0, 60),
            on(960, 0, 62, 90),
            off(1440, 0, 62),
        ]],
    );
    let mut bytes = s.to_bytes();
    bytes.truncate(bytes.len() - 5); // into the last events
    let r = scan(&bytes, ScanOptions::default()).unwrap();
    assert!(!r.notes.is_empty(), "the notes before the cut survive");
}

#[test]
fn not_a_midi_file_is_an_error_that_says_so() {
    let e = scan(b"RIFFnope", ScanOptions::default()).unwrap_err();
    assert!(e.0.contains("not a MIDI file"), "{e}");
    assert!(scan(b"", ScanOptions::default()).is_err());
}

#[test]
fn an_empty_file_has_no_notes() {
    let r = scan(
        &song(FileFormat::Single, vec![vec![]]).to_bytes(),
        ScanOptions::default(),
    )
    .unwrap();
    assert!(r.notes.is_empty());
    assert_eq!(r.notes.densest(1.0), 0);
}

#[test]
fn channel_messages_come_in_order_of_time_with_ties_in_track_order_and_match_the_owned_song() {
    use magnetite_midi::scan::channel_messages;
    let a = vec![
        on(0, 0, 60, 90),
        on(480, 0, 62, 90),
        off(960, 0, 60),
        off(960, 0, 62),
    ];
    let b = vec![
        ev(0, EventKind::Tempo(500_000)),
        on(0, 1, 40, 80),
        on(480, 1, 41, 80),
        off(700, 1, 40),
        off(960, 1, 41),
    ];
    let bytes = song(FileFormat::Parallel, vec![a, b]).to_bytes();
    let owned = Smf::parse(&bytes).unwrap();
    let mut want: Vec<(u64, usize, MidiMessage)> = Vec::new();
    for (t, track) in owned.tracks.iter().enumerate() {
        for e in &track.events {
            if let EventKind::Midi(m) = &e.kind {
                want.push((e.tick, t, m.clone()));
            }
        }
    }
    want.sort_by_key(|(tick, track, _)| (*tick, *track)); // stable: a track's own order within a tick is kept
    let s = scan(
        &bytes,
        ScanOptions {
            notes: false,
            texts: false,
        },
    )
    .unwrap();
    let got: Vec<(u64, MidiMessage)> = channel_messages(&bytes, &s.track_offsets)
        .unwrap()
        .collect();
    assert_eq!(
        got,
        want.into_iter().map(|(t, _, m)| (t, m)).collect::<Vec<_>>()
    );
    assert_eq!(s.title, None);
}

#[test]
fn the_title_is_the_first_non_empty_track_name() {
    let s = song(
        FileFormat::Parallel,
        vec![
            vec![ev(0, EventKind::TrackName("  ".into()))],
            vec![ev(0, EventKind::TrackName("Demo".into()))],
        ],
    );
    assert_eq!(
        scan(&s.to_bytes(), ScanOptions::default())
            .unwrap()
            .title
            .as_deref(),
        Some("Demo")
    );
}
