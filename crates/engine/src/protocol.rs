//! Messages between the UI thread and the audio thread, plus lock-free status.
//!
//! UI → audio: bounded SPSC ring of `Command`. Large payloads travel boxed and
//! the audio thread returns replaced payloads through the garbage ring so that
//! deallocation never happens on the audio thread.

use crate::graph::{Graph, RtArrClip, RtClip, RtLane};
use xdaw_dsp::NoteKind;
use xdaw_sequencer::{Beats, Quantize, TimeSignature};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};

pub const MAX_TRACKS: usize = 128;
pub const MAX_RETURNS: usize = 12;
/// Status slots: tracks, then returns, then master.
pub const STATUS_SLOTS: usize = MAX_TRACKS + MAX_RETURNS + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackIx {
    Track(u16),
    Return(u16),
    Master,
}

impl TrackIx {
    pub fn status_slot(self) -> usize {
        match self {
            TrackIx::Track(i) => i as usize,
            TrackIx::Return(i) => MAX_TRACKS + i as usize,
            TrackIx::Master => MAX_TRACKS + MAX_RETURNS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MixParam {
    Volume,
    Pan,
    Mute,
    Solo,
    Arm,
    Send(u16),
    /// 0 = In, 1 = Auto, 2 = Off.
    Monitor,
}

pub enum Command {
    SetGraph(Box<Graph>),
    /// Replaces a Session slot. The box returns the previous value through the garbage ring.
    SetClip { track: u16, slot: u16, clip: Box<Option<RtClip>> },
    SetArrangement { track: TrackIx, clips: Box<Vec<RtArrClip>> },
    SetAutomation { track: TrackIx, lanes: Box<Vec<RtLane>> },
    Play,
    /// Stop; a second Stop while stopped returns to song start.
    Stop,
    /// Resume from the current position without relocating.
    Continue,
    Locate(Beats),
    SetBpm(f64),
    SetTimeSignature(TimeSignature),
    SetGlobalQuantize(Quantize),
    SetLoop { enabled: bool, start: Beats, length: Beats },
    SetMetronome(bool),
    SetCountIn(u32),
    /// Start playback for recording, preceded by the configured count-in.
    PlayWithCountIn,
    /// Clip launch control pressed (`down`) or released.
    LaunchSlot { track: u16, slot: u16, down: bool },
    LaunchScene { slot: u16 },
    StopTrack { track: u16 },
    StopAllClips,
    BackToArrangement,
    /// Begin/stop capturing input into a Session slot (MIDI recording / overdub).
    SessionRecord { track: u16, slot: u16, on: bool },
    SetMixer { track: TrackIx, param: MixParam, value: f32 },
    SetDeviceParam { track: TrackIx, device: u16, param: u16, value: f32 },
    SetDeviceBypass { track: TrackIx, device: u16, bypass: bool },
    /// Live note input (computer keyboard, on-screen keys) to one track.
    Note { track: u16, kind: NoteKind },
    Panic,
}

/// Returned from the audio thread to be dropped elsewhere.
pub enum Garbage {
    Graph(Box<Graph>),
    Clip(Box<Option<RtClip>>),
    Arrangement(Box<Vec<RtArrClip>>),
    Automation(Box<Vec<RtLane>>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineEvent {
    ClipStarted { track: u16, slot: u16, at: Beats, origin: Beats },
    ClipStopped { track: u16, slot: u16, at: Beats },
    /// A stop fired on a track that had no Session clip (Arrangement released).
    TrackReleased { track: u16, at: Beats },
    TransportStarted { at: Beats },
    TransportStopped { at: Beats },
    /// A recorded note finished (song beats). `slot` is the recording/overdub slot.
    NoteRecorded { track: u16, slot: u16, pitch: u8, velocity: u8, start: Beats, end: Beats, local_start: Beats },
    RecordingStarted { track: u16, slot: u16, at: Beats },
    RecordingStopped { track: u16, slot: u16, at: Beats },
    AutomationOverridden { track: TrackIx },
    /// The arrangement loop wrapped.
    Looped { from: Beats, to: Beats },
}

#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

#[derive(Default)]
pub struct AtomicF64(AtomicU64);

impl AtomicF64 {
    pub fn store(&self, v: f64) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn load(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

pub const NO_SLOT: i32 = -1;
pub const QUEUED_STOP: i32 = -2;

#[derive(Default)]
pub struct TrackStatus {
    /// Playing session slot or `NO_SLOT`.
    pub playing_slot: AtomicI32,
    /// Queued slot, `QUEUED_STOP`, or `NO_SLOT`.
    pub queued: AtomicI32,
    pub recording_slot: AtomicI32,
    /// Clip-local playback position (beats) of the playing clip.
    pub clip_position: AtomicF64,
    pub arrangement_active: AtomicBool,
    pub automation_overridden: AtomicBool,
    pub peak_l: AtomicF32,
    pub peak_r: AtomicF32,
    pub clipped: AtomicBool,
}

/// Callback timing histogram: 64 buckets of 0.1 ms, last bucket open-ended.
pub const HIST_BUCKETS: usize = 64;

pub struct CallbackStats {
    pub count: AtomicU64,
    pub last_ns: AtomicU64,
    pub max_ns: AtomicU64,
    pub total_ns: AtomicU64,
    /// Callbacks whose processing exceeded the buffer duration.
    pub deadline_misses: AtomicU64,
    pub budget_ns: AtomicU64,
    pub histogram: [AtomicU32; HIST_BUCKETS],
    pub frames_last: AtomicU32,
}

impl Default for CallbackStats {
    fn default() -> Self {
        Self {
            count: AtomicU64::new(0),
            last_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            deadline_misses: AtomicU64::new(0),
            budget_ns: AtomicU64::new(0),
            histogram: std::array::from_fn(|_| AtomicU32::new(0)),
            frames_last: AtomicU32::new(0),
        }
    }
}

impl CallbackStats {
    pub fn record(&self, ns: u64, frames: u32, sample_rate: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.last_ns.store(ns, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        self.frames_last.store(frames, Ordering::Relaxed);
        let budget = (frames as f64 / sample_rate * 1e9) as u64;
        self.budget_ns.store(budget, Ordering::Relaxed);
        if ns > budget {
            self.deadline_misses.fetch_add(1, Ordering::Relaxed);
        }
        let bucket = ((ns / 100_000) as usize).min(HIST_BUCKETS - 1);
        self.histogram[bucket].fetch_add(1, Ordering::Relaxed);
    }

    /// Percentile estimate (upper bucket edge, in ms).
    pub fn percentile_ms(&self, p: f64) -> f64 {
        let counts: Vec<u64> = self.histogram.iter().map(|h| h.load(Ordering::Relaxed) as u64).collect();
        let total: u64 = counts.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let target = (total as f64 * p).ceil() as u64;
        let mut acc = 0;
        for (i, c) in counts.iter().enumerate() {
            acc += c;
            if acc >= target {
                return (i + 1) as f64 * 0.1;
            }
        }
        HIST_BUCKETS as f64 * 0.1
    }

    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.max_ns.store(0, Ordering::Relaxed);
        self.total_ns.store(0, Ordering::Relaxed);
        self.deadline_misses.store(0, Ordering::Relaxed);
        for h in &self.histogram {
            h.store(0, Ordering::Relaxed);
        }
    }
}

pub struct SharedStatus {
    pub playing: AtomicBool,
    pub beat: AtomicF64,
    pub bpm: AtomicF64,
    pub sample_rate: AtomicF64,
    pub sample_pos: AtomicI64,
    pub counting_in: AtomicBool,
    pub tracks: Vec<TrackStatus>,
    pub callback: CallbackStats,
    pub dropped_events: AtomicU32,
    pub dropped_commands: AtomicU32,
    pub stream_errors: AtomicU32,
    pub input_overruns: AtomicU32,
    pub nonfinite_blocks: AtomicU32,
    pub input_peak: AtomicF32,
}

impl SharedStatus {
    pub fn new() -> Self {
        let tracks = (0..STATUS_SLOTS)
            .map(|_| {
                let t = TrackStatus::default();
                t.playing_slot.store(NO_SLOT, Ordering::Relaxed);
                t.queued.store(NO_SLOT, Ordering::Relaxed);
                t.recording_slot.store(NO_SLOT, Ordering::Relaxed);
                t.arrangement_active.store(true, Ordering::Relaxed);
                t
            })
            .collect();
        Self {
            playing: AtomicBool::new(false),
            beat: AtomicF64::default(),
            bpm: AtomicF64::default(),
            sample_rate: AtomicF64::default(),
            sample_pos: AtomicI64::new(0),
            counting_in: AtomicBool::new(false),
            tracks,
            callback: CallbackStats::default(),
            dropped_events: AtomicU32::new(0),
            dropped_commands: AtomicU32::new(0),
            stream_errors: AtomicU32::new(0),
            input_overruns: AtomicU32::new(0),
            nonfinite_blocks: AtomicU32::new(0),
            input_peak: AtomicF32::default(),
        }
    }

    pub fn track(&self, ix: TrackIx) -> &TrackStatus {
        &self.tracks[ix.status_slot()]
    }
}

impl Default for SharedStatus {
    fn default() -> Self {
        Self::new()
    }
}
