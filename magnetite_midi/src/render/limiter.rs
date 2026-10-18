//! The limiter on the mix: hundreds of voices sum past full scale, and clipping that is heard as distortion.

/// A lookahead limiter for the mix. Hundreds of voices sum past full scale (peaks of 1.7 on `tau2.5.9.mid`), and clipping that is heard as distortion: the gain
/// instead comes down over the `LOOKAHEAD` frames before a peak and back up over about 100 ms after. Under the ceiling it changes nothing (the samples are
/// the synthesizer's own); it delays by `LOOKAHEAD` frames, which `render` takes back by dropping the first frames and flushing the last.
pub(super) struct Limiter {
    delay: Vec<[f32; 2]>,
    wanted: Vec<f32>,
    smooth: Vec<f32>,
    pos: usize,
    sum: f32,
    held: f32,
    /// How many of `wanted` are below 1: while none is, and the gain has recovered, a frame only passes through the delay.
    reducing: usize,
}

pub(super) const LOOKAHEAD: usize = 128;
const CEILING: f32 = 0.98;

impl Limiter {
    pub(super) fn new() -> Self {
        Limiter {
            delay: vec![[0.0; 2]; LOOKAHEAD],
            wanted: vec![1.0; LOOKAHEAD],
            smooth: vec![1.0; LOOKAHEAD],
            pos: 0,
            sum: LOOKAHEAD as f32,
            held: 1.0,
            reducing: 0,
        }
    }

    /// Replace `frames` (interleaved stereo) with the same signal `LOOKAHEAD` frames later, limited.
    pub(super) fn process(&mut self, frames: &mut [f32], release: f32) {
        // `chunks_exact_mut` and not `as_chunks_mut` (Rust 1.88): this crate states no minimum Rust version and should not gain one for a loop
        #[allow(clippy::chunks_exact_to_as_chunks)]
        for f in frames.chunks_exact_mut(2) {
            let peak = f[0].abs().max(f[1].abs());
            let out = self.delay[self.pos];
            self.delay[self.pos] = [f[0], f[1]];
            let want = if peak > CEILING { CEILING / peak } else { 1.0 };
            self.reducing =
                self.reducing + (want < 1.0) as usize - (self.wanted[self.pos] < 1.0) as usize;
            self.wanted[self.pos] = want;
            if self.reducing == 0 && self.sum == LOOKAHEAD as f32 && self.held == 1.0 {
                self.smooth[self.pos] = 1.0;
                f[0] = out[0];
                f[1] = out[1];
            } else {
                // the lowest gain asked of any frame in the window, then averaged over the window
                let floor = self.wanted.iter().copied().fold(1.0f32, f32::min);
                self.sum += floor - self.smooth[self.pos];
                self.smooth[self.pos] = floor;
                let averaged = self.sum / LOOKAHEAD as f32;
                self.held = if averaged < self.held {
                    averaged
                } else {
                    self.held + (averaged - self.held) * release
                };
                if self.held > 0.99999
                    && self.reducing == 0
                    && self.smooth.iter().all(|&g| g == 1.0)
                {
                    (self.held, self.sum) = (1.0, LOOKAHEAD as f32);
                }
                let gain = self.held.min(1.0);
                f[0] = out[0] * gain;
                f[1] = out[1] * gain;
            }
            self.pos = (self.pos + 1) % LOOKAHEAD;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_the_ceiling_it_only_delays_and_over_it_it_never_exceeds_it() {
        let mut quiet: Vec<f32> = (0..2000)
            .flat_map(|i| [(i as f32 * 0.05).sin() * 0.5; 2])
            .collect();
        let original = quiet.clone();
        let mut l = Limiter::new();
        l.process(&mut quiet, 0.001);
        assert_eq!(
            &quiet[LOOKAHEAD * 2..],
            &original[..original.len() - LOOKAHEAD * 2],
            "a quiet signal is only delayed"
        );

        let mut loud: Vec<f32> = (0..4000)
            .flat_map(|i| [(i as f32 * 0.05).sin() * 1.7; 2])
            .collect();
        let mut l = Limiter::new();
        l.process(&mut loud, 0.001);
        let peak = loud.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak <= CEILING + 1e-4, "peak {peak}");
        assert!(peak > 0.5, "and it is not silenced");
    }
}
