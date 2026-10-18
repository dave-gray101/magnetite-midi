# Magnetite-MIDI

Magnetite-MIDI is a pure Rust SoundFont MIDI synethesizer. It is heavily based on [RustySynth](https://github.com/sinshu/rustysynth).

While RustySynth is focused on simplicity, this fork prioritizes performance and features, with a specific goal of supporting [Black MIDI](https://en.wikipedia.org/wiki/Black_MIDI) files.

## Changes from mainline RustySynth

* Effectively unlimited maximum polyphony
* Processes BiQuadFilters 4 at a time, in order to take advantage of SIMD
* Skipping bounds checks where provably safe
* Caching voice panning angles
* OPTIONAL support for dynamic allocation of voices at runtime


## Optional features

The synthesizer needs no dependencies. Everything else is behind a cargo feature that is off until you ask for it. What differs from RustySynth, and why, is in [DIVERGENCE.md](https://github.com/dave-gray101/magnetite-midi/blob/main/DIVERGENCE.md).

| Feature | Adds |
|---|---|
| `smf` | `MidiMessage`, `Smf` (a Standard MIDI File as written, with a tempo map and a writer) and `MidiFile::from_smf` to play one with the sequencer |
| `scan` | a streaming reader for very large files (notes, markers and text without the owned event list) |
| `render` | render a file to audio on several threads with a limiter, a shared SoundFont cache, and a tiny SoundFont for tests |
| `device` | MIDI ports through `midir`, behind a trait with a mock |
| `karaoke` | `.kar` lyrics. Not implied by any other feature |
| `serde` | `Serialize` and `Deserialize` for the plain types |

```
cargo add magnetite_midi --features render
```

`MidiFile` is the file compiled for playback (messages in seconds, everything else dropped); `Smf` is the file as written, which is what to read, inspect or write. The crate needs Rust 1.87 or newer, with or without features (the synthesizer itself, as in RustySynth 1.3.7, uses `is_multiple_of`, stable since 1.87).

## Examples

An example code to synthesize a simple chord:

```rust
// Load the SoundFont.
let mut sf2 = File::open("TimGM6mb.sf2").unwrap();
let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

// Create the synthesizer.
let settings = SynthesizerSettings::new(44100);
let mut synthesizer = Synthesizer::new(&sound_font, &settings).unwrap();

// Play some notes (middle C, E, G).
synthesizer.note_on(0, 60, 100);
synthesizer.note_on(0, 64, 100);
synthesizer.note_on(0, 67, 100);

// The output buffer (3 seconds).
let sample_count = (3 * settings.sample_rate) as usize;
let mut left: Vec<f32> = vec![0_f32; sample_count];
let mut right: Vec<f32> = vec![0_f32; sample_count];

// Render the waveform.
synthesizer.render(&mut left[..], &mut right[..]);
```

Another example code to synthesize a MIDI file:

```rust
// Load the SoundFont.
let mut sf2 = File::open("TimGM6mb.sf2").unwrap();
let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

// Load the MIDI file.
let mut mid = File::open("flourish.mid").unwrap();
let midi_file = Arc::new(MidiFile::new(&mut mid).unwrap());

// Create the MIDI file sequencer.
let settings = SynthesizerSettings::new(44100);
let synthesizer = Synthesizer::new(&sound_font, &settings).unwrap();
let mut sequencer = MidiFileSequencer::new(synthesizer);

// Play the MIDI file.
sequencer.play(&midi_file, false);

// The output buffer.
let sample_count = (settings.sample_rate as f64 * midi_file.get_length()) as usize;
let mut left: Vec<f32> = vec![0_f32; sample_count];
let mut right: Vec<f32> = vec![0_f32; sample_count];

// Render the waveform.
sequencer.render(&mut left[..], &mut right[..]);
```



## Todo

* __Wave synthesis__
    - [x] SoundFont reader
    - [x] Waveform generator
    - [x] Envelope generator
    - [x] Low-pass filter
    - [x] Vibrato LFO
    - [x] Modulation LFO
* __MIDI message processing__
    - [x] Note on/off
    - [x] Bank selection
    - [x] Modulation
    - [x] Volume control
    - [x] Pan
    - [x] Expression
    - [x] Hold pedal
    - [x] Program change
    - [x] Pitch bend
    - [x] Tuning
* __Effects__
    - [x] Reverb
    - [x] Chorus
* __Other things__
    - [x] Standard MIDI file support
    - [x] MIDI file loop extension support
    - [x] Performace optimization



## License

Magnetite-MIDI is available under [the MIT license](LICENSE.txt).
