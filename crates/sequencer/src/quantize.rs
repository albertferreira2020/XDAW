//! Launch quantization grids.
//!
//! Reference: Live 10 manual 13.3 documents a per-clip chooser with "None",
//! "Global" and explicit values; the Control Bar holds the Global value. The
//! value list below follows the Live 10 Control Bar chooser (bars and note
//! values). Boundaries for note values are measured from the start of the
//! current bar (own proposal; see docs/research/launch-quantization.md).

use crate::time::{Beats, TimeSignature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quantize {
    None,
    Bars8,
    Bars4,
    Bars2,
    #[default]
    Bar1,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
}

impl Quantize {
    pub const ALL: [Quantize; 10] = [
        Quantize::None,
        Quantize::Bars8,
        Quantize::Bars4,
        Quantize::Bars2,
        Quantize::Bar1,
        Quantize::Half,
        Quantize::Quarter,
        Quantize::Eighth,
        Quantize::Sixteenth,
        Quantize::ThirtySecond,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Quantize::None => "None",
            Quantize::Bars8 => "8 Bars",
            Quantize::Bars4 => "4 Bars",
            Quantize::Bars2 => "2 Bars",
            Quantize::Bar1 => "1 Bar",
            Quantize::Half => "1/2",
            Quantize::Quarter => "1/4",
            Quantize::Eighth => "1/8",
            Quantize::Sixteenth => "1/16",
            Quantize::ThirtySecond => "1/32",
        }
    }

    /// Grid size in quarter-note beats, or `None` when unquantized.
    pub fn grid_beats(self, sig: TimeSignature) -> Option<Beats> {
        let bar = sig.bar_beats();
        match self {
            Quantize::None => None,
            Quantize::Bars8 => Some(8.0 * bar),
            Quantize::Bars4 => Some(4.0 * bar),
            Quantize::Bars2 => Some(2.0 * bar),
            Quantize::Bar1 => Some(bar),
            Quantize::Half => Some(2.0),
            Quantize::Quarter => Some(1.0),
            Quantize::Eighth => Some(0.5),
            Quantize::Sixteenth => Some(0.25),
            Quantize::ThirtySecond => Some(0.125),
        }
    }

    fn is_bar_multiple(self) -> bool {
        matches!(self, Quantize::Bars8 | Quantize::Bars4 | Quantize::Bars2 | Quantize::Bar1)
    }

    /// The first grid boundary at or after `beat`.
    ///
    /// A request that lands within `EPSILON` of a boundary fires on that boundary.
    pub fn next_boundary(self, beat: Beats, sig: TimeSignature) -> Beats {
        const EPSILON: f64 = 1e-9;
        let Some(grid) = self.grid_beats(sig) else { return beat };
        if self.is_bar_multiple() {
            return ((beat - EPSILON) / grid).ceil() * grid;
        }
        // Note values restart at each bar line so odd meters keep the downbeat.
        let bar = sig.bar_beats();
        let bar_start = ((beat + EPSILON) / bar).floor() * bar;
        let candidate = bar_start + ((beat - bar_start - EPSILON) / grid).ceil() * grid;
        candidate.min(bar_start + bar)
    }
}

/// Clip-level quantization: either follow the global value or override it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClipQuantize {
    #[default]
    Global,
    Fixed(Quantize),
}

impl ClipQuantize {
    pub fn resolve(self, global: Quantize) -> Quantize {
        match self {
            ClipQuantize::Global => global,
            ClipQuantize::Fixed(q) => q,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_boundaries() {
        let sig = TimeSignature::default();
        assert_eq!(Quantize::Bar1.next_boundary(0.0, sig), 0.0);
        assert_eq!(Quantize::Bar1.next_boundary(0.001, sig), 4.0);
        assert_eq!(Quantize::Bar1.next_boundary(3.999, sig), 4.0);
        assert_eq!(Quantize::Bar1.next_boundary(4.0, sig), 4.0);
        assert_eq!(Quantize::Bars2.next_boundary(4.5, sig), 8.0);
        assert_eq!(Quantize::Bars8.next_boundary(1.0, sig), 32.0);
        assert_eq!(Quantize::None.next_boundary(1.37, sig), 1.37);
    }

    #[test]
    fn note_values_restart_at_bar_line_in_odd_meters() {
        let seven_eight = TimeSignature::new(7, 8).unwrap(); // 3.5 beats per bar
        assert_eq!(Quantize::Quarter.next_boundary(3.2, seven_eight), 3.5);
        assert_eq!(Quantize::Quarter.next_boundary(3.6, seven_eight), 4.5);
        assert_eq!(Quantize::Half.next_boundary(0.1, seven_eight), 2.0);
        assert_eq!(Quantize::Half.next_boundary(2.1, seven_eight), 3.5);
    }

    #[test]
    fn clip_quantize_resolves_global() {
        assert_eq!(ClipQuantize::Global.resolve(Quantize::Bars2), Quantize::Bars2);
        assert_eq!(ClipQuantize::Fixed(Quantize::None).resolve(Quantize::Bars2), Quantize::None);
    }
}
