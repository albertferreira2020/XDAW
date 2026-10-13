//! Realtime snapshot of a project, built on the UI thread and handed to the
//! audio thread. Contains only pre-allocated data; runtime state (voices,
//! launchers, smoothers) is carried from the previous graph on swap.

use crate::protocol::{MAX_RETURNS, MAX_TRACKS};
use xdaw_dsp::instruments::{PADS, SampleData};
use xdaw_dsp::util::{db_to_gain, Smoothed};
use xdaw_dsp::{Device, DeviceKind, DeviceMedia, NoteEvent};
use xdaw_project::{self as model, ClipContent, Id, Monitor, ParamTarget, Project, TrackKind, WarpMode};
use xdaw_sequencer::{Beats, ClipQuantize, ClipRegion, LaunchMode, Quantize, TimeSignature, TrackLauncher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const MAX_BLOCK: usize = 256;
pub const MAX_EVENTS: usize = 1024;
pub const MAX_ACTIVE_NOTES: usize = 256;

/// Looks up decoded media by reference; `None` means missing or not yet loaded.
pub trait MediaSource {
    fn get(&self, media: &model::MediaRef) -> Option<Arc<SampleData>>;
}

impl MediaSource for HashMap<String, Arc<SampleData>> {
    fn get(&self, media: &model::MediaRef) -> Option<Arc<SampleData>> {
        HashMap::get(self, &media.path).cloned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtNote {
    pub pitch: u8,
    pub start: Beats,
    pub length: Beats,
    pub velocity: u8,
}

pub enum RtContent {
    /// `markers`: sorted `(beat, source_seconds)` warp markers (empty = constant tempo).
    Audio { data: Option<Arc<SampleData>>, warp: WarpMode, source_bpm: f64, markers: Vec<(Beats, f64)>, env: RtEnvelopes },
    Midi { notes: Vec<RtNote> },
}

/// Clip envelopes over clip-local beats (`(beat, value)`, sorted; empty = off).
#[derive(Default, Clone)]
pub struct RtEnvelopes {
    /// Gain in dB.
    pub gain: Vec<(Beats, f32)>,
    /// Pan -1..1.
    pub pan: Vec<(Beats, f32)>,
}

impl RtEnvelopes {
    pub fn from_model(a: &model::AudioContent) -> Self {
        let pts = |t: model::ClipEnvTarget| -> Vec<(Beats, f32)> {
            a.envelopes.iter().find(|e| e.target == t).map(|e| e.points.iter().map(|p| (p.beat, p.value)).collect()).unwrap_or_default()
        };
        Self { gain: pts(model::ClipEnvTarget::Gain), pan: pts(model::ClipEnvTarget::Pan) }
    }
}

/// Linear interpolation over sorted `(beat, value)` points (no allocation).
#[inline]
pub fn env_at(pts: &[(Beats, f32)], beat: Beats) -> Option<f32> {
    let first = pts.first()?;
    if beat <= first.0 {
        return Some(first.1);
    }
    let idx = pts.partition_point(|p| p.0 <= beat);
    if idx >= pts.len() {
        return pts.last().map(|p| p.1);
    }
    let (a, b) = (pts[idx - 1], pts[idx]);
    let span = b.0 - a.0;
    if span <= 0.0 {
        return Some(b.1);
    }
    Some(a.1 + (b.1 - a.1) * ((beat - a.0) / span) as f32)
}

pub struct RtClip {
    pub id: Id,
    pub region: ClipRegion,
    pub launch_mode: LaunchMode,
    pub quantize: ClipQuantize,
    pub legato: bool,
    pub gain: f32,
    pub transpose: i32,
    pub follow: xdaw_sequencer::FollowActions,
    pub content: RtContent,
}

pub struct RtArrClip {
    /// Short edge fades against clicks (off for frozen renders, which must stay sample-exact).
    pub edges: bool,
    pub start: Beats,
    pub length: Beats,
    pub offset: Beats,
    pub fade_in: Beats,
    pub fade_out: Beats,
    pub clip: RtClip,
}

impl RtArrClip {
    pub fn end(&self) -> Beats {
        self.start + self.length
    }

    /// Song beat at which clip-local elapsed time 0 occurs.
    pub fn origin(&self) -> Beats {
        self.start - self.offset
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RtTarget {
    Volume,
    Pan,
    Send(usize),
    Device { device: usize, param: usize },
}

pub struct RtLane {
    pub target: RtTarget,
    pub points: Vec<(Beats, f32)>,
}

impl RtLane {
    pub fn value_at(&self, beat: Beats) -> Option<f32> {
        let pts = &self.points;
        let first = pts.first()?;
        if beat <= first.0 {
            return Some(first.1);
        }
        let idx = pts.partition_point(|p| p.0 <= beat);
        if idx >= pts.len() {
            return pts.last().map(|p| p.1);
        }
        let (a, b) = (pts[idx - 1], pts[idx]);
        let span = b.0 - a.0;
        if span <= 0.0 {
            return Some(b.1);
        }
        Some(a.1 + (b.1 - a.1) * ((beat - a.0) / span) as f32)
    }
}

pub struct RtDevice {
    pub id: Id,
    pub device: Device,
    pub bypass: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveNote {
    pub pitch: u8,
    pub end: Beats,
    /// Source of the note: clip id, or `u64::MAX` for live input.
    pub source: Id,
}

/// Capture of one live note while recording.
#[derive(Clone, Copy, Debug)]
pub struct HeldInput {
    pub pitch: u8,
    pub velocity: u8,
    pub start: Beats,
}

/// Fading tail of an audio source that just stopped (click suppression).
#[derive(Clone, Copy, Debug)]
pub struct AudioTail {
    pub from_arrangement: bool,
    pub slot: usize,
    pub origin: Beats,
    pub remaining: u32,
    /// First sample of the current block at which the tail sounds.
    pub start_offset: u32,
}

pub const TAIL_SAMPLES: u32 = 96; // 2 ms at 48 kHz

/// Runtime state that survives graph swaps.
pub struct TrackRuntime {
    pub launcher: TrackLauncher,
    pub volume: Smoothed,
    pub pan_l: Smoothed,
    pub pan_r: Smoothed,
    pub sends: [Smoothed; MAX_RETURNS],
    pub active: [Option<ActiveNote>; MAX_ACTIVE_NOTES],
    pub held_input: [Option<HeldInput>; 128],
    pub recording_slot: Option<usize>,
    /// Arrangement recording window while armed, and whether a take is open.
    pub arr_rec: Option<(Beats, Beats)>,
    pub arr_rec_open: bool,
    pub automation_overridden: [bool; 64],
    pub tail: Option<AudioTail>,
    pub fade_in: u32,
    pub clipped: bool,
    /// Song beat at which the playing clip's Follow Action fires.
    pub follow_at: Option<Beats>,
    pub rng: xdaw_dsp::util::Rng,
}

impl TrackRuntime {
    pub fn new(sr: f32) -> Self {
        Self {
            launcher: TrackLauncher::default(),
            volume: Smoothed::new(1.0, sr, 20.0),
            pan_l: Smoothed::new(std::f32::consts::FRAC_1_SQRT_2, sr, 20.0),
            pan_r: Smoothed::new(std::f32::consts::FRAC_1_SQRT_2, sr, 20.0),
            sends: std::array::from_fn(|_| Smoothed::new(0.0, sr, 20.0)),
            active: [None; MAX_ACTIVE_NOTES],
            held_input: [None; 128],
            recording_slot: None,
            arr_rec: None,
            arr_rec_open: false,
            automation_overridden: [false; 64],
            tail: None,
            fade_in: 0,
            clipped: false,
            follow_at: None,
            rng: xdaw_dsp::util::Rng::new(0x5EED_1234),
        }
    }
}

pub struct RtMixer {
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub arm: bool,
    pub monitor: Monitor,
    pub sends_db: [f32; MAX_RETURNS],
}

pub struct RtTrack {
    pub id: Id,
    pub kind: TrackKind,
    /// Index (in `Graph::tracks`) of the group track this track feeds.
    pub output_group: Option<usize>,
    pub devices: Vec<RtDevice>,
    pub slots: Vec<Option<RtClip>>,
    pub slot_has_stop: Vec<bool>,
    pub arrangement: Vec<RtArrClip>,
    pub automation: Vec<RtLane>,
    pub mixer: RtMixer,
    pub rt: TrackRuntime,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    pub events: Vec<NoteEvent>,
}

impl RtTrack {
    pub fn is_audible_with(&self, any_solo: bool) -> bool {
        // Group tracks pass audio of soloed members; see `render_span`.
        !self.mixer.mute && (!any_solo || self.mixer.solo || matches!(self.kind, TrackKind::Return | TrackKind::Master))
    }
}

pub struct Graph {
    pub sample_rate: f64,
    pub tracks: Vec<RtTrack>,
    pub returns: Vec<RtTrack>,
    pub master: RtTrack,
    pub time_signature: TimeSignature,
    pub global_quantize: Quantize,
    pub return_l: Vec<Vec<f32>>,
    pub return_r: Vec<Vec<f32>>,
    /// Group buses, indexed like `tracks` (only group tracks use theirs).
    pub group_l: Vec<Vec<f32>>,
    pub group_r: Vec<Vec<f32>>,
}

impl Graph {
    pub fn any_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.mixer.solo)
    }
}

pub fn compile_clip(clip: &model::Clip, media: &dyn MediaSource, bpm: f64) -> RtClip {
    let content = match &clip.content {
        ClipContent::Audio(a) if a.warp == WarpMode::Stretch && ((a.source_bpm - bpm).abs() > 1e-6 || !a.markers.is_empty()) => {
            // Use the pitch-preserving rendering for this tempo when it is ready;
            // until then fall back to Re-Pitch so playback never stops.
            let key = model::stretch_key_for(a, clip.region.end, bpm);
            match media.get(&model::MediaRef { path: key }) {
                // The rendering already contains the markers' timing at project tempo.
                Some(stretched) => RtContent::Audio { data: Some(stretched), warp: WarpMode::RePitch, source_bpm: bpm, markers: vec![], env: RtEnvelopes::from_model(a) },
                None => RtContent::Audio { data: media.get(&a.media), warp: WarpMode::RePitch, source_bpm: a.source_bpm.max(1.0), markers: a.marker_pairs(), env: RtEnvelopes::from_model(a) },
            }
        }
        ClipContent::Audio(a) => RtContent::Audio {
            data: media.get(&a.media),
            warp: if a.warp == WarpMode::Stretch { WarpMode::RePitch } else { a.warp },
            source_bpm: a.source_bpm.max(1.0),
            markers: a.marker_pairs(),
            env: RtEnvelopes::from_model(a),
        },
        ClipContent::Midi { notes } => {
            let mut notes: Vec<RtNote> = notes
                .iter()
                .map(|n| RtNote {
                    pitch: (n.pitch as i32 + clip.transpose).clamp(0, 127) as u8,
                    start: n.start,
                    length: n.length.max(1e-6),
                    velocity: n.velocity.max(1),
                })
                .collect();
            notes.sort_by(|a, b| a.start.total_cmp(&b.start));
            RtContent::Midi { notes }
        }
    };
    RtClip {
        id: clip.id,
        region: clip.region,
        launch_mode: clip.launch_mode,
        quantize: clip.quantize,
        legato: clip.legato,
        gain: db_to_gain(clip.gain_db),
        transpose: clip.transpose,
        follow: clip.follow,
        content,
    }
}

pub fn compile_arrangement(track: &model::Track, media: &dyn MediaSource, bpm: f64) -> Vec<RtArrClip> {
    let mut v: Vec<RtArrClip> = track
        .arrangement
        .iter()
        .map(|a| RtArrClip {
            edges: true,
            start: a.start,
            length: a.length,
            offset: a.offset,
            fade_in: a.fade_in,
            fade_out: a.fade_out,
            clip: compile_clip(&a.clip, media, bpm),
        })
        .collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    v
}

pub fn compile_automation(track: &model::Track) -> Vec<RtLane> {
    track
        .automation
        .iter()
        .filter(|l| !l.points.is_empty())
        .filter_map(|l| {
            let target = match l.target {
                ParamTarget::Volume => RtTarget::Volume,
                ParamTarget::Pan => RtTarget::Pan,
                ParamTarget::Send(i) => RtTarget::Send(i),
                ParamTarget::Device { device, param } => {
                    RtTarget::Device { device: track.devices.iter().position(|d| d.id == device)?, param }
                }
            };
            Some(RtLane { target, points: l.points.iter().map(|p| (p.beat, p.value)).collect() })
        })
        .collect()
}

fn compile_track(t: &model::Track, media: &dyn MediaSource, sr: f64, bpm: f64) -> RtTrack {
    // A frozen track plays its rendered audio instead of its clips and devices.
    let frozen_clip = t.frozen.as_ref().map(|f| {
        let clip = model::Clip::rendered_audio(0, "Frozen", t.color, f.media.clone(), f.bpm, f.length);
        RtArrClip { edges: false, start: 0.0, length: f.length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: compile_clip(&clip, media, bpm) }
    });
    let live_devices: &[model::DeviceState] = if frozen_clip.is_some() { &[] } else { &t.devices };
    let devices = live_devices
        .iter()
        .map(|d| {
            let mut m = DeviceMedia::default();
            m.sample = d.sample.as_ref().and_then(|s| media.get(s));
            for (i, pad) in d.pads.iter().enumerate().take(PADS) {
                m.pads[i] = pad.as_ref().and_then(|s| media.get(s));
            }
            let mut device = Device::new(d.kind, sr as f32, m);
            for i in 0..d.kind.params().len() {
                device.set_param(i, d.param(i));
            }
            RtDevice { id: d.id, device, bypass: d.bypass }
        })
        .collect();
    let mut sends_db = [model::SEND_OFF_DB; MAX_RETURNS];
    for (i, s) in t.mixer.sends_db.iter().enumerate().take(MAX_RETURNS) {
        sends_db[i] = *s;
    }
    let mut rt = TrackRuntime::new(sr as f32);
    let (pl, pr) = xdaw_dsp::util::pan_gains(t.mixer.pan);
    rt.volume.reset(db_to_gain(t.mixer.volume_db));
    rt.pan_l.reset(pl);
    rt.pan_r.reset(pr);
    for (s, db) in rt.sends.iter_mut().zip(sends_db) {
        s.reset(db_to_gain(db));
    }
    RtTrack {
        id: t.id,
        kind: t.kind,
        output_group: None,
        devices,
        slots: t.slots.iter().map(|s| s.clip.as_ref().filter(|_| frozen_clip.is_none()).map(|c| compile_clip(c, media, bpm))).collect(),
        slot_has_stop: t.slots.iter().map(|s| s.has_stop_button).collect(),
        arrangement: match frozen_clip {
            Some(c) => vec![c],
            None => compile_arrangement(t, media, bpm),
        },
        automation: compile_automation(t),
        mixer: RtMixer {
            volume_db: t.mixer.volume_db,
            pan: t.mixer.pan,
            mute: t.mixer.mute,
            solo: t.mixer.solo,
            arm: t.mixer.arm,
            monitor: t.mixer.monitor,
            sends_db,
        },
        rt,
        l: vec![0.0; MAX_BLOCK],
        r: vec![0.0; MAX_BLOCK],
        events: Vec::with_capacity(MAX_EVENTS),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("too many tracks ({0}, maximum {MAX_TRACKS})")]
    TooManyTracks(usize),
    #[error("too many return tracks ({0}, maximum {MAX_RETURNS})")]
    TooManyReturns(usize),
    #[error("invalid project: {0}")]
    Invalid(String),
}

/// Builds a graph (allocates; never call on the audio thread).
pub fn build_graph(project: &Project, media: &dyn MediaSource, sample_rate: f64) -> Result<Graph, GraphError> {
    if project.tracks.len() > MAX_TRACKS {
        return Err(GraphError::TooManyTracks(project.tracks.len()));
    }
    if project.returns.len() > MAX_RETURNS {
        return Err(GraphError::TooManyReturns(project.returns.len()));
    }
    if let Some(issue) = project.validate().into_iter().next() {
        return Err(GraphError::Invalid(issue));
    }
    let bpm = project.transport.bpm;
    let mut tracks: Vec<RtTrack> = project.tracks.iter().map(|t| compile_track(t, media, sample_rate, bpm)).collect();
    for (i, t) in project.tracks.iter().enumerate() {
        tracks[i].output_group = t.group.and_then(|g| project.tracks.iter().position(|x| x.id == g && x.kind == TrackKind::Group));
    }
    let groups = project.tracks.len();
    Ok(Graph {
        sample_rate,
        tracks,
        returns: project.returns.iter().map(|t| compile_track(t, media, sample_rate, bpm)).collect(),
        master: compile_track(&project.master, media, sample_rate, bpm),
        time_signature: project.transport.time_signature,
        global_quantize: project.transport.global_quantize,
        return_l: vec![vec![0.0; MAX_BLOCK]; project.returns.len()],
        return_r: vec![vec![0.0; MAX_BLOCK]; project.returns.len()],
        group_l: vec![vec![0.0; MAX_BLOCK]; groups],
        group_r: vec![vec![0.0; MAX_BLOCK]; groups],
    })
}

/// Moves runtime state from a retiring graph into its replacement (audio thread; no allocation).
pub fn transfer_state(old: &mut Graph, new: &mut Graph) {
    fn track(old: &mut [RtTrack], new: &mut RtTrack) {
        let Some(o) = old.iter_mut().find(|o| o.id == new.id) else { return };
        // Keep launcher/voice state; mixer targets come from the new snapshot.
        std::mem::swap(&mut o.rt.launcher, &mut new.rt.launcher);
        std::mem::swap(&mut o.rt.active, &mut new.rt.active);
        std::mem::swap(&mut o.rt.held_input, &mut new.rt.held_input);
        std::mem::swap(&mut o.rt.recording_slot, &mut new.rt.recording_slot);
        new.rt.arr_rec = o.rt.arr_rec;
        new.rt.arr_rec_open = o.rt.arr_rec_open;
        std::mem::swap(&mut o.rt.automation_overridden, &mut new.rt.automation_overridden);
        new.rt.tail = o.rt.tail.take();
        new.rt.follow_at = o.rt.follow_at;
        new.rt.rng = o.rt.rng;
        let (vol, pl, pr) = (new.rt.volume.target(), new.rt.pan_l.target(), new.rt.pan_r.target());
        std::mem::swap(&mut o.rt.volume, &mut new.rt.volume);
        std::mem::swap(&mut o.rt.pan_l, &mut new.rt.pan_l);
        std::mem::swap(&mut o.rt.pan_r, &mut new.rt.pan_r);
        new.rt.volume.set_target(vol);
        new.rt.pan_l.set_target(pl);
        new.rt.pan_r.set_target(pr);
        for (i, s) in new.rt.sends.iter_mut().enumerate() {
            let t = s.target();
            *s = o.rt.sends[i];
            s.set_target(t);
        }
        // A playing slot whose clip disappeared stops.
        if let Some(p) = new.rt.launcher.playing {
            if new.slots.get(p.slot).is_none_or(Option::is_none) && new.rt.recording_slot != Some(p.slot) {
                new.rt.launcher.stop_now();
            }
        }
        // Devices with the same identity keep their DSP state (voices, delay lines).
        for nd in new.devices.iter_mut() {
            if let Some(od) = o.devices.iter_mut().find(|od| od.id == nd.id && od.device.kind() == nd.device.kind()) {
                std::mem::swap(&mut od.device, &mut nd.device);
                // `nd.device` now has the running state; bring the new snapshot's
                // media and parameters in, and leave the old media to retire.
                nd.device.exchange_media(&mut od.device);
                nd.device.copy_params_from(&od.device);
            }
        }
    }
    for t in new.tracks.iter_mut() {
        track(&mut old.tracks, t);
    }
    for t in new.returns.iter_mut() {
        track(&mut old.returns, t);
    }
    track(std::slice::from_mut(&mut old.master), &mut new.master);
}

pub fn device_kind_is_instrument(kind: DeviceKind) -> bool {
    kind.is_instrument()
}

/// Utility for tests and tools: absolute or project-relative path display.
pub fn media_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
