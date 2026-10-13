//! ADSR envelope and band-limited oscillators.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR with linear attack and exponential decay/release (time constants
/// chosen so the segment reaches ~-60 dB of its distance in the given time).
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub stage: Stage,
    value: f32,
    attack_inc: f32,
    decay_coef: f32,
    sustain: f32,
    release_coef: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Self { stage: Stage::Idle, value: 0.0, attack_inc: 1.0, decay_coef: 0.0, sustain: 1.0, release_coef: 0.0 }
    }
}

fn coef(ms: f32, sr: f32) -> f32 {
    let samples = (ms.max(0.1) * 0.001 * sr).max(1.0);
    (-6.9078 / samples).exp() // ln(0.001)
}

impl Adsr {
    pub fn set(&mut self, sr: f32, attack_ms: f32, decay_ms: f32, sustain: f32, release_ms: f32) {
        self.attack_inc = 1.0 / (attack_ms.max(0.0) * 0.001 * sr).max(1.0);
        self.decay_coef = coef(decay_ms, sr);
        self.sustain = sustain.clamp(0.0, 1.0);
        self.release_coef = coef(release_ms, sr);
    }

    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// Forces a 2 ms fade for voice stealing.
    pub fn fast_release(&mut self, sr: f32) {
        self.release_coef = coef(2.0, sr);
        self.gate_off();
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.value += self.attack_inc;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.value = self.sustain + (self.value - self.sustain) * self.decay_coef;
                if (self.value - self.sustain).abs() < 1e-4 {
                    self.value = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.value = self.sustain,
            Stage::Release => {
                self.value *= self.release_coef;
                if self.value < 1e-4 {
                    self.value = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Square,
    Triangle,
    Sine,
}

impl Wave {
    pub const NAMES: &'static [&'static str] = &["Saw", "Square", "Triangle", "Sine"];

    pub fn from_index(i: f32) -> Self {
        match i as u32 {
            0 => Wave::Saw,
            1 => Wave::Square,
            2 => Wave::Triangle,
            _ => Wave::Sine,
        }
    }
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// PolyBLEP oscillator (alias-reduced saw/square).
#[derive(Clone, Copy, Debug, Default)]
pub struct Osc {
    phase: f32,
}

impl Osc {
    pub fn reset(&mut self, phase: f32) {
        self.phase = phase;
    }

    #[inline]
    pub fn next(&mut self, wave: Wave, freq: f32, sr: f32) -> f32 {
        let dt = (freq / sr).clamp(0.0, 0.5);
        let t = self.phase;
        let out = match wave {
            Wave::Sine => (t * std::f32::consts::TAU).sin(),
            Wave::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
            Wave::Square => {
                let mut sq = if t < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(t, dt);
                sq - poly_blep((t + 0.5) % 1.0, dt)
            }
            // Triangle harmonics fall at 12 dB/octave, so the direct form aliases little.
            Wave::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_reaches_sustain_and_releases_to_idle() {
        let sr = 48_000.0;
        let mut e = Adsr::default();
        e.set(sr, 10.0, 50.0, 0.5, 100.0);
        e.gate_on();
        for _ in 0..480 {
            e.next();
        }
        assert!((e.value() - 1.0).abs() < 0.01);
        for _ in 0..4800 {
            e.next();
        }
        assert_eq!(e.stage, Stage::Sustain);
        e.gate_off();
        for _ in 0..(sr as usize / 5) {
            e.next();
        }
        assert_eq!(e.stage, Stage::Idle);
    }

    #[test]
    fn oscillators_are_bounded_and_have_expected_frequency() {
        let sr = 48_000.0;
        for wave in [Wave::Saw, Wave::Square, Wave::Triangle, Wave::Sine] {
            let mut o = Osc::default();
            let mut crossings = 0;
            let mut prev = 0.0;
            for i in 0..48_000 {
                let y = o.next(wave, 440.0, sr);
                assert!(y.abs() <= 1.3, "{wave:?} {y}");
                if i > 100 && prev < 0.0 && y >= 0.0 {
                    crossings += 1;
                }
                prev = y;
            }
            assert!((438..=441).contains(&crossings), "{wave:?} crossings {crossings}");
        }
    }
}
