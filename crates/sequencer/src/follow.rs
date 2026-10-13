//! Follow Actions (Live 10 manual §13.6).
//!
//! A group is a run of adjacent filled slots in one track. The Follow Action
//! Time is measured from where play started. Two actions A/B are chosen by
//! their relative chances. "Previous" at the top of a group wrapping to the
//! last clip is an inference (the manual documents wrapping only for "Next").

use crate::time::Beats;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FollowAction {
    #[default]
    NoAction,
    Stop,
    PlayAgain,
    Previous,
    Next,
    First,
    Last,
    Any,
    Other,
}

impl FollowAction {
    pub const ALL: [FollowAction; 9] = [
        FollowAction::NoAction,
        FollowAction::Stop,
        FollowAction::PlayAgain,
        FollowAction::Previous,
        FollowAction::Next,
        FollowAction::First,
        FollowAction::Last,
        FollowAction::Any,
        FollowAction::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FollowAction::NoAction => "No Action",
            FollowAction::Stop => "Stop",
            FollowAction::PlayAgain => "Play Again",
            FollowAction::Previous => "Previous",
            FollowAction::Next => "Next",
            FollowAction::First => "First",
            FollowAction::Last => "Last",
            FollowAction::Any => "Any",
            FollowAction::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FollowActions {
    /// Follow Action Time in beats (default one bar).
    pub time: Beats,
    pub a: FollowAction,
    pub b: FollowAction,
    pub chance_a: u8,
    pub chance_b: u8,
}

impl Default for FollowActions {
    fn default() -> Self {
        Self { time: 4.0, a: FollowAction::NoAction, b: FollowAction::NoAction, chance_a: 1, chance_b: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowResult {
    Nothing,
    Stop,
    Launch(usize),
}

impl FollowActions {
    pub fn is_active(&self) -> bool {
        let a = self.chance_a > 0 && self.a != FollowAction::NoAction;
        let b = self.chance_b > 0 && self.b != FollowAction::NoAction;
        a || b
    }

    /// Resolves the action for `current` within the group `[first, last]`.
    /// `pick` and `pick2` are uniform random numbers in [0, 1).
    pub fn resolve(&self, current: usize, first: usize, last: usize, pick: f64, pick2: f64) -> FollowResult {
        let total = self.chance_a as u32 + self.chance_b as u32;
        if total == 0 {
            return FollowResult::Nothing;
        }
        let action = if pick * (total as f64) < self.chance_a as f64 { self.a } else { self.b };
        let n = last - first + 1;
        match action {
            FollowAction::NoAction => FollowResult::Nothing,
            FollowAction::Stop => FollowResult::Stop,
            FollowAction::PlayAgain => FollowResult::Launch(current),
            FollowAction::Next => FollowResult::Launch(if current >= last { first } else { current + 1 }),
            FollowAction::Previous => FollowResult::Launch(if current <= first { last } else { current - 1 }),
            FollowAction::First => FollowResult::Launch(first),
            FollowAction::Last => FollowResult::Launch(last),
            FollowAction::Any => FollowResult::Launch(first + ((pick2 * n as f64) as usize).min(n - 1)),
            FollowAction::Other => {
                if n == 1 {
                    return FollowResult::Launch(current);
                }
                let k = ((pick2 * (n - 1) as f64) as usize).min(n - 2);
                let candidate = first + k;
                FollowResult::Launch(if candidate >= current { candidate + 1 } else { candidate })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fa(a: FollowAction) -> FollowActions {
        FollowActions { a, ..Default::default() }
    }

    #[test]
    fn next_wraps_and_previous_wraps() {
        assert_eq!(fa(FollowAction::Next).resolve(3, 2, 4, 0.0, 0.0), FollowResult::Launch(4));
        assert_eq!(fa(FollowAction::Next).resolve(4, 2, 4, 0.0, 0.0), FollowResult::Launch(2));
        assert_eq!(fa(FollowAction::Previous).resolve(2, 2, 4, 0.0, 0.0), FollowResult::Launch(4));
    }

    #[test]
    fn other_never_repeats_when_group_has_more_than_one_clip() {
        let f = fa(FollowAction::Other);
        for i in 0..100 {
            let r = f.resolve(3, 2, 5, 0.0, i as f64 / 100.0);
            assert_ne!(r, FollowResult::Launch(3));
            assert!(matches!(r, FollowResult::Launch(2..=5)));
        }
        assert_eq!(f.resolve(7, 7, 7, 0.0, 0.5), FollowResult::Launch(7));
    }

    #[test]
    fn chances_choose_between_a_and_b() {
        let f = FollowActions { a: FollowAction::Stop, b: FollowAction::First, chance_a: 1, chance_b: 3, ..Default::default() };
        assert_eq!(f.resolve(1, 0, 2, 0.1, 0.0), FollowResult::Stop);
        assert_eq!(f.resolve(1, 0, 2, 0.5, 0.0), FollowResult::Launch(0));
        let never = FollowActions { a: FollowAction::Stop, chance_a: 0, chance_b: 0, ..Default::default() };
        assert_eq!(never.resolve(0, 0, 0, 0.0, 0.0), FollowResult::Nothing);
        assert!(!FollowActions::default().is_active());
    }
}
