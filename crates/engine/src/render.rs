//! The renderer: one code path for realtime playback and offline export.
//!
//! Realtime rules: no allocation, no locks, no I/O and no formatting in
//! `process`. Commands are drained at the start of each callback; replaced
//! payloads are handed back through the garbage ring.

use crate::graph::*;
use crate::protocol::*;
use xdaw_dsp::util::{db_to_gain, pan_gains};
use xdaw_dsp::{Ctx, NoteEvent, NoteKind};
use xdaw_project::{Monitor, TrackKind, WarpMode};
use xdaw_sequencer::{Beats, Clock, LaunchEvent, LaunchMode, Quantize, TrackLauncher};
use rtrb::{Consumer, Producer};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Raw MIDI bytes from an input port.
#[derive(Clone, Copy, Debug, Default)]
pub struct MidiInput {
    pub bytes: [u8; 3],
    pub len: u8,
}

/// Source id for notes played live (not from a clip).
const LIVE: u64 = u64::MAX;
const MAX_LIVE: usize = 256;

pub struct Rings {
    pub commands: Consumer<Command>,
    pub garbage: Producer<Garbage>,
    pub events: Producer<EngineEvent>,
    pub midi_in: Option<Consumer<MidiInput>>,
}

struct EventSink<'a> {
    ring: &'a mut Producer<EngineEvent>,
    status: &'a SharedStatus,
}

impl EventSink<'_> {
    fn push(&mut self, e: EngineEvent) {
        if self.ring.push(e).is_err() {
            self.status.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    /// Clock sample of the span's first frame.
    sample: i64,
    n: usize,
    clock: Clock,
    playing: bool,
    /// False during count-in: transport runs but tracks stay silent.
    audible: bool,
    b0: Beats,
    b1: Beats,
}

impl Span {
    fn beat(&self, offset: usize) -> Beats {
        self.clock.beat_at(self.sample + offset as i64)
    }

    /// Offset of `beat` clamped into `[lo, n)`.
    fn offset(&self, beat: Beats, lo: usize) -> usize {
        let exact = self.clock.sample_at(beat) - self.sample as f64;
        ((exact - 1e-6).ceil().max(lo as f64) as usize).min(self.n.saturating_sub(1)).max(lo)
    }
}

#[derive(Clone, Copy, Default)]
struct Click {
    phase: f32,
    freq: f32,
    remaining: u32,
}

pub struct Renderer {
    graph: Box<Graph>,
    clock: Clock,
    sample: i64,
    playing: bool,
    start_marker: Beats,
    loop_enabled: bool,
    loop_start: Beats,
    loop_len: Beats,
    metronome: bool,
    count_in_bars: u32,
    count_in_until: Option<Beats>,
    pub include_metronome: bool,
    rings: Rings,
    status: Arc<SharedStatus>,
    live: [(Option<u16>, NoteKind); MAX_LIVE],
    live_len: usize,
    master_l: Vec<f32>,
    master_r: Vec<f32>,
    click: Click,
    input: Option<crate::audio_io::InputTap>,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    rec: Option<Producer<crate::recorder::RecMsg>>,
    /// Track currently streaming to the disk writer (one take at a time).
    rec_track: Option<usize>,
    /// Output frames rendered since creation, independent of transport state.
    frames_total: u64,
}

/// Pushes an event only if capacity allows (never reallocates on the audio thread).
#[inline]
fn push_ev(v: &mut Vec<NoteEvent>, e: NoteEvent) {
    if v.len() < v.capacity() {
        v.push(e);
    }
}

impl Renderer {
    pub fn new(graph: Box<Graph>, bpm: f64, rings: Rings, status: Arc<SharedStatus>) -> Self {
        let sr = graph.sample_rate;
        status.sample_rate.store(sr);
        status.bpm.store(bpm);
        Self {
            graph,
            clock: Clock::new(sr, bpm),
            sample: 0,
            playing: false,
            start_marker: 0.0,
            loop_enabled: false,
            loop_start: 0.0,
            loop_len: 16.0,
            metronome: false,
            count_in_bars: 0,
            count_in_until: None,
            include_metronome: true,
            rings,
            status,
            live: [(None, NoteKind::AllOff); MAX_LIVE],
            live_len: 0,
            master_l: vec![0.0; MAX_BLOCK],
            master_r: vec![0.0; MAX_BLOCK],
            click: Click::default(),
            input: None,
            in_l: vec![0.0; MAX_BLOCK],
            in_r: vec![0.0; MAX_BLOCK],
            rec: None,
            rec_track: None,
            frames_total: 0,
        }
    }

    pub fn attach_input(&mut self, tap: crate::audio_io::InputTap) {
        self.input = Some(tap);
    }

    pub fn attach_recorder(&mut self, rec: Producer<crate::recorder::RecMsg>) {
        self.rec = Some(rec);
    }

    /// Pulls `n` input frames (zeros if the input is late; excess latency is dropped).
    fn read_input(&mut self, n: usize) {
        let Some(tap) = self.input.as_mut() else { return };
        let ch = tap.channels.max(1);
        let available = tap.consumer.slots() / ch;
        // Keep input-to-output latency bounded to ~4 blocks.
        let excess = available.saturating_sub(n + 4 * MAX_BLOCK);
        if excess > 0 {
            for _ in 0..excess * ch {
                let _ = tap.consumer.pop();
            }
            self.status.input_overruns.fetch_add(1, Ordering::Relaxed);
        }
        for i in 0..n {
            let mut frame = [0.0f32; 2];
            let mut ok = true;
            for c in 0..ch {
                match tap.consumer.pop() {
                    Ok(x) => {
                        if c < 2 {
                            frame[c] = x;
                        }
                    }
                    Err(_) => ok = false,
                }
            }
            if !ok {
                frame = [0.0; 2];
            }
            if ch == 1 {
                frame[1] = frame[0];
            }
            self.in_l[i] = frame[0];
            self.in_r[i] = frame[1];
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.graph.sample_rate
    }

    pub fn beat(&self) -> Beats {
        self.clock.beat_at(self.sample)
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    fn sink(&mut self) -> EventSink<'_> {
        EventSink { ring: &mut self.rings.events, status: &self.status }
    }

    fn retire(&mut self, g: Garbage) {
        if let Err(rtrb::PushError::Full(g)) = self.rings.garbage.push(g) {
            // Never free on the audio thread: leak instead (counted).
            std::mem::forget(g);
            self.status.dropped_commands.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn track_mut(&mut self, ix: TrackIx) -> Option<&mut RtTrack> {
        match ix {
            TrackIx::Track(i) => self.graph.tracks.get_mut(i as usize),
            TrackIx::Return(i) => self.graph.returns.get_mut(i as usize),
            TrackIx::Master => Some(&mut self.graph.master),
        }
    }

    // ------------------------------------------------------------ commands

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rings.commands.pop() {
            self.apply(cmd);
        }
        if let Some(rx) = self.rings.midi_in.as_mut() {
            while let Ok(m) = rx.pop() {
                if let Some(kind) = xdaw_midi::to_note_kind(&m.bytes[..m.len as usize]) {
                    if self.live_len < MAX_LIVE {
                        self.live[self.live_len] = (None, kind);
                        self.live_len += 1;
                    }
                }
            }
        }
    }

    pub fn apply(&mut self, cmd: Command) {
        let now = self.beat();
        let sig = self.graph.time_signature;
        let gq = self.graph.global_quantize;
        match cmd {
            Command::SetGraph(mut g) => {
                transfer_state(&mut self.graph, &mut g);
                let old = std::mem::replace(&mut self.graph, g);
                self.retire(Garbage::Graph(old));
            }
            Command::SetClip { track, slot, mut clip } => {
                // The incoming box carries the old value back out: no allocation here.
                if let Some(t) = self.graph.tracks.get_mut(track as usize) {
                    if let Some(s) = t.slots.get_mut(slot as usize) {
                        std::mem::swap(s, &mut *clip);
                        let playing_here = t.rt.launcher.playing.is_some_and(|p| p.slot == slot as usize);
                        if s.is_none() && playing_here && t.rt.recording_slot != Some(slot as usize) {
                            end_clip_notes(t, 0);
                            t.rt.launcher.stop_now();
                        }
                    }
                }
                self.retire(Garbage::Clip(clip));
            }
            Command::SetArrangement { track, clips } => {
                let Some(t) = self.track_mut(track) else { return };
                let mut clips = clips;
                std::mem::swap(&mut t.arrangement, &mut *clips);
                self.retire(Garbage::Arrangement(clips));
            }
            Command::SetAutomation { track, lanes } => {
                let Some(t) = self.track_mut(track) else { return };
                let mut lanes = lanes;
                std::mem::swap(&mut t.automation, &mut *lanes);
                self.retire(Garbage::Automation(lanes));
            }
            Command::Play => {
                if !self.playing {
                    self.clock.locate(self.sample, self.start_marker);
                    self.start_transport();
                }
            }
            Command::Continue => {
                if !self.playing {
                    self.start_transport();
                }
            }
            Command::PlayWithCountIn => {
                if !self.playing {
                    let start = self.start_marker;
                    let lead = self.count_in_bars as f64 * sig.bar_beats();
                    self.clock.locate(self.sample, start - lead);
                    self.count_in_until = (lead > 0.0).then_some(start);
                    self.start_transport();
                }
            }
            Command::Stop => {
                if self.playing {
                    self.stop_transport();
                } else {
                    self.start_marker = 0.0;
                    self.clock.locate(self.sample, 0.0);
                }
            }
            Command::Locate(beat) => {
                if self.playing {
                    let delta = beat - now;
                    self.shift_session_time(delta);
                    self.end_clip_notes_all();
                    self.clock.locate(self.sample, beat);
                } else {
                    self.start_marker = beat;
                    self.clock.locate(self.sample, beat);
                }
            }
            Command::SetBpm(bpm) => {
                self.clock.set_bpm_at(self.sample, bpm);
                self.status.bpm.store(self.clock.bpm());
            }
            Command::SetTimeSignature(s) => self.graph.time_signature = s,
            Command::SetGlobalQuantize(q) => self.graph.global_quantize = q,
            Command::SetLoop { enabled, start, length } => {
                self.loop_enabled = enabled;
                self.loop_start = start;
                self.loop_len = length.max(0.25);
            }
            Command::SetMetronome(on) => self.metronome = on,
            Command::SetCountIn(bars) => self.count_in_bars = bars,
            Command::LaunchSlot { track, slot, down } => self.launch_slot(track as usize, slot as usize, down, now),
            Command::LaunchScene { slot } => {
                let immediate = !self.playing;
                self.ensure_playing_for_launch();
                let now = self.beat();
                let slot = slot as usize;
                for t in self.graph.tracks.iter_mut() {
                    let q = |c: &RtClip| if immediate { Quantize::None } else { c.quantize.resolve(gq) };
                    match t.slots.get(slot) {
                        Some(Some(c)) => {
                            let (qq, legato) = (q(c), c.legato);
                            t.rt.launcher.request_launch(slot, now, qq, sig, legato);
                        }
                        Some(None) if t.slot_has_stop.get(slot).copied().unwrap_or(true) => {
                            if t.rt.launcher.playing.is_some() || t.rt.launcher.pending.is_some() {
                                t.rt.launcher.request_stop(now, gq, sig);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Command::StopTrack { track } => {
                if let Some(t) = self.graph.tracks.get_mut(track as usize) {
                    if self.playing {
                        t.rt.launcher.request_stop(now, gq, sig);
                    }
                }
            }
            Command::StopAllClips => {
                if self.playing {
                    for t in self.graph.tracks.iter_mut() {
                        if t.rt.launcher.playing.is_some() || t.rt.launcher.pending.is_some() {
                            t.rt.launcher.request_stop(now, gq, sig);
                        }
                    }
                }
            }
            Command::BackToArrangement => {
                for ti in 0..self.graph.tracks.len() {
                    self.end_source(ti, 0);
                    let t = &mut self.graph.tracks[ti];
                    t.rt.launcher.back_to_arrangement();
                    t.rt.automation_overridden = [false; 64];
                }
                for t in self.graph.returns.iter_mut().chain(std::iter::once(&mut self.graph.master)) {
                    t.rt.automation_overridden = [false; 64];
                }
            }
            Command::SessionRecord { track, slot, on } => self.session_record(track as usize, slot as usize, on, now),
            Command::SetMixer { track, param, value } => {
                let playing = self.playing;
                let Some(t) = self.track_mut(track) else { return };
                let overridden = set_mixer(t, param, value, playing);
                if overridden {
                    self.sink().push(EngineEvent::AutomationOverridden { track });
                }
            }
            Command::SetDeviceParam { track, device, param, value } => {
                let playing = self.playing;
                let Some(t) = self.track_mut(track) else { return };
                if let Some(d) = t.devices.get_mut(device as usize) {
                    d.device.set_param(param as usize, value);
                }
                let target = RtTarget::Device { device: device as usize, param: param as usize };
                if playing && override_lane(t, target) {
                    self.sink().push(EngineEvent::AutomationOverridden { track });
                }
            }
            Command::SetDeviceBypass { track, device, bypass } => {
                if let Some(t) = self.track_mut(track) {
                    if let Some(d) = t.devices.get_mut(device as usize) {
                        d.bypass = bypass;
                        // Voices may have missed note-offs while bypassed.
                        if d.device.kind().is_instrument() {
                            push_ev(&mut t.events, NoteEvent { offset: 0, kind: NoteKind::AllOff });
                        }
                    }
                }
            }
            Command::Note { track, kind } => {
                if self.live_len < MAX_LIVE {
                    self.live[self.live_len] = (Some(track), kind);
                    self.live_len += 1;
                }
            }
            Command::Panic => {
                for t in self.all_tracks_mut() {
                    t.rt.active = [None; MAX_ACTIVE_NOTES];
                    t.rt.held_input = [None; 128];
                    push_ev(&mut t.events, NoteEvent { offset: 0, kind: NoteKind::AllOff });
                }
            }
        }
    }

    fn all_tracks_mut(&mut self) -> impl Iterator<Item = &mut RtTrack> {
        self.graph.tracks.iter_mut().chain(self.graph.returns.iter_mut()).chain(std::iter::once(&mut self.graph.master))
    }

    fn ensure_playing_for_launch(&mut self) {
        if !self.playing {
            self.clock.locate(self.sample, self.start_marker);
            self.start_transport();
        }
    }

    fn start_transport(&mut self) {
        self.playing = true;
        let at = self.beat();
        self.sink().push(EngineEvent::TransportStarted { at });
    }

    fn stop_transport(&mut self) {
        let now = self.beat();
        for ti in 0..self.graph.tracks.len() {
            self.end_source(ti, 0);
            if self.graph.tracks[ti].rt.recording_slot.is_some() {
                self.session_record(ti, 0, false, now);
            }
            let t = &mut self.graph.tracks[ti];
            if let Some(slot) = t.rt.launcher.stop_now() {
                let ev = EngineEvent::ClipStopped { track: ti as u16, slot: slot as u16, at: now };
                self.sink().push(ev);
            }
        }
        for t in self.all_tracks_mut() {
            push_ev(&mut t.events, NoteEvent { offset: 0, kind: NoteKind::AllOff });
            t.rt.active = [None; MAX_ACTIVE_NOTES];
        }
        self.playing = false;
        self.count_in_until = None;
        self.start_marker = now;
        self.sink().push(EngineEvent::TransportStopped { at: now });
    }

    fn launch_slot(&mut self, ti: usize, slot: usize, down: bool, now: Beats) {
        let sig = self.graph.time_signature;
        let gq = self.graph.global_quantize;
        let Some(t) = self.graph.tracks.get(ti) else { return };
        let Some(clip) = t.slots.get(slot).and_then(Option::as_ref) else {
            if down && t.slot_has_stop.get(slot).copied().unwrap_or(true) && self.playing {
                self.graph.tracks[ti].rt.launcher.request_stop(now, gq, sig);
            }
            return;
        };
        let (mode, cq, legato) = (clip.launch_mode, clip.quantize, clip.legato);
        if down {
            let was_stopped = !self.playing;
            self.ensure_playing_for_launch();
            let now = self.beat();
            let q = if was_stopped { Quantize::None } else { cq.resolve(gq) };
            let l = &mut self.graph.tracks[ti].rt.launcher;
            let is_current = l.playing.is_some_and(|p| p.slot == slot) && l.pending.is_none();
            match mode {
                LaunchMode::Toggle if is_current => l.request_stop(now, q, sig),
                LaunchMode::Repeat => l.request_repeat(slot, now, q, sig),
                _ => l.request_launch(slot, now, q, sig, legato),
            }
        } else {
            let l = &mut self.graph.tracks[ti].rt.launcher;
            match mode {
                LaunchMode::Gate => {
                    let ours = l.playing.is_some_and(|p| p.slot == slot)
                        || matches!(l.pending, Some(xdaw_sequencer::Pending::Launch { slot: s, .. }) if s == slot);
                    if ours {
                        l.request_stop(now, cq.resolve(gq), sig);
                    }
                }
                LaunchMode::Repeat => l.release_repeat(slot),
                _ => {}
            }
        }
    }

    fn session_record(&mut self, ti: usize, slot: usize, on: bool, now: Beats) {
        let sig = self.graph.time_signature;
        let gq = self.graph.global_quantize;
        let Some(t) = self.graph.tracks.get_mut(ti) else { return };
        if t.kind == TrackKind::Audio && on && (self.input.is_none() || self.rec.is_none() || self.rec_track.is_some()) {
            return;
        }
        if t.kind == TrackKind::Audio && on && t.slots.get(slot).is_some_and(Option::is_some) {
            return; // audio overdub is not supported
        }
        if on {
            let playing_here = t.rt.launcher.playing.is_some_and(|p| p.slot == slot);
            t.rt.recording_slot = Some(slot);
            if playing_here {
                // Overdub into the running clip from now on.
                let ev = EngineEvent::RecordingStarted { track: ti as u16, slot: slot as u16, at: now };
                self.sink().push(ev);
            } else {
                let was_stopped = !self.playing;
                self.ensure_playing_for_launch();
                let now = self.beat();
                let q = if was_stopped { Quantize::None } else { gq };
                self.graph.tracks[ti].rt.launcher.request_launch(slot, now, q, sig, false);
            }
        } else if let Some(rec) = t.rt.recording_slot.take() {
            // Close held notes at `now`.
            let origin = t.rt.launcher.playing.map(|p| p.origin).unwrap_or(now);
            let region = t.slots.get(rec).and_then(Option::as_ref).map(|c| c.region);
            let mut pending = [None; 128];
            for (p, h) in t.rt.held_input.iter_mut().enumerate() {
                if let Some(h) = h.take() {
                    pending[p] = Some(h);
                }
            }
            for h in pending.into_iter().flatten() {
                let local = local_position(region, h.start - origin);
                let ev = EngineEvent::NoteRecorded {
                    track: ti as u16,
                    slot: rec as u16,
                    pitch: h.pitch,
                    velocity: h.velocity,
                    start: h.start,
                    end: now,
                    local_start: local,
                };
                self.sink().push(ev);
            }
            if self.rec_track == Some(ti) {
                if let Some(tx) = self.rec.as_mut() {
                    let _ = tx.push(crate::recorder::RecMsg::Stop);
                }
                self.rec_track = None;
            }
            self.sink().push(EngineEvent::RecordingStopped { track: ti as u16, slot: rec as u16, at: now });
        }
    }

    /// Ends the current source on a track: clip notes off at `offset`, audio tail.
    fn end_source(&mut self, ti: usize, offset: usize) {
        end_source_of(&mut self.graph.tracks[ti], offset);
    }

    /// Silences every clip source while effects keep running (export tail).
    pub fn release_all_sources(&mut self) {
        for t in self.graph.tracks.iter_mut() {
            end_source_of(t, 0);
            t.rt.launcher.stop_now();
            t.rt.launcher.arrangement_active = false;
        }
    }

    fn end_clip_notes_all(&mut self) {
        for t in self.graph.tracks.iter_mut() {
            end_clip_notes(t, 0);
        }
    }

    /// Keeps Session clips in phase when song time jumps (loop wrap, locate).
    fn shift_session_time(&mut self, delta: Beats) {
        for t in self.graph.tracks.iter_mut() {
            shift_launcher(&mut t.rt.launcher, delta);
            for n in t.rt.active.iter_mut().flatten() {
                n.end += delta;
            }
            for h in t.rt.held_input.iter_mut().flatten() {
                h.start += delta;
            }
            if let Some(tail) = t.rt.tail.as_mut() {
                tail.origin += delta;
            }
        }
    }

    // ------------------------------------------------------------ processing

    /// Renders interleaved output; `channels` >= 1. Realtime entry point.
    pub fn process_interleaved(&mut self, out: &mut [f32], channels: usize) {
        self.drain_commands();
        let frames = out.len() / channels.max(1);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            self.read_input(n);
            self.render_chunk(n);
            for i in 0..n {
                let base = (done + i) * channels;
                if channels == 1 {
                    out[base] = (self.master_l[i] + self.master_r[i]) * 0.5;
                } else {
                    out[base] = self.master_l[i];
                    out[base + 1] = self.master_r[i];
                    for c in 2..channels {
                        out[base + c] = 0.0;
                    }
                }
            }
            done += n;
        }
        self.publish_status();
    }

    /// Renders planar stereo (offline export and tests).
    pub fn process_planar(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.drain_commands();
        let mut done = 0;
        while done < l.len() {
            let n = (l.len() - done).min(MAX_BLOCK);
            self.read_input(n);
            self.render_chunk(n);
            l[done..done + n].copy_from_slice(&self.master_l[..n]);
            r[done..done + n].copy_from_slice(&self.master_r[..n]);
            done += n;
        }
        self.publish_status();
    }

    fn render_chunk(&mut self, n: usize) {
        let mut done = 0;
        while done < n {
            let remaining = n - done;
            let mut len = remaining;
            let mut wrap = false;
            if self.playing && self.loop_enabled && self.count_in_until.is_none() {
                let end = self.loop_start + self.loop_len;
                let b0 = self.beat();
                let b1 = self.clock.beat_at(self.sample + remaining as i64);
                if b0 < end - 1e-9 && b1 >= end - 1e-9 {
                    let off = self.clock.offset_in_block(end, self.sample, remaining).unwrap_or(remaining);
                    len = off.max(1).min(remaining);
                    wrap = true;
                }
                // Positioned beyond the loop end: play through; looping applies only when crossing it.
            }
            self.render_span(done, len);
            done += len;
            if wrap {
                let from = self.beat();
                let delta = self.loop_start - from;
                self.shift_session_time(delta);
                for t in self.graph.tracks.iter_mut() {
                    end_arrangement_notes(t, 0);
                }
                self.clock.locate(self.sample, self.loop_start);
                let to = self.loop_start;
                self.sink().push(EngineEvent::Looped { from, to });
            }
        }
    }

    fn render_span(&mut self, at: usize, n: usize) {
        let b0 = self.beat();
        let counting = self.count_in_until.is_some_and(|c| b0 < c);
        if self.count_in_until.is_some() && !counting {
            self.count_in_until = None;
        }
        self.status.counting_in.store(counting, Ordering::Relaxed);
        let span = Span {
            sample: self.sample,
            n,
            clock: self.clock,
            playing: self.playing,
            audible: self.playing && !counting,
            b0,
            b1: if self.playing { self.clock.beat_at(self.sample + n as i64) } else { b0 },
        };
        let ctx = Ctx { sample_rate: self.graph.sample_rate as f32, bpm: self.clock.bpm() as f32 };
        let sig = self.graph.time_signature;
        let any_solo = self.graph.any_solo();
        let live_len = self.live_len;
        self.live_len = 0;
        let live = self.live;

        for (l, r) in self.graph.return_l.iter_mut().zip(self.graph.return_r.iter_mut()) {
            l[..n].fill(0.0);
            r[..n].fill(0.0);
        }
        let ml = &mut self.master_l[at..at + n];
        let mr = &mut self.master_r[at..at + n];
        ml.fill(0.0);
        mr.fill(0.0);

        let mut sink = EventSink { ring: &mut self.rings.events, status: &self.status };
        for ti in 0..self.graph.tracks.len() {
            let t = &mut self.graph.tracks[ti];
            let receives_live = t.kind == TrackKind::Midi && (t.mixer.arm || t.mixer.monitor == Monitor::In);
            for &(target, kind) in &live[..live_len] {
                let to_us = match target {
                    Some(x) => x as usize == ti,
                    None => receives_live,
                };
                if to_us {
                    push_ev(&mut t.events, NoteEvent { offset: 0, kind });
                    record_live(t, ti, kind, b0, &mut sink);
                    let (pitch, velocity, on) = match kind {
                        NoteKind::On { pitch, velocity } => (pitch, velocity, velocity > 0),
                        NoteKind::Off { pitch } => (pitch, 0, false),
                        _ => (0, 0, false),
                    };
                    if matches!(kind, NoteKind::On { .. } | NoteKind::Off { .. }) {
                        sink.push(EngineEvent::LiveNote { track: ti as u16, pitch, velocity, on, beat: b0, frame: self.frames_total + at as u64, playing: self.playing });
                    }
                }
            }
            if span.playing {
                apply_automation(t, b0);
            }
            render_track_source(t, ti, &span, sig, &mut sink);
            if t.kind == TrackKind::Audio && self.input.is_some() {
                let monitor = t.mixer.monitor == Monitor::In || (t.mixer.monitor == Monitor::Auto && t.mixer.arm);
                let (il, ir) = (&self.in_l[at..at + n], &self.in_r[at..at + n]);
                if monitor {
                    for i in 0..n {
                        t.l[i] += il[i];
                        t.r[i] += ir[i];
                    }
                }
                // Stream the take once its launch boundary has passed.
                if let (Some(rec), Some(p)) = (t.rt.recording_slot, t.rt.launcher.playing) {
                    if p.slot == rec && span.audible && (self.rec_track.is_none() || self.rec_track == Some(ti)) {
                        if let Some(tx) = self.rec.as_mut() {
                            let from = if p.started_at > span.b0 { span.offset(p.started_at, 0) } else { 0 };
                            if self.rec_track.is_none() {
                                let _ = tx.push(crate::recorder::RecMsg::Start { track: ti as u16, slot: rec as u16 });
                                self.rec_track = Some(ti);
                            }
                            for i in from..n {
                                if tx.push(crate::recorder::RecMsg::Frame(il[i], ir[i])).is_err() {
                                    self.status.input_overruns.fetch_add(1, Ordering::Relaxed);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let (gr, gr_r) = (&mut self.graph.return_l, &mut self.graph.return_r);
            process_track(t, n, &ctx, any_solo);
            for (k, (rl, rr)) in gr.iter_mut().zip(gr_r.iter_mut()).enumerate() {
                let send = &mut t.rt.sends[k];
                if send.target() <= 0.0 && !send.is_ramping() {
                    continue;
                }
                for i in 0..n {
                    let g = send.next();
                    rl[i] += t.l[i] * g;
                    rr[i] += t.r[i] * g;
                }
            }
            for i in 0..n {
                ml[i] += t.l[i];
                mr[i] += t.r[i];
            }
            publish_meter(&self.status, TrackIx::Track(ti as u16), &t.l[..n], &t.r[..n]);
        }
        for ri in 0..self.graph.returns.len() {
            let t = &mut self.graph.returns[ri];
            t.events.clear();
            if span.playing {
                apply_automation(t, b0);
            }
            t.l[..n].copy_from_slice(&self.graph.return_l[ri][..n]);
            t.r[..n].copy_from_slice(&self.graph.return_r[ri][..n]);
            process_track(t, n, &ctx, any_solo);
            for i in 0..n {
                ml[i] += t.l[i];
                mr[i] += t.r[i];
            }
            publish_meter(&self.status, TrackIx::Return(ri as u16), &t.l[..n], &t.r[..n]);
        }
        let m = &mut self.graph.master;
        m.events.clear();
        if span.playing {
            apply_automation(m, b0);
        }
        m.l[..n].copy_from_slice(ml);
        m.r[..n].copy_from_slice(mr);
        process_track(m, n, &ctx, false);
        let bad = xdaw_dsp::util::sanitize(&mut m.l[..n]) | xdaw_dsp::util::sanitize(&mut m.r[..n]);
        if bad {
            self.status.nonfinite_blocks.fetch_add(1, Ordering::Relaxed);
        }
        ml.copy_from_slice(&m.l[..n]);
        mr.copy_from_slice(&m.r[..n]);
        publish_meter(&self.status, TrackIx::Master, ml, mr);

        if self.playing && self.include_metronome && (self.metronome || counting) {
            render_click(&mut self.click, &span, sig, ml, mr, ctx.sample_rate);
        }
        self.frames_total += n as u64;
        if self.playing {
            self.sample += n as i64;
        }
    }

    fn publish_status(&self) {
        let s = &self.status;
        s.playing.store(self.playing, Ordering::Relaxed);
        s.beat.store(self.beat());
        s.sample_pos.store(self.sample, Ordering::Relaxed);
        for (i, t) in self.graph.tracks.iter().enumerate() {
            let st = &s.tracks[i];
            let l = &t.rt.launcher;
            st.playing_slot.store(l.playing.map_or(NO_SLOT, |p| p.slot as i32), Ordering::Relaxed);
            st.queued.store(
                match l.pending {
                    Some(xdaw_sequencer::Pending::Launch { slot, .. }) => slot as i32,
                    Some(xdaw_sequencer::Pending::Stop { .. }) => QUEUED_STOP,
                    None => NO_SLOT,
                },
                Ordering::Relaxed,
            );
            st.recording_slot.store(t.rt.recording_slot.map_or(NO_SLOT, |s| s as i32), Ordering::Relaxed);
            st.arrangement_active.store(l.arrangement_active, Ordering::Relaxed);
            st.automation_overridden.store(t.rt.automation_overridden.iter().any(|&o| o), Ordering::Relaxed);
            let pos = l.playing.and_then(|p| {
                let c = t.slots.get(p.slot)?.as_ref()?;
                c.region.position(self.beat() - p.origin)
            });
            st.clip_position.store(pos.unwrap_or(-1.0));
        }
    }
}

// ---------------------------------------------------------------- helpers

fn local_position(region: Option<xdaw_sequencer::ClipRegion>, elapsed: Beats) -> Beats {
    match region {
        Some(r) => r.position(elapsed).unwrap_or(elapsed),
        None => elapsed,
    }
}

fn shift_launcher(l: &mut TrackLauncher, delta: Beats) {
    if let Some(p) = l.playing.as_mut() {
        p.origin += delta;
        p.started_at += delta;
    }
    if let Some(p) = l.pending.as_mut() {
        match p {
            xdaw_sequencer::Pending::Launch { at, .. } | xdaw_sequencer::Pending::Stop { at } => *at += delta,
        }
    }
}

fn end_clip_notes(t: &mut RtTrack, offset: usize) {
    for slot in t.rt.active.iter_mut() {
        if let Some(n) = slot.filter(|n| n.source != LIVE) {
            push_ev(&mut t.events, NoteEvent { offset: offset as u32, kind: NoteKind::Off { pitch: n.pitch } });
            *slot = None;
        }
    }
}

fn end_source_of(t: &mut RtTrack, offset: usize) {
    end_clip_notes(t, offset);
    if let Some(p) = t.rt.launcher.playing {
        if matches!(t.slots.get(p.slot), Some(Some(RtClip { content: RtContent::Audio { .. }, .. }))) {
            t.rt.tail = Some(AudioTail { from_arrangement: false, slot: p.slot, origin: p.origin, remaining: TAIL_SAMPLES, start_offset: offset as u32 });
        }
    }
}

fn end_arrangement_notes(t: &mut RtTrack, offset: usize) {
    if t.rt.launcher.playing.is_none() {
        end_clip_notes(t, offset);
    }
}

fn override_lane(t: &mut RtTrack, target: RtTarget) -> bool {
    if let Some(i) = t.automation.iter().position(|l| l.target == target) {
        if i < 64 && !t.rt.automation_overridden[i] {
            t.rt.automation_overridden[i] = true;
            return true;
        }
    }
    false
}

/// Applies a mixer change; returns true if it overrode automation.
fn set_mixer(t: &mut RtTrack, param: MixParam, value: f32, playing: bool) -> bool {
    let target = match param {
        MixParam::Volume => {
            t.mixer.volume_db = value;
            t.rt.volume.set_target(db_to_gain(value));
            Some(RtTarget::Volume)
        }
        MixParam::Pan => {
            t.mixer.pan = value;
            let (l, r) = pan_gains(value);
            t.rt.pan_l.set_target(l);
            t.rt.pan_r.set_target(r);
            Some(RtTarget::Pan)
        }
        MixParam::Mute => {
            t.mixer.mute = value >= 0.5;
            None
        }
        MixParam::Solo => {
            t.mixer.solo = value >= 0.5;
            None
        }
        MixParam::Arm => {
            t.mixer.arm = value >= 0.5;
            if !t.mixer.arm {
                push_ev(&mut t.events, NoteEvent { offset: 0, kind: NoteKind::AllOff });
                for s in t.rt.active.iter_mut() {
                    if s.is_some_and(|n| n.source == LIVE) {
                        *s = None;
                    }
                }
            }
            None
        }
        MixParam::Send(i) => {
            let i = i as usize;
            if i < MAX_RETURNS {
                t.mixer.sends_db[i] = value;
                t.rt.sends[i].set_target(db_to_gain(value));
            }
            Some(RtTarget::Send(i))
        }
        MixParam::Monitor => {
            t.mixer.monitor = match value as u32 {
                0 => Monitor::In,
                1 => Monitor::Auto,
                _ => Monitor::Off,
            };
            None
        }
    };
    playing && target.is_some_and(|tg| override_lane(t, tg))
}

fn apply_automation(t: &mut RtTrack, beat: Beats) {
    for (i, lane) in t.automation.iter().enumerate() {
        if i < 64 && t.rt.automation_overridden[i] {
            continue;
        }
        let Some(v) = lane.value_at(beat) else { continue };
        match lane.target {
            RtTarget::Volume => t.rt.volume.set_target(db_to_gain(v)),
            RtTarget::Pan => {
                let (l, r) = pan_gains(v);
                t.rt.pan_l.set_target(l);
                t.rt.pan_r.set_target(r);
            }
            RtTarget::Send(s) if s < MAX_RETURNS => t.rt.sends[s].set_target(db_to_gain(v)),
            RtTarget::Send(_) => {}
            RtTarget::Device { device, param } => {
                if let Some(d) = t.devices.get_mut(device) {
                    d.device.set_param(param, v);
                }
            }
        }
    }
}

fn record_live(t: &mut RtTrack, ti: usize, kind: NoteKind, beat: Beats, sink: &mut EventSink) {
    // Live notes are tracked so arm changes/panic can silence them.
    match kind {
        NoteKind::On { pitch, velocity } if velocity > 0 => {
            if let Some(s) = t.rt.active.iter_mut().find(|s| s.is_none()) {
                *s = Some(ActiveNote { pitch, end: f64::INFINITY, source: LIVE });
            }
            if t.rt.recording_slot.is_some() {
                t.rt.held_input[pitch as usize & 127] = Some(HeldInput { pitch, velocity, start: beat });
            }
        }
        NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
            if let Some(s) = t.rt.active.iter_mut().find(|s| s.is_some_and(|n| n.source == LIVE && n.pitch == pitch)) {
                *s = None;
            }
            if let (Some(rec), Some(h)) = (t.rt.recording_slot, t.rt.held_input[pitch as usize & 127].take()) {
                let p = t.rt.launcher.playing;
                let origin = p.map(|p| p.origin).unwrap_or(h.start);
                let region = p.and_then(|p| t.slots.get(p.slot)?.as_ref().map(|c| c.region));
                sink.push(EngineEvent::NoteRecorded {
                    track: ti as u16,
                    slot: rec as u16,
                    pitch,
                    velocity: h.velocity,
                    start: h.start,
                    end: beat.max(h.start + 1.0 / 64.0),
                    local_start: local_position(region, h.start - origin),
                });
            }
        }
        _ => {}
    }
}

/// Schedules this span's clip playback into the track's buffers and events.
fn render_track_source(t: &mut RtTrack, ti: usize, span: &Span, sig: xdaw_sequencer::TimeSignature, sink: &mut EventSink) {
    t.l[..span.n].fill(0.0);
    t.r[..span.n].fill(0.0);
    if !span.playing {
        render_tail(t, span, 0, span.n);
        return;
    }
    let mut from = 0;
    loop {
        let from_beat = span.beat(from);
        schedule_follow_action(t, from_beat, span.b1, sig);
        let Some(at) = t.rt.launcher.transition_in(from_beat, span.b1) else { break };
        let o = span.offset(at, from);
        if span.audible {
            play_source(t, ti, span, from, o, sink);
        }
        // Leaving the old source: clip notes end and audio fades on the boundary sample.
        end_source_of(t, o);
        match t.rt.launcher.fire(at, sig) {
            Some(LaunchEvent::Started { slot, at, replaced }) => {
                if let Some(old) = replaced.filter(|&old| old != slot) {
                    sink.push(EngineEvent::ClipStopped { track: ti as u16, slot: old as u16, at });
                }
                let origin = t.rt.launcher.playing.map_or(at, |p| p.origin);
                t.rt.fade_in = TAIL_SAMPLES;
                t.rt.follow_at = t.slots.get(slot).and_then(Option::as_ref).filter(|c| c.follow.is_active()).map(|c| at + c.follow.time.max(1.0 / 64.0));
                if t.rt.recording_slot == Some(slot) {
                    sink.push(EngineEvent::RecordingStarted { track: ti as u16, slot: slot as u16, at });
                }
                sink.push(EngineEvent::ClipStarted { track: ti as u16, slot: slot as u16, at, origin });
            }
            Some(LaunchEvent::Stopped { slot, at }) => {
                t.rt.follow_at = None;
                sink.push(EngineEvent::ClipStopped { track: ti as u16, slot: slot as u16, at });
            }
            Some(LaunchEvent::StoppedIdle { at }) => sink.push(EngineEvent::TrackReleased { track: ti as u16, at }),
            None => {}
        }
        from = o;
    }
    if span.audible {
        play_source(t, ti, span, from, span.n, sink);
    }
    render_tail(t, span, 0, span.n);
}

/// Turns a due Follow Action into a pending launch/stop at its exact time
/// (quantized only by an explicit clip quantization: Follow Actions bypass the
/// global quantization, Live 10 §13.6).
fn schedule_follow_action(t: &mut RtTrack, from: Beats, to: Beats, sig: xdaw_sequencer::TimeSignature) {
    let Some(f) = t.rt.follow_at else { return };
    if f >= to || t.rt.launcher.pending.is_some_and(|p| p.at() <= f) {
        return;
    }
    let Some(p) = t.rt.launcher.playing else {
        t.rt.follow_at = None;
        return;
    };
    t.rt.follow_at = None;
    let Some(Some(clip)) = t.slots.get(p.slot) else { return };
    let mut first = p.slot;
    while first > 0 && matches!(t.slots.get(first - 1), Some(Some(_))) {
        first -= 1;
    }
    let mut last = p.slot;
    while matches!(t.slots.get(last + 1), Some(Some(_))) {
        last += 1;
    }
    let (r1, r2) = (t.rt.rng.next_unit(), t.rt.rng.next_unit());
    let at_raw = f.max(from);
    let q = match clip.quantize {
        xdaw_sequencer::ClipQuantize::Fixed(q) => q,
        xdaw_sequencer::ClipQuantize::Global => Quantize::None,
    };
    let at = q.next_boundary(at_raw, sig);
    match clip.follow.resolve(p.slot, first, last, r1, r2) {
        xdaw_sequencer::FollowResult::Nothing => {}
        xdaw_sequencer::FollowResult::Stop => t.rt.launcher.pending = Some(xdaw_sequencer::Pending::Stop { at }),
        xdaw_sequencer::FollowResult::Launch(slot) => {
            let legato = t.slots.get(slot).and_then(Option::as_ref).is_some_and(|c| c.legato);
            t.rt.launcher.pending = Some(xdaw_sequencer::Pending::Launch { slot, at, legato });
        }
    }
}

fn play_source(t: &mut RtTrack, ti: usize, span: &Span, a: usize, b: usize, sink: &mut EventSink) {
    if a >= b {
        return;
    }
    let (ba, bb) = (span.beat(a), span.beat(b));
    if let Some(p) = t.rt.launcher.playing {
        let finished = match t.slots.get(p.slot).and_then(Option::as_ref) {
            Some(clip) => play_clip(clip, p.origin, None, ba, bb, a, b, span, &mut t.l, &mut t.r, &mut t.events, &mut t.rt.active, &mut t.rt.fade_in),
            None => false,
        };
        if finished && t.rt.recording_slot != Some(p.slot) {
            // One-shot clip reached its end; the track stays released from the Arrangement.
            end_clip_notes(t, b.saturating_sub(1));
            t.rt.launcher.stop_now();
            sink.push(EngineEvent::ClipStopped { track: ti as u16, slot: p.slot as u16, at: bb });
        }
    } else if t.rt.launcher.arrangement_active {
        let start_idx = t.arrangement.partition_point(|c| c.end() <= ba);
        for c in &t.arrangement[start_idx..] {
            if c.start >= bb {
                break;
            }
            let (ca, cb) = (ba.max(c.start), bb.min(c.end()));
            if ca >= cb {
                continue;
            }
            let oa = if ca > ba { span.offset(ca, a) } else { a };
            let ob = if cb < bb { span.offset(cb, a).max(oa) } else { b };
            let mut no_fade = 0;
            play_clip(&c.clip, c.origin(), Some(c), ca, cb, oa, ob, span, &mut t.l, &mut t.r, &mut t.events, &mut t.rt.active, &mut no_fade);
        }
    }
    // Note-offs for clip notes ending inside [ba, bb).
    for slot in t.rt.active.iter_mut() {
        if let Some(n) = *slot {
            if n.source != LIVE && n.end < bb {
                let o = if n.end <= ba { a } else { span.offset(n.end, a).min(b) };
                push_ev(&mut t.events, NoteEvent { offset: o as u32, kind: NoteKind::Off { pitch: n.pitch } });
                *slot = None;
            }
        }
    }
}

/// Plays one clip over song beats `[ba, bb)` mapped to sample offsets `[a, b)`.
/// Returns true when a non-looping clip has finished.
#[allow(clippy::too_many_arguments)]
fn play_clip(
    clip: &RtClip,
    origin: Beats,
    arr: Option<&RtArrClip>,
    ba: Beats,
    bb: Beats,
    a: usize,
    b: usize,
    span: &Span,
    l: &mut [f32],
    r: &mut [f32],
    events: &mut Vec<NoteEvent>,
    active: &mut [Option<ActiveNote>; MAX_ACTIVE_NOTES],
    fade_in: &mut u32,
) -> bool {
    let region = clip.region;
    match &clip.content {
        RtContent::Midi { notes } => {
            let mut finished = false;
            let e_end = bb - origin;
            if !region.looping && region.position(ba - origin).is_none() {
                return true;
            }
            region.segments(ba - origin, e_end, |la, lb, e| {
                let first = notes.partition_point(|n| n.start < la);
                for note in &notes[first..] {
                    if note.start >= lb {
                        break;
                    }
                    let on_beat = origin + e + (note.start - la);
                    let mut end_beat = on_beat + (note.length.min(region.end - note.start));
                    if let Some(c) = arr {
                        end_beat = end_beat.min(c.end());
                    }
                    let o = span.offset(on_beat, a).min(b.saturating_sub(1).max(a));
                    // Retrigger of a sounding pitch: close it first.
                    for s in active.iter_mut() {
                        if s.is_some_and(|n| n.pitch == note.pitch && n.source != LIVE) {
                            push_ev(events, NoteEvent { offset: o as u32, kind: NoteKind::Off { pitch: note.pitch } });
                            *s = None;
                        }
                    }
                    if events.len() + 2 >= MAX_EVENTS {
                        break;
                    }
                    push_ev(events, NoteEvent { offset: o as u32, kind: NoteKind::On { pitch: note.pitch, velocity: note.velocity } });
                    if let Some(s) = active.iter_mut().find(|s| s.is_none()) {
                        *s = Some(ActiveNote { pitch: note.pitch, end: end_beat, source: clip.id });
                    }
                }
            });
            if !region.looping && region.position(bb - origin).is_none() {
                finished = true;
            }
            finished
        }
        RtContent::Audio { data, warp, source_bpm } => {
            let Some(data) = data.as_deref() else { return false };
            let bps = span.clock.beats_per_sample();
            let spb = span.clock.samples_per_beat();
            let rate_scale = match warp {
                WarpMode::RePitch | WarpMode::Stretch => 1.0,
                WarpMode::Off => *source_bpm / span.clock.bpm(),
            };
            let to_frames = 60.0 / *source_bpm * data.sample_rate;
            let mut finished = false;
            for i in a..b {
                let beat = ba + (i - a) as f64 * bps;
                let elapsed = (beat - origin) * rate_scale;
                let Some(local) = region.position(elapsed) else {
                    finished = true;
                    break;
                };
                let (x, y) = data.read(local * to_frames);
                let mut g = clip.gain;
                if let Some(c) = arr {
                    // Edge declick plus user fades.
                    let from_start = (beat - c.start) * spb;
                    let to_end = (c.end() - beat) * spb;
                    g *= (from_start / TAIL_SAMPLES as f64).min(1.0) as f32;
                    g *= (to_end / TAIL_SAMPLES as f64).min(1.0) as f32;
                    if c.fade_in > 0.0 {
                        g *= ((beat - c.start) / c.fade_in).clamp(0.0, 1.0) as f32;
                    }
                    if c.fade_out > 0.0 {
                        g *= ((c.end() - beat) / c.fade_out).clamp(0.0, 1.0) as f32;
                    }
                } else if *fade_in > 0 {
                    // First sample of a launch is already non-zero (1/TAIL), so onsets stay sample-exact.
                    g *= (TAIL_SAMPLES - *fade_in + 1) as f32 / TAIL_SAMPLES as f32;
                    *fade_in -= 1;
                }
                l[i] += x * g;
                r[i] += y * g;
            }
            finished
        }
    }
}

fn render_tail(t: &mut RtTrack, span: &Span, a: usize, b: usize) {
    let Some(mut tail) = t.rt.tail else { return };
    let Some(Some(clip)) = t.slots.get(tail.slot) else {
        t.rt.tail = None;
        return;
    };
    let RtContent::Audio { data: Some(data), warp, source_bpm } = &clip.content else {
        t.rt.tail = None;
        return;
    };
    let rate_scale = match warp {
        WarpMode::RePitch | WarpMode::Stretch => 1.0,
        WarpMode::Off => *source_bpm / span.clock.bpm(),
    };
    let to_frames = 60.0 / *source_bpm * data.sample_rate;
    let bps = span.clock.beats_per_sample();
    let a = a.max(tail.start_offset as usize);
    tail.start_offset = 0;
    let ba = span.beat(a);
    for i in a..b {
        if tail.remaining == 0 {
            break;
        }
        let beat = ba + (i - a) as f64 * bps;
        let Some(local) = clip.region.position((beat - tail.origin) * rate_scale) else { break };
        let (x, y) = data.read(local * to_frames);
        let g = clip.gain * tail.remaining as f32 / TAIL_SAMPLES as f32;
        t.l[i] += x * g;
        t.r[i] += y * g;
        tail.remaining -= 1;
    }
    t.rt.tail = (tail.remaining > 0).then_some(tail);
}

/// Device chain plus fader, pan and mute/solo for one track.
fn process_track(t: &mut RtTrack, n: usize, ctx: &Ctx, any_solo: bool) {
    t.events.sort_unstable_by_key(|e| (e.offset, matches!(e.kind, NoteKind::On { .. })));
    let audible = t.is_audible_with(any_solo);
    let (l, r) = (&mut t.l[..n], &mut t.r[..n]);
    for d in t.devices.iter_mut() {
        if d.bypass {
            continue;
        }
        d.device.process(&t.events, l, r, ctx);
    }
    t.events.clear();
    for i in 0..n {
        let v = t.rt.volume.next();
        let (pl, pr) = (t.rt.pan_l.next(), t.rt.pan_r.next());
        // Pan law normalised so centre is unity (sqrt2 * cos(pi/4) = 1).
        let g = if audible { v * std::f32::consts::SQRT_2 } else { 0.0 };
        l[i] *= g * pl;
        r[i] *= g * pr;
    }
}

fn publish_meter(status: &SharedStatus, ix: TrackIx, l: &[f32], r: &[f32]) {
    let st = status.track(ix);
    let (mut pl, mut pr) = (0.0f32, 0.0f32);
    for (&a, &b) in l.iter().zip(r) {
        pl = pl.max(a.abs());
        pr = pr.max(b.abs());
    }
    st.peak_l.store(st.peak_l.load().max(pl));
    st.peak_r.store(st.peak_r.load().max(pr));
    if pl >= 1.0 || pr >= 1.0 {
        st.clipped.store(true, Ordering::Relaxed);
    }
}

fn render_click(click: &mut Click, span: &Span, sig: xdaw_sequencer::TimeSignature, l: &mut [f32], r: &mut [f32], sr: f32) {
    let unit = sig.unit_beats();
    let bar = sig.bar_beats();
    let mut k = (span.b0 / unit - 1e-9).ceil() as i64;
    let mut pos = 0;
    loop {
        let beat = k as f64 * unit;
        if beat >= span.b1 {
            break;
        }
        let o = span.offset(beat, pos);
        render_click_range(click, l, r, pos, o, sr);
        let in_bar = (beat / bar).rem_euclid(1.0);
        let downbeat = in_bar < 1e-6 || in_bar > 1.0 - 1e-6;
        *click = Click { phase: 0.0, freq: if downbeat { 1568.0 } else { 1046.5 }, remaining: (0.03 * sr) as u32 };
        pos = o;
        k += 1;
    }
    render_click_range(click, l, r, pos, span.n, sr);
}

fn render_click_range(click: &mut Click, l: &mut [f32], r: &mut [f32], a: usize, b: usize, sr: f32) {
    for i in a..b {
        if click.remaining == 0 {
            return;
        }
        let env = click.remaining as f32 / (0.03 * sr);
        let s = (click.phase * std::f32::consts::TAU).sin() * env * env * 0.35;
        click.phase = (click.phase + click.freq / sr).fract();
        click.remaining -= 1;
        l[i] += s;
        r[i] += s;
    }
}
