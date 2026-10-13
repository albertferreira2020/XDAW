//! Explicit conversions between samples, seconds, beats and bars.
//!
//! A *beat* is always a quarter note. Musical positions are `f64` beats measured
//! from song start (bar 1, beat 1 == 0.0). The audio clock is an integer sample
//! counter; the tempo map anchors each tempo segment to an exact sample so the
//! beat position is recomputed from integers instead of accumulated, which keeps
//! long loops free of drift.

use serde::{Deserialize, Serialize};

/// Quarter-note beats measured from song start.
pub type Beats = f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self { numerator: 4, denominator: 4 }
    }
}

impl TimeSignature {
    /// Validates the Live 10 documented range: numerator 1–99, denominator 1, 2, 4, 8 or 16.
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        let ok = (1..=99).contains(&numerator) && matches!(denominator, 1 | 2 | 4 | 8 | 16);
        ok.then_some(Self { numerator, denominator })
    }

    /// Length of one bar in quarter-note beats.
    pub fn bar_beats(&self) -> Beats {
        self.numerator as f64 * 4.0 / self.denominator as f64
    }

    /// Length of the signature's counting unit (denominator note) in quarter-note beats.
    pub fn unit_beats(&self) -> Beats {
        4.0 / self.denominator as f64
    }
}

/// Bars-beats-sixteenths position, 1-based like a musical ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarsBeatsSixteenths {
    pub bar: i64,
    pub beat: i64,
    pub sixteenth: i64,
}

impl std::fmt::Display for BarsBeatsSixteenths {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.bar, self.beat, self.sixteenth)
    }
}

/// Converts a beat position to a 1-based bars.beats.sixteenths display value.
/// Negative positions (count-in) produce negative bar numbers, e.g. -2.1.1.
pub fn beats_to_bbs(position: Beats, sig: TimeSignature) -> BarsBeatsSixteenths {
    let bar_len = sig.bar_beats();
    let unit = sig.unit_beats();
    let bar_index = (position / bar_len + 1e-9).floor();
    let within = (position - bar_index * bar_len).max(0.0);
    let beat_index = (within / unit + 1e-9).floor();
    let sixteenth_index = ((within - beat_index * unit) / 0.25 + 1e-9).floor();
    let bar = bar_index as i64;
    BarsBeatsSixteenths {
        bar: if bar >= 0 { bar + 1 } else { bar },
        beat: beat_index as i64 + 1,
        sixteenth: sixteenth_index as i64 + 1,
    }
}

pub fn seconds_to_samples(seconds: f64, sample_rate: f64) -> f64 {
    seconds * sample_rate
}

pub fn samples_to_seconds(samples: f64, sample_rate: f64) -> f64 {
    samples / sample_rate
}

pub fn beats_to_seconds(beats: Beats, bpm: f64) -> f64 {
    beats * 60.0 / bpm
}

pub fn seconds_to_beats(seconds: f64, bpm: f64) -> Beats {
    seconds * bpm / 60.0
}

/// Lowest and highest tempo accepted, matching the documented Live 10 tempo range.
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 999.0;

pub fn clamp_bpm(bpm: f64) -> f64 {
    if bpm.is_finite() { bpm.clamp(MIN_BPM, MAX_BPM) } else { 120.0 }
}

/// The authoritative musical clock driven by the audio callback.
///
/// `sample` counts processed frames since the clock was created. A tempo change
/// re-anchors the segment at the exact sample where it takes effect, so
/// `beat_at` never accumulates floating-point increments.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    sample_rate: f64,
    anchor_sample: i64,
    anchor_beat: Beats,
    bpm: f64,
}

impl Clock {
    pub fn new(sample_rate: f64, bpm: f64) -> Self {
        Self { sample_rate, anchor_sample: 0, anchor_beat: 0.0, bpm: clamp_bpm(bpm) }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    pub fn beats_per_sample(&self) -> f64 {
        self.bpm / (60.0 * self.sample_rate)
    }

    pub fn samples_per_beat(&self) -> f64 {
        60.0 * self.sample_rate / self.bpm
    }

    pub fn beat_at(&self, sample: i64) -> Beats {
        self.anchor_beat + (sample - self.anchor_sample) as f64 * self.beats_per_sample()
    }

    /// Fractional sample position of a beat under the current tempo segment.
    pub fn sample_at(&self, beat: Beats) -> f64 {
        self.anchor_sample as f64 + (beat - self.anchor_beat) * self.samples_per_beat()
    }

    /// Changes tempo effective at `sample`, keeping the beat at that sample continuous.
    pub fn set_bpm_at(&mut self, sample: i64, bpm: f64) {
        let beat = self.beat_at(sample);
        self.anchor_sample = sample;
        self.anchor_beat = beat;
        self.bpm = clamp_bpm(bpm);
    }

    /// Moves the musical position so that `sample` corresponds to `beat`.
    pub fn locate(&mut self, sample: i64, beat: Beats) {
        self.anchor_sample = sample;
        self.anchor_beat = beat;
    }

    /// First integer sample at or after `beat`, given as an offset from `block_start`.
    /// Returns `None` when the beat is not inside `[block_start, block_start + len)`.
    pub fn offset_in_block(&self, beat: Beats, block_start: i64, len: usize) -> Option<usize> {
        let exact = self.sample_at(beat) - block_start as f64;
        // Tolerance absorbs representational error of exact grid positions.
        let offset = (exact - 1e-6).ceil().max(0.0);
        (offset < len as f64).then_some(offset as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbs_display_is_one_based_and_counts_in_negative_bars() {
        let sig = TimeSignature::default();
        assert_eq!(beats_to_bbs(0.0, sig).to_string(), "1.1.1");
        assert_eq!(beats_to_bbs(4.0, sig).to_string(), "2.1.1");
        assert_eq!(beats_to_bbs(5.25, sig).to_string(), "2.2.2");
        assert_eq!(beats_to_bbs(-8.0, sig).to_string(), "-2.1.1");
        let six_eight = TimeSignature::new(6, 8).unwrap();
        assert_eq!(six_eight.bar_beats(), 3.0);
        assert_eq!(beats_to_bbs(3.5, six_eight).to_string(), "2.2.1");
    }

    #[test]
    fn time_signature_rejects_values_outside_documented_range() {
        assert!(TimeSignature::new(0, 4).is_none());
        assert!(TimeSignature::new(100, 4).is_none());
        assert!(TimeSignature::new(7, 3).is_none());
        assert!(TimeSignature::new(99, 16).is_some());
    }

    #[test]
    fn clock_round_trips_and_tempo_change_is_continuous() {
        let mut clock = Clock::new(48_000.0, 120.0);
        assert_eq!(clock.samples_per_beat(), 24_000.0);
        assert_eq!(clock.beat_at(48_000), 2.0);
        clock.set_bpm_at(48_000, 60.0);
        assert_eq!(clock.beat_at(48_000), 2.0);
        assert_eq!(clock.beat_at(96_000), 3.0);
        assert_eq!(clock.sample_at(4.0), 144_000.0);
    }

    #[test]
    fn no_drift_after_many_loops() {
        // 44.1 kHz at 128 BPM: samples per beat is not an integer (20671.875).
        let clock = Clock::new(44_100.0, 128.0);
        let bar = 4.0;
        let loops = 10_000_i64; // ~5.2 hours
        let target = loops as f64 * bar;
        let exact = target * clock.samples_per_beat();
        let offset_sample = clock.offset_in_block(target, exact.floor() as i64, 4).unwrap();
        let chosen = exact.floor() + offset_sample as f64;
        assert!(chosen - exact >= 0.0 && chosen - exact < 1.0);
    }
}
