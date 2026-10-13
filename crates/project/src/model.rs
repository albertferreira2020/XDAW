//! Project data model. Owned by the UI thread; the engine receives compiled snapshots.

use xdaw_dsp::DeviceKind;
use xdaw_sequencer::{Beats, ClipQuantize, ClipRegion, LaunchMode, Quantize, TimeSignature};
use serde::{Deserialize, Serialize};

pub type Id = u64;

pub const FORMAT_NAME: &str = "xdaw-project";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub transport: TransportSettings,
    pub tracks: Vec<Track>,
    pub returns: Vec<Track>,
    pub master: Track,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub midi_mappings: Vec<ControlMapping>,
    #[serde(default)]
    pub key_mappings: Vec<ControlMapping>,
    pub next_id: Id,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransportSettings {
    pub bpm: f64,
    pub time_signature: TimeSignature,
    pub global_quantize: Quantize,
    pub metronome: bool,
    /// Count-in length in bars before recording (0 = none).
    pub count_in_bars: u32,
    pub loop_enabled: bool,
    pub loop_start: Beats,
    pub loop_length: Beats,
}

impl Default for TransportSettings {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            time_signature: TimeSignature::default(),
            global_quantize: Quantize::Bar1,
            metronome: false,
            count_in_bars: 0,
            loop_enabled: false,
            loop_start: 0.0,
            loop_length: 16.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Midi,
    Return,
    Master,
    /// Sums its member tracks (`Track::group`) through its own devices and mixer.
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Monitor {
    In,
    #[default]
    Auto,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mixer {
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub arm: bool,
    pub monitor: Monitor,
    /// Send levels in dB per return track (index-aligned with `Project::returns`).
    pub sends_db: Vec<f32>,
    /// Audio input channel pair for recording (audio tracks); `None` = no input.
    pub input_channel: Option<u16>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self { volume_db: 0.0, pan: 0.0, mute: false, solo: false, arm: false, monitor: Monitor::Auto, sends_db: vec![], input_channel: None }
    }
}

pub const SEND_OFF_DB: f32 = -70.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub mixer: Mixer,
    pub devices: Vec<DeviceState>,
    /// Session slots, index-aligned with `Project::scenes` (empty for return/master).
    pub slots: Vec<Slot>,
    pub arrangement: Vec<ArrangementClip>,
    pub automation: Vec<AutomationLane>,
    /// Group track this track feeds (one level of grouping).
    #[serde(default)]
    pub group: Option<Id>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub id: Id,
    pub kind: DeviceKind,
    pub params: Vec<f32>,
    pub bypass: bool,
    /// Sampler source.
    #[serde(default)]
    pub sample: Option<MediaRef>,
    /// Drum Rack pads (16).
    #[serde(default)]
    pub pads: Vec<Option<MediaRef>>,
    #[serde(default)]
    pub preset_name: Option<String>,
}

impl DeviceState {
    pub fn new(id: Id, kind: DeviceKind) -> Self {
        Self {
            id,
            kind,
            params: kind.params().iter().map(|p| p.default).collect(),
            bypass: false,
            sample: None,
            pads: if kind == DeviceKind::DrumRack { vec![None; xdaw_dsp::instruments::PADS] } else { vec![] },
            preset_name: None,
        }
    }

    /// Parameters normalised to the current descriptor list (older files may be shorter).
    pub fn param(&self, i: usize) -> f32 {
        let desc = &self.kind.params()[i];
        desc.clamp(self.params.get(i).copied().unwrap_or(desc.default))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub clip: Option<Clip>,
    pub has_stop_button: bool,
}

impl Default for Slot {
    fn default() -> Self {
        Self { clip: None, has_stop_button: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: Id,
    pub name: String,
    pub color: [u8; 3],
}

/// Path to media; relative paths resolve against the project file's directory.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaRef {
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WarpMode {
    /// Playback rate follows tempo / source tempo: pitch changes with tempo (honest resampling).
    #[default]
    RePitch,
    /// Plays at the file's original speed; tempo has no effect.
    Off,
    /// Follows tempo with pitch preserved (offline WSOLA rendering, see `xdaw_dsp::stretch`).
    Stretch,
}

/// Media-cache key of a clip's stretched rendering at a project tempo.
pub fn stretch_key(path: &str, source_bpm: f64, bpm: f64) -> String {
    format!("{path}#stretch={:.4}", source_bpm / bpm)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    /// Position in the source file (seconds).
    pub source_seconds: f64,
    /// Clip-local beat that position is pinned to.
    pub beat: Beats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioContent {
    pub media: MediaRef,
    pub warp: WarpMode,
    /// Tempo the material was recorded/rendered at.
    pub source_bpm: f64,
    #[serde(default)]
    pub markers: Vec<WarpMarker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub start: Beats,
    pub length: Beats,
    pub velocity: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipContent {
    Audio(AudioContent),
    Midi { notes: Vec<Note> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub name: String,
    pub color: [u8; 3],
    pub content: ClipContent,
    pub region: ClipRegion,
    pub launch_mode: LaunchMode,
    pub quantize: ClipQuantize,
    pub legato: bool,
    pub gain_db: f32,
    /// Transposition in semitones (MIDI: note offset; audio Re-Pitch: rate change).
    #[serde(default)]
    pub transpose: i32,
    /// Session Follow Actions (Live 10 §13.6).
    #[serde(default)]
    pub follow: xdaw_sequencer::FollowActions,
}

impl Clip {
    pub fn is_midi(&self) -> bool {
        matches!(self.content, ClipContent::Midi { .. })
    }

    pub fn length(&self) -> Beats {
        self.region.end - self.region.start
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArrangementClip {
    pub id: Id,
    /// Song position where the clip begins.
    pub start: Beats,
    /// Length on the timeline.
    pub length: Beats,
    /// Clip-local playback position at `start` (for split/trimmed clips).
    pub offset: Beats,
    pub fade_in: Beats,
    pub fade_out: Beats,
    pub clip: Clip,
}

impl ArrangementClip {
    pub fn end(&self) -> Beats {
        self.start + self.length
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParamTarget {
    Volume,
    Pan,
    Send(usize),
    Device { device: Id, param: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: Beats,
    /// Value in the target's native unit (dB, pan, device units).
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    pub target: ParamTarget,
    pub points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// Linear interpolation; holds first/last value outside the points.
    pub fn value_at(&self, beat: Beats) -> Option<f32> {
        let pts = &self.points;
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
        let t = ((beat - a.beat) / span) as f32;
        Some(a.value + (b.value - a.value) * t)
    }

    pub fn insert(&mut self, point: AutomationPoint) {
        let idx = self.points.partition_point(|p| p.beat < point.beat);
        if self.points.get(idx).is_some_and(|p| (p.beat - point.beat).abs() < 1e-9) {
            self.points[idx] = point;
        } else {
            self.points.insert(idx, point);
        }
    }
}

/// What a MIDI control or computer key is mapped to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MappingTarget {
    LaunchSlot { track: Id, scene: Id },
    LaunchScene { scene: Id },
    StopTrack { track: Id },
    Param { track: Id, target: ParamTarget },
    TransportPlay,
    TransportStop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MappingSource {
    MidiNote { channel: u8, note: u8 },
    MidiCc { channel: u8, cc: u8 },
    Key { key: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlMapping {
    pub source: MappingSource,
    pub target: MappingTarget,
}

pub const TRACK_COLORS: [[u8; 3]; 8] = [
    [233, 120, 83],
    [240, 196, 80],
    [154, 205, 92],
    [88, 196, 170],
    [92, 160, 230],
    [160, 120, 220],
    [220, 110, 170],
    [180, 180, 180],
];

impl Project {
    pub fn new(name: &str) -> Self {
        let mut p = Project {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            name: name.into(),
            transport: TransportSettings::default(),
            tracks: vec![],
            returns: vec![],
            master: Track::new(0, "Master", TrackKind::Master, [200, 200, 200], 0, 0),
            scenes: vec![],
            midi_mappings: vec![],
            key_mappings: vec![],
            next_id: 1,
        };
        p.master.id = p.alloc_id();
        p
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_scene(&mut self, name: &str) -> usize {
        let id = self.alloc_id();
        self.scenes.push(Scene { id, name: name.into(), color: [120, 120, 120] });
        for t in self.tracks.iter_mut() {
            t.slots.push(Slot::default());
        }
        self.scenes.len() - 1
    }

    pub fn insert_scene(&mut self, index: usize, name: &str) {
        let id = self.alloc_id();
        let index = index.min(self.scenes.len());
        self.scenes.insert(index, Scene { id, name: name.into(), color: [120, 120, 120] });
        for t in self.tracks.iter_mut() {
            t.slots.insert(index, Slot::default());
        }
    }

    pub fn remove_scene(&mut self, index: usize) {
        if index < self.scenes.len() {
            self.scenes.remove(index);
            for t in self.tracks.iter_mut() {
                if index < t.slots.len() {
                    t.slots.remove(index);
                }
            }
        }
    }

    pub fn add_track(&mut self, kind: TrackKind, name: &str) -> usize {
        let id = self.alloc_id();
        let color = TRACK_COLORS[self.tracks.len() % TRACK_COLORS.len()];
        let t = Track::new(id, name, kind, color, self.scenes.len(), self.returns.len());
        self.tracks.push(t);
        self.tracks.len() - 1
    }

    pub fn add_return(&mut self, name: &str) -> usize {
        let id = self.alloc_id();
        let t = Track::new(id, name, TrackKind::Return, [150, 150, 200], 0, 0);
        self.returns.push(t);
        for t in self.tracks.iter_mut() {
            t.mixer.sends_db.push(SEND_OFF_DB);
        }
        self.returns.len() - 1
    }

    pub fn add_device(&mut self, track: TrackRef, kind: DeviceKind) -> Id {
        let id = self.alloc_id();
        let t = self.track_mut(track);
        let state = DeviceState::new(id, kind);
        if kind.is_instrument() {
            // An instrument goes first and replaces an existing instrument.
            if t.devices.first().is_some_and(|d| d.kind.is_instrument()) {
                t.devices[0] = state;
            } else {
                t.devices.insert(0, state);
            }
        } else {
            t.devices.push(state);
        }
        id
    }

    pub fn track(&self, r: TrackRef) -> &Track {
        match r {
            TrackRef::Track(i) => &self.tracks[i],
            TrackRef::Return(i) => &self.returns[i],
            TrackRef::Master => &self.master,
        }
    }

    pub fn track_mut(&mut self, r: TrackRef) -> &mut Track {
        match r {
            TrackRef::Track(i) => &mut self.tracks[i],
            TrackRef::Return(i) => &mut self.returns[i],
            TrackRef::Master => &mut self.master,
        }
    }

    pub fn find_track(&self, id: Id) -> Option<TrackRef> {
        if let Some(i) = self.tracks.iter().position(|t| t.id == id) {
            return Some(TrackRef::Track(i));
        }
        if let Some(i) = self.returns.iter().position(|t| t.id == id) {
            return Some(TrackRef::Return(i));
        }
        (self.master.id == id).then_some(TrackRef::Master)
    }

    pub fn all_tracks(&self) -> impl Iterator<Item = (TrackRef, &Track)> {
        self.tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (TrackRef::Track(i), t))
            .chain(self.returns.iter().enumerate().map(|(i, t)| (TrackRef::Return(i), t)))
            .chain(std::iter::once((TrackRef::Master, &self.master)))
    }

    /// Every media reference in the project (for loading, missing-file checks and collection).
    pub fn media_refs(&self) -> Vec<MediaRef> {
        let mut out = vec![];
        for (_, t) in self.all_tracks() {
            for d in &t.devices {
                out.extend(d.sample.iter().cloned());
                out.extend(d.pads.iter().flatten().cloned());
            }
            let clips = t.slots.iter().filter_map(|s| s.clip.as_ref()).chain(t.arrangement.iter().map(|a| &a.clip));
            for c in clips {
                if let ClipContent::Audio(a) = &c.content {
                    out.push(a.media.clone());
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out.dedup();
        out
    }

    pub fn for_each_media_mut(&mut self, mut f: impl FnMut(&mut MediaRef)) {
        let tracks = self.tracks.iter_mut().chain(self.returns.iter_mut()).chain(std::iter::once(&mut self.master));
        for t in tracks {
            for d in t.devices.iter_mut() {
                d.sample.iter_mut().for_each(&mut f);
                d.pads.iter_mut().flatten().for_each(&mut f);
            }
            let clips = t.slots.iter_mut().filter_map(|s| s.clip.as_mut()).chain(t.arrangement.iter_mut().map(|a| &mut a.clip));
            for c in clips {
                if let ClipContent::Audio(a) = &mut c.content {
                    f(&mut a.media);
                }
            }
        }
    }

    /// Structural validation: slot alignment, send counts, sorted automation, ids.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = vec![];
        if self.format != FORMAT_NAME {
            issues.push(format!("unexpected format '{}'", self.format));
        }
        for t in &self.tracks {
            if t.kind == TrackKind::Group && (t.group.is_some() || t.slots.iter().any(|s| s.clip.is_some()) || !t.arrangement.is_empty()) {
                issues.push(format!("group track '{}' cannot hold clips or be nested", t.name));
            }
            if let Some(g) = t.group {
                if !self.tracks.iter().any(|x| x.id == g && x.kind == TrackKind::Group) {
                    issues.push(format!("track '{}' belongs to a missing group", t.name));
                }
            }
            if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Group) {
                issues.push(format!("track '{}' has kind {:?} in the track list", t.name, t.kind));
            }
            if t.slots.len() != self.scenes.len() {
                issues.push(format!("track '{}' has {} slots for {} scenes", t.name, t.slots.len(), self.scenes.len()));
            }
            if t.mixer.sends_db.len() != self.returns.len() {
                issues.push(format!("track '{}' has {} sends for {} returns", t.name, t.mixer.sends_db.len(), self.returns.len()));
            }
            for s in t.slots.iter().filter_map(|s| s.clip.as_ref()) {
                let audio_on_midi = t.kind == TrackKind::Midi && !s.is_midi();
                let midi_on_audio = t.kind == TrackKind::Audio && s.is_midi();
                if audio_on_midi || midi_on_audio {
                    issues.push(format!("clip '{}' does not match track '{}' type", s.name, t.name));
                }
            }
        }
        for (_, t) in self.all_tracks() {
            for lane in &t.automation {
                if lane.points.windows(2).any(|w| w[0].beat > w[1].beat) {
                    issues.push(format!("automation on '{}' is not sorted", t.name));
                }
            }
            for d in &t.devices {
                if d.kind.is_instrument() && t.kind != TrackKind::Midi {
                    issues.push(format!("instrument on non-MIDI track '{}'", t.name));
                }
            }
        }
        if self.returns.iter().any(|r| !r.mixer.sends_db.is_empty()) {
            issues.push("return tracks cannot send (feedback routing is not supported)".into());
        }
        issues
    }
}

impl Track {
    pub fn new(id: Id, name: &str, kind: TrackKind, color: [u8; 3], scenes: usize, returns: usize) -> Self {
        let mut mixer = Mixer::default();
        if matches!(kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Group) {
            mixer.sends_db = vec![SEND_OFF_DB; returns];
        }
        if kind == TrackKind::Audio {
            mixer.input_channel = Some(0);
        }
        Self {
            id,
            name: name.into(),
            kind,
            color,
            mixer,
            devices: vec![],
            slots: if matches!(kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Group) { vec![Slot::default(); scenes] } else { vec![] },
            arrangement: vec![],
            automation: vec![],
            group: None,
        }
    }

    pub fn lane(&self, target: ParamTarget) -> Option<&AutomationLane> {
        self.automation.iter().find(|l| l.target == target)
    }

    pub fn lane_mut(&mut self, target: ParamTarget) -> &mut AutomationLane {
        if let Some(i) = self.automation.iter().position(|l| l.target == target) {
            return &mut self.automation[i];
        }
        self.automation.push(AutomationLane { target, points: vec![] });
        self.automation.last_mut().unwrap()
    }

    /// Removes arrangement material in `[from, to)`, splitting clips that straddle the range.
    pub fn clear_arrangement_range(&mut self, from: Beats, to: Beats, mut new_id: impl FnMut() -> Id) {
        let mut out = Vec::with_capacity(self.arrangement.len());
        for c in self.arrangement.drain(..) {
            if c.end() <= from || c.start >= to {
                out.push(c);
                continue;
            }
            if c.start < from {
                let mut left = c.clone();
                left.length = from - c.start;
                left.fade_out = left.fade_out.min(left.length);
                out.push(left);
            }
            if c.end() > to {
                let mut right = c.clone();
                right.id = new_id();
                right.offset = c.offset + (to - c.start);
                right.start = to;
                right.length = c.end() - to;
                right.fade_in = 0.0;
                out.push(right);
            }
        }
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.arrangement = out;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackRef {
    Track(usize),
    Return(usize),
    Master,
}
