//! One MIDI message as a plain value: parsed from device bytes, written back to bytes, and the same type for the channel
//! messages inside a Standard MIDI File. All byte-level work is `midly`'s (`midly::live`); this type only fixes a shape that is
//! easy to match on, to serialise (with the `serde` feature) and to pass around by value.

use midly::live::{LiveEvent, SystemCommon, SystemRealtime};
use midly::num::{u14, u4, u7};
use midly::MidiMessage as Voice;

/// A MIDI message. Channels are 0 to 15 (channel 1 of a keyboard's display is 0 here); keys, velocities, controllers and
/// values are 0 to 127.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MidiMessage {
    NoteOff {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    /// A note-on with velocity 0 is a note-off on the wire; [`MidiMessage::is_note_off`] knows both.
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    PolyPressure {
        channel: u8,
        key: u8,
        pressure: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    ChannelPressure {
        channel: u8,
        pressure: u8,
    },
    /// -8192 to 8191, 0 = centre.
    PitchBend {
        channel: u8,
        value: i16,
    },
    /// The payload of a system-exclusive message, without the leading `F0` and the trailing `F7`.
    SysEx(Vec<u8>),
    /// 14-bit song position in MIDI beats (sixteenth notes).
    SongPosition(u16),
    SongSelect(u8),
    TuneRequest,
    /// MIDI clock: 24 per quarter note.
    Clock,
    Start,
    Continue,
    Stop,
    ActiveSensing,
    Reset,
    /// A valid message this type has no variant for (MTC quarter frames, undefined status bytes): the original bytes.
    Raw(Vec<u8>),
}

impl MidiMessage {
    /// Parse the bytes of one message, as a device delivers them. `None` for bytes that are not a whole message.
    pub fn parse(bytes: &[u8]) -> Option<MidiMessage> {
        match LiveEvent::parse(bytes).ok()? {
            LiveEvent::Midi { channel, message } => Some(Self::from_voice(channel, message)),
            LiveEvent::Common(c) => Some(match c {
                SystemCommon::SysEx(data) => {
                    MidiMessage::SysEx(data.iter().map(|b| b.as_int()).collect())
                }
                SystemCommon::SongPosition(p) => MidiMessage::SongPosition(p.as_int()),
                SystemCommon::SongSelect(s) => MidiMessage::SongSelect(s.as_int()),
                SystemCommon::TuneRequest => MidiMessage::TuneRequest,
                _ => MidiMessage::Raw(bytes.to_vec()),
            }),
            LiveEvent::Realtime(r) => Some(match r {
                SystemRealtime::TimingClock => MidiMessage::Clock,
                SystemRealtime::Start => MidiMessage::Start,
                SystemRealtime::Continue => MidiMessage::Continue,
                SystemRealtime::Stop => MidiMessage::Stop,
                SystemRealtime::ActiveSensing => MidiMessage::ActiveSensing,
                SystemRealtime::Reset => MidiMessage::Reset,
                _ => MidiMessage::Raw(bytes.to_vec()),
            }),
        }
    }

    /// The bytes to send to a device.
    pub fn to_bytes(&self) -> Vec<u8> {
        if let MidiMessage::Raw(bytes) = self {
            return bytes.clone();
        }
        let sysex: Vec<u7>;
        let event = if let Some((channel, message)) = self.to_voice() {
            LiveEvent::Midi { channel, message }
        } else {
            match self {
                MidiMessage::SysEx(data) => {
                    sysex = data.iter().map(|b| u7::from_int_lossy(*b)).collect();
                    LiveEvent::Common(SystemCommon::SysEx(&sysex))
                }
                MidiMessage::SongPosition(p) => {
                    LiveEvent::Common(SystemCommon::SongPosition(u14::from_int_lossy(*p)))
                }
                MidiMessage::SongSelect(s) => {
                    LiveEvent::Common(SystemCommon::SongSelect(u7::from_int_lossy(*s)))
                }
                MidiMessage::TuneRequest => LiveEvent::Common(SystemCommon::TuneRequest),
                MidiMessage::Clock => LiveEvent::Realtime(SystemRealtime::TimingClock),
                MidiMessage::Start => LiveEvent::Realtime(SystemRealtime::Start),
                MidiMessage::Continue => LiveEvent::Realtime(SystemRealtime::Continue),
                MidiMessage::Stop => LiveEvent::Realtime(SystemRealtime::Stop),
                MidiMessage::ActiveSensing => LiveEvent::Realtime(SystemRealtime::ActiveSensing),
                MidiMessage::Reset => LiveEvent::Realtime(SystemRealtime::Reset),
                _ => unreachable!("channel voice messages and Raw are handled above"),
            }
        };
        let mut out = Vec::with_capacity(3);
        event
            .write_std(&mut out)
            .expect("writing to a Vec does not fail");
        out
    }

    /// The channel of a channel message (0 to 15).
    pub fn channel(&self) -> Option<u8> {
        match self {
            MidiMessage::NoteOff { channel, .. }
            | MidiMessage::NoteOn { channel, .. }
            | MidiMessage::PolyPressure { channel, .. }
            | MidiMessage::ControlChange { channel, .. }
            | MidiMessage::ProgramChange { channel, .. }
            | MidiMessage::ChannelPressure { channel, .. }
            | MidiMessage::PitchBend { channel, .. } => Some(*channel),
            _ => None,
        }
    }

    /// A note-off, or a note-on of velocity 0.
    pub fn is_note_off(&self) -> bool {
        matches!(
            self,
            MidiMessage::NoteOff { .. } | MidiMessage::NoteOn { velocity: 0, .. }
        )
    }

    /// A note-on that sounds (velocity above 0).
    pub fn is_note_on(&self) -> bool {
        matches!(self, MidiMessage::NoteOn { velocity, .. } if *velocity > 0)
    }

    /// Whether this is one of the real-time messages that arrive between any two others (clock, start, stop...).
    pub fn is_realtime(&self) -> bool {
        matches!(
            self,
            MidiMessage::Clock
                | MidiMessage::Start
                | MidiMessage::Continue
                | MidiMessage::Stop
                | MidiMessage::ActiveSensing
                | MidiMessage::Reset
        )
    }

    /// A channel message from the form `midly` uses inside files and live events.
    pub(crate) fn from_voice(channel: u4, message: Voice) -> MidiMessage {
        let channel = channel.as_int();
        match message {
            Voice::NoteOff { key, vel } => MidiMessage::NoteOff {
                channel,
                key: key.as_int(),
                velocity: vel.as_int(),
            },
            Voice::NoteOn { key, vel } => MidiMessage::NoteOn {
                channel,
                key: key.as_int(),
                velocity: vel.as_int(),
            },
            Voice::Aftertouch { key, vel } => MidiMessage::PolyPressure {
                channel,
                key: key.as_int(),
                pressure: vel.as_int(),
            },
            Voice::Controller { controller, value } => MidiMessage::ControlChange {
                channel,
                controller: controller.as_int(),
                value: value.as_int(),
            },
            Voice::ProgramChange { program } => MidiMessage::ProgramChange {
                channel,
                program: program.as_int(),
            },
            Voice::ChannelAftertouch { vel } => MidiMessage::ChannelPressure {
                channel,
                pressure: vel.as_int(),
            },
            Voice::PitchBend { bend } => MidiMessage::PitchBend {
                channel,
                value: bend.as_int(),
            },
        }
    }

    /// The `midly` form of a channel message (`None` for the others).
    pub(crate) fn to_voice(&self) -> Option<(u4, Voice)> {
        let k = |v: u8| u7::from_int_lossy(v);
        Some(match *self {
            MidiMessage::NoteOff {
                channel,
                key,
                velocity,
            } => (
                u4::from_int_lossy(channel),
                Voice::NoteOff {
                    key: k(key),
                    vel: k(velocity),
                },
            ),
            MidiMessage::NoteOn {
                channel,
                key,
                velocity,
            } => (
                u4::from_int_lossy(channel),
                Voice::NoteOn {
                    key: k(key),
                    vel: k(velocity),
                },
            ),
            MidiMessage::PolyPressure {
                channel,
                key,
                pressure,
            } => (
                u4::from_int_lossy(channel),
                Voice::Aftertouch {
                    key: k(key),
                    vel: k(pressure),
                },
            ),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => (
                u4::from_int_lossy(channel),
                Voice::Controller {
                    controller: k(controller),
                    value: k(value),
                },
            ),
            MidiMessage::ProgramChange { channel, program } => (
                u4::from_int_lossy(channel),
                Voice::ProgramChange {
                    program: k(program),
                },
            ),
            MidiMessage::ChannelPressure { channel, pressure } => (
                u4::from_int_lossy(channel),
                Voice::ChannelAftertouch { vel: k(pressure) },
            ),
            MidiMessage::PitchBend { channel, value } => (
                u4::from_int_lossy(channel),
                Voice::PitchBend {
                    bend: midly::PitchBend::from_int(value),
                },
            ),
            _ => return None,
        })
    }
}

impl MidiMessage {
    /// A channel message as a synthesizer takes it, `(channel, command, data1, data2)`: the command is the status byte without its channel (`0x90` for a note-on), and a pitch bend is split
    /// into its two 7-bit halves. `None` for every message that is not a channel message (system messages, including sysex, never reach a synthesizer).
    pub fn synth_args(&self) -> Option<(i32, i32, i32, i32)> {
        Some(match *self {
            MidiMessage::NoteOff {
                channel,
                key,
                velocity,
            } => (channel as i32, 0x80, key as i32, velocity as i32),
            MidiMessage::NoteOn {
                channel,
                key,
                velocity,
            } => (channel as i32, 0x90, key as i32, velocity as i32),
            MidiMessage::PolyPressure {
                channel,
                key,
                pressure,
            } => (channel as i32, 0xA0, key as i32, pressure as i32),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => (channel as i32, 0xB0, controller as i32, value as i32),
            MidiMessage::ProgramChange { channel, program } => {
                (channel as i32, 0xC0, program as i32, 0)
            }
            MidiMessage::ChannelPressure { channel, pressure } => {
                (channel as i32, 0xD0, pressure as i32, 0)
            }
            MidiMessage::PitchBend { channel, value } => {
                let v = (value as i32 + 8192).clamp(0, 16383);
                (channel as i32, 0xE0, v & 0x7F, v >> 7)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_survives_bytes_and_back() {
        let all = [
            MidiMessage::NoteOn {
                channel: 3,
                key: 60,
                velocity: 100,
            },
            MidiMessage::NoteOff {
                channel: 15,
                key: 127,
                velocity: 0,
            },
            MidiMessage::PolyPressure {
                channel: 0,
                key: 64,
                pressure: 9,
            },
            MidiMessage::ControlChange {
                channel: 1,
                controller: 7,
                value: 99,
            },
            MidiMessage::ProgramChange {
                channel: 9,
                program: 40,
            },
            MidiMessage::ChannelPressure {
                channel: 2,
                pressure: 33,
            },
            MidiMessage::PitchBend {
                channel: 4,
                value: -8192,
            },
            MidiMessage::PitchBend {
                channel: 4,
                value: 8191,
            },
            MidiMessage::PitchBend {
                channel: 4,
                value: 0,
            },
            MidiMessage::SysEx(vec![0x7E, 0x7F, 0x09, 0x01]),
            MidiMessage::SongPosition(1234),
            MidiMessage::SongSelect(5),
            MidiMessage::TuneRequest,
            MidiMessage::Clock,
            MidiMessage::Start,
            MidiMessage::Continue,
            MidiMessage::Stop,
            MidiMessage::ActiveSensing,
            MidiMessage::Reset,
        ];
        for m in all {
            let bytes = m.to_bytes();
            assert_eq!(
                MidiMessage::parse(&bytes).as_ref(),
                Some(&m),
                "{m:?} as {bytes:02X?}"
            );
        }
    }

    #[test]
    fn the_bytes_are_the_wire_format() {
        assert_eq!(
            MidiMessage::NoteOn {
                channel: 2,
                key: 60,
                velocity: 100
            }
            .to_bytes(),
            [0x92, 60, 100]
        );
        assert_eq!(
            MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 127
            }
            .to_bytes(),
            [0xB0, 64, 127]
        );
        assert_eq!(
            MidiMessage::PitchBend {
                channel: 0,
                value: 0
            }
            .to_bytes(),
            [0xE0, 0x00, 0x40]
        );
        assert_eq!(MidiMessage::Clock.to_bytes(), [0xF8]);
        assert_eq!(
            MidiMessage::SysEx(vec![1, 2]).to_bytes(),
            [0xF0, 1, 2, 0xF7]
        );
    }

    #[test]
    fn a_note_on_of_velocity_zero_is_a_note_off() {
        let m = MidiMessage::parse(&[0x90, 60, 0]).unwrap();
        assert!(m.is_note_off() && !m.is_note_on());
        assert!(MidiMessage::parse(&[0x90, 60, 1]).unwrap().is_note_on());
        assert!(MidiMessage::parse(&[0x80, 60, 64]).unwrap().is_note_off());
    }

    #[test]
    fn what_is_not_a_whole_message_is_none_and_the_unknown_is_kept_raw() {
        assert_eq!(MidiMessage::parse(&[]), None);
        assert_eq!(
            MidiMessage::parse(&[0x90, 60]),
            None,
            "a note-on needs three bytes"
        );
        let mtc = [0xF1, 0x23];
        let m = MidiMessage::parse(&mtc).unwrap();
        assert_eq!(
            m,
            MidiMessage::Raw(mtc.to_vec()),
            "an MTC quarter frame has no variant here and is passed through"
        );
        assert_eq!(m.to_bytes(), mtc);
    }

    #[test]
    fn channels_and_realtime() {
        assert_eq!(
            MidiMessage::NoteOn {
                channel: 7,
                key: 1,
                velocity: 1
            }
            .channel(),
            Some(7)
        );
        assert_eq!(MidiMessage::Clock.channel(), None);
        assert!(MidiMessage::Clock.is_realtime() && !MidiMessage::SysEx(vec![]).is_realtime());
    }
}
