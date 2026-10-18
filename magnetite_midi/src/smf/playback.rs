//! Playing an [`Smf`] with the sequencer: [`MidiFile::from_smf`].
//!
//! [`MidiFile`] is a file *compiled for playback*: channel messages and tempo changes merged into one timeline in seconds, loop points resolved, everything else dropped. [`Smf`] is the file
//! as written, tolerant of what the sequencer's own loader refuses (a file in an RMID wrapper, damaged tracks, format 2, a missing end-of-track), so building a `MidiFile` from one lets the
//! sequencer, with its speed control and looping, play anything [`Smf`] can read.
//!
//! **It shares the loader's own code, not a copy of it.** The messages are made by the same `Message` constructors (so the loop controllers of each [`MidiFileLoopType`] mean what they mean there)
//! and the tracks are merged by the same `merge_tracks` (so the tempo arithmetic is one implementation). What is repeated is the insertion of a [`MidiFileLoopType::LoopPoint`]: a dozen lines
//! that the loader keeps inline. A test compares the two loaders on a set of files, and on a directory of real ones when `MAGNETITE_MIDI_CORPUS` names it.
//!
//! What differs, on purpose: sysex and system messages are dropped, as in the loader; a file in SMPTE time is refused, since the sequencer has no tempo to apply to it. A track ends where
//! the file said it ends (an [`Smf`] keeps an end-of-track that marks silence after the last sound), so the length is the loader's too.

use super::{Division, Error, EventKind, Smf};
use crate::midifile::Message;
use crate::{MidiFile, MidiFileLoopType};

impl MidiFile {
    /// Compile an [`Smf`] for the sequencer, with the default loop type (the loop starts at the beginning).
    pub fn from_smf(smf: &Smf) -> Result<MidiFile, Error> {
        MidiFile::from_smf_with_loop_type(smf, MidiFileLoopType::LoopPoint(0))
    }

    /// Compile an [`Smf`] for the sequencer with a loop convention (see [`MidiFileLoopType`]).
    pub fn from_smf_with_loop_type(
        smf: &Smf,
        loop_type: MidiFileLoopType,
    ) -> Result<MidiFile, Error> {
        let Division::Ticks(resolution) = smf.division else {
            return Err(Error(
                "a file in SMPTE time has no tempo to play it by".into(),
            ));
        };
        if smf.tracks.is_empty() {
            return Err(Error("the file has no tracks".into()));
        }
        let tick_of = |tick: u64| {
            i32::try_from(tick)
                .map_err(|_| Error(format!("tick {tick} does not fit the sequencer's 32 bits")))
        };
        let mut message_lists: Vec<Vec<Message>> = Vec::with_capacity(smf.tracks.len());
        let mut tick_lists: Vec<Vec<i32>> = Vec::with_capacity(smf.tracks.len());
        for track in &smf.tracks {
            let mut events: Vec<_> = track.events.iter().collect();
            events.sort_by_key(|e| e.tick); // stable: events of one tick stay in file order
            let (mut messages, mut ticks) = (Vec::new(), Vec::new());
            for e in events {
                let message = match &e.kind {
                    EventKind::Midi(m) => match m.synth_args() {
                        Some((channel, command, data1, data2)) => {
                            let status = (command | channel) as u8;
                            if command == 0xC0 || command == 0xD0 {
                                Message::common1(status, data1 as u8)
                            } else {
                                Message::common2(status, data1 as u8, data2 as u8, loop_type)
                            }
                        }
                        None => continue, // sysex and system messages: the loader discards them
                    },
                    EventKind::Tempo(micros) => Message::tempo_change(*micros as i32),
                    _ => continue,
                };
                messages.push(message);
                ticks.push(tick_of(e.tick)?);
            }
            messages.push(Message::EndOfTrack);
            ticks.push(tick_of(track.end_tick())?);
            message_lists.push(messages);
            tick_lists.push(ticks);
        }
        if let MidiFileLoopType::LoopPoint(loop_point) = loop_type {
            if loop_point == 0 {
                return Ok(MidiFile::merged(&message_lists, &tick_lists, resolution));
            }
            let loop_point = loop_point as i32;
            let (tick_list, message_list) = (&mut tick_lists[0], &mut message_lists[0]);
            if loop_point <= *tick_list.last().unwrap() {
                if let Some(i) = tick_list.iter().position(|t| *t >= loop_point) {
                    tick_list.insert(i, loop_point);
                    message_list.insert(i, Message::LoopStart);
                }
            } else {
                tick_list.push(loop_point);
                message_list.push(Message::LoopStart);
            }
        }
        Ok(MidiFile::merged(&message_lists, &tick_lists, resolution))
    }

    fn merged(
        message_lists: &[Vec<Message>],
        tick_lists: &[Vec<i32>],
        resolution: u16,
    ) -> MidiFile {
        let (messages, times) =
            MidiFile::merge_tracks(message_lists, tick_lists, resolution as i32);
        MidiFile { messages, times }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smf::{Event, FileFormat, Track};
    use crate::MidiMessage;
    use std::io::Cursor;

    fn ev(tick: u64, kind: EventKind) -> Event {
        Event { tick, kind }
    }
    fn on(tick: u64, channel: u8, key: u8) -> Event {
        ev(
            tick,
            EventKind::Midi(MidiMessage::NoteOn {
                channel,
                key,
                velocity: 90,
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
    fn cc(tick: u64, controller: u8) -> Event {
        ev(
            tick,
            EventKind::Midi(MidiMessage::ControlChange {
                channel: 0,
                controller,
                value: 1,
            }),
        )
    }
    fn file(format: FileFormat, tracks: Vec<Vec<Event>>) -> Vec<u8> {
        Smf {
            format,
            division: Division::Ticks(480),
            tracks: tracks.into_iter().map(|events| Track { events }).collect(),
        }
        .to_bytes()
    }

    /// Files that exercise what the loader does: several tracks with events on one tick, tempo changes, every kind of channel message, the loop controllers, sysex and text to be dropped.
    fn corpus() -> Vec<(&'static str, Vec<u8>)> {
        let notes = |ch: u8, from: u64| -> Vec<Event> {
            (0..16)
                .flat_map(|i| {
                    [
                        on(from + i * 240, ch, 60 + i as u8),
                        off(from + i * 240 + 200, ch, 60 + i as u8),
                    ]
                })
                .collect()
        };
        let mut all_kinds = vec![
            ev(
                0,
                EventKind::Midi(MidiMessage::ProgramChange {
                    channel: 1,
                    program: 24,
                }),
            ),
            ev(
                0,
                EventKind::Midi(MidiMessage::ChannelPressure {
                    channel: 1,
                    pressure: 40,
                }),
            ),
            ev(
                10,
                EventKind::Midi(MidiMessage::PolyPressure {
                    channel: 1,
                    key: 62,
                    pressure: 33,
                }),
            ),
            ev(
                20,
                EventKind::Midi(MidiMessage::PitchBend {
                    channel: 1,
                    value: -3000,
                }),
            ),
            ev(
                30,
                EventKind::Midi(MidiMessage::PitchBend {
                    channel: 1,
                    value: 8191,
                }),
            ),
            ev(
                40,
                EventKind::Midi(MidiMessage::SysEx(vec![0x7E, 0x7F, 0x09, 0x01])),
            ),
            ev(40, EventKind::Text("dropped".into())),
            ev(
                50,
                EventKind::Midi(MidiMessage::ControlChange {
                    channel: 1,
                    controller: 7,
                    value: 100,
                }),
            ),
        ];
        all_kinds.extend(notes(1, 100));
        let mut tempo_track = vec![
            ev(0, EventKind::Tempo(500_000)),
            ev(960, EventKind::Tempo(400_000)),
            ev(2400, EventKind::Tempo(750_000)),
        ];
        tempo_track.push(ev(
            4000,
            EventKind::Midi(MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 127,
            }),
        ));
        let loops = |controllers: &[(u64, u8)]| -> Vec<Event> {
            let mut v = notes(0, 0);
            v.extend(controllers.iter().map(|(t, c)| cc(*t, *c)));
            v
        };
        vec![
            ("one track", file(FileFormat::Single, vec![notes(0, 0)])),
            (
                "tempo track and two parts, events together",
                file(
                    FileFormat::Parallel,
                    vec![tempo_track.clone(), notes(0, 0), notes(1, 0)],
                ),
            ),
            (
                "every kind of channel message",
                file(FileFormat::Parallel, vec![tempo_track, all_kinds]),
            ),
            (
                "rpg maker loop",
                file(FileFormat::Single, vec![loops(&[(480, 111)])]),
            ),
            (
                "incredible machine loop",
                file(FileFormat::Single, vec![loops(&[(480, 110), (1920, 111)])]),
            ),
            (
                "final fantasy loop",
                file(FileFormat::Single, vec![loops(&[(480, 116), (1920, 117)])]),
            ),
        ]
    }

    /// The same two loaders, the same file: the same messages in the same order, at the same times to a nanosecond.
    fn same(name: &str, bytes: &[u8], loop_type: MidiFileLoopType) {
        let upstream = MidiFile::new_with_loop_type(&mut Cursor::new(bytes), loop_type)
            .unwrap_or_else(|e| panic!("{name}: the loader refused it: {e}"));
        let ours = MidiFile::from_smf_with_loop_type(&Smf::parse(bytes).unwrap(), loop_type)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            upstream.messages.len(),
            ours.messages.len(),
            "{name} {loop_type:?}: {} against {} messages",
            upstream.messages.len(),
            ours.messages.len()
        );
        for (i, (a, b)) in upstream.messages.iter().zip(&ours.messages).enumerate() {
            assert_eq!(
                format!("{a:?}"),
                format!("{b:?}"),
                "{name} {loop_type:?}: message {i}"
            );
            assert!(
                (upstream.times[i] - ours.times[i]).abs() < 1e-9,
                "{name} {loop_type:?}: message {i} at {} against {}",
                upstream.times[i],
                ours.times[i]
            );
        }
    }

    #[test]
    fn it_compiles_what_the_loader_does() {
        for (name, bytes) in corpus() {
            for loop_type in [
                MidiFileLoopType::LoopPoint(0),
                MidiFileLoopType::LoopPoint(960),
                MidiFileLoopType::LoopPoint(100_000),
                MidiFileLoopType::RpgMaker,
                MidiFileLoopType::IncredibleMachine,
                MidiFileLoopType::FinalFantasy,
            ] {
                same(name, &bytes, loop_type);
            }
        }
    }

    #[test]
    fn what_the_loader_refuses_it_plays() {
        // format 2 (the loader says unsupported format), and a file with no end-of-track event on its track (the loader reads past the end)
        let mut format2 = file(
            FileFormat::Single,
            vec![vec![on(0, 0, 60), off(480, 0, 60)]],
        );
        format2[9] = 2;
        assert!(MidiFile::new(&mut Cursor::new(&format2)).is_err());
        let song = Smf::parse(&format2).unwrap();
        let played = MidiFile::from_smf(&song).unwrap();
        assert!(
            (played.get_length() - 0.5).abs() < 1e-9,
            "{}",
            played.get_length()
        );
        // a file in SMPTE time has no tempo to play by
        let smpte = Smf {
            format: FileFormat::Single,
            division: Division::Smpte {
                fps: 25.0,
                subframe: 40,
            },
            tracks: vec![Track {
                events: vec![on(0, 0, 60)],
            }],
        };
        assert!(MidiFile::from_smf(&smpte).is_err());
    }

    /// Opt-in: every `.mid` file in the directory `MAGNETITE_MIDI_CORPUS` names, wherever the loader accepts it: the same messages at the same times, end-of-track included.
    #[test]
    fn real_files_agree_where_the_loader_accepts_them() {
        let Some(dir) = std::env::var_os("MAGNETITE_MIDI_CORPUS") else {
            eprintln!("skipped: set MAGNETITE_MIDI_CORPUS to a folder of .mid files to compare the loaders on them");
            return;
        };
        let (mut compared, mut refused) = (0, 0);
        for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mid"))
            {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let Ok(upstream) = MidiFile::new(&mut Cursor::new(&bytes)) else {
                refused += 1;
                continue;
            };
            let ours = MidiFile::from_smf(&Smf::parse(&bytes).unwrap()).unwrap();
            assert_eq!(
                upstream.messages.len(),
                ours.messages.len(),
                "{}: message counts",
                path.display()
            );
            for i in 0..upstream.messages.len() {
                assert_eq!(
                    format!("{:?}", upstream.messages[i]),
                    format!("{:?}", ours.messages[i]),
                    "{}: message {i}",
                    path.display()
                );
                assert!(
                    (upstream.times[i] - ours.times[i]).abs() < 1e-6,
                    "{}: message {i} at {} against {}",
                    path.display(),
                    upstream.times[i],
                    ours.times[i]
                );
            }
            assert!(
                (ours.get_length() - upstream.get_length()).abs() < 1e-6,
                "{}: length {} against {}",
                path.display(),
                ours.get_length(),
                upstream.get_length()
            );
            compared += 1;
        }
        eprintln!("{compared} real file(s) agree, {refused} refused by the loader");
    }
}
