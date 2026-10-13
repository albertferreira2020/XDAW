//! Parameter descriptors and note events shared by instruments and effects.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    /// Continuous value; `log` maps the UI control logarithmically (frequencies, times).
    Continuous { log: bool },
    /// Discrete choice; value is the index.
    Choice(&'static [&'static str]),
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamDesc {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    pub kind: ParamKind,
}

impl ParamDesc {
    pub const fn lin(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Continuous { log: false } }
    }

    pub const fn log(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Continuous { log: true } }
    }

    pub const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], default: f32) -> Self {
        Self { id, name, min: 0.0, max: (options.len() - 1) as f32, default, unit: "", kind: ParamKind::Choice(options) }
    }

    pub const fn toggle(id: &'static str, name: &'static str, default: bool) -> Self {
        Self { id, name, min: 0.0, max: 1.0, default: if default { 1.0 } else { 0.0 }, unit: "", kind: ParamKind::Toggle }
    }

    pub fn clamp(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.default;
        }
        let v = v.clamp(self.min, self.max);
        match self.kind {
            ParamKind::Continuous { .. } => v,
            _ => v.round(),
        }
    }

    /// Maps a value to [0, 1] for knobs, automation lanes and MIDI mapping.
    pub fn to_normalized(&self, v: f32) -> f32 {
        let v = self.clamp(v);
        match self.kind {
            ParamKind::Continuous { log: true } if self.min > 0.0 => (v / self.min).ln() / (self.max / self.min).ln(),
            _ => (v - self.min) / (self.max - self.min),
        }
    }

    pub fn from_normalized(&self, n: f32) -> f32 {
        let n = if n.is_finite() { n.clamp(0.0, 1.0) } else { 0.0 };
        let v = match self.kind {
            ParamKind::Continuous { log: true } if self.min > 0.0 => self.min * (self.max / self.min).powf(n),
            _ => self.min + n * (self.max - self.min),
        };
        self.clamp(v)
    }

    pub fn format(&self, v: f32) -> String {
        match self.kind {
            ParamKind::Choice(opts) => opts.get(v as usize).copied().unwrap_or("?").to_string(),
            ParamKind::Toggle => if v >= 0.5 { "On" } else { "Off" }.to_string(),
            ParamKind::Continuous { .. } => {
                if self.unit == "Hz" && v >= 1000.0 {
                    format!("{:.2} kHz", v / 1000.0)
                } else if v.abs() >= 100.0 {
                    format!("{v:.0} {}", self.unit)
                } else {
                    format!("{v:.2} {}", self.unit)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    On { pitch: u8, velocity: u8 },
    Off { pitch: u8 },
    Sustain(bool),
    AllOff,
}

/// A note event positioned inside the current processing block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteEvent {
    pub offset: u32,
    pub kind: NoteKind,
}

/// Per-block processing context.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    pub sample_rate: f32,
    pub bpm: f32,
}

pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_mapping_round_trips() {
        let cutoff = ParamDesc::log("cutoff", "Cutoff", 20.0, 20_000.0, 1000.0, "Hz");
        for v in [20.0, 200.0, 1000.0, 19_999.0] {
            let back = cutoff.from_normalized(cutoff.to_normalized(v));
            assert!((back - v).abs() / v < 1e-4);
        }
        assert_eq!(cutoff.clamp(f32::NAN), 1000.0);
        let mode = ParamDesc::choice("mode", "Mode", &["A", "B", "C"], 0.0);
        assert_eq!(mode.from_normalized(0.6), 1.0);
        assert_eq!(mode.format(2.0), "C");
    }

    #[test]
    fn a4_is_440() {
        assert_eq!(midi_to_hz(69.0), 440.0);
        assert!((midi_to_hz(81.0) - 880.0).abs() < 1e-3);
    }
}
