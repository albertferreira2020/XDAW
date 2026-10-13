//! Pitch-preserving time stretching by WSOLA (waveform-similarity overlap-add).
//!
//! Algorithm: Hann frames of `FRAME` samples, synthesis hop `FRAME/2`. For each
//! output frame the nominal input position advances by `hop / stretch`; the
//! actual position is chosen within ±`TOLERANCE` samples to maximise
//! cross-correlation with the natural continuation of the previous frame,
//! which keeps waveforms phase-coherent. Channels share the chosen positions.
//!
//! Limitations (declared): transients can be smeared or doubled at large
//! ratios; very low frequencies (< ~25 Hz at 48 kHz) and polyphonic material
//! with long tones may show phasiness; ratios are limited to 0.25–4. This is an
//! offline process (runs on a worker thread), not used in the audio callback.

use crate::instruments::SampleData;

pub const FRAME: usize = 2048;
pub const HOP: usize = FRAME / 2;
pub const TOLERANCE: usize = 512;

/// Returns `data` stretched in time by `stretch` (2.0 = twice as long) at unchanged pitch.
pub fn wsola(data: &SampleData, stretch: f64) -> SampleData {
    let stretch = stretch.clamp(0.25, 4.0);
    let n_in = data.frames();
    let channels = data.channels.len().max(1);
    let n_out = (n_in as f64 * stretch).round() as usize;
    if n_in < FRAME * 2 || (stretch - 1.0).abs() < 1e-9 {
        // Too short to analyse (or no change): plain copy keeps length semantics simple.
        let mut out: Vec<Vec<f32>> = data.channels.clone();
        for ch in out.iter_mut() {
            ch.resize(n_out.max(1), 0.0);
        }
        return SampleData { channels: out, sample_rate: data.sample_rate };
    }
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FRAME as f32).cos()).collect();
    let mut out = vec![vec![0.0f32; n_out + FRAME]; channels];
    let mut norm = vec![0.0f32; n_out + FRAME];
    // Mono mix used for similarity search.
    let mono: Vec<f32> = (0..n_in).map(|i| data.channels.iter().map(|c| c[i]).sum::<f32>() / channels as f32).collect();
    let input_hop = HOP as f64 / stretch;
    let mut prev_pos: isize = 0;
    let mut k = 0usize;
    loop {
        let out_pos = k * HOP;
        if out_pos >= n_out {
            break;
        }
        let nominal = (k as f64 * input_hop).round() as isize;
        let pos = if k == 0 {
            0
        } else {
            // Natural continuation of the previous frame.
            let target = prev_pos + HOP as isize;
            let lo = (nominal - TOLERANCE as isize).max(0);
            let hi = (nominal + TOLERANCE as isize).min(n_in as isize - FRAME as isize);
            let mut best = nominal.clamp(0, (n_in - FRAME) as isize);
            let mut best_score = f32::NEG_INFINITY;
            if hi >= lo && target >= 0 && (target as usize) + HOP <= n_in {
                let reference = &mono[target as usize..(target as usize + HOP).min(n_in)];
                // Coarse search (step 4) then refine around the best candidate.
                let score = |p: isize| -> f32 {
                    let seg = &mono[p as usize..p as usize + reference.len()];
                    seg.iter().zip(reference).map(|(a, b)| a * b).sum()
                };
                let mut p = lo;
                while p <= hi {
                    let s = score(p);
                    if s > best_score {
                        best_score = s;
                        best = p;
                    }
                    p += 4;
                }
                for p in (best - 3).max(lo)..=(best + 3).min(hi) {
                    let s = score(p);
                    if s > best_score {
                        best_score = s;
                        best = p;
                    }
                }
            }
            best
        };
        let pos = pos.clamp(0, (n_in - FRAME) as isize) as usize;
        for (c, ch) in data.channels.iter().enumerate() {
            for i in 0..FRAME {
                out[c][out_pos + i] += ch[pos + i] * window[i];
            }
        }
        for i in 0..FRAME {
            norm[out_pos + i] += window[i];
        }
        prev_pos = pos as isize;
        k += 1;
    }
    for ch in out.iter_mut() {
        for (i, x) in ch.iter_mut().enumerate().take(n_out) {
            let w = norm[i];
            *x = if w > 1e-3 { *x / w } else { 0.0 };
        }
        ch.truncate(n_out);
    }
    SampleData { channels: out, sample_rate: data.sample_rate }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, seconds: f32, sr: f32) -> SampleData {
        SampleData { channels: vec![(0..(seconds * sr) as usize).map(|i| (std::f32::consts::TAU * freq * i as f32 / sr).sin() * 0.5).collect()], sample_rate: sr as f64 }
    }

    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn stretch_changes_length_but_not_pitch() {
        let sr = 48_000.0;
        let src = sine(440.0, 2.0, sr);
        for ratio in [0.75, 1.25, 1.5] {
            let out = wsola(&src, ratio);
            assert_eq!(out.frames(), (96_000.0 * ratio) as usize);
            // Measure pitch in the steady middle (skip edges).
            let mid = &out.channels[0][4096..out.frames() - 4096];
            let seconds = mid.len() as f32 / sr;
            let freq = zero_crossings(mid) as f32 / seconds;
            assert!((freq - 440.0).abs() < 440.0 * 0.01, "ratio {ratio}: {freq} Hz");
            // Amplitude stays close to the source (no comb-filter collapse).
            let rms = (mid.iter().map(|x| x * x).sum::<f32>() / mid.len() as f32).sqrt();
            assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.05, "ratio {ratio}: rms {rms}");
        }
    }

    #[test]
    fn identity_and_short_inputs() {
        let src = sine(100.0, 0.01, 48_000.0);
        assert_eq!(wsola(&src, 2.0).frames(), 960);
        let long = sine(100.0, 1.0, 48_000.0);
        assert_eq!(wsola(&long, 1.0).channels[0], long.channels[0]);
    }
}
