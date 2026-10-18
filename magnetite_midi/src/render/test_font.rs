//! A SoundFont made in memory, for tests.

/// A minimal, valid SoundFont 2 (about 5 kB) for tests: every preset (bank 0 and the drum bank 128, program 0) plays one looped sine sample whose root key is 69, so
/// key 69 sounds at 440 Hz and every other key follows. It exists so the test suite needs no real font.
pub fn tiny_soundfont() -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend(p);
        }
        chunk(b"LIST", &body)
    }
    fn name(s: &str) -> [u8; 20] {
        let mut n = [0u8; 20];
        n[..s.len()].copy_from_slice(s.as_bytes());
        n
    }
    fn gen_rec(oper: u16, amount: i16) -> Vec<u8> {
        let mut v = oper.to_le_bytes().to_vec();
        v.extend(amount.to_le_bytes());
        v
    }
    fn bag(generator: u16, modulator: u16) -> Vec<u8> {
        let mut v = generator.to_le_bytes().to_vec();
        v.extend(modulator.to_le_bytes());
        v
    }
    fn phdr(label: &str, preset: u16, bank: u16, bag: u16) -> Vec<u8> {
        let mut v = name(label).to_vec();
        for x in [preset, bank, bag] {
            v.extend(x.to_le_bytes());
        }
        v.extend([0u8; 12]); // library, genre, morphology
        v
    }

    const CYCLE: usize = 100;
    const FRAMES: usize = CYCLE * 16;
    let mut smpl = Vec::new();
    for i in 0..FRAMES {
        let s = ((i as f64 * std::f64::consts::TAU / CYCLE as f64).sin() * 16_000.0) as i16;
        smpl.extend(s.to_le_bytes());
    }
    smpl.extend([0u8; 46 * 2]); // the 46 zero frames the format asks for after a sample

    let info = list(
        b"INFO",
        &[
            chunk(b"ifil", &[2, 0, 1, 0]), // version 2.01
            chunk(b"isng", b"EMU8000\0"),
            chunk(b"INAM", b"magnetite test font\0"),
        ],
    );
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);

    let mut phdrs = phdr("Sine", 0, 0, 0);
    phdrs.extend(phdr("Sine drums", 0, 128, 1));
    phdrs.extend(phdr("EOP", 0, 0, 2));
    let pbag = [bag(0, 0), bag(1, 0), bag(2, 0)].concat();
    let pgen = [gen_rec(41, 0), gen_rec(41, 0), gen_rec(0, 0)].concat(); // instrument 0, twice, then the terminator

    let mut inst = name("Sine").to_vec();
    inst.extend(0u16.to_le_bytes());
    inst.extend(name("EOI"));
    inst.extend(1u16.to_le_bytes());
    let ibag = [bag(0, 0), bag(4, 0)].concat();
    // looped continuously, root key 69, a release of 0.3 s so a note-off does not click, then the sample (the last generator of a zone)
    let igen = [
        gen_rec(54, 1),
        gen_rec(58, 69),
        gen_rec(38, -2000),
        gen_rec(53, 0),
        gen_rec(0, 0),
    ]
    .concat();

    let mut shdr = name("sine").to_vec();
    for x in [0u32, FRAMES as u32, 0, FRAMES as u32 - 1, 44_100] {
        shdr.extend(x.to_le_bytes());
    }
    shdr.push(69); // original pitch
    shdr.push(-4i8 as u8); // the sample is 441 Hz: four cents flat of that to play A at 440
    shdr.extend(0u16.to_le_bytes()); // sample link
    shdr.extend(1u16.to_le_bytes()); // mono
    shdr.extend(name("EOS"));
    shdr.extend([0u8; 26]);

    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdrs),
            chunk(b"pbag", &pbag),
            chunk(b"pmod", &[0u8; 10]),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &ibag),
            chunk(b"imod", &[0u8; 10]),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let mut body = b"sfbk".to_vec();
    for p in [info, sdta, pdta] {
        body.extend(p);
    }
    chunk(b"RIFF", &body)
}
