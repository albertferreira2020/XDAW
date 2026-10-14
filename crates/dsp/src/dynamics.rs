//! Bus Compressor, Gate and Multiband Compressor (spec: docs/specs/devices.md D2, D3).

use crate::device::{Io, Processor};
use crate::effects::comp_curve;
use crate::filter::{Biquad, BiquadKind, Coeffs};
use crate::params::{Ctx, ParamDesc};
use crate::util::{db_to_gain, Smoothed};

#[inline]
fn coef(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms.max(0.001) * 0.001 * sr)).exp()
}

/// Detector sample `i`: the sidechain when routed, else the signal itself.
#[inline]
fn detector(sc: Option<(&[f32], &[f32])>, l: &[f32], r: &[f32], i: usize) -> (f32, f32) {
    match sc {
        Some((a, b)) => (a.get(i).copied().unwrap_or(0.0), b.get(i).copied().unwrap_or(0.0)),
        None => (l[i], r[i]),
    }
}

// ---------------------------------------------------------------- Bus Compressor

pub const BUS_RATIOS: &[&str] = &["2:1", "4:1", "10:1"];
const BUS_RATIO: [f32; 3] = [2.0, 4.0, 10.0];
pub const BUS_ATTACKS: &[&str] = &["0.1 ms", "0.3 ms", "1 ms", "3 ms", "10 ms", "30 ms"];
const BUS_ATTACK_MS: [f32; 6] = [0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
pub const BUS_RELEASES: &[&str] = &["0.1 s", "0.2 s", "0.4 s", "0.6 s", "0.8 s", "1.2 s", "Auto"];
const BUS_RELEASE_MS: [f32; 6] = [100.0, 200.0, 400.0, 600.0, 800.0, 1200.0];
const AUTO: usize = 6;

pub const BUS_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("threshold", "Threshold", -40.0, 0.0, -10.0, "dB"),
    ParamDesc::choice("ratio", "Ratio", BUS_RATIOS, 1.0),
    ParamDesc::choice("attack", "Attack", BUS_ATTACKS, 3.0),
    ParamDesc::choice("release", "Release", BUS_RELEASES, 6.0),
    ParamDesc::lin("makeup", "Makeup", 0.0, 20.0, 0.0, "dB"),
    ParamDesc::lin("mix", "Mix", 0.0, 100.0, 100.0, "%"),
    ParamDesc::toggle("soft_clip", "Soft Clip", false),
];

/// Release time (ms) of the Auto mode for an over-threshold run of `run_s` seconds:
/// short bursts release fast, sustained material slowly.
pub fn auto_release_ms(run_s: f32) -> f32 {
    100.0 + 1100.0 * (run_s / 1.0).clamp(0.0, 1.0)
}

pub struct BusComp {
    params: [f32; 7],
    ms: f32,
    env_db: f32,
    /// Seconds of the current (or last) run above threshold.
    run: f32,
    above: bool,
    makeup: Smoothed,
    mix: Smoothed,
    sr: f32,
    pub gain_reduction_db: f32,
}

impl BusComp {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 7] = std::array::from_fn(|i| BUS_PARAMS[i].default);
        Self { params, ms: 0.0, env_db: 0.0, run: 0.0, above: false, makeup: Smoothed::new(1.0, sr, 20.0), mix: Smoothed::new(1.0, sr, 20.0), sr, gain_reduction_db: 0.0 }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = BUS_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.makeup.set_target(db_to_gain(self.params[4]));
            self.mix.set_target(self.params[5] / 100.0);
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let p = self.params;
        let (thr, ratio) = (p[0], BUS_RATIO[p[1] as usize]);
        let att = coef(BUS_ATTACK_MS[p[2] as usize], self.sr);
        let rel_choice = p[3] as usize;
        let det = coef(1.0, self.sr);
        let dt = 1.0 / self.sr;
        let clip = p[6] >= 0.5;
        let mut max_gr: f32 = 0.0;
        for i in 0..l.len() {
            let (dl, dr) = detector(sc, l, r, i);
            let power = (dl * dl + dr * dr) * 0.5;
            self.ms = power + det * (self.ms - power);
            // +3 dB so a sine reads at its peak level.
            let level = 10.0 * (self.ms.max(1e-12) * 2.0).log10();
            let above = level > thr;
            if above {
                self.run = if self.above { self.run + dt } else { 0.0 };
            }
            self.above = above;
            let target = (level - thr).max(0.0) * (1.0 - 1.0 / ratio);
            let c = if target > self.env_db {
                att
            } else if rel_choice == AUTO {
                coef(auto_release_ms(self.run), self.sr)
            } else {
                coef(BUS_RELEASE_MS[rel_choice.min(5)], self.sr)
            };
            self.env_db = target + c * (self.env_db - target);
            if !self.env_db.is_finite() {
                self.env_db = 0.0;
            }
            max_gr = max_gr.max(self.env_db);
            let g = db_to_gain(-self.env_db) * self.makeup.next();
            let m = self.mix.next();
            for x in [&mut l[i], &mut r[i]] {
                let mut y = *x * g;
                if clip {
                    y = y.tanh();
                }
                *x = *x * (1.0 - m) + y * m;
            }
        }
        self.gain_reduction_db = max_gr;
    }
}

impl Processor for BusComp {
    fn set_param(&mut self, i: usize, v: f32) {
        BusComp::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.params
    }
    fn process(&mut self, io: Io, _ctx: &Ctx) {
        BusComp::process(self, io.l, io.r, io.sidechain)
    }
    fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction_db
    }
}

// ---------------------------------------------------------------- Gate

pub const GATE_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("threshold", "Threshold", -80.0, 0.0, -40.0, "dB"),
    ParamDesc::lin("return", "Return", 0.0, 24.0, 3.0, "dB"),
    ParamDesc::log("attack", "Attack", 0.02, 50.0, 0.5, "ms"),
    ParamDesc::lin("hold", "Hold", 0.0, 500.0, 20.0, "ms"),
    ParamDesc::log("release", "Release", 5.0, 2_000.0, 100.0, "ms"),
    ParamDesc::lin("floor", "Floor", -80.0, 0.0, -80.0, "dB"),
];

pub struct Gate {
    params: [f32; 6],
    peak: f32,
    open: bool,
    hold_left: u32,
    gain: f32,
    sr: f32,
    pub gain_reduction_db: f32,
}

impl Gate {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 6] = std::array::from_fn(|i| GATE_PARAMS[i].default);
        Self { gain: db_to_gain(params[5]), params, peak: 0.0, open: false, hold_left: 0, sr, gain_reduction_db: 0.0 }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = GATE_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let p = self.params;
        let open_at = db_to_gain(p[0]);
        let close_at = db_to_gain(p[0] - p[1]);
        let (att, rel) = (coef(p[2], self.sr), coef(p[4], self.sr));
        let decay = coef(10.0, self.sr);
        let hold = (p[3] * 0.001 * self.sr) as u32;
        let floor = db_to_gain(p[5]);
        let mut min_gain: f32 = 1.0;
        for i in 0..l.len() {
            let (dl, dr) = detector(sc, l, r, i);
            self.peak = dl.abs().max(dr.abs()).max(self.peak * decay);
            if self.peak >= open_at {
                self.open = true;
                self.hold_left = hold;
            } else if self.open && self.peak < close_at {
                if self.hold_left == 0 {
                    self.open = false;
                } else {
                    self.hold_left -= 1;
                }
            }
            let (target, c) = if self.open { (1.0, att) } else { (floor, rel) };
            self.gain = target + c * (self.gain - target);
            min_gain = min_gain.min(self.gain);
            l[i] *= self.gain;
            r[i] *= self.gain;
        }
        self.gain_reduction_db = -20.0 * min_gain.max(1e-6).log10();
    }
}

impl Processor for Gate {
    fn set_param(&mut self, i: usize, v: f32) {
        Gate::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.params
    }
    fn process(&mut self, io: Io, _ctx: &Ctx) {
        Gate::process(self, io.l, io.r, io.sidechain)
    }
    fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction_db
    }
}

// ---------------------------------------------------------------- Multiband Compressor

pub const MB_PARAMS: &[ParamDesc] = &[
    ParamDesc::log("low_xover", "Low X-Over", 50.0, 1_000.0, 200.0, "Hz"),
    ParamDesc::log("high_xover", "High X-Over", 1_000.0, 12_000.0, 2_500.0, "Hz"),
    ParamDesc::lin("low_threshold", "Low Thresh", -60.0, 0.0, -20.0, "dB"),
    ParamDesc::lin("low_ratio", "Low Ratio", 1.0, 20.0, 2.0, ":1"),
    ParamDesc::lin("low_gain", "Low Gain", -24.0, 24.0, 0.0, "dB"),
    ParamDesc::lin("mid_threshold", "Mid Thresh", -60.0, 0.0, -20.0, "dB"),
    ParamDesc::lin("mid_ratio", "Mid Ratio", 1.0, 20.0, 2.0, ":1"),
    ParamDesc::lin("mid_gain", "Mid Gain", -24.0, 24.0, 0.0, "dB"),
    ParamDesc::lin("high_threshold", "High Thresh", -60.0, 0.0, -20.0, "dB"),
    ParamDesc::lin("high_ratio", "High Ratio", 1.0, 20.0, 2.0, ":1"),
    ParamDesc::lin("high_gain", "High Gain", -24.0, 24.0, 0.0, "dB"),
    ParamDesc::log("attack", "Attack", 0.1, 200.0, 10.0, "ms"),
    ParamDesc::log("release", "Release", 5.0, 2_000.0, 150.0, "ms"),
    ParamDesc::lin("output", "Output", -24.0, 24.0, 0.0, "dB"),
];

/// 4th-order Linkwitz–Riley section: two identical Butterworth biquads.
#[derive(Clone, Copy, Default)]
struct Lr4([Biquad; 2]);

impl Lr4 {
    fn set(&mut self, c: Coeffs) {
        self.0[0].coeffs = c;
        self.0[1].coeffs = c;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.0[0].process(x);
        self.0[1].process(y)
    }
}

/// Per-channel crossover state. The low band also passes an all-pass made of
/// the high crossover's LP + HP, so all three bands share its phase and sum flat.
#[derive(Clone, Copy, Default)]
struct Crossover {
    lp1: Lr4,
    hp1: Lr4,
    lp2: Lr4,
    hp2: Lr4,
    ap_lp: Lr4,
    ap_hp: Lr4,
}

impl Crossover {
    fn set(&mut self, sr: f64, f1: f64, f2: f64) {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let (lp1, hp1) = (Coeffs::new(BiquadKind::LowPass, sr, f1, q, 0.0), Coeffs::new(BiquadKind::HighPass, sr, f1, q, 0.0));
        let (lp2, hp2) = (Coeffs::new(BiquadKind::LowPass, sr, f2, q, 0.0), Coeffs::new(BiquadKind::HighPass, sr, f2, q, 0.0));
        self.lp1.set(lp1);
        self.hp1.set(hp1);
        self.lp2.set(lp2);
        self.hp2.set(hp2);
        self.ap_lp.set(lp2);
        self.ap_hp.set(hp2);
    }

    #[inline]
    fn split(&mut self, x: f32) -> [f32; 3] {
        let low = self.lp1.process(x);
        let rest = self.hp1.process(x);
        let low = self.ap_lp.process(low) + self.ap_hp.process(low);
        [low, self.lp2.process(rest), self.hp2.process(rest)]
    }
}

pub struct Multiband {
    params: [f32; 14],
    xover: [Crossover; 2],
    env_db: [f32; 3],
    output: Smoothed,
    gains: [Smoothed; 3],
    sr: f32,
    pub gain_reduction_db: f32,
}

impl Multiband {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 14] = std::array::from_fn(|i| MB_PARAMS[i].default);
        let mut m = Self { params, xover: Default::default(), env_db: [0.0; 3], output: Smoothed::new(1.0, sr, 20.0), gains: [Smoothed::new(1.0, sr, 20.0); 3], sr, gain_reduction_db: 0.0 };
        m.update_crossovers();
        m
    }

    /// Crossover frequencies, keeping the high one at least 1.5× the low one.
    pub fn crossovers(&self) -> (f32, f32) {
        let f1 = self.params[0];
        (f1, self.params[1].max(f1 * 1.5))
    }

    fn update_crossovers(&mut self) {
        let (f1, f2) = self.crossovers();
        for x in self.xover.iter_mut() {
            x.set(self.sr as f64, f1 as f64, f2 as f64);
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        let Some(d) = MB_PARAMS.get(i) else { return };
        self.params[i] = d.clamp(v);
        match i {
            0 | 1 => self.update_crossovers(),
            4 | 7 | 10 => self.gains[(i - 4) / 3].set_target(db_to_gain(self.params[i])),
            13 => self.output.set_target(db_to_gain(self.params[13])),
            _ => {}
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let p = self.params;
        let (att, rel) = (coef(p[11], self.sr), coef(p[12], self.sr));
        let mut max_gr: f32 = 0.0;
        for i in 0..l.len() {
            let bl = self.xover[0].split(l[i]);
            let br = self.xover[1].split(r[i]);
            let out = self.output.next();
            let (mut yl, mut yr) = (0.0, 0.0);
            for b in 0..3 {
                let (thr, ratio) = (p[2 + b * 3], p[3 + b * 3]);
                let peak = bl[b].abs().max(br[b].abs()).max(1e-6);
                let target = comp_curve(20.0 * peak.log10(), thr, ratio, 6.0);
                let c = if target > self.env_db[b] { att } else { rel };
                self.env_db[b] = target + c * (self.env_db[b] - target);
                if !self.env_db[b].is_finite() {
                    self.env_db[b] = 0.0;
                }
                max_gr = max_gr.max(self.env_db[b]);
                let g = db_to_gain(-self.env_db[b]) * self.gains[b].next();
                yl += bl[b] * g;
                yr += br[b] * g;
            }
            l[i] = yl * out;
            r[i] = yr * out;
        }
        self.gain_reduction_db = max_gr;
    }
}

impl Processor for Multiband {
    fn set_param(&mut self, i: usize, v: f32) {
        Multiband::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.params
    }
    fn process(&mut self, io: Io, _ctx: &Ctx) {
        Multiband::process(self, io.l, io.r)
    }
    fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SR).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn bus_comp_reduces_by_the_ratio_in_steady_state() {
        // Sine at -4 dBFS peak, threshold -16, 4:1 → 12 dB over → 9 dB reduction.
        let mut c = BusComp::new(SR);
        c.set_param(0, -16.0);
        c.set_param(3, 0.0); // 0.1 s release
        let amp = db_to_gain(-4.0);
        let mut l = sine(500.0, 96_000, amp);
        let mut r = l.clone();
        c.process(&mut l, &mut r, None);
        let out_db = 20.0 * (rms(&l[48_000..]) * std::f32::consts::SQRT_2).log10();
        assert!((out_db - (-13.0)).abs() < 0.6, "output {out_db} dB, expected -13");
    }

    #[test]
    fn bus_comp_auto_release_depends_on_how_long_the_signal_was_loud() {
        assert_eq!(auto_release_ms(0.0), 100.0);
        assert_eq!(auto_release_ms(5.0), 1200.0);
        assert!(auto_release_ms(0.3) > 300.0 && auto_release_ms(0.3) < 500.0);
    }

    #[test]
    fn bus_comp_soft_clip_never_exceeds_full_scale() {
        let mut c = BusComp::new(SR);
        c.set_param(0, 0.0);
        c.set_param(4, 20.0);
        c.set_param(6, 1.0);
        let mut l = sine(100.0, 9600, 1.0);
        let mut r = l.clone();
        c.process(&mut l, &mut r, None);
        assert!(l.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn sidechain_drives_the_detector() {
        // Silent sidechain: no reduction. Loud sidechain: reduction although the input is quiet.
        let quiet = sine(500.0, 48_000, 0.05);
        let loud = sine(60.0, 48_000, 1.0);
        let silent = vec![0.0f32; 48_000];
        let run = |sc: &[f32]| {
            let mut c = BusComp::new(SR);
            c.set_param(0, -20.0);
            let (mut l, mut r) = (quiet.clone(), quiet.clone());
            c.process(&mut l, &mut r, Some((sc, sc)));
            rms(&l[24_000..]) / rms(&quiet[24_000..])
        };
        assert!((run(&silent) - 1.0).abs() < 1e-3);
        assert!(run(&loud) < 0.35);
    }

    #[test]
    fn gate_closes_to_floor_and_respects_hold_and_return() {
        let mut g = Gate::new(SR);
        g.set_param(0, -30.0);
        g.set_param(3, 50.0); // hold 50 ms
        g.set_param(4, 5.0); // fast release
        // 100 ms loud, then 200 ms at -40 dB (below threshold - return).
        let mut l: Vec<f32> = sine(300.0, 4800, 0.5).into_iter().chain(sine(300.0, 9600, 0.01)).collect();
        let mut r = l.clone();
        g.process(&mut l[..4800], &mut r[..4800], None);
        assert!(g.is_open());
        g.process(&mut l[4800..4800 + 1920], &mut r[4800..4800 + 1920], None); // 40 ms < hold + peak decay
        assert!(g.is_open(), "hold keeps the gate open");
        g.process(&mut l[4800 + 1920..], &mut r[4800 + 1920..], None);
        assert!(!g.is_open());
        let tail = rms(&l[l.len() - 2400..]);
        assert!(tail < 1e-5, "closed gate must reach the -80 dB floor, rms {tail}");
    }

    #[test]
    fn multiband_sums_flat_when_not_compressing() {
        // Thresholds at 0 dB and quiet input: bands only split and recombine.
        for f in [40.0, 150.0, 200.0, 700.0, 2_500.0, 6_000.0, 15_000.0] {
            let mut m = Multiband::new(SR);
            for t in [2, 5, 8] {
                m.set_param(t, 0.0);
            }
            let input = sine(f, 48_000, 0.1);
            let (mut l, mut r) = (input.clone(), input.clone());
            m.process(&mut l, &mut r);
            let db = 20.0 * (rms(&l[24_000..]) / rms(&input[24_000..])).log10();
            assert!(db.abs() < 0.1, "{f} Hz: {db} dB");
        }
    }

    #[test]
    fn multiband_compresses_only_the_band_that_is_loud() {
        let mut m = Multiband::new(SR);
        m.set_param(3, 20.0); // low ratio 20:1
        m.set_param(2, -30.0);
        let lowsig = sine(60.0, 48_000, 0.5);
        let highsig = sine(8_000.0, 48_000, 0.01);
        let run = |m: &mut Multiband, x: &[f32]| {
            let (mut l, mut r) = (x.to_vec(), x.to_vec());
            m.process(&mut l, &mut r);
            rms(&l[24_000..]) / rms(&x[24_000..])
        };
        assert!(run(&mut m, &lowsig) < 0.2);
        let mut m2 = Multiband::new(SR);
        m2.set_param(3, 20.0);
        m2.set_param(2, -30.0);
        assert!((run(&mut m2, &highsig) - 1.0).abs() < 0.02);
    }
}
