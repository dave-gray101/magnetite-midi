//! Standard MIDI Files: reading, writing, the tempo map, notes. Songs are built in code, and the damaged and unusual files are
//! built byte by byte, so the tests need no fixtures.

use magnetite_midi::smf::{Division, Event, EventKind, FileFormat, Smf, Track};
use magnetite_midi::MidiMessage;

fn ev(tick: u64, kind: EventKind) -> Event {
    Event { tick, kind }
}

fn note_on(tick: u64, key: u8) -> Event {
    ev(
        tick,
        EventKind::Midi(MidiMessage::NoteOn {
            channel: 0,
            key,
            velocity: 90,
        }),
    )
}

fn note_off(tick: u64, key: u8) -> Event {
    ev(
        tick,
        EventKind::Midi(MidiMessage::NoteOff {
            channel: 0,
            key,
            velocity: 0,
        }),
    )
}

fn sig(numerator: u8, denominator_pow2: u8) -> EventKind {
    EventKind::TimeSignature {
        numerator,
        denominator_pow2,
        clocks: 24,
        thirty_seconds: 8,
    }
}

fn single(division: Division, events: Vec<Event>) -> Smf {
    Smf {
        format: FileFormat::Single,
        division,
        tracks: vec![Track { events }],
    }
}

/// 3/4 at 120 BPM for four bars, then 4/4 at 150 BPM (400000 us per quarter).
fn song() -> Smf {
    let conductor = Track {
        events: vec![
            ev(0, EventKind::TrackName("conductor".into())),
            ev(0, EventKind::Tempo(500_000)),
            ev(0, sig(3, 2)),
            ev(
                0,
                EventKind::KeySignature {
                    sharps: -3,
                    minor: true,
                },
            ),
            ev(12 * 480, EventKind::Tempo(400_000)),
            ev(12 * 480, sig(4, 2)),
            ev(12 * 480, EventKind::Marker("chorus".into())),
        ],
    };
    let part = Track {
        events: vec![
            note_on(0, 60),
            note_off(480, 60),
            note_on(480, 64),
            note_off(2000, 64),
            note_on(3000, 67),
        ],
    };
    Smf {
        format: FileFormat::Parallel,
        division: Division::Ticks(480),
        tracks: vec![conductor, part],
    }
}

#[test]
fn a_song_survives_writing_and_reading() {
    let s = song();
    assert_eq!(Smf::parse(&s.to_bytes()).unwrap(), s);
}

#[test]
fn ticks_become_seconds_across_a_tempo_change() {
    let map = song().tempo_map();
    assert!(
        (map.secs_at(480) - 0.5).abs() < 1e-9,
        "one quarter at 120 BPM"
    );
    assert!(
        (map.secs_at(12 * 480) - 6.0).abs() < 1e-9,
        "twelve quarters"
    );
    assert!(
        (map.secs_at(12 * 480 + 480) - 6.4).abs() < 1e-9,
        "the next quarter is 0.4 s at 150 BPM"
    );
    assert!(
        (map.bpm_at(0).unwrap() - 120.0).abs() < 1e-9
            && (map.bpm_at(6000).unwrap() - 150.0).abs() < 1e-9
    );
    for tick in [0u64, 1, 479, 5760, 5761, 9999] {
        assert_eq!(
            map.tick_at(map.secs_at(tick)),
            tick,
            "tick {tick} survives the round trip through seconds"
        );
    }
    assert_eq!(map.tick_at(-1.0), 0);
}

#[test]
fn a_file_without_tempo_is_120_bpm_in_4_4() {
    let map = single(Division::Ticks(96), vec![note_on(0, 60), note_off(96, 60)]).tempo_map();
    assert!((map.secs_at(96) - 0.5).abs() < 1e-9);
    assert_eq!(map.signature_at(0), (4, 4));
}

#[test]
fn the_last_tempo_on_a_tick_wins() {
    let map = single(
        Division::Ticks(480),
        vec![
            ev(0, EventKind::Tempo(1_000_000)),
            ev(0, EventKind::Tempo(250_000)),
        ],
    )
    .tempo_map();
    assert!((map.bpm_at(0).unwrap() - 240.0).abs() < 1e-9);
    assert_eq!(map.tempo_changes().len(), 1);
}

#[test]
fn bars_and_beats_follow_the_time_signature() {
    let map = song().tempo_map();
    let bars = map.bars(12 * 480 + 2 * 1920);
    assert_eq!(bars.len(), 6);
    assert_eq!(
        (bars[0].numerator, bars[0].denominator, bars[0].start_tick),
        (3, 4, 0)
    );
    assert_eq!(bars[3].start_tick, 9 * 480);
    assert_eq!(
        (bars[4].numerator, bars[4].start_tick),
        (4, 12 * 480),
        "the fifth bar is the first of the 4/4 part"
    );
    assert_eq!(bars[5].start_tick, 12 * 480 + 1920);
    assert!((bars[4].start_secs - 6.0).abs() < 1e-9);
    let beats = map.beats(12 * 480 + 1920);
    assert_eq!(beats.len(), 12 + 4);
    assert_eq!(
        (beats[12].0, beats[12].2, beats[12].3),
        (12 * 480, 4, 0),
        "bar 4 beat 0 is the downbeat after the change"
    );
    assert!((beats[13].1 - 6.4).abs() < 1e-9);
}

#[test]
fn a_compound_signature_counts_beats_in_eighths() {
    let map = single(Division::Ticks(480), vec![ev(0, sig(6, 3))]).tempo_map();
    let bars = map.bars(2000);
    assert_eq!(
        (bars[0].numerator, bars[0].denominator, bars[0].beat_ticks),
        (6, 8, 240)
    );
    assert_eq!(bars[1].start_tick, 1440, "six eighths is three quarters");
}

#[test]
fn a_signature_off_the_bar_line_starts_a_new_bar() {
    let bars = single(
        Division::Ticks(480),
        vec![ev(0, sig(4, 2)), ev(2 * 480, sig(3, 2))],
    )
    .tempo_map()
    .bars(4000);
    assert_eq!(
        (bars[0].start_tick, bars[1].start_tick, bars[1].numerator),
        (0, 960, 3)
    );
}

#[test]
fn notes_are_paired_and_the_unfinished_end_with_their_track() {
    let notes = song().notes();
    assert_eq!(notes.len(), 3);
    assert_eq!(
        (notes[0].key, notes[0].start_tick, notes[0].end_tick),
        (60, 0, 480)
    );
    assert_eq!(
        (notes[1].key, notes[1].start_tick, notes[1].end_tick),
        (64, 480, 2000)
    );
    assert_eq!(
        (notes[2].key, notes[2].start_tick, notes[2].end_tick),
        (67, 3000, 3000),
        "never released: ends at its track's last event"
    );
}

#[test]
fn a_note_on_of_velocity_zero_ends_a_note_and_repeated_notes_pair_in_order() {
    let s = single(
        Division::Ticks(480),
        vec![
            note_on(0, 60),
            note_on(100, 60),
            ev(
                200,
                EventKind::Midi(MidiMessage::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 0,
                }),
            ),
            note_off(300, 60),
            note_off(400, 60), // no note left: ignored
        ],
    );
    assert_eq!(
        s.notes()
            .iter()
            .map(|n| (n.start_tick, n.end_tick))
            .collect::<Vec<_>>(),
        [(0, 200), (100, 300)]
    );
}

#[test]
fn key_signatures_names_and_length() {
    let s = song();
    assert_eq!(s.key_signatures(), [(0, -3, true)]);
    assert_eq!(s.tracks[0].name(), Some("conductor"));
    assert!(
        (s.length_secs() - 6.0).abs() < 1e-9,
        "the song ends at its last event of any track: the marker at tick 5760, 6 s in"
    );
}

#[test]
fn sysex_and_other_meta_events_survive() {
    let s = single(
        Division::Ticks(480),
        vec![
            ev(
                0,
                EventKind::Midi(MidiMessage::SysEx(vec![0x7E, 0x7F, 0x09, 0x01])),
            ),
            ev(
                10,
                EventKind::Meta {
                    kind: 0x7F,
                    data: vec![1, 2, 3],
                },
            ),
            ev(20, EventKind::Lyric("la".into())),
        ],
    );
    assert_eq!(Smf::parse(&s.to_bytes()).unwrap(), s);
}

// hand-built files: what the damaged and unusual look like

fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = tag.to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    v
}

fn header(format: u16, tracks: u16, division: u16) -> Vec<u8> {
    let mut body = Vec::new();
    for v in [format, tracks, division] {
        body.extend(v.to_be_bytes());
    }
    chunk(b"MThd", &body)
}

// note-on 60 and 62 (the second in running status), then both off, then end of track: 20 ticks long
const BODY: [u8; 18] = [
    0x00, 0x90, 60, 64, 10, 62, 64, 10, 0x80, 60, 0, 0, 62, 0, 0, 0xFF, 0x2F, 0x00,
];

#[test]
fn running_status_is_read() {
    let mut f = header(0, 1, 480);
    f.extend(chunk(b"MTrk", &BODY));
    let s = Smf::parse(&f).unwrap();
    assert_eq!(s.format, FileFormat::Single);
    assert_eq!(s.notes().len(), 2);
}

#[test]
fn a_file_without_end_of_track_or_cut_short_is_read() {
    let mut f = header(1, 1, 480);
    f.extend(chunk(b"MTrk", &BODY[..BODY.len() - 3]));
    assert_eq!(
        Smf::parse(&f).unwrap().notes().len(),
        2,
        "no end-of-track event"
    );
    let mut f = header(1, 1, 480);
    let mut t = chunk(b"MTrk", &BODY);
    t.truncate(t.len() - 5);
    f.extend(t);
    assert!(Smf::parse(&f).is_ok(), "chunk shorter than its header says");
}

#[test]
fn smpte_time_has_fixed_ticks_and_no_bars() {
    let mut f = header(0, 1, 0xE728); // -25 frames per second, 40 ticks per frame
    f.extend(chunk(b"MTrk", &BODY));
    let s = Smf::parse(&f).unwrap();
    assert_eq!(
        s.division,
        Division::Smpte {
            fps: 25.0,
            subframe: 40
        }
    );
    let map = s.tempo_map();
    assert!(
        (map.secs_at(1000) - 1.0).abs() < 1e-9,
        "1000 ticks = 25 frames * 40"
    );
    assert!(map.bars(5000).is_empty() && map.bpm_at(0).is_none() && map.quarters_at(0).is_none());
    assert_eq!(Smf::parse(&s.to_bytes()).unwrap().division, s.division);
}

#[test]
fn every_smpte_rate_survives_writing_and_reading() {
    for fps in [24.0, 25.0, 29.97, 30.0] {
        let s = single(
            Division::Smpte { fps, subframe: 80 },
            vec![note_on(0, 60), note_off(100, 60)],
        );
        let back = Smf::parse(&s.to_bytes()).unwrap();
        let Division::Smpte { fps: got, subframe } = back.division else {
            panic!("not SMPTE")
        };
        assert!(
            (got - fps).abs() < 0.01 && subframe == 80,
            "{fps} came back as {got}"
        );
    }
}

#[test]
fn format_two_sequences_are_put_end_to_end() {
    let mut f = header(2, 2, 480);
    f.extend(chunk(b"MTrk", &BODY));
    f.extend(chunk(b"MTrk", &BODY));
    let s = Smf::parse(&f).unwrap();
    assert_eq!(s.format, FileFormat::Sequential);
    let starts: Vec<u64> = s
        .notes()
        .iter()
        .filter(|n| n.key == 60)
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(
        starts,
        [0, 20],
        "the second sequence starts where the first ended (tick 20)"
    );
}

#[test]
fn an_rmid_wrapper_is_read_through() {
    let mut inner = header(0, 1, 480);
    inner.extend(chunk(b"MTrk", &BODY));
    let mut data = b"RMID".to_vec();
    data.extend(b"data");
    data.extend((inner.len() as u32).to_le_bytes());
    data.extend(&inner);
    let mut f = b"RIFF".to_vec();
    f.extend((data.len() as u32).to_le_bytes());
    f.extend(data);
    assert_eq!(Smf::parse(&f).unwrap().notes().len(), 2);
}

#[test]
fn what_is_not_a_midi_file_is_an_error_not_a_panic() {
    assert!(Smf::parse(b"RIFF....WAVEfmt ").is_err());
    assert!(Smf::parse(&[]).is_err());
    assert!(Smf::parse(&[0u8; 64]).is_err());
}

#[test]
fn the_silence_after_the_last_sound_is_kept_and_a_redundant_end_is_not() {
    let note = |end: Option<u64>| {
        let mut events = vec![
            ev(
                0,
                EventKind::Midi(MidiMessage::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 90,
                }),
            ),
            ev(
                480,
                // another key: the note on 60 is never ended
                EventKind::Midi(MidiMessage::NoteOff {
                    channel: 0,
                    key: 61,
                    velocity: 0,
                }),
            ),
        ];
        if let Some(tick) = end {
            events.push(ev(tick, EventKind::EndOfTrack));
        }
        Smf {
            format: FileFormat::Single,
            division: Division::Ticks(480),
            tracks: vec![Track { events }],
        }
    };
    // an end-of-track a bar after the last event: kept through the bytes, it sets the length and ends the note still sounding there
    let with_silence = note(Some(1920));
    let back = Smf::parse(&with_silence.to_bytes()).unwrap();
    assert_eq!(back, with_silence, "the end survives bytes and back");
    assert_eq!(back.end_tick(), 1920);
    assert!(
        (back.length_secs() - 2.0).abs() < 1e-9,
        "four beats at 120 BPM: {}",
        back.length_secs()
    );
    assert_eq!(
        back.notes()[0].end_tick,
        1920,
        "the note never ended: it ends with the track"
    );
    // an end-of-track on the last event is the writer's own: not kept, so a song read back is the song written
    let plain = note(None);
    let again = Smf::parse(&plain.to_bytes()).unwrap();
    assert_eq!(again, plain);
    assert!(!again.tracks[0]
        .events
        .iter()
        .any(|e| e.kind == EventKind::EndOfTrack));
}
