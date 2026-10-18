mod error;

mod array_math;
mod binary_reader;
mod four_cc;
mod read_counter;

mod generator;
mod generator_type;
mod instrument;
mod instrument_info;
mod instrument_region;
mod loop_mode;
mod preset;
mod preset_info;
mod preset_region;
mod sample_header;
mod soundfont;
mod soundfont_info;
mod soundfont_math;
mod soundfont_parameters;
mod soundfont_sampledata;
mod soundfont_version;
mod zone;
mod zone_info;

mod bi_quad_filter;
mod channel;
mod envelope_stage;
mod lfo;
mod modulation_envelope;
mod oscillator;
mod region_ex;
mod region_pair;
mod synthesizer;
mod synthesizer_settings;
mod voice;
mod voice_collection;
mod volume_envelope;

mod midifile;
mod midifile_looptype;
mod midifile_sequencer;

mod chorus;
mod reverb;

pub use self::error::MidiFileError;
pub use self::error::SoundFontError;
pub use self::error::SynthesizerError;
pub use self::instrument::Instrument;
pub use self::instrument_region::InstrumentRegion;
pub use self::loop_mode::LoopMode;
pub use self::midifile::MidiFile;
pub use self::midifile_looptype::MidiFileLoopType;
pub use self::midifile_sequencer::MidiFileSequencer;
pub use self::preset::Preset;
pub use self::preset_region::PresetRegion;
pub use self::sample_header::SampleHeader;
pub use self::soundfont::SoundFont;
pub use self::soundfont_info::SoundFontInfo;
pub use self::soundfont_version::SoundFontVersion;
pub use self::synthesizer::Synthesizer;
pub use self::synthesizer_settings::SynthesizerSettings;

// Additions of this fork, each behind a feature and in a module of the same name (see DIVERGENCE.md): nothing above this line is changed by them.
#[cfg(feature = "device")]
pub mod device;
#[cfg(feature = "karaoke")]
pub mod karaoke;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "scan")]
pub mod scan;
#[cfg(feature = "smf")]
pub mod smf;
#[cfg(feature = "smf")]
pub use self::smf::MidiMessage;
