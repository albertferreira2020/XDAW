//! Per-track Session launch state machine.
//!
//! Contracts (docs/research/launch-quantization.md):
//! - A track plays at most one Session clip at a time (Live 10 manual 7.2).
//! - Launch/stop requests are scheduled to the next boundary of the resolved
//!   quantization; at that boundary the previous clip stops and the new one
//!   starts on the same sample (manual 13.3).
//! - Legato launches take over the previous clip's play position (13.5).
//! - Starting a Session clip or pressing a Clip Stop button releases the track
//!   from Arrangement playback until "Back to Arrangement" (7.5).
//!
//! The type holds only `Copy` data so the audio thread can use it without
//! allocation.

use crate::quantize::Quantize;
use crate::time::{Beats, TimeSignature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LaunchMode {
    #[default]
    Trigger,
    Gate,
    Toggle,
    Repeat,
}

impl LaunchMode {
    pub const ALL: [LaunchMode; 4] = [LaunchMode::Trigger, LaunchMode::Gate, LaunchMode::Toggle, LaunchMode::Repeat];

    pub fn label(self) -> &'static str {
        match self {
            LaunchMode::Trigger => "Trigger",
            LaunchMode::Gate => "Gate",
            LaunchMode::Toggle => "Toggle",
            LaunchMode::Repeat => "Repeat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playing {
    pub slot: usize,
    /// Song beat at which clip-local position 0 occurs (before looping).
    pub origin: Beats,
    /// Song beat at which the clip became audible.
    pub started_at: Beats,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pending {
    Launch { slot: usize, at: Beats, legato: bool },
    Stop { at: Beats },
}

impl Pending {
    pub fn at(&self) -> Beats {
        match *self {
            Pending::Launch { at, .. } | Pending::Stop { at } => at,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaunchEvent {
    Started { slot: usize, at: Beats, replaced: Option<usize> },
    Stopped { slot: usize, at: Beats },
    /// A stop fired while nothing was playing; only Arrangement playback was released.
    StoppedIdle { at: Beats },
}

/// Repeat-mode retrigger while the launch control is held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Repeat {
    slot: usize,
    quantize: Quantize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackLauncher {
    pub playing: Option<Playing>,
    pub pending: Option<Pending>,
    /// True while the track follows the Arrangement.
    pub arrangement_active: bool,
    repeat: Option<Repeat>,
}

impl Default for TrackLauncher {
    fn default() -> Self {
        Self { playing: None, pending: None, arrangement_active: true, repeat: None }
    }
}

impl TrackLauncher {
    pub fn request_launch(&mut self, slot: usize, now: Beats, quantize: Quantize, sig: TimeSignature, legato: bool) {
        let at = quantize.next_boundary(now, sig);
        self.pending = Some(Pending::Launch { slot, at, legato });
    }

    /// Repeat mode: launch now (quantized) and keep relaunching each grid step until released.
    pub fn request_repeat(&mut self, slot: usize, now: Beats, quantize: Quantize, sig: TimeSignature) {
        self.request_launch(slot, now, quantize, sig, false);
        self.repeat = (quantize != Quantize::None).then_some(Repeat { slot, quantize });
    }

    pub fn release_repeat(&mut self, slot: usize) {
        if self.repeat.is_some_and(|r| r.slot == slot) {
            self.repeat = None;
        }
    }

    pub fn request_stop(&mut self, now: Beats, quantize: Quantize, sig: TimeSignature) {
        self.repeat = None;
        let at = quantize.next_boundary(now, sig);
        self.pending = Some(Pending::Stop { at });
    }

    /// Stops immediately without quantization (transport stop, panic).
    pub fn stop_now(&mut self) -> Option<usize> {
        self.pending = None;
        self.repeat = None;
        self.playing.take().map(|p| p.slot)
    }

    pub fn back_to_arrangement(&mut self) -> Option<usize> {
        self.arrangement_active = true;
        self.stop_now()
    }

    /// Beat of the pending transition if it falls in `[from, to)`.
    pub fn transition_in(&self, from: Beats, to: Beats) -> Option<Beats> {
        self.pending.map(|p| p.at().max(from)).filter(|&at| at < to)
    }

    /// Applies the pending transition. `at` must come from `transition_in`.
    pub fn fire(&mut self, at: Beats, sig: TimeSignature) -> Option<LaunchEvent> {
        let pending = self.pending.take()?;
        self.arrangement_active = false;
        match pending {
            Pending::Launch { slot, legato, .. } => {
                let previous = self.playing;
                let origin = match previous {
                    Some(p) if legato => p.origin,
                    _ => at,
                };
                self.playing = Some(Playing { slot, origin, started_at: at });
                if let Some(r) = self.repeat.filter(|r| r.slot == slot) {
                    let step = r.quantize.grid_beats(sig).unwrap_or(1.0);
                    self.pending = Some(Pending::Launch { slot, at: at + step, legato: false });
                }
                Some(LaunchEvent::Started { slot, at, replaced: previous.map(|p| p.slot) })
            }
            Pending::Stop { .. } => Some(match self.playing.take() {
                Some(p) => LaunchEvent::Stopped { slot: p.slot, at },
                None => LaunchEvent::StoppedIdle { at },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIG: TimeSignature = TimeSignature { numerator: 4, denominator: 4 };

    #[test]
    fn stopped_to_queued_to_playing_at_next_bar() {
        let mut t = TrackLauncher::default();
        t.request_launch(2, 1.3, Quantize::Bar1, SIG, false);
        assert_eq!(t.pending, Some(Pending::Launch { slot: 2, at: 4.0, legato: false }));
        assert_eq!(t.transition_in(1.3, 3.9), None);
        let at = t.transition_in(3.9, 4.1).unwrap();
        assert_eq!(at, 4.0);
        let ev = t.fire(at, SIG).unwrap();
        assert_eq!(ev, LaunchEvent::Started { slot: 2, at: 4.0, replaced: None });
        assert_eq!(t.playing.unwrap().origin, 4.0);
        assert!(!t.arrangement_active);
    }

    #[test]
    fn launching_another_clip_replaces_on_same_boundary() {
        let mut t = TrackLauncher::default();
        t.request_launch(0, 0.0, Quantize::Bar1, SIG, false);
        t.fire(0.0, SIG);
        t.request_launch(1, 5.0, Quantize::Bar1, SIG, false);
        assert_eq!(t.playing.unwrap().slot, 0, "old clip keeps playing until the boundary");
        let ev = t.fire(t.transition_in(5.0, 9.0).unwrap(), SIG).unwrap();
        assert_eq!(ev, LaunchEvent::Started { slot: 1, at: 8.0, replaced: Some(0) });
    }

    #[test]
    fn legato_keeps_origin() {
        let mut t = TrackLauncher::default();
        t.request_launch(0, 0.0, Quantize::Bar1, SIG, false);
        t.fire(0.0, SIG);
        t.request_launch(1, 6.3, Quantize::None, SIG, true);
        t.fire(6.3, SIG);
        assert_eq!(t.playing.unwrap().origin, 0.0);
        assert_eq!(t.playing.unwrap().started_at, 6.3);
    }

    #[test]
    fn stop_without_clip_still_releases_arrangement() {
        let mut t = TrackLauncher::default();
        t.request_stop(2.0, Quantize::Bar1, SIG);
        let ev = t.fire(t.transition_in(2.0, 5.0).unwrap(), SIG).unwrap();
        assert_eq!(ev, LaunchEvent::StoppedIdle { at: 4.0 });
        assert!(!t.arrangement_active);
        t.back_to_arrangement();
        assert!(t.arrangement_active);
    }

    #[test]
    fn repeat_relaunches_every_grid_step_until_released() {
        let mut t = TrackLauncher::default();
        t.request_repeat(0, 0.1, Quantize::Quarter, SIG);
        t.fire(t.transition_in(0.0, 2.0).unwrap(), SIG);
        assert_eq!(t.pending.unwrap().at(), 2.0);
        t.fire(2.0, SIG);
        assert_eq!(t.pending.unwrap().at(), 3.0);
        t.release_repeat(0);
        t.fire(3.0, SIG);
        assert_eq!(t.pending, None);
    }
}
