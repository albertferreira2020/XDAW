//! Clip-local time: mapping song beats to positions inside a (possibly looping) clip.

use crate::time::Beats;
use serde::{Deserialize, Serialize};

/// Region of a clip that plays. With `looping`, playback starts at `start`
/// and wraps from `end` back to `loop_start`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipRegion {
    pub start: Beats,
    pub loop_start: Beats,
    pub end: Beats,
    pub looping: bool,
}

impl ClipRegion {
    pub fn looped(length: Beats) -> Self {
        Self { start: 0.0, loop_start: 0.0, end: length, looping: true }
    }

    pub fn loop_len(&self) -> Beats {
        (self.end - self.loop_start).max(1e-9)
    }

    /// Clip-local position after `elapsed` beats of playback, or `None` when a
    /// one-shot region has finished.
    pub fn position(&self, elapsed: Beats) -> Option<Beats> {
        let p = self.start + elapsed.max(0.0);
        if p < self.end {
            return Some(p);
        }
        if !self.looping {
            return None;
        }
        Some(self.loop_start + (p - self.loop_start).rem_euclid(self.loop_len()))
    }

    /// Calls `f(local_from, local_to, elapsed_at_from)` for each contiguous
    /// clip-local segment covered by elapsed playback `[e0, e1)`.
    pub fn segments(&self, e0: Beats, e1: Beats, mut f: impl FnMut(Beats, Beats, Beats)) {
        let mut e = e0.max(0.0);
        let mut guard = 0;
        while e < e1 && guard < 64 {
            guard += 1;
            let Some(local) = self.position(e) else { return };
            // Distance to the region end from this local position.
            let to_end = self.end - local;
            let span = to_end.min(e1 - e);
            if span <= 0.0 {
                // Exactly at end: advance into the wrapped position.
                e += 1e-12;
                continue;
            }
            f(local, local + span, e);
            e += span;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looping_position_wraps_without_drift() {
        let r = ClipRegion::looped(4.0);
        assert_eq!(r.position(0.0), Some(0.0));
        assert_eq!(r.position(5.0), Some(1.0));
        assert_eq!(r.position(4.0 * 1_000_000.0 + 0.5), Some(0.5));
    }

    #[test]
    fn one_shot_ends() {
        let r = ClipRegion { looping: false, ..ClipRegion::looped(2.0) };
        assert_eq!(r.position(1.9), Some(1.9));
        assert_eq!(r.position(2.0), None);
    }

    #[test]
    fn start_offset_before_loop() {
        let r = ClipRegion { start: 0.0, loop_start: 2.0, end: 4.0, looping: true };
        assert_eq!(r.position(3.0), Some(3.0));
        assert_eq!(r.position(4.5), Some(2.5));
    }

    #[test]
    fn segments_split_at_loop_end() {
        let r = ClipRegion::looped(4.0);
        let mut segs = vec![];
        r.segments(3.0, 9.0, |a, b, e| segs.push((a, b, e)));
        assert_eq!(segs, vec![(3.0, 4.0, 3.0), (0.0, 4.0, 4.0), (0.0, 1.0, 8.0)]);
    }
}
