//! Slice points for audio: transient detection and fixed grids.
//!
//! Detection (own method): a 5 ms-hop energy envelope in dB; an onset is a hop
//! whose rise over the previous 3 hops exceeds a threshold that depends on the
//! sensitivity, and which is the local maximum of that rise within `MIN_GAP`.

use crate::instruments::SampleData;

const HOP_SECONDS: f64 = 0.005;
/// Closest two slices may be.
pub const MIN_GAP_SECONDS: f64 = 0.06;

/// Mono energy envelope in dB, one value per hop.
fn envelope_db(data: &SampleData, from: usize, to: usize, hop: usize) -> Vec<f32> {
    let mut out = vec![];
    let mut i = from;
    while i < to {
        let end = (i + hop).min(to);
        let mut e = 0.0f64;
        for ch in &data.channels {
            for &x in &ch[i..end] {
                e += (x as f64) * (x as f64);
            }
        }
        let n = ((end - i) * data.channels.len().max(1)) as f64;
        out.push((10.0 * (e / n.max(1.0) + 1e-12).log10()) as f32);
        i = end;
    }
    out
}

/// Onset times in seconds inside `[from_s, to_s)` of the file, sorted. The range
/// start is always the first slice point. `sensitivity` 0..1 (higher finds more).
pub fn detect_onsets(data: &SampleData, from_s: f64, to_s: f64, sensitivity: f32) -> Vec<f64> {
    let sr = data.sample_rate;
    let frames = data.frames();
    let from = ((from_s * sr) as usize).min(frames);
    let to = ((to_s * sr) as usize).min(frames);
    let mut out = vec![from as f64 / sr];
    let hop = ((HOP_SECONDS * sr) as usize).max(1);
    if to <= from + hop * 4 {
        return out;
    }
    let env = envelope_db(data, from, to, hop);
    // Rise of each hop over the quietest of the 3 hops before it.
    let threshold = 14.0 - 10.0 * sensitivity.clamp(0.0, 1.0);
    let rise: Vec<f32> = (0..env.len())
        .map(|i| {
            let prev = env[i.saturating_sub(3)..i].iter().copied().fold(f32::MAX, f32::min);
            if i == 0 || prev == f32::MAX { 0.0 } else { env[i] - prev }
        })
        .collect();
    let min_gap = (MIN_GAP_SECONDS / HOP_SECONDS) as usize;
    let mut last: Option<usize> = None;
    for i in 1..rise.len() {
        let is_peak = rise[i] >= threshold && rise[i] >= rise[i - 1] && rise.get(i + 1).is_none_or(|n| rise[i] > *n);
        if !is_peak || env[i] < -60.0 {
            continue;
        }
        if last.is_some_and(|l| i - l < min_gap) {
            continue;
        }
        // The attack begins one hop earlier than its biggest rise.
        let t = (from + i.saturating_sub(1) * hop) as f64 / sr;
        if t - out[0] > MIN_GAP_SECONDS {
            out.push(t);
            last = Some(i);
        }
    }
    out
}

/// Evenly spaced slice points (seconds) over `[from_s, to_s)` every `step_s`.
pub fn grid_points(from_s: f64, to_s: f64, step_s: f64) -> Vec<f64> {
    let mut out = vec![];
    let mut t = from_s;
    while t < to_s - 1e-9 && out.len() < 4096 {
        out.push(t);
        t += step_s.max(1e-3);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn clicks(at: &[f64], len_s: f64) -> SampleData {
        let mut x = vec![0.0f32; (len_s * SR) as usize];
        for &t in at {
            let s = (t * SR) as usize;
            for k in 0..2400 {
                if s + k < x.len() {
                    x[s + k] = 0.8 * (-(k as f32) / 600.0).exp() * ((k as f32) * 0.3).sin();
                }
            }
        }
        SampleData { channels: vec![x], sample_rate: SR }
    }

    #[test]
    fn finds_each_click_near_its_position() {
        let d = clicks(&[0.5, 1.0, 1.75], 2.5);
        let o = detect_onsets(&d, 0.0, 2.5, 0.6);
        assert_eq!(o.len(), 4, "start + 3 clicks, got {o:?}");
        for (found, want) in o[1..].iter().zip([0.5, 1.0, 1.75]) {
            assert!((found - want).abs() < 0.015, "{found} vs {want}");
        }
    }

    #[test]
    fn silence_and_short_ranges_give_only_the_start() {
        let d = SampleData { channels: vec![vec![0.0; 48_000]], sample_rate: SR };
        assert_eq!(detect_onsets(&d, 0.0, 1.0, 1.0), vec![0.0]);
        let d = clicks(&[0.1], 1.0);
        assert_eq!(detect_onsets(&d, 0.0, 0.01, 1.0), vec![0.0]);
    }

    #[test]
    fn range_start_is_the_first_point_and_points_stay_inside() {
        let d = clicks(&[0.2, 0.7, 1.2], 1.6);
        let o = detect_onsets(&d, 0.6, 1.4, 0.6);
        assert!((o[0] - 0.6).abs() < 1e-6);
        assert!(o.iter().all(|t| *t >= 0.6 && *t < 1.4));
        assert!(o.iter().any(|t| (t - 0.7).abs() < 0.015) && o.iter().any(|t| (t - 1.2).abs() < 0.015));
    }

    #[test]
    fn higher_sensitivity_never_finds_fewer_slices() {
        let d = clicks(&[0.3, 0.9], 1.5);
        assert!(detect_onsets(&d, 0.0, 1.5, 1.0).len() >= detect_onsets(&d, 0.0, 1.5, 0.0).len());
    }

    #[test]
    fn grid_points_are_even() {
        assert_eq!(grid_points(1.0, 2.0, 0.25), vec![1.0, 1.25, 1.5, 1.75]);
    }
}
