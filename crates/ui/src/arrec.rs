//! Arrangement recording on armed tracks.
//!
//! Audio is streamed to disk by the engine (`Command::ArrRecord`, punch window
//! and one take per loop pass); MIDI is collected here from live note events.
//! Every finished pass becomes a take lane on its track (`Track::record_take`),
//! and the newest take is the comp that plays.
//!
//! MIDI timing is block-accurate (the engine reports the song position at the
//! start of the audio block a note arrived in).

use std::collections::{HashMap, VecDeque};
use xdaw_project::Note;
use xdaw_sequencer::Beats;

#[derive(Default)]
struct MidiPass {
    start: Beats,
    notes: Vec<Note>,
    open: HashMap<u8, (u8, Beats)>,
}

#[derive(Default)]
pub struct ArrRec {
    pub on: bool,
    /// Song position where the current pass began (record start or loop start).
    pub pass_start: Beats,
    midi: HashMap<usize, MidiPass>,
    /// Start beats of audio takes the engine opened, oldest first, per track.
    audio_starts: HashMap<usize, VecDeque<Beats>>,
}

/// A finished MIDI pass: where it starts, how long it is and its notes (relative to `start`).
pub struct MidiTake {
    pub start: Beats,
    pub length: Beats,
    pub notes: Vec<Note>,
}

impl ArrRec {
    pub fn begin(&mut self, at: Beats) {
        self.on = true;
        self.pass_start = at;
        self.midi.clear();
        self.audio_starts.clear();
    }

    pub fn audio_started(&mut self, track: usize, at: Beats) {
        self.audio_starts.entry(track).or_default().push_back(at);
    }

    pub fn audio_take_start(&mut self, track: usize) -> Option<Beats> {
        self.audio_starts.get_mut(&track)?.pop_front()
    }

    /// Feeds one live note of an armed MIDI track. Notes outside `range` are ignored.
    pub fn live_note(&mut self, track: usize, pitch: u8, velocity: u8, on: bool, beat: Beats, range: (Beats, Beats)) {
        let (from, to) = range;
        let pass_start = self.pass_start;
        if on {
            if beat < from || beat >= to {
                return;
            }
            let pass = self.midi.entry(track).or_insert_with(|| MidiPass { start: pass_start.max(from), ..Default::default() });
            pass.open.insert(pitch, (velocity, beat));
        } else if let Some(pass) = self.midi.get_mut(&track) {
            if let Some((velocity, start)) = pass.open.remove(&pitch) {
                let end = beat.min(to);
                pass.notes.push(Note { pitch, start: start - pass.start, length: (end - start).max(1.0 / 32.0), velocity });
            }
        }
    }

    /// Closes the pass of `track` at `end` (held notes end there).
    pub fn finish_midi(&mut self, track: usize, end: Beats) -> Option<MidiTake> {
        let mut pass = self.midi.remove(&track)?;
        for (pitch, (velocity, start)) in pass.open.drain() {
            pass.notes.push(Note { pitch, start: start - pass.start, length: (end - start).max(1.0 / 32.0), velocity });
        }
        if pass.notes.is_empty() {
            return None;
        }
        pass.notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        let last = pass.notes.iter().map(|n| n.start + n.length).fold(0.0, f64::max);
        Some(MidiTake { start: pass.start, length: (end - pass.start).max(last).max(1.0 / 16.0), notes: pass.notes })
    }

    pub fn midi_tracks(&self) -> Vec<usize> {
        self.midi.keys().copied().collect()
    }

    /// A new pass begins at `at` (loop wrap): later notes belong to it.
    pub fn next_pass(&mut self, at: Beats) {
        self.pass_start = at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: (Beats, Beats) = (f64::NEG_INFINITY, f64::INFINITY);

    #[test]
    fn collects_notes_relative_to_the_pass_start() {
        let mut r = ArrRec::default();
        r.begin(4.0);
        r.live_note(1, 60, 100, true, 5.0, ALL);
        r.live_note(1, 60, 0, false, 5.5, ALL);
        r.live_note(1, 64, 90, true, 6.0, ALL); // still held
        let t = r.finish_midi(1, 8.0).unwrap();
        assert_eq!((t.start, t.length), (4.0, 4.0));
        assert_eq!(t.notes.len(), 2);
        assert_eq!((t.notes[0].pitch, t.notes[0].start, t.notes[0].length), (60, 1.0, 0.5));
        assert_eq!((t.notes[1].pitch, t.notes[1].start, t.notes[1].length), (64, 2.0, 2.0), "held note ends with the pass");
        assert!(r.finish_midi(1, 9.0).is_none(), "consumed");
    }

    #[test]
    fn punch_window_drops_notes_outside_and_truncates_at_punch_out() {
        let mut r = ArrRec::default();
        r.begin(0.0);
        let w = (8.0, 12.0);
        r.live_note(0, 60, 100, true, 6.0, w); // before punch-in
        r.live_note(0, 62, 100, true, 11.0, w);
        r.live_note(0, 62, 0, false, 13.0, w); // released after punch-out
        let t = r.finish_midi(0, 12.0).unwrap();
        assert_eq!(t.start, 8.0, "take starts at punch-in");
        assert_eq!(t.notes.len(), 1);
        assert_eq!((t.notes[0].start, t.notes[0].length), (3.0, 1.0));
    }

    #[test]
    fn empty_passes_make_no_take_and_loop_passes_restart() {
        let mut r = ArrRec::default();
        r.begin(0.0);
        assert!(r.finish_midi(0, 4.0).is_none());
        r.live_note(0, 60, 100, true, 1.0, ALL);
        let first = r.finish_midi(0, 4.0).unwrap();
        assert_eq!(first.start, 0.0);
        r.next_pass(0.0);
        r.live_note(0, 61, 100, true, 0.5, ALL);
        assert_eq!(r.finish_midi(0, 4.0).unwrap().notes[0].pitch, 61);
    }

    #[test]
    fn audio_take_starts_pair_up_in_order() {
        let mut r = ArrRec::default();
        r.audio_started(2, 0.0);
        r.audio_started(2, 4.0);
        assert_eq!(r.audio_take_start(2), Some(0.0));
        assert_eq!(r.audio_take_start(2), Some(4.0));
        assert_eq!(r.audio_take_start(2), None);
    }
}
