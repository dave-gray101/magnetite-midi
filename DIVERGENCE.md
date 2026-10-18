# Where this fork differs from RustySynth

Magnetite-MIDI is a fork of [RustySynth](https://github.com/sinshu/rustysynth) (remote `sinshu`). Upstream has declined our changes, but we still want its improvements, so this file says exactly what is ours, so that `git merge sinshu/main` stays cheap and a conflict is easy to read.

**Rules**

1. Anything new goes in **new files**, behind a **cargo feature**, and builds only on upstream's public API (plus the crate-private items `MidiFile` already shares inside the crate).
2. Upstream-owned files are touched only where this list says, and the touch is kept as small as it can be.
3. CI (`.github/workflows/ci.yml`) checks every change: formatting, clippy, the tests on Linux, Windows and macOS, each feature on its own, the minimum Rust version the crate declares, the docs, and the package as extracted. The daily upstream sync starts it for its own branch. After a merge from upstream, also run `MAGNETITE_MIDI_CORPUS=<folder of .mid files> cargo test --features smf real_files` and the same for `agreement` (the tests that compare the two loaders and `Smf` with `scan` on real files, which CI skips because the files are not in the repository).
4. Fixes that are not about our features are welcome upstream as pull requests.

## Upstream-owned files that differ

Checked against `sinshu/main` file by file: these are all of them, and every other file under `src/` is identical to upstream's. The daily `.github/workflows/sync-upstream.yml` merges upstream into `upstream-sync` and opens a pull request, or an issue if it conflicts: a conflict can only be in the files above.

| File | How | Why |
|---|---|---|
| `magnetite_midi/src/bi_quad_filter.rs`, `oscillator.rs`, `synthesizer_settings.rs`, `voice.rs`, `voice_collection.rs`, `error.rs` | the polyphony and performance commit `a060fe4` (see CHANGELOG v0.1.0) | Black MIDI files |
| `magnetite_midi/src/midifile.rs` | one word: `merge_tracks` is `pub(crate)` | `MidiFile::from_smf` merges tracks with the loader's own code, so there is one implementation of the tempo arithmetic |
| `magnetite_midi/src/lib.rs` | lines appended at the end: the feature-gated `mod` declarations | the new modules |
| `magnetite_midi/src/soundfont.rs` | the test helper `samples_dir_path` (two lines, in `#[cfg(test)]`) | the two fixtures it reads moved from the repository's `samples/` into `magnetite_midi/samples/`, so that they are in the published package and its tests pass there (upstream's published crate omits them) |
| `magnetite_midi/Cargo.toml` | the `[features]` block, three optional dependencies (`midly` without its default `rayon`), the `[[test]]` entries, `rust-version`, docs.rs metadata | the features; the real minimum Rust, which the compiler now reports clearly |

`midifile_sequencer.rs` and the synthesizer itself are upstream's, unchanged. Upstream's unmerged `add_message_hook` branch (a hook on each message the sequencer sends) conflicts with upstream's own `main`, not with this fork: its base is older than upstream's last changes to the sequencer.

The directory is `magnetite_midi/` where upstream's is `rustysynth/`. Git's rename detection follows it in a merge.

## Files that are ours (nothing upstream touches)

All behind a feature that is **off by default**: with no features the crate is upstream's synthesizer with our polyphony work and no dependencies.

Each feature is a module of the same name, in a directory of its own.

| Feature | Module | What |
|---|---|---|
| `smf` | `smf/` (`mod.rs`, `message.rs`, `tempo.rs`, `shared.rs`, `wire.rs`, `playback.rs`) | `MidiMessage`; `Smf`, the Standard MIDI File as written (every event at its tick, all metadata, a writer); `TempoMap` (bars and beats); what `Smf` and `scan` share (`shared.rs`); the conversion to and from `midly` and the text of a file (`wire.rs`); `MidiFile::from_smf`, which compiles an `Smf` for the sequencer (`playback.rs`) |
| `scan` | `scan/` | a streaming reader for very large files: notes, markers, lyrics and text without the owned event list |
| `render` | `render/` (`mod.rs`, `limiter.rs`, `font.rs`, `test_font.rs`) | a file to audio on several threads; the limiter; the shared SoundFont cache; `tiny_soundfont` |
| `device` | `device/` | MIDI ports through `midir` behind a trait, with a mock |
| `karaoke` | `karaoke/` | `.kar` lyrics. Not implied by any other feature |
| `serde` | (derives) | `Serialize` / `Deserialize` for the plain types |

`MidiFile` and `Smf` are two views of one file: **`MidiFile` is the file compiled for playback** (channel messages and tempo merged into one timeline in seconds, everything else dropped, immutable), **`Smf` is the file as written** (lossless, including where the file says it ends; tolerant of damage, format 2 and RMID; writable). `MidiFile::from_smf` is how the first is made from the second; `Smf` and `scan` (the streaming reader) share one definition of the tempo map, the key signatures and what a note is (`MetaEvents` and `NotePairer` in `smf.rs`), and `tests/agreement.rs` compares them on generated files and, with `MAGNETITE_MIDI_CORPUS`, on real ones; a test compares it with upstream's loader on generated files for every loop type, and on real files when `MAGNETITE_MIDI_CORPUS` names a folder.

## Edition and Rust version

The crate stays on edition 2021, as upstream is. A bump would touch two upstream lines (`error.rs`), the `edition` of three manifests, and the formatting of three upstream files unless `rustfmt.toml` pins `style_edition = "2021"`. New code here avoids let chains for that reason.

**The minimum Rust is 1.87, with or without features, and upstream's code sets it**: `generator.rs` and four more core files call `is_multiple_of`, stable in 1.87 (RustySynth 1.3.7 as published does too). Measured on 1.70, 1.82, 1.86, 1.87, 1.88 and 1.89: 1.86 fails in the core with no features; 1.87 builds every feature, all targets. `rust-version = "1.87"` says so, so an older compiler gets a clear message and not an error in the middle of a source file. The `device` feature's own dependencies (`midir`, on Windows the `windows` crates) need 1.82 at least, so removing `is_multiple_of` from the core would not take it below that.
