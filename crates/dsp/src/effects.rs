//! Original audio effects. Buffers are allocated at construction only.

use crate::filter::{Biquad, BiquadKind, Coeffs, Svf, SvfMode};
use crate::params::{Ctx, ParamDesc};
use crate::util::{db_to_gain, pan_gains, Smoothed};

// ---------------------------------------------------------------- Gain / utility

pub const GAIN_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("gain", "Gain", -36.0, 36.0, 0.0, "dB"),
    ParamDesc::lin("pan", "Pan", -1.0, 1.0, 0.0, ""),
    ParamDesc::lin("width", "Width", 0.0, 2.0, 1.0, ""),
];

pub struct GainFx {
    gain: Smoothed,
    left: Smoothed,
    right: Smoothed,
    width: Smoothed,
    params: [f32; 3],
}

impl GainFx {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let (pl, pr) = pan_gains(0.0);
        let norm = std::f32::consts::SQRT_2;
        Self {
            gain: Smoothed::new(1.0, sr, 20.0),
            left: Smoothed::new(pl * norm, sr, 20.0),
            right: Smoothed::new(pr * norm, sr, 20.0),
            width: Smoothed::new(1.0, sr, 20.0),
            params: [0.0, 0.0, 1.0],
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        let Some(d) = GAIN_PARAMS.get(i) else { return };
        self.params[i] = d.clamp(v);
        self.gain.set_target(db_to_gain(self.params[0]));
        // Utility-style balance: centre is unity on both sides.
        let (pl, pr) = pan_gains(self.params[1]);
        self.left.set_target(pl * std::f32::consts::SQRT_2);
        self.right.set_target(pr * std::f32::consts::SQRT_2);
        self.width.set_target(self.params[2]);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let g = self.gain.next();
            let w = self.width.next();
            let (mid, side) = ((l[i] + r[i]) * 0.5, (l[i] - r[i]) * 0.5 * w);
            l[i] = (mid + side) * g * self.left.next().min(1.0);
            r[i] = (mid - side) * g * self.right.next().min(1.0);
        }
    }
}

// ---------------------------------------------------------------- 3-band EQ

pub const EQ_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("low_gain", "Low Gain", -18.0, 18.0, 0.0, "dB"),
    ParamDesc::log("low_freq", "Low Freq", 30.0, 1_000.0, 120.0, "Hz"),
    ParamDesc::lin("mid_gain", "Mid Gain", -18.0, 18.0, 0.0, "dB"),
    ParamDesc::log("mid_freq", "Mid Freq", 100.0, 10_000.0, 1_000.0, "Hz"),
    ParamDesc::lin("mid_q", "Mid Q", 0.2, 8.0, 0.8, ""),
    ParamDesc::lin("high_gain", "High Gain", -18.0, 18.0, 0.0, "dB"),
    ParamDesc::log("high_freq", "High Freq", 1_000.0, 18_000.0, 6_000.0, "Hz"),
];

pub struct Eq3 {
    params: [f32; 7],
    smoothed: [Smoothed; 7],
    bands: [[Biquad; 3]; 2],
    sr: f32,
}

impl Eq3 {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; 7] = std::array::from_fn(|i| EQ_PARAMS[i].default);
        let smoothed = std::array::from_fn(|i| Smoothed::new(params[i], sr, 30.0));
        let mut eq = Self { params, smoothed, bands: [[Biquad::default(); 3]; 2], sr };
        eq.update_coeffs();
        eq
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = EQ_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.smoothed[i].set_target(self.params[i]);
        }
    }

    fn update_coeffs(&mut self) {
        let p: [f32; 7] = std::array::from_fn(|i| self.smoothed[i].current());
        let sr = self.sr as f64;
        let c = [
            Coeffs::new(BiquadKind::LowShelf, sr, p[1] as f64, 0.707, p[0] as f64),
            Coeffs::new(BiquadKind::Peak, sr, p[3] as f64, p[4] as f64, p[2] as f64),
            Coeffs::new(BiquadKind::HighShelf, sr, p[6] as f64, 0.707, p[5] as f64),
        ];
        for ch in self.bands.iter_mut() {
            for (b, c) in ch.iter_mut().zip(c) {
                b.coeffs = c;
            }
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (n, (lc, rc)) in l.chunks_mut(16).zip(r.chunks_mut(16)).enumerate() {
            let _ = n;
            let ramping = self.smoothed.iter().any(Smoothed::is_ramping);
            for s in self.smoothed.iter_mut() {
                for _ in 0..lc.len() {
                    s.next();
                }
            }
            if ramping {
                self.update_coeffs();
            }
            for (ch, buf) in [lc, rc].into_iter().enumerate() {
                for x in buf.iter_mut() {
                    let mut y = *x;
                    for b in self.bands[ch].iter_mut() {
                        y = b.process(y);
                    }
                    *x = y;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- Filter

pub const FILTER_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("mode", "Mode", &["Low-pass", "High-pass", "Band-pass"], 0.0),
    ParamDesc::log("cutoff", "Cutoff", 30.0, 18_000.0, 18_000.0, "Hz"),
    ParamDesc::lin("resonance", "Resonance", 0.0, 0.95, 0.1, ""),
];

pub struct FilterFx {
    params: [f32; 3],
    mode: SvfMode,
    cutoff: Smoothed,
    resonance: f32,
    filters: [Svf; 2],
    sr: f32,
}

impl FilterFx {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        Self { params: [0.0, 18_000.0, 0.1], mode: SvfMode::LowPass, cutoff: Smoothed::new(18_000.0, sr, 30.0), resonance: 0.1, filters: [Svf::default(); 2], sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        let Some(d) = FILTER_PARAMS.get(i) else { return };
        let v = d.clamp(v);
        self.params[i] = v;
        match i {
            0 => {
                self.mode = match v as u32 {
                    0 => SvfMode::LowPass,
                    1 => SvfMode::HighPass,
                    _ => SvfMode::BandPass,
                }
            }
            1 => self.cutoff.set_target(v),
            _ => self.resonance = v,
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let c = self.cutoff.next();
            if i & 7 == 0 {
                for f in self.filters.iter_mut() {
                    f.set(self.sr, c, self.resonance);
                }
            }
            l[i] = self.filters[0].process(l[i], self.mode);
            r[i] = self.filters[1].process(r[i], self.mode);
        }
    }
}

// ---------------------------------------------------------------- Compressor

pub const COMP_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("threshold", "Threshold", -60.0, 0.0, -18.0, "dB"),
    ParamDesc::lin("ratio", "Ratio", 1.0, 20.0, 4.0, ":1"),
    ParamDesc::log("attack", "Attack", 0.1, 200.0, 10.0, "ms"),
    ParamDesc::log("release", "Release", 5.0, 2_000.0, 120.0, "ms"),
    ParamDesc::lin("knee", "Knee", 0.0, 18.0, 6.0, "dB"),
    ParamDesc::lin("makeup", "Makeup", -12.0, 24.0, 0.0, "dB"),
];

/// Feed-forward, stereo-linked peak compressor with a soft knee computed in dB.
pub struct Compressor {
    params: [f32; 6],
    env_db: f32,
    attack: f32,
    release: f32,
    makeup: Smoothed,
    sr: f32,
    /// Most recent gain reduction in dB (positive), for metering.
    pub gain_reduction_db: f32,
}

impl Compressor {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params = std::array::from_fn(|i| COMP_PARAMS[i].default);
        let mut c = Self { params, env_db: 0.0, attack: 0.0, release: 0.0, makeup: Smoothed::new(1.0, sr, 20.0), sr, gain_reduction_db: 0.0 };
        c.update();
        c
    }

    fn update(&mut self) {
        self.attack = (-1.0 / (self.params[2] * 0.001 * self.sr)).exp();
        self.release = (-1.0 / (self.params[3] * 0.001 * self.sr)).exp();
        self.makeup.set_target(db_to_gain(self.params[5]));
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = COMP_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.update();
        }
    }

    /// Static curve: gain reduction in dB for an input level in dB.
    pub fn curve(&self, x_db: f32) -> f32 {
        let (t, ratio, knee) = (self.params[0], self.params[1], self.params[4]);
        let over = x_db - t;
        let slope = 1.0 - 1.0 / ratio;
        if 2.0 * over < -knee {
            0.0
        } else if knee > 0.0 && 2.0 * over.abs() <= knee {
            slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
        } else {
            slope * over
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut max_gr: f32 = 0.0;
        for i in 0..l.len() {
            let peak = l[i].abs().max(r[i].abs()).max(1e-6);
            let target = self.curve(20.0 * peak.log10());
            let coef = if target > self.env_db { self.attack } else { self.release };
            self.env_db = target + coef * (self.env_db - target);
            if !self.env_db.is_finite() {
                self.env_db = 0.0;
            }
            max_gr = max_gr.max(self.env_db);
            let g = db_to_gain(-self.env_db) * self.makeup.next();
            l[i] *= g;
            r[i] *= g;
        }
        self.gain_reduction_db = max_gr;
    }
}

// ---------------------------------------------------------------- Delay

pub const DELAY_SYNC_NAMES: &[&str] = &["1/16", "1/8", "1/8 dot", "1/4", "1/4 dot", "1/2", "1 Bar"];
const DELAY_SYNC_BEATS: [f32; 7] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0];

pub const DELAY_PARAMS: &[ParamDesc] = &[
    ParamDesc::toggle("sync", "Sync", true),
    ParamDesc::choice("division", "Division", DELAY_SYNC_NAMES, 2.0),
    ParamDesc::log("time_ms", "Time", 1.0, 2_000.0, 250.0, "ms"),
    ParamDesc::lin("feedback", "Feedback", 0.0, 0.95, 0.4, ""),
    ParamDesc::toggle("ping_pong", "Ping Pong", true),
    ParamDesc::log("damp", "Damping", 500.0, 18_000.0, 6_000.0, "Hz"),
    ParamDesc::lin("mix", "Dry/Wet", 0.0, 1.0, 0.3, ""),
];

/// Stereo delay with tempo sync. Time changes glide (tape-like) to avoid clicks.
pub struct Delay {
    params: [f32; 7],
    buf: [Vec<f32>; 2],
    write: usize,
    time: Smoothed,
    mix: Smoothed,
    damp: [Biquad; 2],
    sr: f32,
}

pub const MAX_DELAY_SECONDS: f32 = 4.0;

impl Delay {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let n = (sr * MAX_DELAY_SECONDS) as usize + 4;
        let params = std::array::from_fn(|i| DELAY_PARAMS[i].default);
        let mut d = Self {
            params,
            buf: [vec![0.0; n], vec![0.0; n]],
            write: 0,
            time: Smoothed::new(0.0, sr, 80.0),
            mix: Smoothed::new(params[6], sr, 20.0),
            damp: [Biquad::default(); 2],
            sr,
        };
        d.update_damp();
        d
    }

    fn update_damp(&mut self) {
        let c = Coeffs::new(BiquadKind::LowPass, self.sr as f64, self.params[5] as f64, 0.707, 0.0);
        self.damp = [Biquad::new(c), Biquad::new(c)];
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = DELAY_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            match i {
                5 => {
                    let c = Coeffs::new(BiquadKind::LowPass, self.sr as f64, self.params[5] as f64, 0.707, 0.0);
                    self.damp[0].coeffs = c;
                    self.damp[1].coeffs = c;
                }
                6 => self.mix.set_target(self.params[6]),
                _ => {}
            }
        }
    }

    fn delay_samples(&self, bpm: f32) -> f32 {
        let secs = if self.params[0] >= 0.5 {
            DELAY_SYNC_BEATS[self.params[1] as usize] * 60.0 / bpm.max(1.0)
        } else {
            self.params[2] * 0.001
        };
        (secs * self.sr).clamp(1.0, self.buf[0].len() as f32 - 3.0)
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        let target = self.delay_samples(ctx.bpm);
        if self.time.current() == 0.0 {
            self.time.reset(target);
        }
        self.time.set_target(target);
        let n = self.buf[0].len();
        let fb = self.params[3];
        let ping = self.params[4] >= 0.5;
        for i in 0..l.len() {
            let d = self.time.next();
            let read = (self.write as f32 + n as f32 - d) % n as f32;
            let i0 = read.floor() as usize;
            let t = read - i0 as f32;
            let i1 = (i0 + 1) % n;
            let wet_l = self.buf[0][i0] * (1.0 - t) + self.buf[0][i1] * t;
            let wet_r = self.buf[1][i0] * (1.0 - t) + self.buf[1][i1] * t;
            let fl = self.damp[0].process(wet_l) * fb;
            let fr = self.damp[1].process(wet_r) * fb;
            let (in_l, in_r) = (l[i], r[i]);
            if ping {
                self.buf[0][self.write] = (in_l + in_r) * 0.5 + fr;
                self.buf[1][self.write] = fl;
            } else {
                self.buf[0][self.write] = in_l + fl;
                self.buf[1][self.write] = in_r + fr;
            }
            for ch in 0..2 {
                if !self.buf[ch][self.write].is_finite() {
                    self.buf[ch][self.write] = 0.0;
                }
            }
            self.write = (self.write + 1) % n;
            let m = self.mix.next();
            l[i] = in_l * (1.0 - m) + wet_l * m;
            r[i] = in_r * (1.0 - m) + wet_r * m;
        }
    }
}

// ---------------------------------------------------------------- Reverb

pub const REVERB_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("size", "Size", 0.0, 1.0, 0.6, ""),
    ParamDesc::lin("damping", "Damping", 0.0, 1.0, 0.4, ""),
    ParamDesc::lin("width", "Width", 0.0, 1.0, 1.0, ""),
    ParamDesc::lin("predelay", "Pre-delay", 0.0, 200.0, 10.0, "ms"),
    ParamDesc::lin("mix", "Dry/Wet", 0.0, 1.0, 0.25, ""),
];

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.idx];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        b - x
    }
}

/// Schroeder/Moorer network (parallel damped combs into series allpasses),
/// tunings scaled from the public-domain Freeverb design to the sample rate.
pub struct Reverb {
    params: [f32; 5],
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    pre: Vec<f32>,
    pre_idx: usize,
    mix: Smoothed,
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

impl Reverb {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let mk = |spread: usize| -> (Vec<Comb>, Vec<Allpass>) {
            (
                COMB_TUNING
                    .iter()
                    .map(|&t| Comb { buf: vec![0.0; (((t + spread) as f32) * scale) as usize], idx: 0, store: 0.0 })
                    .collect(),
                ALLPASS_TUNING
                    .iter()
                    .map(|&t| Allpass { buf: vec![0.0; (((t + spread) as f32) * scale) as usize], idx: 0 })
                    .collect(),
            )
        };
        let (cl, al) = mk(0);
        let (cr, ar) = mk(STEREO_SPREAD);
        let params = std::array::from_fn(|i| REVERB_PARAMS[i].default);
        Self {
            params,
            combs: [cl, cr],
            allpasses: [al, ar],
            pre: vec![0.0; (sr * 0.2) as usize + 1],
            pre_idx: 0,
            mix: Smoothed::new(params[4], sr, 20.0),
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = REVERB_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            if i == 4 {
                self.mix.set_target(self.params[4]);
            }
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        let feedback = 0.7 + 0.28 * self.params[0];
        let damp = self.params[1] * 0.4;
        let width = self.params[2];
        let (wet1, wet2) = (0.5 + width / 2.0, (1.0 - width) / 2.0);
        let pre_len = ((self.params[3] * 0.001 * ctx.sample_rate) as usize).min(self.pre.len() - 1);
        for i in 0..l.len() {
            let input = (l[i] + r[i]) * 0.015;
            // Pre-delay line.
            self.pre[self.pre_idx] = input;
            let read = (self.pre_idx + self.pre.len() - pre_len) % self.pre.len();
            let x = self.pre[read];
            self.pre_idx = (self.pre_idx + 1) % self.pre.len();
            let mut out = [0.0f32; 2];
            for ch in 0..2 {
                let mut acc = 0.0;
                for c in self.combs[ch].iter_mut() {
                    acc += c.process(x, feedback, damp);
                }
                for a in self.allpasses[ch].iter_mut() {
                    acc = a.process(acc);
                }
                out[ch] = if acc.is_finite() { acc } else { 0.0 };
            }
            let m = self.mix.next();
            let (ol, or) = (out[0] * wet1 + out[1] * wet2, out[1] * wet1 + out[0] * wet2);
            l[i] = l[i] * (1.0 - m) + ol * m;
            r[i] = r[i] * (1.0 - m) + or * m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: Ctx = Ctx { sample_rate: 48_000.0, bpm: 120.0 };

    #[test]
    fn silence_in_silence_out_for_all_effects() {
        let (mut l, mut r) = (vec![0.0f32; 4096], vec![0.0f32; 4096]);
        Eq3::new(48_000.0).process(&mut l, &mut r);
        FilterFx::new(48_000.0).process(&mut l, &mut r);
        Compressor::new(48_000.0).process(&mut l, &mut r);
        Delay::new(48_000.0).process(&mut l, &mut r, &CTX);
        Reverb::new(48_000.0).process(&mut l, &mut r, &CTX);
        GainFx::new(48_000.0).process(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|&x| x == 0.0));
    }

    #[test]
    fn gain_utility_is_unity_at_default_and_applies_db() {
        let mut g = GainFx::new(48_000.0);
        let (mut l, mut r) = (vec![0.5; 64], vec![0.25; 64]);
        g.process(&mut l, &mut r);
        assert!((l[63] - 0.5).abs() < 1e-5 && (r[63] - 0.25).abs() < 1e-5);
        g.set_param(0, -6.0206);
        let (mut l, mut r) = (vec![1.0; 2000], vec![1.0; 2000]);
        g.process(&mut l, &mut r);
        assert!((l[1999] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn delay_impulse_arrives_at_synced_time() {
        let mut d = Delay::new(48_000.0);
        d.set_param(4, 0.0); // no ping-pong
        d.set_param(6, 1.0); // fully wet
        d.set_param(1, 3.0); // 1/4 at 120 BPM = 24000 samples
        let n = 30_000;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        l[0] = 1.0;
        r[0] = 1.0;
        // Mix smoothing: skip; check echo location only.
        d.process(&mut l, &mut r, &CTX);
        let peak = l.iter().enumerate().skip(100).max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as i64 - 24_000).abs() <= 1, "echo at {peak}");
    }

    #[test]
    fn compressor_static_curve_and_reduction() {
        let mut c = Compressor::new(48_000.0);
        c.set_param(0, -20.0);
        c.set_param(1, 4.0);
        c.set_param(4, 0.0);
        assert_eq!(c.curve(-30.0), 0.0);
        assert!((c.curve(-8.0) - 9.0).abs() < 1e-4); // 12 dB over at 4:1 -> 9 dB reduction
        let (mut l, mut r) = (vec![0.5f32; 48_000], vec![0.5f32; 48_000]); // -6 dBFS
        c.process(&mut l, &mut r);
        let out_db = 20.0 * l[47_999].log10();
        assert!((out_db - (-16.5)).abs() < 0.2, "{out_db}");
    }

    #[test]
    fn reverb_decays_and_stays_finite() {
        let mut rv = Reverb::new(48_000.0);
        rv.set_param(4, 1.0);
        let n = 48_000 * 6;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        l[0] = 1.0;
        rv.process(&mut l, &mut r, &CTX);
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        let early = energy(&l[0..48_000]);
        let late = energy(&l[n - 48_000..]);
        assert!(early > 0.0 && late < early * 1e-3, "early {early} late {late}");
        assert!(l.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn effects_survive_non_finite_input() {
        let (mut l, mut r) = (vec![f32::NAN; 256], vec![f32::INFINITY; 256]);
        let mut eq = Eq3::new(48_000.0);
        eq.set_param(0, 6.0);
        eq.process(&mut l, &mut r);
        let (mut l, mut r) = (vec![0.1; 256], vec![0.1; 256]);
        eq.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
    }
}
