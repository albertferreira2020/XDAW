//! Warp markers: piecewise-linear mapping from clip-local beats to source time.
//!
//! Markers pin `(beat, source_seconds)` pairs. Between markers the mapping is
//! linear; before the first and after the last marker it continues with the
//! clip's source tempo (`60 / source_bpm` seconds per beat), so a clip without
//! markers behaves exactly like plain Re-Pitch.

use crate::time::Beats;

/// Maps a clip-local beat to source seconds. `markers` must be sorted by beat.
#[inline]
pub fn source_seconds(markers: &[(Beats, f64)], source_bpm: f64, beat: Beats) -> f64 {
    let spb = 60.0 / source_bpm.max(1e-6);
    match markers {
        [] => beat * spb,
        [(b, s)] => s + (beat - b) * spb,
        _ => {
            let idx = markers.partition_point(|m| m.0 <= beat);
            if idx == 0 {
                let (b, s) = markers[0];
                return s + (beat - b) * spb;
            }
            if idx >= markers.len() {
                let (b, s) = markers[markers.len() - 1];
                return s + (beat - b) * spb;
            }
            let (b0, s0) = markers[idx - 1];
            let (b1, s1) = markers[idx];
            if b1 - b0 <= 1e-9 {
                return s1;
            }
            s0 + (beat - b0) / (b1 - b0) * (s1 - s0)
        }
    }
}

/// Segments `(beat_from, beat_to, sec_from, sec_to)` covering `[0, end_beat)`,
/// used to render pitch-preserving stretches piecewise.
pub fn segments(markers: &[(Beats, f64)], source_bpm: f64, end_beat: Beats) -> Vec<(Beats, Beats, f64, f64)> {
    let mut cuts: Vec<Beats> = vec![0.0];
    cuts.extend(markers.iter().map(|m| m.0).filter(|&b| b > 0.0 && b < end_beat));
    cuts.push(end_beat);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    cuts.windows(2)
        .map(|w| (w[0], w[1], source_seconds(markers, source_bpm, w[0]), source_seconds(markers, source_bpm, w[1])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_markers_is_source_tempo() {
        assert_eq!(source_seconds(&[], 120.0, 4.0), 2.0);
    }

    #[test]
    fn markers_interpolate_and_extrapolate() {
        // Beat 4 pinned to 1.5 s (faster than 120 BPM's 2.0 s).
        let m = [(0.0, 0.0), (4.0, 1.5)];
        assert_eq!(source_seconds(&m, 120.0, 2.0), 0.75);
        assert_eq!(source_seconds(&m, 120.0, 4.0), 1.5);
        assert_eq!(source_seconds(&m, 120.0, 6.0), 2.5); // continues at 0.5 s/beat
        assert_eq!(source_seconds(&m, 120.0, -1.0), -0.5);
        let s = segments(&m, 120.0, 8.0);
        assert_eq!(s, vec![(0.0, 4.0, 0.0, 1.5), (4.0, 8.0, 1.5, 3.5)]);
    }
}
