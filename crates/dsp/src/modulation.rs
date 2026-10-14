//! Modulation effects: Motion Filter, Chorus, Phaser (spec: docs/specs/devices.md D2).

use crate::device::{processor, Io, Processor};
use crate::filter::{Svf, SvfMode};
use crate::params::{Ctx, ParamDesc};
use crate::util::{Rng, Smoothed};
use std::f32::consts::TAU;

pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw", "Random"];
pub const LFO_SYNC: &[&str] = &["Off", "4/1", "2/1", "1/1", "1/2", "1/4", "1/8", "1/16"];
/// Cycle length in beats of each synced `LFO_SYNC` entry (index 0 = free).
const SYNC_BEATS: [f64; 8] = [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];

/// Low-frequency oscillator, bipolar output in [-1, 1].
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    pub phase: f64,
    held: f32,
    rng: Rng,
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        Self { phase: 0.0, held: 0.0, rng: Rng::new(seed) }
    }

    /// Value at `phase + offset` (cycles). Random holds one value per cycle.
    pub fn value(&self, shape: usize, offset: f64) -> f32 {
        let p = (self.phase + offset).rem_euclid(1.0) as f32;
        match shape {
            0 => (p * TAU).sin(),
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => if p < 0.5 { 1.0 } else { -1.0 },
            3 => 2.0 * p - 1.0,
            _ => self.held,
        }
    }

    /// Moves the phase to `phase`; a new random value is drawn on each wrap.
    pub fn set_phase(&mut self, phase: f64) {
        if phase.floor() != self.phase.floor() || phase < self.phase {
            self.held = self.rng.next_bipolar();
        }
        self.phase = phase;
    }

    pub fn advance(&mut self, cycles: f64) {
        let p = self.phase + cycles;
        self.set_phase(if p > 1e6 { p.fract() } else { p });
    }
}

// ---------------------------------------------------------------- Motion Filter

pub const MOTION_TYPES: &[&str] = &["Low Pass", "High Pass", "Band Pass"];

pub const MOTION_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("type", "Type", MOTION_TYPES, 0.0),
    ParamDesc::log("cutoff", "Cutoff", 20.0, 18_000.0, 1_200.0, "Hz"),
    ParamDesc::lin("resonance", "Resonance", 0.0, 0.95, 0.3, ""),
    ParamDesc::lin("lfo_amount", "LFO Amount", 0.0, 1.0, 0.3, ""),
    ParamDesc::log("lfo_rate", "LFO Rate", 0.01, 20.0, 1.0, "Hz"),
    ParamDesc::choice("lfo_sync", "LFO Sync", LFO_SYNC, 0.0),
    ParamDesc::choice("lfo_shape", "LFO Shape", LFO_SHAPES, 0.0),
    ParamDesc::lin("stereo", "Stereo Phase", 0.0, 180.0, 0.0, "°"),
    ParamDesc::lin("env_amount", "Env Amount", -1.0, 1.0, 0.0, ""),
    ParamDesc::log("env_attack", "Env Attack", 0.1, 100.0, 5.0, "ms"),
    ParamDesc::log("env_release", "Env Release", 10.0, 2_000.0, 200.0, "ms"),
    ParamDesc::lin("mix", "Mix", 0.0, 100.0, 100.0, "%"),
];

/// Control-rate interval (samples) for filter coefficient updates.
const CONTROL: usize = 16;

pub struct MotionFilter {
    params: [f32; 12],
    cutoff: Smoothed,
    mix: Smoothed,
    lfo: Lfo,
    env: f32,
    filters: [Svf; 2],
    sr: f32,
}

impl MotionFilter {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 12] = std::array::from_fn(|i| MOTION_PARAMS[i].default);
        Self { cutoff: Smoothed::new(params[1], sr, 30.0), mix: Smoothed::new(1.0, sr, 20.0), params, lfo: Lfo::new(0x5eed), env: 0.0, filters: [Svf::default(); 2], sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = MOTION_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.cutoff.set_target(self.params[1]);
            self.mix.set_target(self.params[11] / 100.0);
        }
    }

    /// Cutoff for the given modulation sources (spec D2, Motion Filter).
    pub fn modulated_cutoff(base: f32, lfo: f32, lfo_amount: f32, env: f32, env_amount: f32) -> f32 {
        (base * 2f32.powf(lfo * lfo_amount * 4.0 + env * env_amount * 6.0)).clamp(20.0, 20_000.0)
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        let p = self.params;
        let mode = [SvfMode::LowPass, SvfMode::HighPass, SvfMode::BandPass][p[0] as usize];
        let sync = SYNC_BEATS[p[5] as usize];
        let shape = p[6] as usize;
        let stereo = (p[7] / 360.0) as f64;
        let att = (-1.0 / (p[9] * 0.001 * self.sr)).exp();
        let rel = (-1.0 / (p[10] * 0.001 * self.sr)).exp();
        let n = l.len();
        let mut i = 0;
        while i < n {
            let end = (i + CONTROL).min(n);
            // LFO phase: locked to the song grid when synced and playing.
            if sync > 0.0 && ctx.playing {
                self.lfo.set_phase((ctx.beat + i as f64 * ctx.beats_per_sample) / sync);
            }
            let cycles_per_sample = if sync > 0.0 { ctx.beats_per_sample / sync } else { p[4] as f64 / self.sr as f64 };
            for k in i..end {
                let x = l[k].abs().max(r[k].abs());
                let c = if x > self.env { att } else { rel };
                self.env = (x + c * (self.env - x)).min(1.0);
            }
            let base = self.cutoff.current();
            for _ in i..end {
                self.cutoff.next();
            }
            for (ch, f) in self.filters.iter_mut().enumerate() {
                let lfo = self.lfo.value(shape, if ch == 1 { stereo } else { 0.0 });
                f.set(self.sr, Self::modulated_cutoff(base, lfo, p[3], self.env, p[8]), p[2]);
            }
            for k in i..end {
                let m = self.mix.next();
                let (x, y) = (l[k], r[k]);
                l[k] = x * (1.0 - m) + self.filters[0].process(x, mode) * m;
                r[k] = y * (1.0 - m) + self.filters[1].process(y, mode) * m;
            }
            if !(sync > 0.0 && ctx.playing) {
                self.lfo.advance(cycles_per_sample * (end - i) as f64);
            }
            i = end;
        }
    }
}

processor!(MotionFilter, |s, io, ctx| s.process(io.l, io.r, ctx));

// ---------------------------------------------------------------- Chorus

pub const CHORUS_PARAMS: &[ParamDesc] = &[
    ParamDesc::log("rate", "Rate", 0.05, 5.0, 0.6, "Hz"),
    ParamDesc::lin("depth", "Depth", 0.0, 10.0, 3.0, "ms"),
    ParamDesc::lin("delay", "Delay", 1.0, 30.0, 8.0, "ms"),
    ParamDesc::lin("feedback", "Feedback", -0.9, 0.9, 0.0, ""),
    ParamDesc::lin("width", "Width", 0.0, 1.0, 1.0, ""),
    ParamDesc::lin("mix", "Mix", 0.0, 100.0, 50.0, "%"),
];

/// Longest delay a chorus tap can reach (Delay + Depth maxima, with margin).
const CHORUS_MAX_MS: f32 = 45.0;

pub struct Chorus {
    params: [f32; 6],
    lines: [Vec<f32>; 2],
    pos: usize,
    phase: f64,
    delay: Smoothed,
    depth: Smoothed,
    mix: Smoothed,
    sr: f32,
}

#[inline]
fn read_frac(line: &[f32], pos: usize, delay_samples: f32) -> f32 {
    let len = line.len();
    // `pos` holds the previous input sample, which is one sample back already.
    let d = delay_samples.clamp(1.0, (len - 2) as f32) - 1.0;
    let back = d.floor() as usize;
    let frac = d - back as f32;
    let a = line[(pos + len - back) % len];
    let b = line[(pos + len - back - 1) % len];
    a + (b - a) * frac
}

impl Chorus {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let len = (CHORUS_MAX_MS * 0.001 * sr) as usize + 4;
        let params: [f32; 6] = std::array::from_fn(|i| CHORUS_PARAMS[i].default);
        Self {
            lines: [vec![0.0; len], vec![0.0; len]],
            pos: 0,
            phase: 0.0,
            delay: Smoothed::new(params[2], sr, 50.0),
            depth: Smoothed::new(params[1], sr, 50.0),
            mix: Smoothed::new(params[5] / 100.0, sr, 20.0),
            params,
            sr,
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = CHORUS_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.depth.set_target(self.params[1]);
            self.delay.set_target(self.params[2]);
            self.mix.set_target(self.params[5] / 100.0);
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (fb, width) = (self.params[3], self.params[4]);
        let inc = self.params[0] as f64 / self.sr as f64;
        let ms = self.sr * 0.001;
        let len = self.lines[0].len();
        for i in 0..l.len() {
            let (base, depth, m) = (self.delay.next(), self.depth.next(), self.mix.next());
            let mut wet = [0.0f32; 2];
            for (ch, w) in wet.iter_mut().enumerate() {
                // Two taps per side, 90° apart; the right side is offset by a half cycle.
                let off = ch as f64 * 0.5;
                let mut acc = 0.0;
                for tap in 0..2 {
                    let ph = ((self.phase + off + tap as f64 * 0.25).fract() as f32 * TAU).sin();
                    acc += read_frac(&self.lines[ch], self.pos, (base + depth * 0.5 * (1.0 + ph)) * ms);
                }
                *w = acc * 0.5;
            }
            let mid = (wet[0] + wet[1]) * 0.5;
            let side = (wet[0] - wet[1]) * 0.5 * width;
            let (wl, wr) = (mid + side, mid - side);
            let next = (self.pos + 1) % len;
            self.lines[0][next] = l[i] + wl * fb;
            self.lines[1][next] = r[i] + wr * fb;
            self.pos = next;
            l[i] = l[i] * (1.0 - m) + wl * m;
            r[i] = r[i] * (1.0 - m) + wr * m;
            self.phase = (self.phase + inc).fract();
        }
    }
}

processor!(Chorus, |s, io, ctx| s.process(io.l, io.r));

// ---------------------------------------------------------------- Phaser

pub const PHASER_STAGES: &[&str] = &["4", "8", "12"];
const MAX_STAGES: usize = 12;

pub const PHASER_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("stages", "Stages", PHASER_STAGES, 1.0),
    ParamDesc::log("rate", "Rate", 0.05, 10.0, 0.4, "Hz"),
    ParamDesc::lin("depth", "Depth", 0.0, 4.0, 2.0, "oct"),
    ParamDesc::log("center", "Center", 100.0, 5_000.0, 800.0, "Hz"),
    ParamDesc::lin("feedback", "Feedback", -0.95, 0.95, 0.5, ""),
    ParamDesc::lin("spread", "Spread", 0.0, 180.0, 90.0, "°"),
    ParamDesc::lin("mix", "Mix", 0.0, 100.0, 50.0, "%"),
];

#[derive(Clone, Copy, Default)]
struct Allpass1 {
    x1: f32,
    y1: f32,
}

impl Allpass1 {
    #[inline]
    fn process(&mut self, x: f32, a: f32) -> f32 {
        let y = a * x + self.x1 - a * self.y1;
        self.x1 = x;
        self.y1 = if y.is_finite() { y } else { 0.0 };
        self.y1
    }
}

pub struct Phaser {
    params: [f32; 7],
    stages: [[Allpass1; MAX_STAGES]; 2],
    last: [f32; 2],
    phase: f64,
    mix: Smoothed,
    sr: f32,
}

impl Phaser {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 7] = std::array::from_fn(|i| PHASER_PARAMS[i].default);
        Self { mix: Smoothed::new(params[6] / 100.0, sr, 20.0), params, stages: [[Allpass1::default(); MAX_STAGES]; 2], last: [0.0; 2], phase: 0.0, sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = PHASER_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.mix.set_target(self.params[6] / 100.0);
        }
    }

    /// First-order all-pass coefficient placing the 90° point at `freq`.
    fn coef(freq: f32, sr: f32) -> f32 {
        let t = (std::f32::consts::PI * freq.clamp(20.0, sr * 0.45) / sr).tan();
        (t - 1.0) / (t + 1.0)
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let p = self.params;
        let count = [4, 8, 12][p[0] as usize];
        let inc = p[1] as f64 / self.sr as f64;
        let spread = (p[5] / 360.0) as f64;
        let n = l.len();
        let mut i = 0;
        while i < n {
            let end = (i + CONTROL).min(n);
            let mut a = [0.0f32; 2];
            for (ch, a) in a.iter_mut().enumerate() {
                let lfo = (((self.phase + ch as f64 * spread).fract()) as f32 * TAU).sin();
                *a = Self::coef(p[3] * 2f32.powf(lfo * p[2] * 0.5), self.sr);
            }
            for k in i..end {
                let m = self.mix.next();
                for (ch, x) in [&mut l[k], &mut r[k]].into_iter().enumerate() {
                    let mut y = *x + self.last[ch] * p[4];
                    for s in self.stages[ch][..count].iter_mut() {
                        y = s.process(y, a[ch]);
                    }
                    self.last[ch] = y.clamp(-4.0, 4.0);
                    *x = *x * (1.0 - m) + y * m;
                }
            }
            self.phase = (self.phase + inc * (end - i) as f64).fract();
            i = end;
        }
    }
}

processor!(Phaser, |s, io, ctx| s.process(io.l, io.r));

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize) -> Vec<f32> {
        let mut rng = Rng::new(7);
        (0..n).map(|_| rng.next_bipolar() * 0.3).collect()
    }

    #[test]
    fn lfo_shapes_stay_in_range_and_random_holds_per_cycle() {
        let mut lfo = Lfo::new(1);
        for shape in 0..LFO_SHAPES.len() {
            for k in 0..100 {
                lfo.set_phase(k as f64 * 0.37);
                let v = lfo.value(shape, 0.0);
                assert!((-1.0..=1.0).contains(&v));
            }
        }
        lfo.set_phase(3.1);
        let a = lfo.value(4, 0.0);
        lfo.set_phase(3.6);
        assert_eq!(lfo.value(4, 0.0), a, "random must hold within a cycle");
        lfo.set_phase(4.05);
        assert_ne!(lfo.value(4, 0.0), a);
    }

    #[test]
    fn motion_filter_cutoff_mapping() {
        assert_eq!(MotionFilter::modulated_cutoff(1000.0, 0.0, 1.0, 0.0, 1.0), 1000.0);
        // Full LFO swing is ±4 octaves at amount 1.
        assert!((MotionFilter::modulated_cutoff(1000.0, 1.0, 1.0, 0.0, 0.0) - 16_000.0).abs() < 1.0);
        assert!((MotionFilter::modulated_cutoff(1000.0, -0.5, 1.0, 0.0, 0.0) - 250.0).abs() < 0.1);
        // Envelope: +6 octaves at full level and amount, clamped to 20 kHz.
        assert_eq!(MotionFilter::modulated_cutoff(1000.0, 0.0, 0.0, 1.0, 1.0), 20_000.0);
    }

    #[test]
    fn motion_filter_synced_lfo_follows_song_position() {
        let sr = 48_000.0;
        let mut f = MotionFilter::new(sr);
        f.set_param(5, 5.0); // 1/4 = one cycle per beat
        let mut ctx = Ctx::stopped(sr, 120.0);
        ctx.playing = true;
        ctx.beat = 10.25;
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        f.process(&mut l, &mut r, &ctx);
        let expected = 10.25 + 48.0 * ctx.beats_per_sample;
        assert!((f.lfo.phase - expected).abs() < 1e-9, "phase {} expected {expected}", f.lfo.phase);
    }

    #[test]
    fn modulation_effects_are_finite_and_dry_at_zero_mix() {
        let sr = 48_000.0;
        let input = noise(9600);
        let ctx = Ctx::stopped(sr, 120.0);
        for which in 0..3 {
            let (mut l, mut r) = (input.clone(), input.clone());
            match which {
                0 => {
                    let mut d = MotionFilter::new(sr);
                    d.set_param(2, 0.95);
                    d.set_param(3, 1.0);
                    d.set_param(4, 20.0);
                    d.process(&mut l, &mut r, &ctx);
                }
                1 => {
                    let mut d = Chorus::new(sr);
                    d.set_param(3, 0.9);
                    d.process(&mut l, &mut r);
                }
                _ => {
                    let mut d = Phaser::new(sr);
                    d.set_param(0, 2.0);
                    d.set_param(4, 0.95);
                    d.process(&mut l, &mut r);
                }
            }
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 8.0), "device {which}");
        }
        let mut c = Chorus::new(sr);
        c.set_param(5, 0.0);
        let (mut l, mut r) = (input.clone(), input.clone());
        c.process(&mut l, &mut r);
        assert!(l[2000..].iter().zip(&input[2000..]).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn chorus_wet_is_a_delayed_copy() {
        // Depth 0, rate irrelevant: the wet signal is the input delayed by Delay ms.
        let sr = 48_000.0;
        let mut c = Chorus::new(sr);
        c.set_param(1, 0.0);
        c.set_param(2, 10.0);
        c.set_param(5, 100.0);
        let mut l = vec![0.0f32; 2000];
        l[100] = 1.0;
        let mut r = l.clone();
        // Let parameter ramps settle first.
        let (mut zl, mut zr) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
        c.process(&mut zl, &mut zr);
        c.process(&mut l, &mut r);
        let peak = l.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, 100 + 480);
    }

    #[test]
    fn phaser_is_allpass_when_fully_wet_without_feedback() {
        // All-pass stages keep the energy of broadband input.
        let sr = 48_000.0;
        let mut p = Phaser::new(sr);
        p.set_param(4, 0.0);
        p.set_param(6, 100.0);
        let input = noise(96_000);
        let (mut l, mut r) = (input.clone(), input.clone());
        let (mut zl, mut zr) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
        p.process(&mut zl, &mut zr);
        p.process(&mut l, &mut r);
        let e = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
        let ratio = e(&l[4800..]) / e(&input[4800..]);
        assert!((ratio - 1.0).abs() < 0.05, "energy ratio {ratio}");
    }
}
