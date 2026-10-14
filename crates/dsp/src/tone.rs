//! Tone shaping: Parametric EQ and Saturator (spec: docs/specs/devices.md D2).

use crate::device::{processor, Io, Processor};
use crate::filter::{Biquad, BiquadKind, Coeffs, DcBlocker};
use crate::params::{Ctx, ParamDesc};
use crate::util::{db_to_gain, Smoothed};

// ---------------------------------------------------------------- Parametric EQ

pub const PEQ_BANDS: usize = 8;
/// Parameters per band: On, Type, Freq, Gain, Q.
pub const PEQ_STRIDE: usize = 5;
pub const PEQ_TYPES: &[&str] = &["Low Cut 12", "Low Cut 24", "Low Shelf", "Bell", "Notch", "High Shelf", "High Cut 12", "High Cut 24"];
const BELL: f32 = 3.0;

macro_rules! peq_band {
    ($n:literal, $ty:expr, $freq:expr) => {
        [
            ParamDesc::toggle(concat!("b", $n, "_on"), concat!($n, " On"), false),
            ParamDesc::choice(concat!("b", $n, "_type"), concat!($n, " Type"), PEQ_TYPES, $ty),
            ParamDesc::log(concat!("b", $n, "_freq"), concat!($n, " Freq"), 20.0, 20_000.0, $freq, "Hz"),
            ParamDesc::lin(concat!("b", $n, "_gain"), concat!($n, " Gain"), -15.0, 15.0, 0.0, "dB"),
            ParamDesc::log(concat!("b", $n, "_q"), concat!($n, " Q"), 0.1, 18.0, 0.71, ""),
        ]
    };
}

const fn flatten_bands(bands: [[ParamDesc; PEQ_STRIDE]; PEQ_BANDS], output: ParamDesc) -> [ParamDesc; PEQ_BANDS * PEQ_STRIDE + 1] {
    let mut out = [output; PEQ_BANDS * PEQ_STRIDE + 1];
    let mut b = 0;
    while b < PEQ_BANDS {
        let mut k = 0;
        while k < PEQ_STRIDE {
            out[b * PEQ_STRIDE + k] = bands[b][k];
            k += 1;
        }
        b += 1;
    }
    out
}

const PEQ_TABLE: [ParamDesc; PEQ_BANDS * PEQ_STRIDE + 1] = flatten_bands(
    [
        peq_band!(1, 0.0, 30.0),
        peq_band!(2, BELL, 100.0),
        peq_band!(3, BELL, 250.0),
        peq_band!(4, BELL, 600.0),
        peq_band!(5, BELL, 1_500.0),
        peq_band!(6, BELL, 3_500.0),
        peq_band!(7, BELL, 8_000.0),
        peq_band!(8, 6.0, 16_000.0),
    ],
    ParamDesc::lin("output", "Output", -24.0, 24.0, 0.0, "dB"),
);
pub const PEQ_PARAMS: &[ParamDesc] = &PEQ_TABLE;
const PEQ_N: usize = PEQ_BANDS * PEQ_STRIDE + 1;

/// Coefficients of one band: two biquad stages (the second is identity
/// unless the type is a 24 dB/oct cut).
pub fn peq_band_coeffs(kind: usize, sr: f64, freq: f64, gain_db: f64, q: f64) -> [Coeffs; 2] {
    // Second stage of a 4th-order Butterworth pair; Q = 0.71 gives the flat response.
    let q2 = q * 1.848;
    let q1 = 0.541;
    match kind {
        0 => [Coeffs::new(BiquadKind::HighPass, sr, freq, q, 0.0), Coeffs::IDENTITY],
        1 => [Coeffs::new(BiquadKind::HighPass, sr, freq, q1, 0.0), Coeffs::new(BiquadKind::HighPass, sr, freq, q2, 0.0)],
        2 => [Coeffs::new(BiquadKind::LowShelf, sr, freq, q, gain_db), Coeffs::IDENTITY],
        3 => [Coeffs::new(BiquadKind::Peak, sr, freq, q, gain_db), Coeffs::IDENTITY],
        4 => [Coeffs::new(BiquadKind::Notch, sr, freq, q, 0.0), Coeffs::IDENTITY],
        5 => [Coeffs::new(BiquadKind::HighShelf, sr, freq, q, gain_db), Coeffs::IDENTITY],
        6 => [Coeffs::new(BiquadKind::LowPass, sr, freq, q, 0.0), Coeffs::IDENTITY],
        _ => [Coeffs::new(BiquadKind::LowPass, sr, freq, q1, 0.0), Coeffs::new(BiquadKind::LowPass, sr, freq, q2, 0.0)],
    }
}

/// Summed magnitude in dB at `freq` for a parameter set (UI curve, tests).
pub fn peq_response_db(params: &[f32], sr: f64, freq: f64) -> f64 {
    let p = |i: usize| params.get(i).copied().unwrap_or(PEQ_PARAMS[i].default);
    let mut db = p(PEQ_N - 1) as f64;
    for b in 0..PEQ_BANDS {
        let o = b * PEQ_STRIDE;
        if p(o) < 0.5 {
            continue;
        }
        for c in peq_band_coeffs(p(o + 1) as usize, sr, p(o + 2) as f64, p(o + 3) as f64, p(o + 4) as f64) {
            db += 20.0 * c.magnitude(freq, sr).max(1e-12).log10();
        }
    }
    db
}

pub struct ParamEq {
    params: [f32; PEQ_N],
    /// Smoothed Freq, Gain and Q per band.
    smooth: [[Smoothed; 3]; PEQ_BANDS],
    output: Smoothed,
    stages: [[[Biquad; 2]; PEQ_BANDS]; 2],
    dirty: bool,
    sr: f32,
}

impl ParamEq {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; PEQ_N] = std::array::from_fn(|i| PEQ_PARAMS[i].default);
        let smooth = std::array::from_fn(|b| std::array::from_fn(|k| Smoothed::new(params[b * PEQ_STRIDE + 2 + k], sr, 30.0)));
        let mut eq = Self { params, smooth, output: Smoothed::new(1.0, sr, 20.0), stages: Default::default(), dirty: true, sr };
        eq.update_coeffs();
        eq
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        let Some(d) = PEQ_PARAMS.get(i) else { return };
        let v = d.clamp(v);
        let changed = self.params[i] != v;
        self.params[i] = v;
        if i == PEQ_N - 1 {
            self.output.set_target(db_to_gain(v));
            return;
        }
        let (b, k) = (i / PEQ_STRIDE, i % PEQ_STRIDE);
        if k >= 2 {
            self.smooth[b][k - 2].set_target(v);
        } else if changed {
            // On/Type switch: new filter shape, so start its state from rest.
            for ch in self.stages.iter_mut() {
                for s in ch[b].iter_mut() {
                    s.reset();
                }
            }
            self.dirty = true;
        }
    }

    fn update_coeffs(&mut self) {
        let sr = self.sr as f64;
        for b in 0..PEQ_BANDS {
            let o = b * PEQ_STRIDE;
            let c = if self.params[o] >= 0.5 {
                let [f, g, q] = self.smooth[b].map(|s| s.current() as f64);
                peq_band_coeffs(self.params[o + 1] as usize, sr, f, g, q)
            } else {
                [Coeffs::IDENTITY; 2]
            };
            for ch in self.stages.iter_mut() {
                ch[b][0].coeffs = c[0];
                ch[b][1].coeffs = c[1];
            }
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (lc, rc) in l.chunks_mut(16).zip(r.chunks_mut(16)) {
            let mut ramping = self.dirty;
            for band in self.smooth.iter_mut() {
                for s in band.iter_mut() {
                    ramping |= s.is_ramping();
                    for _ in 0..lc.len() {
                        s.next();
                    }
                }
            }
            if ramping {
                self.update_coeffs();
                self.dirty = false;
            }
            for i in 0..lc.len() {
                let g = self.output.next();
                for (ch, x) in [&mut lc[i], &mut rc[i]].into_iter().enumerate() {
                    let mut y = *x;
                    for b in 0..PEQ_BANDS {
                        if self.params[b * PEQ_STRIDE] >= 0.5 {
                            let [s0, s1] = &mut self.stages[ch][b];
                            y = s1.process(s0.process(y));
                        }
                    }
                    *x = y * g;
                }
            }
        }
    }
}

processor!(ParamEq, |s, io, ctx| s.process(io.l, io.r));

// ---------------------------------------------------------------- Saturator

pub const SAT_CURVES: &[&str] = &["Soft", "Hard", "Warm", "Fold"];

pub const SAT_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("drive", "Drive", 0.0, 36.0, 6.0, "dB"),
    ParamDesc::choice("curve", "Curve", SAT_CURVES, 0.0),
    ParamDesc::lin("bias", "Bias", -1.0, 1.0, 0.3, ""),
    ParamDesc::lin("output", "Output", -36.0, 12.0, 0.0, "dB"),
    ParamDesc::lin("mix", "Mix", 0.0, 100.0, 100.0, "%"),
];

/// The waveshaping curves (spec D2, Saturator).
pub fn saturate(curve: usize, x: f32, bias: f32) -> f32 {
    match curve {
        0 => x.tanh(),
        1 => x.clamp(-1.0, 1.0),
        2 => {
            // Shifting the operating point of tanh makes the curve asymmetric
            // (even harmonics); subtracting the shifted zero keeps silence silent,
            // and dividing by the larger swing keeps the peak within ±1.
            let b = (bias * 0.5).tanh();
            ((x + bias * 0.5).tanh() - b) / (1.0 + b.abs())
        }
        _ => (x * std::f32::consts::FRAC_PI_2).sin(),
    }
}

pub struct Saturator {
    params: [f32; 5],
    drive: Smoothed,
    output: Smoothed,
    mix: Smoothed,
    dc: [DcBlocker; 2],
}

impl Saturator {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params = std::array::from_fn(|i| SAT_PARAMS[i].default);
        let mut s = Self { params, drive: Smoothed::new(1.0, sr, 20.0), output: Smoothed::new(1.0, sr, 20.0), mix: Smoothed::new(1.0, sr, 20.0), dc: [DcBlocker::new(sr); 2] };
        s.update();
        s.drive.reset(s.drive.target());
        s.output.reset(s.output.target());
        s.mix.reset(s.mix.target());
        s
    }

    fn update(&mut self) {
        self.drive.set_target(db_to_gain(self.params[0]));
        self.output.set_target(db_to_gain(self.params[3]));
        self.mix.set_target(self.params[4] / 100.0);
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = SAT_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            self.update();
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (curve, bias) = (self.params[1] as usize, self.params[2]);
        for i in 0..l.len() {
            let (d, o, m) = (self.drive.next(), self.output.next(), self.mix.next());
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let wet = self.dc[ch].process(saturate(curve, *x * d, bias)) * o;
                *x = *x * (1.0 - m) + wet * m;
            }
        }
    }
}

processor!(Saturator, |s, io, ctx| s.process(io.l, io.r));

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn set_band(eq: &mut ParamEq, band: usize, kind: f32, freq: f32, gain: f32, q: f32) {
        let o = band * PEQ_STRIDE;
        for (k, v) in [1.0, kind, freq, gain, q].into_iter().enumerate() {
            eq.set_param(o + k, v);
        }
    }

    #[test]
    fn eq_is_transparent_with_all_bands_off() {
        let mut eq = ParamEq::new(48_000.0);
        let mut l = sine(1000.0, 48_000.0, 4800, 0.5);
        let mut r = l.clone();
        let orig = l.clone();
        eq.process(&mut l, &mut r);
        assert!(l.iter().zip(&orig).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn eq_bell_matches_its_displayed_curve() {
        let sr = 48_000.0;
        let mut eq = ParamEq::new(sr);
        set_band(&mut eq, 3, BELL, 1000.0, 9.0, 1.0);
        let curve = peq_response_db(eq.param_values(), sr as f64, 1000.0);
        assert!((curve - 9.0).abs() < 0.05, "curve {curve}");
        let mut l = sine(1000.0, sr, 24_000, 0.1);
        let mut r = l.clone();
        eq.process(&mut l, &mut r);
        let measured = 20.0 * (rms(&l[12_000..]) / rms(&sine(1000.0, sr, 24_000, 0.1)[12_000..])).log10();
        assert!((measured - 9.0).abs() < 0.1, "measured {measured} dB");
    }

    #[test]
    fn eq_cut_slopes_are_12_and_24_db_per_octave() {
        let sr = 48_000.0;
        let mut p = [0.0f32; PEQ_N];
        for (i, d) in PEQ_PARAMS.iter().enumerate() {
            p[i] = d.default;
        }
        p[0] = 1.0; // band 1 on, Low Cut 12 at 1 kHz
        p[2] = 1000.0;
        let a = peq_response_db(&p, sr, 250.0) - peq_response_db(&p, sr, 125.0);
        p[1] = 1.0; // Low Cut 24
        let b = peq_response_db(&p, sr, 250.0) - peq_response_db(&p, sr, 125.0);
        assert!((a - 12.0).abs() < 0.5, "12 dB slope measured {a}");
        assert!((b - 24.0).abs() < 0.8, "24 dB slope measured {b}");
        // Q 0.71 makes the 24 dB pair a 4th-order Butterworth: -3 dB at the corner.
        assert!((peq_response_db(&p, sr, 1000.0) + 3.0).abs() < 0.3);
    }

    #[test]
    fn saturator_curves_are_bounded_odd_or_silent_at_zero() {
        for c in 0..SAT_CURVES.len() {
            assert!(saturate(c, 0.0, 0.7).abs() < 1e-6, "curve {c} not silent at 0");
            for k in -100..=100 {
                let y = saturate(c, k as f32 * 0.2, 0.7);
                assert!(y.is_finite() && y.abs() <= 1.0 + 1e-6);
            }
        }
        // Warm is asymmetric when biased: even harmonics present.
        assert!((saturate(2, 0.8, 1.0) + saturate(2, -0.8, 1.0)).abs() > 0.05);
    }

    #[test]
    fn saturator_dry_mix_is_identity_and_output_has_no_dc() {
        let sr = 48_000.0;
        let mut s = Saturator::new(sr);
        s.set_param(4, 0.0);
        let mut l = sine(220.0, sr, 4800, 0.5);
        let mut r = l.clone();
        let orig = l.clone();
        // Mix ramps over 20 ms from 100 %; compare after the ramp.
        s.process(&mut l, &mut r);
        assert!(l[2000..].iter().zip(&orig[2000..]).all(|(a, b)| (a - b).abs() < 1e-6));
        let mut s = Saturator::new(sr);
        s.set_param(1, 2.0);
        s.set_param(2, 1.0);
        s.set_param(0, 24.0);
        let mut l = sine(110.0, sr, 96_000, 0.8);
        let mut r = l.clone();
        s.process(&mut l, &mut r);
        let dc = l[48_000..].iter().sum::<f32>() / 48_000.0;
        assert!(dc.abs() < 0.01, "dc {dc}");
    }
}
