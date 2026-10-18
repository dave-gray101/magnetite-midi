//! The tempo map: ticks, seconds, quarter notes and bars of one song, from its tempo and time-signature events.

use super::{Division, Event, EventKind};

/// A bar of the tempo map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// From 0.
    pub index: u32,
    pub start_tick: u64,
    pub start_secs: f64,
    pub numerator: u8,
    /// The note value of the beat: 4 = quarter, 8 = eighth.
    pub denominator: u16,
    /// Ticks in a beat (a quarter note times 4 / `denominator`).
    pub beat_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TempoPoint {
    tick: u64,
    secs: f64,
    micros_per_quarter: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SigPoint {
    tick: u64,
    numerator: u8,
    denominator_pow2: u8,
}

const DEFAULT_TEMPO: u32 = 500_000; // 120 BPM, what a file without a tempo event means
const DEFAULT_SIG: (u8, u8) = (4, 2); // 4/4

/// Ticks, seconds, quarter notes and bars of one song, from its tempo and time-signature events (all tracks, since any track may
/// carry them). Times are `f64`: a long song at a high resolution does not fit `f32`.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    division: Division,
    tempos: Vec<TempoPoint>,
    sigs: Vec<SigPoint>,
}

impl TempoMap {
    /// A map with one tempo and 4/4: for building a file from scratch.
    pub fn constant(ticks_per_quarter: u16, bpm: f64) -> Self {
        let tempos = vec![TempoPoint {
            tick: 0,
            secs: 0.0,
            micros_per_quarter: (60_000_000.0 / bpm).round() as u32,
        }];
        Self {
            division: Division::Ticks(ticks_per_quarter),
            tempos,
            sigs: Vec::new(),
        }
    }

    pub(crate) fn build(division: Division, events: &[&Event]) -> Self {
        // the last of several tempo events on one tick wins; the map starts at tick 0 with the default
        let mut tempo_events: Vec<(u64, u32)> = Vec::new();
        let mut sigs: Vec<SigPoint> = Vec::new();
        for e in events {
            match &e.kind {
                EventKind::Tempo(t) if *t > 0 => match tempo_events.last_mut() {
                    Some(last) if last.0 == e.tick => last.1 = *t,
                    _ => tempo_events.push((e.tick, *t)),
                },
                EventKind::TimeSignature {
                    numerator,
                    denominator_pow2,
                    ..
                } if *numerator > 0 => match sigs.last_mut() {
                    Some(last) if last.tick == e.tick => {
                        (last.numerator, last.denominator_pow2) = (*numerator, *denominator_pow2)
                    }
                    _ => sigs.push(SigPoint {
                        tick: e.tick,
                        numerator: *numerator,
                        denominator_pow2: *denominator_pow2,
                    }),
                },
                _ => {}
            }
        }
        let mut tempos = Vec::with_capacity(tempo_events.len() + 1);
        if tempo_events.first().is_none_or(|t| t.0 > 0) {
            tempos.push(TempoPoint {
                tick: 0,
                secs: 0.0,
                micros_per_quarter: DEFAULT_TEMPO,
            });
        }
        for (tick, micros_per_quarter) in tempo_events {
            let secs = match tempos.last() {
                Some(prev) => {
                    prev.secs + Self::span_secs(division, tick - prev.tick, prev.micros_per_quarter)
                }
                None => 0.0,
            };
            tempos.push(TempoPoint {
                tick,
                secs,
                micros_per_quarter,
            });
        }
        Self {
            division,
            tempos,
            sigs,
        }
    }

    fn span_secs(division: Division, ticks: u64, micros_per_quarter: u32) -> f64 {
        match division {
            Division::Ticks(ppq) => {
                ticks as f64 * micros_per_quarter as f64 / ppq.max(1) as f64 / 1e6
            }
            Division::Smpte { fps, subframe } => ticks as f64 / (fps * subframe.max(1) as f64),
        }
    }

    pub fn division(&self) -> Division {
        self.division
    }

    fn point_at_tick(&self, tick: u64) -> &TempoPoint {
        let i = self.tempos.partition_point(|p| p.tick <= tick);
        &self.tempos[i.saturating_sub(1)]
    }

    /// Seconds from the start of the song to `tick`.
    pub fn secs_at(&self, tick: u64) -> f64 {
        let p = self.point_at_tick(tick);
        p.secs + Self::span_secs(self.division, tick - p.tick, p.micros_per_quarter)
    }

    /// The tick at `secs` (rounded to the nearest; negative times are tick 0).
    pub fn tick_at(&self, secs: f64) -> u64 {
        if secs <= 0.0 {
            return 0;
        }
        let i = self.tempos.partition_point(|p| p.secs <= secs);
        let p = &self.tempos[i.saturating_sub(1)];
        let per_tick = Self::span_secs(self.division, 1, p.micros_per_quarter);
        p.tick + ((secs - p.secs) / per_tick).round() as u64
    }

    /// Quarter notes from the start to `tick` (`None` for an SMPTE file, which has no quarter notes).
    pub fn quarters_at(&self, tick: u64) -> Option<f64> {
        match self.division {
            Division::Ticks(ppq) => Some(tick as f64 / ppq.max(1) as f64),
            Division::Smpte { .. } => None,
        }
    }

    /// The tempo at `tick` in beats (quarter notes) per minute; `None` for an SMPTE file.
    pub fn bpm_at(&self, tick: u64) -> Option<f64> {
        matches!(self.division, Division::Ticks(_))
            .then(|| 60_000_000.0 / self.point_at_tick(tick).micros_per_quarter as f64)
    }

    /// Every tempo of the song: `(tick, seconds, beats per minute)`. Always starts at tick 0.
    pub fn tempo_changes(&self) -> Vec<(u64, f64, f64)> {
        self.tempos
            .iter()
            .map(|p| (p.tick, p.secs, 60_000_000.0 / p.micros_per_quarter as f64))
            .collect()
    }

    /// The time signature at `tick`: `(numerator, denominator)`.
    pub fn signature_at(&self, tick: u64) -> (u8, u16) {
        let i = self.sigs.partition_point(|s| s.tick <= tick);
        let (n, d) = if i == 0 {
            DEFAULT_SIG
        } else {
            (
                self.sigs[i - 1].numerator,
                self.sigs[i - 1].denominator_pow2,
            )
        };
        (n, 1u16 << d.min(15))
    }

    /// The bars from tick 0 up to `end_tick`. A bar ends at the next time signature if that comes first, so a signature that is
    /// not on a bar line still starts a new bar. Empty for an SMPTE file.
    pub fn bars(&self, end_tick: u64) -> Vec<Bar> {
        let Division::Ticks(ppq) = self.division else {
            return Vec::new();
        };
        let ppq = ppq.max(1) as u64;
        let mut out = Vec::new();
        let mut tick = 0u64;
        let mut index = 0u32;
        while tick < end_tick {
            let (numerator, denominator) = self.signature_at(tick);
            let beat_ticks = (ppq * 4 / denominator as u64).max(1);
            out.push(Bar {
                index,
                start_tick: tick,
                start_secs: self.secs_at(tick),
                numerator,
                denominator,
                beat_ticks,
            });
            index += 1;
            let next_sig = self.sigs.iter().map(|s| s.tick).find(|t| *t > tick);
            let bar_end = tick + numerator as u64 * beat_ticks;
            tick = next_sig.map_or(bar_end, |s| s.min(bar_end));
        }
        out
    }

    /// The tick and second of every beat from the start to `end_tick`, with the bar and the place in it: `(tick, secs, bar index,
    /// beat in bar)`. This is the beat grid the file defines, exactly.
    pub fn beats(&self, end_tick: u64) -> Vec<(u64, f64, u32, u8)> {
        let bars = self.bars(end_tick);
        let mut out = Vec::new();
        for (i, bar) in bars.iter().enumerate() {
            let bar_end = bars
                .get(i + 1)
                .map_or(end_tick, |b| b.start_tick)
                .min(end_tick);
            for b in 0..bar.numerator {
                let tick = bar.start_tick + b as u64 * bar.beat_ticks;
                if tick >= bar_end {
                    break;
                }
                out.push((tick, self.secs_at(tick), bar.index, b));
            }
        }
        out
    }
}
