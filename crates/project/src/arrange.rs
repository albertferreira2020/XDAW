//! Arrangement editing built on the plain model: take lanes and comping,
//! crossfades, MIDI consolidation, locators, clip envelopes and automation
//! drawing. Everything here is pure data manipulation (unit-tested) so the UI
//! and the engine share one definition of each operation.

use crate::model::*;
use serde::{Deserialize, Serialize};
use xdaw_sequencer::{Beats, ClipRegion};

/// Named marker on the Arrangement timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    pub id: Id,
    pub name: String,
    pub beat: Beats,
}

/// One recorded pass. Lanes are alternatives; `Track::arrangement` is the comp
/// that actually plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakeLane {
    pub id: Id,
    pub name: String,
    pub clips: Vec<ArrangementClip>,
}

/// Device output rendered to a file; while present it replaces the track's
/// clips and devices during playback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frozen {
    pub media: MediaRef,
    /// Tempo the render was made at (the file is played unwarped at this tempo).
    pub bpm: f64,
    /// Length covered by the render, in beats from song position 0.
    pub length: Beats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipEnvTarget {
    /// Gain in dB on top of the clip gain.
    Gain,
    /// Pan -1..1 (constant-power law like the track pan).
    Pan,
}

impl ClipEnvTarget {
    pub const ALL: [ClipEnvTarget; 2] = [ClipEnvTarget::Gain, ClipEnvTarget::Pan];

    pub fn name(self) -> &'static str {
        match self {
            ClipEnvTarget::Gain => "Clip Gain",
            ClipEnvTarget::Pan => "Clip Pan",
        }
    }

    /// `(min, max, neutral)` in the target's native unit.
    pub fn range(self) -> (f32, f32, f32) {
        match self {
            ClipEnvTarget::Gain => (-36.0, 12.0, 0.0),
            ClipEnvTarget::Pan => (-1.0, 1.0, 0.0),
        }
    }
}

/// Envelope over clip-local beats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipEnvelope {
    pub target: ClipEnvTarget,
    pub points: Vec<AutomationPoint>,
}

impl ClipEnvelope {
    pub fn new(target: ClipEnvTarget) -> Self {
        Self { target, points: vec![] }
    }

    /// Value at a clip-local beat; the neutral value when there are no points.
    pub fn value_at(&self, beat: Beats) -> f32 {
        interpolate(&self.points, beat).unwrap_or(self.target.range().2)
    }
}

/// Linear interpolation over sorted points; holds the end values outside.
pub fn interpolate(pts: &[AutomationPoint], beat: Beats) -> Option<f32> {
    let first = pts.first()?;
    if beat <= first.beat {
        return Some(first.value);
    }
    let idx = pts.partition_point(|p| p.beat <= beat);
    if idx >= pts.len() {
        return pts.last().map(|p| p.value);
    }
    let (a, b) = (pts[idx - 1], pts[idx]);
    let span = b.beat - a.beat;
    if span <= 0.0 {
        return Some(b.value);
    }
    Some(a.value + (b.value - a.value) * ((beat - a.beat) / span) as f32)
}

/// Inserts keeping order; a point within 1e-9 beats of another replaces it.
pub fn insert_point(pts: &mut Vec<AutomationPoint>, point: AutomationPoint) {
    let idx = pts.partition_point(|p| p.beat < point.beat);
    if pts.get(idx).is_some_and(|p| (p.beat - point.beat).abs() < 1e-9) {
        pts[idx] = point;
    } else {
        pts.insert(idx, point);
    }
}

/// Width of the near-vertical edge between two held steps.
pub const STEP_EDGE: Beats = 1.0 / 512.0;

/// Drawing tools shared by track automation lanes and clip envelopes.
pub trait Drawable {
    fn points_mut(&mut self) -> &mut Vec<AutomationPoint>;

    /// Removes the points in `[from, to]`.
    fn erase_range(&mut self, from: Beats, to: Beats) {
        let (a, b) = (from.min(to), from.max(to));
        self.points_mut().retain(|p| p.beat < a - 1e-9 || p.beat > b + 1e-9);
    }

    /// Replaces `[b0, b1]` by a straight ramp sampled every `step` beats.
    fn draw_line(&mut self, b0: Beats, v0: f32, b1: Beats, v1: f32, step: Beats) {
        let (b0, v0, b1, v1) = if b0 <= b1 { (b0, v0, b1, v1) } else { (b1, v1, b0, v0) };
        self.erase_range(b0, b1);
        let step = step.max(1.0 / 64.0);
        let n = ((b1 - b0) / step).ceil().max(1.0) as usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let p = AutomationPoint { beat: b0 + (b1 - b0) * t, value: v0 + (v1 - v0) * t as f32 };
            insert_point(self.points_mut(), p);
        }
    }

    /// Pencil: holds `value` across the grid cell `[start, end)` (a stair step).
    fn draw_cell(&mut self, start: Beats, end: Beats, value: f32) {
        self.erase_range(start, end - STEP_EDGE * 0.5);
        insert_point(self.points_mut(), AutomationPoint { beat: start, value });
        insert_point(self.points_mut(), AutomationPoint { beat: (end - STEP_EDGE).max(start), value });
    }
}

impl Drawable for AutomationLane {
    fn points_mut(&mut self) -> &mut Vec<AutomationPoint> {
        &mut self.points
    }
}

impl Drawable for ClipEnvelope {
    fn points_mut(&mut self) -> &mut Vec<AutomationPoint> {
        &mut self.points
    }
}

impl ArrangementClip {
    /// The part of this clip inside `[from, to)` with the same source alignment,
    /// or `None` when they do not overlap. Fades survive only at uncut edges.
    pub fn slice(&self, from: Beats, to: Beats, id: Id) -> Option<ArrangementClip> {
        let (a, b) = (from.max(self.start), to.min(self.end()));
        if b - a <= 1e-9 {
            return None;
        }
        let mut c = self.clone();
        c.id = id;
        c.start = a;
        c.length = b - a;
        c.offset = self.offset + (a - self.start);
        if a > self.start {
            c.fade_in = 0.0;
        }
        if b < self.end() {
            c.fade_out = 0.0;
        }
        c.fade_in = c.fade_in.min(c.length);
        c.fade_out = c.fade_out.min(c.length);
        Some(c)
    }
}

impl Track {
    /// Stores a finished recording as a new take lane and makes it the active
    /// comp over its range. Material it replaces is kept as an "Original" lane
    /// the first time, so nothing recorded over is lost.
    pub fn record_take(&mut self, start: Beats, length: Beats, clip: Clip, mut new_id: impl FnMut() -> Id) {
        let end = start + length;
        if self.take_lanes.is_empty() && self.arrangement.iter().any(|c| c.start < end && c.end() > start) {
            let kept: Vec<ArrangementClip> = self.arrangement.clone();
            let id = new_id();
            self.take_lanes.push(TakeLane { id, name: "Original".into(), clips: kept });
        }
        let make = |id: Id| ArrangementClip { id, start, length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: clip.clone() };
        let n = self.take_lanes.len() + 1;
        let lane_id = new_id();
        let clip_id = new_id();
        self.take_lanes.push(TakeLane { id: lane_id, name: format!("Take {n}"), clips: vec![make(clip_id)] });
        self.clear_arrangement_range(start, end, &mut new_id);
        let active = new_id();
        self.arrangement.push(make(active));
        self.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.show_takes = true;
    }

    /// Makes `[from, to)` of take lane `lane` the comp (a clean cut, no fades).
    pub fn comp_from_take(&mut self, lane: usize, from: Beats, to: Beats, mut new_id: impl FnMut() -> Id) -> bool {
        let (from, to) = (from.min(to), from.max(to));
        let Some(l) = self.take_lanes.get(lane) else { return false };
        let pieces: Vec<ArrangementClip> = l.clips.iter().filter_map(|c| c.slice(from, to, 0)).collect();
        if pieces.is_empty() {
            return false;
        }
        self.clear_arrangement_range(from, to, &mut new_id);
        for mut p in pieces {
            p.id = new_id();
            self.arrangement.push(p);
        }
        self.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
        true
    }

    pub fn remove_take_lane(&mut self, lane: usize) {
        if lane < self.take_lanes.len() {
            self.take_lanes.remove(lane);
        }
        if self.take_lanes.is_empty() {
            self.show_takes = false;
        }
    }

    /// Overlaps arrangement clips `left` and `left + 1` by `len` beats centred on
    /// their shared edge and fades them against each other. Returns the overlap
    /// actually applied (limited by the right clip's available source).
    pub fn crossfade(&mut self, left: usize, len: Beats) -> Result<Beats, &'static str> {
        let Some((a, b)) = self.arrangement.get(left).zip(self.arrangement.get(left + 1)) else { return Err("select two neighbouring clips") };
        if a.clip.is_midi() || b.clip.is_midi() {
            return Err("crossfades need audio clips");
        }
        if (a.end() - b.start).abs() > 1e-6 {
            return Err("clips must touch (no gap, no overlap)");
        }
        let avail = b.clip.region.start + b.offset;
        let len = len.min(2.0 * avail).min(2.0 * a.length).min(2.0 * b.length);
        if len < 1.0 / 128.0 {
            return Err("no source material before the second clip to crossfade into");
        }
        let h = len / 2.0;
        let (a, b) = {
            let (x, y) = self.arrangement.split_at_mut(left + 1);
            (&mut x[left], &mut y[0])
        };
        a.length += h;
        a.fade_out = len;
        b.start -= h;
        b.offset -= h;
        b.length += h;
        b.fade_in = len;
        Ok(len)
    }

    /// Merges the MIDI clips overlapping `[from, to)` into one clip, unrolling
    /// loops. Returns false when there is nothing to merge.
    pub fn consolidate_midi(&mut self, from: Beats, to: Beats, mut new_id: impl FnMut() -> Id) -> bool {
        let (from, to) = (from.min(to), from.max(to));
        let len = to - from;
        if len <= 1e-9 {
            return false;
        }
        let mut notes: Vec<Note> = vec![];
        let mut color = None;
        for c in self.arrangement.iter().filter(|c| c.start < to && c.end() > from) {
            let ClipContent::Midi { notes: src } = &c.clip.content else { continue };
            color.get_or_insert(c.clip.color);
            let origin = c.start - c.offset;
            let (lo, hi) = (from.max(c.start), to.min(c.end()));
            c.clip.region.segments(lo - origin, hi - origin, |la, lb, e| {
                for n in src.iter().filter(|n| n.start >= la - 1e-9 && n.start < lb) {
                    let song = origin + e + (n.start - la);
                    let length = n.length.min(c.clip.region.end - n.start).min(to - song);
                    if length > 1e-9 {
                        notes.push(Note { pitch: n.pitch, start: song - from, length, velocity: n.velocity });
                    }
                }
            });
        }
        let Some(color) = color else { return false };
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        self.clear_arrangement_range(from, to, &mut new_id);
        let clip = Clip {
            id: new_id(),
            name: "Consolidated".into(),
            color,
            content: ClipContent::Midi { notes },
            region: ClipRegion::looped(len),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        };
        self.arrangement.push(ArrangementClip { id: new_id(), start: from, length: len, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
        self.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
        true
    }
}

impl Clip {
    /// Mirrors the playable region for a source file `file_beats` long that has
    /// been reversed; warp markers cannot be mirrored and are dropped.
    pub fn reverse_region(&mut self, file_beats: Beats) {
        let r = self.region;
        self.region = ClipRegion { start: file_beats - r.end, loop_start: file_beats - r.end, end: file_beats - r.start, looping: r.looping };
        if let ClipContent::Audio(a) = &mut self.content {
            a.markers.clear();
        }
    }

    /// An audio clip playing `media` (rendered at `bpm`) for `length` beats.
    pub fn rendered_audio(id: Id, name: &str, color: [u8; 3], media: MediaRef, bpm: f64, length: Beats) -> Clip {
        Clip {
            id,
            name: name.into(),
            color,
            content: ClipContent::Audio(AudioContent { media, warp: WarpMode::RePitch, source_bpm: bpm, markers: vec![], envelopes: vec![] }),
            region: ClipRegion::looped(length),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        }
    }
}

impl ArrangementClip {
    /// Re-aligns this clip after its source was reversed (see `Clip::reverse_region`),
    /// so the same stretch of timeline plays the same stretch of audio backwards.
    pub fn reverse(&mut self, file_beats: Beats) {
        let rl = self.clip.region.end - self.clip.region.start;
        let looping = self.clip.region.looping;
        let (o, len) = (self.offset, self.length);
        self.clip.reverse_region(file_beats);
        self.offset = if looping && rl > 1e-9 { (-(o + len)).rem_euclid(rl) } else { (rl - o - len).max(0.0) };
    }
}

impl Track {
    /// Replaces `[from, to)` with one audio clip (consolidation of rendered material).
    pub fn replace_with_audio(&mut self, from: Beats, to: Beats, clip: Clip, mut new_id: impl FnMut() -> Id) {
        self.clear_arrangement_range(from, to, &mut new_id);
        self.arrangement.push(ArrangementClip { id: new_id(), start: from, length: to - from, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
        self.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
    }

    /// Commits a freeze: the rendered audio becomes the track's only content and
    /// its devices are removed, leaving a plain audio track.
    pub fn flatten_to_audio(&mut self, clip: Clip, length: Beats, clip_id: Id) {
        self.kind = TrackKind::Audio;
        self.devices.clear();
        self.macros.clear();
        for s in self.slots.iter_mut() {
            *s = Slot::default();
        }
        self.automation.retain(|l| matches!(l.target, ParamTarget::Volume | ParamTarget::Pan | ParamTarget::Send(_)));
        self.take_lanes.clear();
        self.show_takes = false;
        self.frozen = None;
        self.mixer.input_channel = Some(0);
        self.arrangement = vec![ArrangementClip { id: clip_id, start: 0.0, length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip }];
    }
}

impl TransportSettings {
    /// Range in which Arrangement recording writes: the loop brace when a punch
    /// switch is on, unbounded otherwise.
    pub fn punch_range(&self) -> (Beats, Beats) {
        let from = if self.punch_in { self.loop_start } else { f64::NEG_INFINITY };
        let to = if self.punch_out { self.loop_start + self.loop_length } else { f64::INFINITY };
        (from, to)
    }
}

impl Project {
    pub fn add_locator(&mut self, beat: Beats) -> Id {
        let id = self.alloc_id();
        let n = self.locators.len() + 1;
        self.locators.push(Locator { id, name: format!("Locator {n}"), beat: beat.max(0.0) });
        self.locators.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        id
    }

    pub fn remove_locator(&mut self, id: Id) {
        self.locators.retain(|l| l.id != id);
    }

    pub fn next_locator(&self, beat: Beats) -> Option<&Locator> {
        self.locators.iter().find(|l| l.beat > beat + 1e-6)
    }

    pub fn prev_locator(&self, beat: Beats) -> Option<&Locator> {
        self.locators.iter().rev().find(|l| l.beat < beat - 1e-6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdaw_dsp::DeviceKind;

    fn audio(len: f64) -> Clip {
        Clip {
            id: 0,
            name: "a".into(),
            color: [1, 2, 3],
            content: ClipContent::Audio(AudioContent { media: MediaRef { path: "x.wav".into() }, warp: WarpMode::RePitch, source_bpm: 120.0, markers: vec![], envelopes: vec![] }),
            region: ClipRegion::looped(len),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        }
    }

    fn ids() -> impl FnMut() -> Id {
        let mut n = 1000;
        move || {
            n += 1;
            n
        }
    }

    fn arr(id: Id, start: f64, length: f64, offset: f64, clip: Clip) -> ArrangementClip {
        ArrangementClip { id, start, length, offset, fade_in: 0.0, fade_out: 0.0, clip }
    }

    #[test]
    fn recording_over_material_keeps_it_as_original_and_adds_a_take() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Audio, "A");
        p.tracks[t].arrangement.push(arr(1, 0.0, 8.0, 0.0, audio(8.0)));
        p.tracks[t].record_take(4.0, 4.0, audio(4.0), ids());
        let tr = &p.tracks[t];
        assert_eq!(tr.take_lanes.len(), 2);
        assert_eq!(tr.take_lanes[0].name, "Original");
        assert_eq!(tr.take_lanes[0].clips[0].length, 8.0, "original is untouched");
        assert_eq!(tr.arrangement.len(), 2, "old clip trimmed + new take");
        assert_eq!((tr.arrangement[0].start, tr.arrangement[0].length), (0.0, 4.0));
        assert_eq!((tr.arrangement[1].start, tr.arrangement[1].length), (4.0, 4.0));
        assert!(tr.show_takes);
    }

    #[test]
    fn comping_swaps_a_range_to_another_take_and_keeps_source_alignment() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Audio, "A");
        let tr = &mut p.tracks[t];
        tr.record_take(0.0, 8.0, audio(8.0), ids()); // take 1
        tr.record_take(0.0, 8.0, audio(8.0), ids()); // take 2 is now the comp
        assert_eq!(tr.take_lanes.len(), 2);
        assert!(tr.comp_from_take(0, 2.0, 5.0, ids()));
        let a = &tr.arrangement;
        assert_eq!(a.len(), 3);
        assert_eq!((a[0].start, a[0].end()), (0.0, 2.0));
        assert_eq!((a[1].start, a[1].end(), a[1].offset), (2.0, 5.0, 2.0), "take 1 slice keeps its source position");
        assert_eq!((a[2].start, a[2].end(), a[2].offset), (5.0, 8.0, 5.0));
        let mut seen: Vec<Id> = a.iter().map(|c| c.id).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3, "ids unique");
        assert!(!tr.comp_from_take(9, 0.0, 1.0, ids()));
    }

    #[test]
    fn crossfade_overlaps_and_fades_neighbours() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Audio, "A");
        let tr = &mut p.tracks[t];
        tr.arrangement.push(arr(1, 0.0, 4.0, 0.0, audio(16.0)));
        tr.arrangement.push(arr(2, 4.0, 4.0, 4.0, audio(16.0)));
        let applied = tr.crossfade(0, 1.0).unwrap();
        assert_eq!(applied, 1.0);
        let (a, b) = (&tr.arrangement[0], &tr.arrangement[1]);
        assert_eq!((a.end(), b.start), (4.5, 3.5));
        assert_eq!((a.fade_out, b.fade_in), (1.0, 1.0));
        assert_eq!(b.offset, 3.5, "right clip's source stays aligned");
        // Not touching → refused; MIDI → refused.
        tr.arrangement.push(arr(3, 20.0, 2.0, 0.0, audio(4.0)));
        assert!(tr.crossfade(1, 1.0).is_err());
    }

    #[test]
    fn crossfade_is_limited_by_available_source() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Audio, "A");
        let tr = &mut p.tracks[t];
        tr.arrangement.push(arr(1, 0.0, 4.0, 0.0, audio(16.0)));
        tr.arrangement.push(arr(2, 4.0, 4.0, 0.0, audio(16.0))); // starts at source 0: nothing before it
        assert!(tr.crossfade(0, 1.0).is_err());
        tr.arrangement[1].offset = 0.25;
        assert_eq!(tr.crossfade(0, 1.0).unwrap(), 0.5);
    }

    fn midi(notes: Vec<Note>, len: f64) -> Clip {
        Clip { content: ClipContent::Midi { notes }, ..audio(len) }
    }

    #[test]
    fn consolidate_unrolls_loops_and_clips_to_range() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Midi, "M");
        let tr = &mut p.tracks[t];
        // 2-beat loop with one note, placed over beats 1..7.
        tr.arrangement.push(arr(1, 1.0, 6.0, 0.0, midi(vec![Note { pitch: 60, start: 0.5, length: 1.0, velocity: 90 }], 2.0)));
        assert!(tr.consolidate_midi(2.0, 6.0, ids()));
        assert_eq!(tr.arrangement.iter().filter(|c| c.start < 2.0 || c.end() > 6.0).count(), 2, "outside parts remain");
        let merged = tr.arrangement.iter().find(|c| c.start == 2.0).unwrap();
        assert_eq!(merged.length, 4.0);
        let ClipContent::Midi { notes } = &merged.clip.content else { panic!() };
        // Loop passes start at 1,3,5 → notes at 1.5, 3.5, 5.5; 1.5 is inside [2,6)? no (1.5 <2) so 3.5 and 5.5 remain.
        let starts: Vec<f64> = notes.iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![1.5, 3.5]);
        assert!(!tr.consolidate_midi(30.0, 31.0, ids()), "nothing to merge");
    }

    #[test]
    fn drawing_tools_edit_lanes_and_envelopes() {
        let mut lane = AutomationLane { target: ParamTarget::Volume, points: vec![] };
        lane.draw_line(0.0, 0.0, 4.0, -12.0, 1.0);
        assert_eq!(lane.points.len(), 5);
        assert!((lane.value_at(2.0).unwrap() + 6.0).abs() < 1e-5);
        // A second ramp replaces the overlapped part.
        lane.draw_line(1.0, 0.0, 3.0, 0.0, 1.0);
        assert_eq!(lane.value_at(2.0), Some(0.0));
        lane.erase_range(1.0, 3.0);
        assert!(lane.points.iter().all(|p| p.beat < 1.0 || p.beat > 3.0));
        // Pencil cells hold their value then jump.
        let mut env = ClipEnvelope::new(ClipEnvTarget::Gain);
        env.draw_cell(0.0, 1.0, -6.0);
        env.draw_cell(1.0, 2.0, 0.0);
        assert_eq!(env.value_at(0.5), -6.0);
        assert_eq!(env.value_at(1.5), 0.0);
        assert!(env.points.windows(2).all(|w| w[0].beat <= w[1].beat));
        assert_eq!(ClipEnvelope::new(ClipEnvTarget::Pan).value_at(3.0), 0.0, "no points = neutral");
    }

    #[test]
    fn locators_stay_sorted_and_navigate() {
        let mut p = Project::new("t");
        p.add_locator(16.0);
        p.add_locator(4.0);
        let c = p.add_locator(8.0);
        assert_eq!(p.locators.iter().map(|l| l.beat).collect::<Vec<_>>(), vec![4.0, 8.0, 16.0]);
        assert_eq!(p.next_locator(4.0).map(|l| l.beat), Some(8.0));
        assert_eq!(p.prev_locator(8.0).map(|l| l.beat), Some(4.0));
        assert!(p.prev_locator(4.0).is_none());
        p.remove_locator(c);
        assert_eq!(p.locators.len(), 2);
    }

    #[test]
    fn punch_range_follows_the_loop_brace() {
        let mut t = TransportSettings { loop_start: 8.0, loop_length: 4.0, ..Default::default() };
        assert_eq!(t.punch_range(), (f64::NEG_INFINITY, f64::INFINITY));
        t.punch_in = true;
        t.punch_out = true;
        assert_eq!(t.punch_range(), (8.0, 12.0));
    }

    #[test]
    fn old_projects_without_the_new_fields_still_load() {
        let mut p = Project::new("old");
        p.add_track(TrackKind::Audio, "A");
        p.add_device(TrackRef::Track(0), DeviceKind::Gain);
        let mut v: serde_json::Value = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("locators");
        for t in v["tracks"].as_array_mut().unwrap() {
            for k in ["take_lanes", "show_takes", "folded", "frozen"] {
                t.as_object_mut().unwrap().remove(k);
            }
        }
        v["transport"].as_object_mut().unwrap().remove("punch_in");
        let back: Project = serde_json::from_value(v).unwrap();
        assert!(back.locators.is_empty() && back.tracks[0].take_lanes.is_empty() && !back.transport.punch_in);
    }

    #[test]
    fn reversing_mirrors_region_and_offset() {
        // 8-beat file, clip plays beats 2..6 of it; arrangement shows its middle 1.5 beats from offset 1.
        let mut c = audio(8.0);
        c.region = ClipRegion { start: 2.0, loop_start: 2.0, end: 6.0, looping: false };
        let mut a = arr(1, 10.0, 1.5, 1.0, c);
        a.reverse(8.0);
        assert_eq!((a.clip.region.start, a.clip.region.end), (2.0, 6.0), "symmetric region maps onto itself");
        assert_eq!(a.offset, 1.5, "was 1.0 in, now mirrored: 4 - 1.0 - 1.5");
        // Asymmetric region.
        let mut c = audio(8.0);
        c.region = ClipRegion { start: 0.0, loop_start: 0.0, end: 3.0, looping: true };
        let mut b = arr(2, 0.0, 3.0, 0.0, c);
        b.reverse(8.0);
        assert_eq!((b.clip.region.start, b.clip.region.end), (5.0, 8.0));
        assert_eq!(b.offset, 0.0);
        // Reversing twice restores the alignment.
        b.reverse(8.0);
        assert_eq!((b.clip.region.start, b.clip.region.end, b.offset), (0.0, 3.0, 0.0));
    }

    #[test]
    fn flatten_leaves_a_plain_audio_track() {
        let mut p = Project::new("t");
        p.add_scene("S");
        let t = p.add_track(TrackKind::Midi, "M");
        p.add_device(TrackRef::Track(t), DeviceKind::PolySynth);
        p.tracks[t].arrangement.push(arr(1, 0.0, 4.0, 0.0, midi(vec![], 4.0)));
        p.tracks[t].mixer.volume_db = -3.0;
        let clip = Clip::rendered_audio(9, "Flattened", [1, 1, 1], MediaRef { path: "f.wav".into() }, 120.0, 8.0);
        p.tracks[t].flatten_to_audio(clip, 8.0, 10);
        let tr = &p.tracks[t];
        assert_eq!(tr.kind, TrackKind::Audio);
        assert!(tr.devices.is_empty() && tr.arrangement.len() == 1 && !tr.arrangement[0].clip.is_midi());
        assert_eq!(tr.mixer.volume_db, -3.0, "mixer settings survive");
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn replace_with_audio_trims_neighbours() {
        let mut p = Project::new("t");
        let t = p.add_track(TrackKind::Audio, "A");
        let tr = &mut p.tracks[t];
        tr.arrangement.push(arr(1, 0.0, 8.0, 0.0, audio(8.0)));
        tr.replace_with_audio(2.0, 4.0, audio(2.0), ids());
        assert_eq!(tr.arrangement.iter().map(|c| (c.start, c.end())).collect::<Vec<_>>(), vec![(0.0, 2.0), (2.0, 4.0), (4.0, 8.0)]);
    }
}
