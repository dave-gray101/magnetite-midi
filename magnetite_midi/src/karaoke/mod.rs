//! Karaoke (`.kar`) lyrics: the words of a MIDI file with the time of every syllable.
//!
//! A `.kar` is a Standard MIDI File whose lyric track (usually named `Words`) holds one text event per syllable. The convention (Soft Karaoke / "MIDI karaoke file"):
//!
//! * Header lines are text events that start with `@`: `@KMIDI KARAOKE FILE` (the format tag), `@V0100` (version), `@LENGL` (language), `@I...` (information) and
//!   `@T...` (the first is the title, the second the artist, later ones are credits).
//! * Every other text event is a syllable. A leading `/` starts a new line and a leading `\` starts a new paragraph (a verse); a few files end a syllable with a line break
//!   instead. A syllable that ends in a space ends a word; one that does not goes on into the next.
//! * Some files use the Lyric meta event (0x05) for the syllables instead of Text (0x01), with the same line marks and no headers. Both are read.
//!
//! [`parse`] takes what [`crate::scan::scan`] found and does not look at the file again; [`read`] does both for the bytes of a file.

use crate::scan::{scan, ScanOptions, TextKind, TimedText};
use crate::smf::Error;
use std::collections::HashMap;

/// A syllable and when it starts, in seconds of the file's own timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Syllable {
    pub time: f32,
    /// As sung: never starts with a space, and ends with one when the word ends here.
    pub text: String,
}

/// A line of the song.
#[derive(Clone, Debug, PartialEq)]
pub struct KaraokeLine {
    /// The start of the first syllable.
    pub time: f32,
    /// The first line of a verse: after a `\`, after a long pause, or the first of the song.
    pub stanza_start: bool,
    pub syllables: Vec<Syllable>,
}

impl KaraokeLine {
    /// The words of the line as one string.
    pub fn text(&self) -> String {
        let joined: String = self.syllables.iter().map(|s| s.text.as_str()).collect();
        joined.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// The lyrics of a karaoke file, with what its headers say about the song.
#[derive(Clone, Debug, PartialEq)]
pub struct Karaoke {
    /// The first `@T` line.
    pub title: Option<String>,
    /// The second `@T` line.
    pub artist: Option<String>,
    /// The `@L` line: `ENGL`, `FRAN`, `JAPA`...
    pub language: Option<String>,
    /// The `@I` lines and any `@T` line after the second.
    pub info: Vec<String>,
    pub lines: Vec<KaraokeLine>,
}

/// How lines are told apart in a file that does not mark them, and what makes a verse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KaraokeOptions {
    /// In a file with no line marks, the singing pausing for longer than this many seconds starts a new line.
    pub line_gap_secs: f32,
    /// A line that starts more than this many seconds after the one before it starts a verse.
    pub stanza_gap_secs: f32,
}

impl Default for KaraokeOptions {
    fn default() -> Self {
        Self {
            line_gap_secs: 1.5,
            stanza_gap_secs: 8.0,
        }
    }
}

/// Text events needed to take a file with no `@` header for a karaoke file (a plain MIDI file has a few text events for other things).
const MIN_UNMARKED_SYLLABLES: usize = 8;

/// The lyrics in the text a scan found, or `None` when it holds none.
pub fn parse(texts: &[TimedText], options: &KaraokeOptions) -> Option<Karaoke> {
    let mut title = None;
    let mut artist = None;
    let mut language = None;
    let mut info = Vec::new();
    let mut titles = 0;
    let mut headers = 0;
    for t in texts.iter().filter(|t| t.kind == TextKind::Text) {
        let Some(rest) = t.text.trim_start().strip_prefix('@') else {
            continue;
        };
        headers += 1;
        let Some(tag) = rest.chars().next() else {
            continue;
        };
        let value = rest[tag.len_utf8()..].trim().to_owned();
        match tag.to_ascii_uppercase() {
            'T' => {
                titles += 1;
                match titles {
                    1 => title = Some(value).filter(|v| !v.is_empty()),
                    2 => artist = Some(value).filter(|v| !v.is_empty()),
                    _ if !value.is_empty() => info.push(value),
                    _ => {}
                }
            }
            'L' => language = Some(value).filter(|v| !v.is_empty()),
            'I' if !value.is_empty() => info.push(value),
            _ => {}
        }
    }

    // the syllables: the Lyric events if there are any, else the text events of the track that has the most of them
    let lyric_events: Vec<&TimedText> =
        texts.iter().filter(|t| t.kind == TextKind::Lyric).collect();
    let syllables: Vec<&TimedText> = if !lyric_events.is_empty() {
        lyric_events
    } else {
        let mut per_track: HashMap<u16, usize> = HashMap::new();
        for t in texts
            .iter()
            .filter(|t| t.kind == TextKind::Text && !t.text.trim_start().starts_with('@'))
        {
            *per_track.entry(t.track).or_default() += 1;
        }
        let best = per_track
            .iter()
            .max_by_key(|(track, n)| (**n, std::cmp::Reverse(**track)));
        match best {
            Some((&track, &n)) if headers > 0 || n >= MIN_UNMARKED_SYLLABLES => texts
                .iter()
                .filter(|t| {
                    t.kind == TextKind::Text
                        && t.track == track
                        && !t.text.trim_start().starts_with('@')
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    if syllables.is_empty() {
        return None;
    }

    let marked = syllables
        .iter()
        .any(|s| s.text.starts_with(['/', '\\']) || s.text.ends_with(['\r', '\n']));
    let mut lines: Vec<KaraokeLine> = Vec::new();
    let mut current: Vec<Syllable> = Vec::new();
    let mut new_stanza = true;
    let mut break_next = false;
    let mut last = f32::MIN;
    let flush = |current: &mut Vec<Syllable>,
                 lines: &mut Vec<KaraokeLine>,
                 stanza: bool,
                 options: &KaraokeOptions| {
        if current.iter().all(|s| s.text.trim().is_empty()) {
            current.clear();
            return;
        }
        let time = current[0].time;
        let gap = lines
            .last()
            .is_none_or(|prev| time - prev.time > options.stanza_gap_secs);
        lines.push(KaraokeLine {
            time,
            stanza_start: stanza || gap,
            syllables: std::mem::take(current),
        });
    };
    for s in syllables {
        let mut text = s.text.as_str();
        let paragraph = text.starts_with('\\');
        let line = paragraph || text.starts_with('/');
        text = text.trim_start_matches(['/', '\\']);
        let ends_line = text.ends_with(['\r', '\n']);
        text = text.trim_end_matches(['\r', '\n']);
        if line || break_next || (!marked && s.time - last > options.line_gap_secs) {
            flush(&mut current, &mut lines, new_stanza, options);
            new_stanza = paragraph;
        }
        break_next = ends_line;
        last = s.time;
        if text.is_empty() {
            continue;
        }
        // a syllable that starts with a space ends the word before it
        if text.starts_with(char::is_whitespace) {
            if let Some(prev) = current.last_mut() {
                if !prev.text.ends_with(char::is_whitespace) {
                    prev.text.push(' ');
                }
            }
        }
        let text = text.trim_start();
        if !text.is_empty() {
            current.push(Syllable {
                time: s.time,
                text: text.to_owned(),
            });
        }
    }
    flush(&mut current, &mut lines, new_stanza, options);
    if lines.is_empty() {
        return None;
    }
    Some(Karaoke {
        title,
        artist,
        language,
        info,
        lines,
    })
}

/// A karaoke file read: its lyrics, and how long its music lasts (to tell whether it is the arrangement of a given recording).
#[derive(Clone, Debug, PartialEq)]
pub struct KaraokeFile {
    pub karaoke: Karaoke,
    /// Seconds from the start to the last event, on the file's own tempo map.
    pub duration: f32,
}

/// Read the lyrics and the length of the bytes of a MIDI or karaoke file, with the default [`KaraokeOptions`]. An error if the bytes are not a MIDI file or have no lyrics.
pub fn read(bytes: &[u8]) -> Result<KaraokeFile, Error> {
    let s = scan(
        bytes,
        ScanOptions {
            notes: false,
            texts: true,
        },
    )?;
    let karaoke = parse(&s.texts, &KaraokeOptions::default())
        .ok_or_else(|| Error("the file has no karaoke lyrics".into()))?;
    Ok(KaraokeFile {
        karaoke,
        duration: s.tempo_map.secs_at(s.end_tick) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(time: f32, kind: TextKind, text: &str, track: u16) -> TimedText {
        TimedText {
            time,
            kind,
            text: text.into(),
            track,
        }
    }
    fn words(k: &Karaoke) -> Vec<(f32, String, bool)> {
        k.lines
            .iter()
            .map(|l| (l.time, l.text(), l.stanza_start))
            .collect()
    }

    #[test]
    fn a_kar_file_gives_its_headers_and_its_syllables() {
        let texts = vec![
            t(0.0, TextKind::Text, "Created by someone", 0), // another track's text is not the lyric
            t(0.0, TextKind::Text, "@KMIDI KARAOKE FILE", 1),
            t(0.0, TextKind::Text, "@LENGL", 1),
            t(0.0, TextKind::Text, "@TYesterday", 1),
            t(0.0, TextKind::Text, "@TThe Beatles", 1),
            t(0.0, TextKind::Text, "@IArranged by A. Person", 1),
            t(1.0, TextKind::Text, "\\Yes", 1),
            t(1.5, TextKind::Text, "ter", 1),
            t(2.0, TextKind::Text, "day ", 1),
            t(2.5, TextKind::Text, "all ", 1),
            t(3.0, TextKind::Text, "my ", 1),
            t(3.5, TextKind::Text, "troub", 1),
            t(4.0, TextKind::Text, "les ", 1),
            t(5.0, TextKind::Text, "/seemed ", 1),
            t(5.5, TextKind::Text, "so ", 1),
            t(6.0, TextKind::Text, "far ", 1),
            t(7.0, TextKind::Text, "\\Now", 1),
            t(7.5, TextKind::Text, " it", 1), // a leading space ends the word before
            t(8.0, TextKind::Text, " looks", 1),
        ];
        let k = parse(&texts, &KaraokeOptions::default()).unwrap();
        assert_eq!(
            (
                k.title.as_deref(),
                k.artist.as_deref(),
                k.language.as_deref()
            ),
            (Some("Yesterday"), Some("The Beatles"), Some("ENGL"))
        );
        assert_eq!(k.info, ["Arranged by A. Person"]);
        assert_eq!(
            words(&k),
            vec![
                (1.0, "Yesterday all my troubles".into(), true),
                (5.0, "seemed so far".into(), false),
                (7.0, "Now it looks".into(), true)
            ]
        );
        assert_eq!(
            k.lines[0]
                .syllables
                .iter()
                .map(|s| (s.time, s.text.as_str()))
                .take(3)
                .collect::<Vec<_>>(),
            [(1.0, "Yes"), (1.5, "ter"), (2.0, "day ")]
        );
        assert_eq!(
            k.lines[2]
                .syllables
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["Now ", "it ", "looks"],
            "spaces move to the end of the word"
        );
    }

    #[test]
    fn lyric_events_are_read_like_text_events_and_a_file_without_marks_is_split_on_pauses() {
        let texts = vec![
            t(1.0, TextKind::Lyric, "one ", 2),
            t(1.5, TextKind::Lyric, "two ", 2),
            t(5.0, TextKind::Lyric, "three ", 2),
            t(5.5, TextKind::Lyric, "four", 2),
        ];
        let k = parse(&texts, &KaraokeOptions::default()).unwrap();
        assert_eq!(
            words(&k),
            vec![
                (1.0, "one two".into(), true),
                (5.0, "three four".into(), false)
            ]
        );
        assert_eq!(k.title, None, "no header, no title");
    }

    #[test]
    fn a_plain_midi_file_with_a_few_text_events_has_no_lyrics() {
        let texts = vec![
            t(0.0, TextKind::Text, "Piano", 0),
            t(0.0, TextKind::Text, "Copyright 1999", 0),
            t(0.0, TextKind::Marker, "Verse", 0),
        ];
        assert_eq!(parse(&texts, &KaraokeOptions::default()), None);
    }

    #[test]
    fn a_syllable_that_ends_in_a_line_break_ends_the_line() {
        let texts = vec![
            t(1.0, TextKind::Lyric, "hello\r", 0),
            t(2.0, TextKind::Lyric, "world", 0),
        ];
        let k = parse(&texts, &KaraokeOptions::default()).unwrap();
        assert_eq!(
            words(&k).iter().map(|w| w.1.as_str()).collect::<Vec<_>>(),
            ["hello", "world"]
        );
    }

    #[test]
    fn a_file_is_read_for_its_lyrics_and_its_length() {
        use crate::smf::{Division, Event, EventKind, FileFormat, Smf, Track};
        use crate::MidiMessage;
        let text = |tick: u64, t: &str| Event {
            tick,
            kind: EventKind::Text(t.into()),
        };
        let mut events = vec![
            text(0, "@KMIDI KARAOKE FILE"),
            text(0, "@TYesterday"),
            text(0, "@TThe Beatles"),
            text(0, "/Yes"),
            text(480, "ter"),
            text(960, "day "),
        ];
        events.push(Event {
            tick: 4 * 480,
            kind: EventKind::Midi(MidiMessage::NoteOn {
                channel: 0,
                key: 60,
                velocity: 90,
            }),
        });
        let bytes = Smf {
            format: FileFormat::Single,
            division: Division::Ticks(480),
            tracks: vec![Track { events }],
        }
        .to_bytes();
        let file = read(&bytes).unwrap();
        assert_eq!(
            (
                file.karaoke.title.as_deref(),
                file.karaoke.artist.as_deref()
            ),
            (Some("Yesterday"), Some("The Beatles"))
        );
        assert!(
            (file.duration - 2.0).abs() < 1e-3,
            "four beats at 120 BPM: {}",
            file.duration
        );
        // no lyrics, and not a MIDI file at all
        let silent = Smf {
            format: FileFormat::Single,
            division: Division::Ticks(480),
            tracks: vec![Track { events: vec![] }],
        }
        .to_bytes();
        assert!(read(&silent).unwrap_err().0.contains("no karaoke lyrics"));
        assert!(read(b"not midi").is_err());
    }
}
