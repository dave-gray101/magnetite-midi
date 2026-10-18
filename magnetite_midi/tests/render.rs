//! Playing a song through a SoundFont. Most tests use the tiny generated font (`tiny_soundfont`), so they need no real font; the tests
//! that use a real one skip, saying so, when `TimGM6mb.sf2` is not at the repository root.

use magnetite_midi::render::*;
use magnetite_midi::smf::{Division, Event, EventKind, FileFormat, Smf, Track};
use magnetite_midi::MidiMessage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SR: usize = 44_100;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("magnetite-midi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn font_file(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, tiny_soundfont()).unwrap();
    p
}

fn tiny() -> Arc<magnetite_midi::SoundFont> {
    Arc::new(
        magnetite_midi::SoundFont::new(&mut std::io::Cursor::new(tiny_soundfont()))
            .expect("the generated font is a valid SoundFont"),
    )
}

fn ev(tick: u64, kind: EventKind) -> Event {
    Event { tick, kind }
}

fn on(tick: u64, channel: u8, key: u8) -> Event {
    ev(
        tick,
        EventKind::Midi(MidiMessage::NoteOn {
            channel,
            key,
            velocity: 100,
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

/// 120 BPM, 480 ticks per quarter: a beat is 0.5 s, 960 ticks a second.
fn song(events: Vec<Event>) -> Smf {
    Smf {
        format: FileFormat::Single,
        division: Division::Ticks(480),
        tracks: vec![Track { events }],
    }
}

fn render_all(
    song: &Smf,
    font: &Arc<magnetite_midi::SoundFont>,
    opts: RenderOptions,
) -> (Vec<f32>, RenderSummary) {
    let mut pcm = Vec::new();
    let summary = render(&song.to_bytes(), font, &opts, &mut |chunk| {
        pcm.extend_from_slice(chunk);
        true
    })
    .unwrap();
    (pcm, summary)
}

fn dry() -> RenderOptions {
    RenderOptions {
        reverb_and_chorus: false,
        ..RenderOptions::default()
    }
}

/// The left channel.
fn left(pcm: &[f32]) -> Vec<f32> {
    pcm.chunks(2).map(|f| f[0]).collect()
}

/// The first frame that is clearly sounding.
fn onset(pcm: &[f32]) -> Option<usize> {
    left(pcm).iter().position(|s| s.abs() > 0.01)
}

/// Frequency from rising zero crossings over `from..to` seconds.
fn frequency(pcm: &[f32], from: f64, to: f64) -> f64 {
    let l = left(pcm);
    let (a, b) = ((from * SR as f64) as usize, (to * SR as f64) as usize);
    let crossings = l[a..b]
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f64 / (to - from)
}

// ------------------------------------------------------------------------------------------------------------ choosing a font

#[test]
fn a_font_is_loaded_once_for_every_source_that_uses_it() {
    let dir = temp("cache");
    let path = font_file(&dir, "a.sf2");
    let first = load_soundfont(&path).unwrap();
    let second = load_soundfont(&path).unwrap();
    assert!(Arc::ptr_eq(&first, &second), "one copy");
    let again = load_soundfont(&dir.join(".").join("a.sf2")).unwrap();
    assert!(Arc::ptr_eq(&first, &again), "however the path is spelled");
    drop((first, second, again));
    assert!(
        load_soundfont(&path).is_ok(),
        "freed when the last user lets go, and loadable again"
    );
}

#[test]
fn what_is_not_a_soundfont_is_an_error_that_names_the_file() {
    let dir = temp("bad");
    let junk = dir.join("junk.sf2");
    std::fs::write(&junk, b"this is not a soundfont").unwrap();
    let e = load_soundfont(&junk).unwrap_err().to_string();
    assert!(
        e.contains("junk.sf2") && e.contains("not a SoundFont"),
        "{e}"
    );
    assert!(load_soundfont(&dir.join("absent.sf2")).is_err());
}

#[test]
fn a_compressed_sf3_is_refused_with_a_clear_message() {
    let mut bytes = tiny_soundfont();
    let at = bytes.windows(4).position(|w| w == b"smpl").unwrap() + 8; // the first sample bytes
    bytes[at..at + 4].copy_from_slice(b"OggS");
    let dir = temp("sf3");
    let path = dir.join("compressed.sf2");
    std::fs::write(&path, bytes).unwrap();
    let e = load_soundfont(&path).unwrap_err().to_string();
    assert!(e.contains("SF3") && e.contains("only SF2"), "{e}");
}

// ------------------------------------------------------------------------------------------------------------- rendering

#[test]
fn a_note_sounds_at_its_pitch_and_a_key_an_octave_up_sounds_twice_as_high() {
    let font = tiny();
    let (pcm, _) = render_all(&song(vec![on(0, 0, 69), off(3840, 0, 69)]), &font, dry());
    let f = frequency(&pcm, 0.5, 1.5);
    assert!((f - 440.0).abs() < 2.0, "key 69 is {f} Hz");
    let (pcm, _) = render_all(&song(vec![on(0, 0, 81), off(3840, 0, 81)]), &font, dry());
    let f = frequency(&pcm, 0.5, 1.5);
    assert!((f - 880.0).abs() < 4.0, "key 81 is {f} Hz");
}

#[test]
fn a_note_starts_at_its_tick_within_the_synthesizers_block() {
    let font = tiny();
    // a note on the third beat of the second bar: tick 3840 = 4 s at 120 BPM
    let (pcm, _) = render_all(&song(vec![on(3840, 0, 69), off(4800, 0, 69)]), &font, dry());
    let start = onset(&pcm).expect("it sounds");
    let wanted = 4 * SR;
    assert!(
        start >= wanted && start < wanted + 2 * 64 + 30,
        "the note starts at frame {start}, its tick is frame {wanted}"
    );
}

#[test]
fn a_tempo_change_moves_later_notes() {
    let font = tiny();
    // 120 BPM for the first 960 ticks (1 s), then 60 BPM for the next 960 (2 s): the note at tick 1920 is 3 s in
    let s = song(vec![
        ev(0, EventKind::Tempo(500_000)),
        ev(960, EventKind::Tempo(1_000_000)),
        on(1920, 0, 69),
        off(2400, 0, 69),
    ]);
    let (pcm, _) = render_all(&s, &font, dry());
    let start = onset(&pcm).unwrap() as f64 / SR as f64;
    assert!(
        (start - 3.0).abs() < 0.01,
        "the note starts at {start} s, not at 2 s (where it would be without the slower tempo)"
    );
}

#[test]
fn the_render_is_the_song_and_a_tail() {
    let font = tiny();
    let s = song(vec![on(0, 0, 60), off(960, 0, 60)]); // 1 s of song
    let (pcm, summary) = render_all(
        &s,
        &font,
        RenderOptions {
            tail_secs: 2.0,
            ..dry()
        },
    );
    assert_eq!(summary.frames as usize * 2, pcm.len());
    assert!((summary.song_secs - 1.0).abs() < 1e-9);
    assert_eq!(
        summary.frames as usize,
        3 * SR,
        "one second of song, two of tail"
    );
    let tail_peak = left(&pcm)[(2.5 * SR as f64) as usize..]
        .iter()
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(
        tail_peak < 0.001,
        "the note has died away by the end of the tail ({tail_peak})"
    );
}

#[test]
fn rendering_twice_gives_the_same_samples() {
    let font = tiny();
    let s = song(vec![
        on(0, 0, 60),
        on(0, 0, 64),
        on(0, 0, 67),
        off(1920, 0, 60),
        off(1920, 0, 64),
        off(1920, 0, 67),
        on(2880, 1, 72),
        off(3840, 1, 72),
    ]);
    let a = render_all(&s, &font, RenderOptions::default()).0;
    let b = render_all(&s, &font, RenderOptions::default()).0;
    assert!(
        a == b,
        "the same song and font give identical audio (including the reverb)"
    );
    assert!(a.iter().any(|x| x.abs() > 0.01));
}

#[test]
fn the_chunk_size_does_not_change_the_audio() {
    let font = tiny();
    let s = song(vec![
        on(0, 0, 60),
        off(960, 0, 60),
        on(1000, 0, 64),
        off(2000, 0, 64),
    ]);
    let big = render_all(
        &s,
        &font,
        RenderOptions {
            chunk_frames: 4096,
            ..dry()
        },
    )
    .0;
    let small = render_all(
        &s,
        &font,
        RenderOptions {
            chunk_frames: 500,
            ..dry()
        },
    )
    .0;
    assert_eq!(big.len(), small.len());
    assert!(big == small);
}

#[test]
fn the_drum_channel_and_other_channels_sound_and_a_missing_program_does_not_stop_it() {
    let font = tiny();
    for (what, events) in [
        ("channel 10", vec![on(0, 9, 36), off(480, 9, 36)]),
        ("channel 16", vec![on(0, 15, 60), off(480, 15, 60)]),
        (
            "a program the font does not have",
            vec![
                ev(
                    0,
                    EventKind::Midi(MidiMessage::ProgramChange {
                        channel: 0,
                        program: 40,
                    }),
                ),
                on(0, 0, 60),
                off(480, 0, 60),
            ],
        ),
        (
            "a bank select",
            vec![
                ev(
                    0,
                    EventKind::Midi(MidiMessage::ControlChange {
                        channel: 2,
                        controller: 0,
                        value: 3,
                    }),
                ),
                on(0, 2, 60),
                off(480, 2, 60),
            ],
        ),
    ] {
        let (pcm, _) = render_all(&song(events), &font, dry());
        assert!(onset(&pcm).is_some(), "{what} is silent");
    }
}

#[test]
fn pitch_bend_and_volume_change_what_is_heard() {
    let font = tiny();
    let plain = render_all(&song(vec![on(0, 0, 69), off(3840, 0, 69)]), &font, dry()).0;
    let bent = render_all(
        &song(vec![
            ev(
                0,
                EventKind::Midi(MidiMessage::PitchBend {
                    channel: 0,
                    value: 4096,
                }),
            ),
            on(0, 0, 69),
            off(3840, 0, 69),
        ]),
        &font,
        dry(),
    )
    .0;
    let (f0, f1) = (frequency(&plain, 0.5, 1.5), frequency(&bent, 0.5, 1.5));
    assert!(
        f1 > f0 * 1.05,
        "half the bend range up (a default of two semitones) is a semitone up: {f0} then {f1}"
    );
    let quiet = render_all(
        &song(vec![
            ev(
                0,
                EventKind::Midi(MidiMessage::ControlChange {
                    channel: 0,
                    controller: 7,
                    value: 20,
                }),
            ),
            on(0, 0, 69),
            off(3840, 0, 69),
        ]),
        &font,
        dry(),
    )
    .0;
    let peak = |p: &[f32]| left(p).iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(
        peak(&quiet) < peak(&plain) * 0.5,
        "channel volume 20 is much quieter than the default"
    );
}

#[test]
fn an_empty_song_is_only_silence_and_a_stop_ends_the_render() {
    let font = tiny();
    let (pcm, summary) = render_all(&song(vec![]), &font, dry());
    assert!(pcm.iter().all(|x| *x == 0.0));
    assert_eq!(summary.song_secs, 0.0);

    let s = song(vec![on(0, 0, 60), off(96_000, 0, 60)]); // 100 s of song
    let mut chunks = 0;
    let result = render(&s.to_bytes(), &font, &dry(), &mut |_| {
        chunks += 1;
        chunks < 3
    });
    assert!(
        result.is_err() && chunks == 3,
        "the sink said stop after {chunks} chunks"
    );
}

// -------------------------------------------------------------------------------------------------------------- a real font

#[test]
fn a_real_font_plays_a_chord_progression() {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "TimGM6mb.sf2"]
        .iter()
        .collect();
    if !path.exists() {
        eprintln!(
            "skipped: {} is not there (the repository has one at its root)",
            path.display()
        );
        return;
    }
    let font = load_soundfont(&path).unwrap();
    // C, Am, F, G in whole bars at 120 BPM on a piano (program 0), and a kick on the beats
    let mut events = vec![];
    for (bar, chord) in [[60u8, 64, 67], [57, 60, 64], [53, 57, 60], [55, 59, 62]]
        .iter()
        .enumerate()
    {
        for k in chord {
            events.push(on(bar as u64 * 1920, 0, *k));
            events.push(off(bar as u64 * 1920 + 1800, 0, *k));
        }
    }
    for beat in 0..16u64 {
        events.push(on(beat * 480, 9, 36));
        events.push(off(beat * 480 + 120, 9, 36));
    }
    events.sort_by_key(|e| e.tick);
    let started = std::time::Instant::now();
    let (pcm, summary) = render_all(&song(events), &font, RenderOptions::default());
    let took = started.elapsed().as_secs_f64();
    let secs = summary.frames as f64 / SR as f64;
    eprintln!(
        "rendered {secs:.1} s in {took:.3} s ({:.0}x real time) with {}",
        secs / took,
        path.display()
    );
    assert!(secs > 8.0 && onset(&pcm).is_some());
    assert!(
        left(&pcm).iter().all(|s| s.abs() <= 1.0),
        "no clipping beyond full scale"
    );
}

#[test]
fn the_voice_limit_bounds_how_loud_a_wall_of_notes_gets() {
    let font = tiny();
    // 120 notes at once for a second
    let mut events = Vec::new();
    for k in 0..120u8 {
        events.push(on(0, 0, 24 + k % 100));
    }
    for k in 0..120u8 {
        events.push(off(960, 0, 24 + k % 100));
    }
    let energy = |max_polyphony| {
        let (pcm, _) = render_all(
            &song(events.clone()),
            &font,
            RenderOptions {
                max_polyphony,
                ..dry()
            },
        );
        pcm.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
    };
    let (few, many) = (energy(8), energy(256));
    assert!(
        few < many * 0.5,
        "8 voices sound a good deal quieter than 120: {few} against {many}"
    );
}

#[test]
fn the_render_adds_synthesizers_as_the_notes_need_them() {
    let font = tiny();
    let chord = |count: usize| {
        let mut events = Vec::new();
        for k in 0..count {
            events.push(on(0, (k / 100) as u8, 24 + (k % 100) as u8));
        }
        for k in 0..count {
            events.push(off(960, (k / 100) as u8, 24 + (k % 100) as u8));
        }
        song(events)
    };
    let opts = RenderOptions {
        synthesizers: 4,
        ..dry()
    };
    let (_, few) = render_all(&chord(20), &font, opts);
    assert_eq!(
        (few.synthesizers, few.peak_demand),
        (1, 20),
        "twenty notes need one synthesizer"
    );
    let (_, many) = render_all(&chord(600), &font, opts);
    assert_eq!(
        (many.synthesizers, many.peak_demand),
        (4, 600),
        "600 notes use all four that are allowed"
    );
    let (_, capped) = render_all(
        &chord(600),
        &font,
        RenderOptions {
            synthesizers: 2,
            ..dry()
        },
    );
    assert_eq!(
        capped.synthesizers, 2,
        "the synthesizers allowed bound the threads"
    );
    // the threads' timing does not change the sound
    let again = render_all(&chord(600), &font, opts).0;
    assert_eq!(
        render_all(&chord(600), &font, opts).0,
        again,
        "two renders of the same file are the same samples"
    );
}

#[test]
fn no_note_is_stolen_while_the_voices_allowed_are_not_used_up() {
    let font = tiny();
    // 600 notes at once for a second, 20 voices allowed for each synthesizer (one: a limit this small is not shared out): they steal; unlimited, all sound
    let mut events = Vec::new();
    for k in 0..600usize {
        events.push(on(0, (k / 100) as u8, 24 + (k % 100) as u8));
        events.push(off(960, (k / 100) as u8, 24 + (k % 100) as u8));
    }
    let s = song(events);
    // a pure measure of how many notes sound: the unlimited render is the loudest the limiter allows, the starved one is quieter before it
    let rms = |pcm: &[f32]| {
        (pcm.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt()
    };
    let starved = rms(&render_all(
        &s,
        &font,
        RenderOptions {
            max_polyphony: 12,
            synthesizers: 1,
            ..dry()
        },
    )
    .0);
    let full = rms(&render_all(
        &s,
        &font,
        RenderOptions {
            synthesizers: 4,
            ..dry()
        },
    )
    .0);
    assert!(
        starved < full,
        "12 voices sound less than 600 ({starved} against {full})"
    );
}

#[test]
fn notes_end_on_the_synthesizer_that_started_them() {
    let font = tiny();
    // a note, off at one second: with several synthesizers its off must reach the one that holds it, so the tail is silent
    let s = song(vec![
        on(0, 0, 60),
        on(0, 0, 64),
        on(0, 0, 67),
        off(960, 0, 60),
        off(960, 0, 64),
        off(960, 0, 67),
    ]);
    let (pcm, _) = render_all(
        &s,
        &font,
        RenderOptions {
            synthesizers: 4,
            tail_secs: 3.0,
            ..dry()
        },
    );
    let tail = left(&pcm)[(2.5 * SR as f64) as usize..]
        .iter()
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(
        tail < 1e-3,
        "all three notes ended: the tail's peak is {tail}"
    );
}

#[test]
fn a_stop_ends_a_render_that_runs_several_synthesizers() {
    let font = tiny();
    let s = song(
        (0..200u8)
            .flat_map(|k| {
                [
                    on(k as u64 * 480, 0, 40 + k % 60),
                    off(k as u64 * 480 + 400, 0, 40 + k % 60),
                ]
            })
            .collect(),
    );
    let bytes = s.to_bytes();
    let mut got = 0usize;
    let r = render(
        &bytes,
        &font,
        &RenderOptions {
            synthesizers: 4,
            ..dry()
        },
        &mut |c| {
            got += c.len();
            got < 100_000
        },
    );
    assert!(r.is_err(), "a render the sink stopped is an error");
}

#[test]
fn a_render_can_start_in_the_middle_of_the_song() {
    let font = tiny();
    // a note held from 0 to 1.5 s, a second from 2 s to 2.5 s
    let s = song(vec![
        on(0, 0, 69),
        off(1440, 0, 69),
        on(1920, 0, 72),
        off(2400, 0, 72),
    ]);
    let (full, full_summary) = render_all(&s, &font, dry());
    let start = SR as u64; // 1 s: inside the first note
    let (part, part_summary) = render_all(
        &s,
        &font,
        RenderOptions {
            start_frame: start,
            ..dry()
        },
    );
    assert_eq!(
        part_summary.frames + start,
        full_summary.frames,
        "it ends where the full render ends"
    );
    assert_eq!(part.len() as u64 + start * 2, full.len() as u64);
    // the note begun before the start is not sounded, so nothing is heard until the second note, at the same time as in the full render
    let heard = onset(&part).expect("the second note sounds") + SR;
    assert!(
        (heard as i64 - 2 * SR as i64).abs() < 256,
        "the second note sounds at frame {heard}, the song has it at {}",
        2 * SR
    );
    // and from there on it is as loud as in the full render (the same notes; the phase of the wave may differ by the odd frame)
    let (a, b) = (left(&full), left(&part));
    let energy = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let (full_rms, part_rms) = (
        energy(&a[2 * SR + 200..2 * SR + 8000]),
        energy(&b[SR + 200..SR + 8000]),
    );
    assert!(
        full_rms > 0.01 && (full_rms - part_rms).abs() < 0.05 * full_rms,
        "rms {part_rms} against {full_rms}"
    );
}
