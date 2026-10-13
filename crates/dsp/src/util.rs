//! Gain/pan laws, parameter smoothing, denormal/NaN guards and dither.

pub fn db_to_gain(db: f32) -> f32 {
    if db <= -96.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 1e-5 { -100.0 } else { 20.0 * gain.log10() }
}

/// Constant-power pan law (-3 dB at centre). `pan` in [-1, 1]. Returns (left, right).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Replaces non-finite values with silence; returns true if any were found.
pub fn sanitize(buf: &mut [f32]) -> bool {
    let mut bad = false;
    for s in buf.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
            bad = true;
        }
    }
    bad
}

/// Linear ramp to a target over a fixed time; click-free parameter changes.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_samples: u32,
}

impl Smoothed {
    pub fn new(value: f32, sample_rate: f32, ramp_ms: f32) -> Self {
        let ramp_samples = ((ramp_ms * 0.001 * sample_rate) as u32).max(1);
        Self { current: value, target: value, step: 0.0, remaining: 0, ramp_samples }
    }

    pub fn set_target(&mut self, target: f32) {
        if !target.is_finite() || target == self.target {
            return;
        }
        self.target = target;
        self.remaining = self.ramp_samples;
        self.step = (target - self.current) / self.ramp_samples as f32;
    }

    /// Jumps to a value without ramping (initialisation, transport locate).
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 { self.target } else { self.current + self.step };
        }
        self.current
    }

    pub fn is_ramping(&self) -> bool {
        self.remaining > 0
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn current(&self) -> f32 {
        self.current
    }
}

/// Small deterministic PRNG (xorshift32) for noise and dither; no allocation, no global state.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Uniform in [0, 1).
    pub fn next_unit(&mut self) -> f64 {
        self.next_u32() as f64 / (u32::MAX as f64 + 1.0)
    }
}

/// TPDF dither of +/- 1 LSB at the given bit depth.
pub struct Dither {
    rng: Rng,
    lsb: f32,
}

impl Dither {
    pub fn new(bits: u32, seed: u32) -> Self {
        Self { rng: Rng::new(seed), lsb: 1.0 / (1u64 << (bits - 1)) as f32 }
    }

    #[inline]
    pub fn apply(&mut self, x: f32) -> f32 {
        let tpdf = (self.rng.next_bipolar() + self.rng.next_bipolar()) * 0.5;
        x + tpdf * self.lsb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_is_constant_power() {
        for i in -10..=10 {
            let (l, r) = pan_gains(i as f32 / 10.0);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
        let (l, r) = pan_gains(0.0);
        assert!((gain_to_db(l) + 3.0103).abs() < 0.01 && (l - r).abs() < 1e-6);
        assert_eq!(pan_gains(-1.0).1.abs() < 1e-6, true);
    }

    #[test]
    fn db_round_trip() {
        for db in [-60.0, -12.0, 0.0, 6.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-3);
        }
        assert_eq!(db_to_gain(-200.0), 0.0);
    }

    #[test]
    fn smoothing_reaches_target_and_bounds_step() {
        let mut s = Smoothed::new(0.0, 48_000.0, 10.0);
        s.set_target(1.0);
        let mut prev = 0.0;
        let mut max_step: f32 = 0.0;
        for _ in 0..480 {
            let v = s.next();
            max_step = max_step.max((v - prev).abs());
            prev = v;
        }
        assert_eq!(prev, 1.0);
        assert!(max_step <= 1.0 / 480.0 + 1e-6, "max step {max_step}");
        s.set_target(f32::NAN);
        assert_eq!(s.next(), 1.0);
    }
}
