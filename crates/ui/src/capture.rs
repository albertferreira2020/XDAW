//! Capture MIDI: turns recently played (unrecorded) notes into a clip.
//!
//! Specification (own; Live 10 §16.8 describes the workflow, not the limits):
//! - History per track: the last `MAX_NOTES` notes, dropping notes older than
//!   `MAX_AGE_SECONDS` of output time.
//! - The captured phrase is the latest run of notes without a gap longer than
//!   `GAP_BARS` bars.
//! - Notes played while the transport ran keep their song positions; the clip
//!   starts on the bar line before the first note. Notes played while stopped are
//!   placed from the first note at the current tempo (no tempo detection).
//! - Clip length rounds up to whole bars (minimum one bar).

use std::collections::{HashMap, VecDeque};
use xdaw_project::Note;

pub const MAX_NOTES: usize = 2048;
pub const MAX_AGE_SECONDS: f64 = 600.0;
pub const GAP_BARS: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub pitch: u8,
    pub velocity: u8,
    pub start_frame: u64,
    pub end_frame: u64,
    /// Song beats when the transport was running at note-on and note-off.
    pub beats: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct History {
    pub notes: VecDeque<Played>,
    open: HashMap<u8, (u8, u64, Option<f64>)>,
}

impl History {
    pub fn note(&mut self, pitch: u8, velocity: u8, on: bool, beat: f64, frame: u64, playing: bool, sample_rate: f64) {
        if on {
            self.open.insert(pitch, (velocity, frame, playing.then_some(beat)));
            return;
        }
        let Some((velocity, start_frame, start_beat)) = self.open.remove(&pitch) else { return };
        let beats = match (start_beat, playing) {
            (Some(b0), true) => Some((b0, beat.max(b0 + 1.0 / 64.0))),
            _ => None,
        };
        self.notes.push_back(Played { pitch, velocity, start_frame, end_frame: frame.max(start_frame + 1), beats });
        while self.notes.len() > MAX_NOTES {
            self.notes.pop_front();
        }
        let horizon = (MAX_AGE_SECONDS * sample_rate) as u64;
        while self.notes.front().is_some_and(|n| frame.saturating_sub(n.end_frame) > horizon) {
            self.notes.pop_front();
        }
    }

    /// Returns clip notes (clip-relative beats) and the clip length.
    pub fn capture(&self, bpm: f64, sample_rate: f64, bar_beats: f64) -> Option<(Vec<Note>, f64)> {
        let last = self.notes.back()?;
        let frames_per_beat = sample_rate * 60.0 / bpm;
        let gap_frames = (GAP_BARS * bar_beats * frames_per_beat) as u64;
        // Walk back to the start of the latest phrase.
        let mut first_idx = self.notes.len() - 1;
        let mut phrase_start = last.start_frame;
        for i in (0..self.notes.len() - 1).rev() {
            let n = self.notes[i];
            if phrase_start.saturating_sub(n.end_frame) > gap_frames && n.end_frame < phrase_start {
                break;
            }
            first_idx = i;
            phrase_start = phrase_start.min(n.start_frame);
        }
        let phrase: Vec<Played> = self.notes.iter().skip(first_idx).copied().collect();
        let all_timed = phrase.iter().all(|n| n.beats.is_some());
        let mut notes: Vec<Note> = if all_timed {
            let first = phrase.iter().map(|n| n.beats.unwrap().0).fold(f64::INFINITY, f64::min);
            let origin = (first / bar_beats).floor() * bar_beats;
            phrase
                .iter()
                .map(|n| {
                    let (s, e) = n.beats.unwrap();
                    Note { pitch: n.pitch, start: s - origin, length: e - s, velocity: n.velocity }
                })
                .collect()
        } else {
            let first = phrase.iter().map(|n| n.start_frame).min().unwrap_or(0);
            phrase
                .iter()
                .map(|n| Note {
                    pitch: n.pitch,
                    start: (n.start_frame - first) as f64 / frames_per_beat,
                    length: ((n.end_frame - n.start_frame) as f64 / frames_per_beat).max(1.0 / 64.0),
                    velocity: n.velocity,
                })
                .collect()
        };
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        let end = notes.iter().map(|n| n.start + n.length).fold(0.0, f64::max);
        let length = ((end / bar_beats).ceil() * bar_beats).max(bar_beats);
        Some((notes, length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    #[test]
    fn captures_latest_phrase_with_song_positions() {
        let mut h = History::default();
        // An old phrase, then a gap of several bars, then the phrase to capture (transport running, 120 BPM).
        h.note(60, 100, true, 0.0, 0, true, SR);
        h.note(60, 0, false, 1.0, 24_000, true, SR);
        let base = 24_000 * 20; // 20 beats later
        h.note(64, 90, true, 21.0, base, true, SR);
        h.note(64, 0, false, 21.5, base + 12_000, true, SR);
        h.note(67, 80, true, 22.0, base + 24_000, true, SR);
        h.note(67, 0, false, 23.0, base + 48_000, true, SR);
        let (notes, len) = h.capture(120.0, SR, 4.0).unwrap();
        assert_eq!(notes.len(), 2, "old phrase excluded");
        assert_eq!((notes[0].pitch, notes[0].start, notes[0].length), (64, 1.0, 0.5)); // bar starts at beat 20
        assert_eq!(notes[1].start, 2.0);
        assert_eq!(len, 4.0);
    }

    #[test]
    fn notes_played_while_stopped_use_frames_at_current_tempo() {
        let mut h = History::default();
        h.note(60, 100, true, 0.0, 1_000, false, SR);
        h.note(60, 0, false, 0.0, 13_000, false, SR); // 0.5 beat at 120 BPM
        h.note(62, 100, true, 0.0, 49_000, false, SR); // 2 beats after first
        h.note(62, 0, false, 0.0, 61_000, false, SR);
        let (notes, len) = h.capture(120.0, SR, 4.0).unwrap();
        assert_eq!(notes.iter().map(|n| (n.start, n.length)).collect::<Vec<_>>(), vec![(0.0, 0.5), (2.0, 0.5)]);
        assert_eq!(len, 4.0);
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        for i in 0..(MAX_NOTES as u64 + 10) {
            h.note(60, 100, true, 0.0, i * 100, false, SR);
            h.note(60, 0, false, 0.0, i * 100 + 50, false, SR);
        }
        assert_eq!(h.notes.len(), MAX_NOTES);
        h.note(61, 100, true, 0.0, (MAX_AGE_SECONDS as u64 + 100) * SR as u64, false, SR);
        h.note(61, 0, false, 0.0, (MAX_AGE_SECONDS as u64 + 101) * SR as u64, false, SR);
        assert_eq!(h.notes.len(), 1, "notes older than the age limit are dropped");
    }
}
