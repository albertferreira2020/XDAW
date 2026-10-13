//! Biquad (RBJ cookbook, transposed direct form II in f64) and a
//! trapezoidal state-variable filter for modulated synth filtering.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    pub fn new(kind: BiquadKind, sample_rate: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let freq = freq.clamp(10.0, sample_rate * 0.49);
        let q = q.max(0.05);
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::Peak => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
            BiquadKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            BiquadKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Magnitude response at `freq` (for tests and EQ display).
    pub fn magnitude(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (re1, im1) = (w.cos(), -w.sin());
        let (re2, im2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = ((self.b0 + self.b1 * re1 + self.b2 * re2).powi(2) + (self.b1 * im1 + self.b2 * im2).powi(2)).sqrt();
        let den = ((1.0 + self.a1 * re1 + self.a2 * re2).powi(2) + (self.a1 * im1 + self.a2 * im2).powi(2)).sqrt();
        num / den
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    pub coeffs: Coeffs,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(coeffs: Coeffs) -> Self {
        Self { coeffs, z1: 0.0, z2: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let c = &self.coeffs;
        let x = x as f64;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        if !y.is_finite() {
            self.reset();
            return 0.0;
        }
        y as f32
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvfMode {
    LowPass,
    HighPass,
    BandPass,
}

/// Simper/Cytomic trapezoidal SVF: stable under fast cutoff modulation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    g: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Svf {
    /// `resonance` in [0, 1); 1 approaches self-oscillation.
    pub fn set(&mut self, sample_rate: f32, cutoff: f32, resonance: f32) {
        let cutoff = cutoff.clamp(20.0, sample_rate * 0.45);
        self.g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        self.k = 2.0 - 2.0 * resonance.clamp(0.0, 0.98);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }

    #[inline]
    pub fn process(&mut self, x: f32, mode: SvfMode) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if !self.ic1.is_finite() || !self.ic2.is_finite() {
            self.reset();
            return 0.0;
        }
        match mode {
            SvfMode::LowPass => v2,
            SvfMode::BandPass => v1,
            SvfMode::HighPass => x - self.k * v1 - v2,
        }
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_rms_through(f: &mut impl FnMut(f32) -> f32, freq: f32, sr: f32) -> f32 {
        let n = (sr * 0.5) as usize;
        let mut acc = 0.0;
        for i in 0..n {
            let y = f((2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin());
            if i > n / 2 {
                acc += y * y;
            }
        }
        (acc / (n - n / 2 - 1) as f32).sqrt() * std::f32::consts::SQRT_2
    }

    #[test]
    fn lowpass_response_matches_design_within_tolerance() {
        let sr = 48_000.0;
        let c = Coeffs::new(BiquadKind::LowPass, sr, 1000.0, std::f64::consts::FRAC_1_SQRT_2, 0.0);
        // Butterworth: -3.01 dB at cutoff, ~-40 dB/decade beyond.
        assert!((20.0 * c.magnitude(1000.0, sr).log10() + 3.01).abs() < 0.05);
        assert!(20.0 * c.magnitude(10_000.0, sr).log10() < -38.0);
        let mut bq = Biquad::new(c);
        let measured = sine_rms_through(&mut |x| bq.process(x), 100.0, sr as f32);
        assert!((measured - 1.0).abs() < 0.01, "passband gain {measured}");
    }

    #[test]
    fn peak_and_shelf_gain() {
        let sr = 44_100.0;
        let peak = Coeffs::new(BiquadKind::Peak, sr, 2000.0, 1.0, 6.0);
        assert!((20.0 * peak.magnitude(2000.0, sr).log10() - 6.0).abs() < 0.01);
        let shelf = Coeffs::new(BiquadKind::LowShelf, sr, 200.0, 0.707, -12.0);
        assert!((20.0 * shelf.magnitude(20.0, sr).log10() + 12.0).abs() < 0.3);
        assert!((20.0 * shelf.magnitude(10_000.0, sr).log10()).abs() < 0.1);
    }

    #[test]
    fn svf_is_stable_under_extreme_modulation_and_recovers_from_nan() {
        let mut f = Svf::default();
        let mut x = 0.3f32;
        for i in 0..100_000 {
            let cutoff = if i % 2 == 0 { 20.0 } else { 20_000.0 };
            f.set(48_000.0, cutoff, 0.98);
            x = -x;
            let y = f.process(x, SvfMode::LowPass);
            assert!(y.is_finite() && y.abs() < 100.0);
        }
        assert_eq!(f.process(f32::NAN, SvfMode::LowPass), 0.0);
        assert!(f.process(0.5, SvfMode::LowPass).is_finite());
    }
}
