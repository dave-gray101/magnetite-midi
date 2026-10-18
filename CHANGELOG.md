# v0.2.0

New, optional, and off by default: with no features the crate is as before. See [DIVERGENCE.md](DIVERGENCE.md).

- `smf` feature: `MidiMessage`, `Smf` (a Standard MIDI File as written: every event at its tick, a tempo map with bars and beats, notes, a writer, tolerant of damaged files, format 2 and RMID, and keeping where the file says it ends), key-signature and MIDI-sniffing helpers, and `MidiFile::from_smf` / `from_smf_with_loop_type` to play an `Smf` with the sequencer.
- `scan` feature: a streaming reader (`scan`, `NoteTable`) for files with millions of notes. It and `Smf` share their definitions of the tempo map, key signatures and notes.
- `render` feature: render a file to audio on several threads with a limiter (`render::render`), a shared SoundFont cache (`load_soundfont`) and a tiny test SoundFont (`tiny_soundfont`).
- `device` feature: MIDI ports through `midir` behind a trait, with a mock for tests.
- `karaoke` feature: `.kar` lyrics (`karaoke::parse`, `karaoke::read`).
- `serde` feature: `Serialize` and `Deserialize` for the plain types.
- `rust-version = "1.87"`: the real minimum (RustySynth 1.3.7 already needed it, through `is_multiple_of`); an older compiler now says so.
- `midly` is used without its default features, which pulled in `rayon` for nothing.
- The fixtures of two unit tests are in the package, so the tests pass from the published crate.

# v0.1.0

First release of Magnetite-MIDI, a fork of RustySynth. Based on rustysynth v1.3.7; the entries below this one are upstream's history.

- Renamed the crate to `magnetite_midi`.
- Maximum polyphony can now be set up to 16,777,216 voices (previously 256); the lowest-priority voice is only stolen when all are in use.
- Added `SynthesizerSettings::initial_polyphony` to optionally allocate voices on demand at runtime instead of all up front.
- Process `BiQuadFilter`s four at a time to take advantage of SIMD.
- Skip bounds checks where provably safe.
- Cache voice panning angles.

# v1.3.7

- Fixed potential IO issue.
- Fixed an issue where some SoundFonts could produce excessively loud output ([thanks to @wehrwolfmann](https://github.com/sinshu/rustysynth/pull/59)).
- Revised sanity checks related to loop points ([thanks to @wehrwolfmann](https://github.com/sinshu/rustysynth/pull/60)).
- Added runtime handling for invalid loop ranges by disabling looping ([thanks to @wehrwolfmann](https://github.com/sinshu/rustysynth/pull/61)).

# v1.3.6

- Various code clean-ups ([thanks to @sevonj](https://github.com/sinshu/rustysynth/issues/42)).
- Fixed segfault caused by empty `smpl` sub-chunks ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/48)).
- Added sanity check for zero-length loops ([thanks to @eswartz](https://github.com/sinshu/rustysynth/pull/51)).

# v1.3.5

- Improved error reporting for invalid SoundFonts ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/32)).
- Fixed issue where the loop mode was not handled correctly ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/36)).
- All public types now implement the `Debug` trait.

# v1.3.4

- Some minor optimization.
- Fixed an issue where reading certain invalid SoundFonts results in a panic instead of an error ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/31)).

# v1.3.3

- Some minor optimizations.
- Fixed an issue where the pitch bend range was incorrect in certain MIDI files ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/29)).

# v1.3.2

- Added sanity check in loading SoundFonts ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/24)).

# v1.3.1

- Now all the error types don't use heap allocation.

# v1.3.0

- Fixed issue where loading large SoundFont files would fail ([thanks to @paxbun](https://github.com/sinshu/rustysynth/pull/12)).
- Error types no longer allocate `String` ([thanks to @paxbun](https://github.com/sinshu/rustysynth/pull/12)).

# v1.2.1

- Minor tweaks to make the code idiomatic.
- Added `get_sample_id` method to `InstrumentRegion` ([thanks to @pomscyth](https://github.com/sinshu/rustysynth/pull/11)).
- Added `get_instrument_id` method to `PresetRegion`.

# v1.2.0

- Added ability to set the loop point when playing MIDI files.
- Added ability to change the playback speed on the fly when playing MIDI files.
- Added doc comments.

# v1.1.2

- Optimized chorus for better performance.

# v1.1.1

- Fixed issue where reading MIDI files with events inserted after EOT would fail ([thanks to @ArthurCose](https://github.com/sinshu/rustysynth/pull/9)).

# v1.1.0

- Error types are now `non_exhaustive`.
- Loading SoundFont3 explicitly fails with an error `SoundFontError::UnsupportedSampleFormat`.

# v1.0.0

- Introduced custom error types for error reporting.
- Removed unnecessary code.

# v0.9.2

- Refactored the entire code to be more idiomatic ([thanks to @joseluis](https://github.com/sinshu/rustysynth/pull/6)).
- Fixed issue where locks occurred during the rendering process.

# v0.9.1

- Modified the API to accommodate multi-threaded applications ([thanks to @sapir](https://github.com/sinshu/rustysynth/pull/5)).

# v0.9.0

- Implemented reverb and chorus.

# v0.1.0

- First release.
