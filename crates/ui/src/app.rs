//! Application state and per-frame orchestration.

use crate::runtime::AudioRuntime;
use crate::{arrangement, browser, clip_editor, devices, dialogs, session, theme, transport};
use egui::{Key, Modifiers};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use xdaw_engine::audio_io::AudioSettings;
use xdaw_engine::media::MediaCache;
use xdaw_engine::protocol::{Command, EngineEvent, TrackIx};
use xdaw_midi::keyboard::ComputerKeyboard;
use xdaw_project::history::Edit;
use xdaw_project::*;
use xdaw_sequencer::{Beats, ClipRegion};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Session,
    Arrangement,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detail {
    Clip,
    Devices,
}

/// Where the selected clip lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipLoc {
    Slot { track: usize, slot: usize },
    Arrangement { track: usize, id: Id },
}

#[derive(Default)]
pub struct Selection {
    pub track: Option<TrackRef>,
    pub clip: Option<ClipLoc>,
    pub scene: usize,
    pub notes: Vec<usize>,
    pub arr_clips: Vec<(usize, Id)>,
}

/// A Session clip being logged into the Arrangement while Arrangement Record is on.
#[derive(Clone)]
struct OpenRecord {
    start: Beats,
    origin: Beats,
    clip: Clip,
}

/// MIDI notes captured for a new Session clip.
#[derive(Default, Clone)]
struct MidiTake {
    slot: usize,
    start: Beats,
    notes: Vec<Note>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapMode {
    Off,
    Midi,
    Key,
}

pub struct Clipboard {
    pub arrangement: Vec<(usize, ArrangementClip)>,
    pub clip: Option<Clip>,
    pub notes: Vec<Note>,
}

pub struct XdawApp {
    pub project: Project,
    pub path: Option<PathBuf>,
    pub history: History,
    committed: Project,
    saved: Project,
    pub pending_label: Option<String>,
    pub media: MediaCache,
    pub media_gen: u64,
    pub rt: AudioRuntime,
    pub view: View,
    pub detail: Detail,
    pub sel: Selection,
    pub status_line: String,
    pub arr: arrangement::ArrangementState,
    pub piano: clip_editor::PianoState,
    pub browser: browser::BrowserState,
    pub kb: ComputerKeyboard,
    pub kb_enabled: bool,
    held_keys: HashMap<Key, u8>,
    pub arrangement_record: bool,
    open_records: HashMap<usize, OpenRecord>,
    midi_takes: HashMap<usize, MidiTake>,
    audio_takes: HashMap<usize, (usize, Beats)>,
    pub dialogs: dialogs::Dialogs,
    autosave_at: Instant,
    pub map_mode: MapMode,
    pub map_target: Option<MappingTarget>,
    pub clipboard: Clipboard,
    pub follow: bool,
    meter_peaks: Vec<(f32, f32)>,
    pub automation_overridden: bool,
    pub pending_launch_scene: Option<usize>,
    last_frame: Instant,
    script: Option<crate::script::Script>,
}

const AUTOSAVE_EVERY: Duration = Duration::from_secs(30);

impl XdawApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let project = Self::empty_project();
        let mut app = Self {
            committed: project.clone(),
            saved: project.clone(),
            project,
            path: None,
            history: History::default(),
            pending_label: None,
            media: MediaCache::new(),
            media_gen: 0,
            rt: AudioRuntime::new(AudioSettings::default()),
            view: View::Session,
            detail: Detail::Devices,
            sel: Selection::default(),
            status_line: String::new(),
            arr: arrangement::ArrangementState::default(),
            piano: clip_editor::PianoState::default(),
            browser: browser::BrowserState::new(),
            kb: ComputerKeyboard::default(),
            kb_enabled: true,
            held_keys: HashMap::new(),
            arrangement_record: false,
            open_records: HashMap::new(),
            midi_takes: HashMap::new(),
            audio_takes: HashMap::new(),
            dialogs: dialogs::Dialogs::default(),
            autosave_at: Instant::now(),
            map_mode: MapMode::Off,
            map_target: None,
            clipboard: Clipboard { arrangement: vec![], clip: None, notes: vec![] },
            follow: true,
            meter_peaks: vec![],
            automation_overridden: false,
            pending_launch_scene: None,
            last_frame: Instant::now(),
            script: crate::script::Script::from_env(),
        };
        // Crash recovery for an unsaved session.
        if open.is_none() {
            if let Some(auto) = io::recovery_candidate(None) {
                app.dialogs.recovery = Some((auto, None));
            }
        }
        match open {
            Some(p) => app.open_path(&p),
            None => app.start_audio(),
        }
        app
    }

    pub fn empty_project() -> Project {
        let mut p = Project::new("Untitled");
        for i in 0..8 {
            p.add_scene(&format!("{}", i + 1));
        }
        for i in 0..2 {
            p.add_track(TrackKind::Audio, &format!("{} Audio", i + 1));
        }
        for i in 0..2 {
            let t = p.add_track(TrackKind::Midi, &format!("{} MIDI", i + 3));
            p.add_device(TrackRef::Track(t), xdaw_dsp::DeviceKind::PolySynth);
        }
        p.add_return("A Reverb");
        p.add_return("B Delay");
        p.add_device(TrackRef::Return(0), xdaw_dsp::DeviceKind::Reverb);
        p.add_device(TrackRef::Return(1), xdaw_dsp::DeviceKind::Delay);
        p.returns[0].devices[0].params[4] = 1.0;
        p.returns[1].devices[0].params[6] = 1.0;
        p
    }

    pub fn project_dir(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf))
    }

    pub fn recordings_dir(&self) -> PathBuf {
        match self.project_dir() {
            Some(d) => d.join("Samples").join("Recorded"),
            None => io::app_support_dir().join("Recordings"),
        }
    }

    pub fn start_audio(&mut self) {
        self.request_media();
        let dir = self.recordings_dir();
        self.rt.start(&self.project, &self.media, dir);
        if let Some(e) = &self.rt.error {
            self.status_line = format!("Audio device unavailable: {e}");
        }
    }

    /// Queues decoding for every referenced media file.
    pub fn request_media(&mut self) {
        let dir = self.project_dir();
        for m in self.project.media_refs() {
            self.media.request(&m.path, io::resolve_media(&m, dir.as_deref()));
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    pub fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        format!("{}{} — XDAW", name, if self.is_dirty() { " *" } else { "" })
    }

    // ------------------------------------------------------------ files

    pub fn new_project(&mut self) {
        self.rt.send(Command::Stop);
        self.project = Self::empty_project();
        self.path = None;
        self.after_load();
    }

    pub fn open_path(&mut self, path: &Path) {
        match io::load(path) {
            Ok(p) => {
                self.project = p;
                self.path = Some(path.to_path_buf());
                if let Some(auto) = io::recovery_candidate(Some(path)) {
                    self.dialogs.recovery = Some((auto, Some(path.to_path_buf())));
                }
                self.after_load();
                let missing = io::missing_media(&self.project, self.project_dir().as_deref());
                self.status_line = if missing.is_empty() {
                    format!("Opened {}", path.display())
                } else {
                    format!("Opened {} — {} missing media file(s); affected clips play silence", path.display(), missing.len())
                };
                self.dialogs.missing = missing;
            }
            Err(e) => self.status_line = format!("Open failed: {e}"),
        }
    }

    fn after_load(&mut self) {
        self.history = History::default();
        self.committed = self.project.clone();
        self.saved = self.project.clone();
        self.sel = Selection::default();
        self.open_records.clear();
        self.midi_takes.clear();
        self.media = MediaCache::new();
        self.media_gen += 1;
        self.start_audio();
    }

    pub fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_to(&p),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) -> bool {
        let Some(path) = rfd::FileDialog::new().add_filter("XDAW project", &[io::EXTENSION]).set_file_name("Song.xdaw").save_file() else {
            return false;
        };
        let path = if path.extension().is_none() { path.with_extension(io::EXTENSION) } else { path };
        self.save_to(&path)
    }

    pub fn save_as_path(&mut self, path: &Path) -> bool {
        self.save_to(path)
    }

    fn save_to(&mut self, path: &Path) -> bool {
        // Rebase media for the new location: resolve against the old folder, then
        // store relative only when the file lies inside the new project folder.
        if let Some(dir) = path.parent() {
            let old_dir = self.project_dir();
            self.project.for_each_media_mut(|m| {
                let abs = io::resolve_media(m, old_dir.as_deref());
                m.path = if abs.starts_with(dir) { io::relativize(&abs, dir) } else { abs.to_string_lossy().to_string() };
            });
        }
        self.project.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        match io::save(&self.project, path) {
            Ok(()) => {
                let was_unsaved = self.path.is_none();
                self.path = Some(path.to_path_buf());
                self.saved = self.project.clone();
                self.committed = self.project.clone();
                io::discard_autosave(Some(path));
                if was_unsaved {
                    io::discard_autosave(None);
                }
                // Media keys may have changed from absolute to relative.
                self.media = MediaCache::new();
                self.request_media();
                self.media_gen += 1;
                self.status_line = format!("Saved {}", path.display());
                true
            }
            Err(e) => {
                self.status_line = format!("Save failed: {e}");
                false
            }
        }
    }

    pub fn collect_and_save(&mut self) {
        if self.path.is_none() && !self.save_as() {
            return;
        }
        let Some(dir) = self.project_dir() else { return };
        match io::collect_media(&mut self.project, &dir) {
            Ok(n) => {
                self.status_line = format!("Collected {n} file(s) into {}", dir.join("Samples").display());
                self.save();
            }
            Err(e) => self.status_line = format!("Collect failed: {e}"),
        }
    }

    fn autosave(&mut self) {
        if self.autosave_at.elapsed() < AUTOSAVE_EVERY {
            return;
        }
        self.autosave_at = Instant::now();
        if self.is_dirty() {
            if let Err(e) = io::write_autosave(&self.project, self.path.as_deref()) {
                self.status_line = format!("Autosave failed: {e}");
            }
        }
    }

    // ------------------------------------------------------------ undo

    pub fn label(&mut self, l: &str) {
        self.pending_label = Some(l.to_string());
    }

    /// Records one undo step for everything changed since the last commit.
    fn commit_undo(&mut self, pointer_down: bool) {
        if pointer_down || self.project == self.committed {
            return;
        }
        let before = &self.committed;
        let after = &self.project;
        let same_shape = before.tracks.len() == after.tracks.len()
            && before.returns.len() == after.returns.len()
            && before.scenes == after.scenes
            && before.midi_mappings == after.midi_mappings
            && before.key_mappings == after.key_mappings
            && before.name == after.name;
        let edit = if same_shape {
            let changed: Vec<TrackRef> = after.all_tracks().filter(|(r, t)| before.track(*r) != *t).map(|(r, _)| r).collect();
            match (changed.as_slice(), before.transport == after.transport) {
                ([r], true) if before.next_id == after.next_id || true => {
                    Edit::Track { at: *r, before: Box::new(before.track(*r).clone()), after: Box::new(after.track(*r).clone()) }
                }
                ([], false) => Edit::Transport { before: before.transport.clone(), after: after.transport.clone() },
                _ => Edit::Project { before: Box::new(before.clone()), after: Box::new(after.clone()) },
            }
        } else {
            Edit::Project { before: Box::new(before.clone()), after: Box::new(after.clone()) }
        };
        let label = self.pending_label.take().unwrap_or_else(|| "Edit".into());
        self.history.record(label, edit);
        self.committed = self.project.clone();
    }

    pub fn undo(&mut self) {
        if let Some(l) = self.history.undo(&mut self.project) {
            self.status_line = format!("Undo {l}");
        }
        self.committed = self.project.clone();
        self.fix_selection();
    }

    pub fn redo(&mut self) {
        if let Some(l) = self.history.redo(&mut self.project) {
            self.status_line = format!("Redo {l}");
        }
        self.committed = self.project.clone();
        self.fix_selection();
    }

    fn fix_selection(&mut self) {
        if let Some(TrackRef::Track(i)) = self.sel.track {
            if i >= self.project.tracks.len() {
                self.sel.track = None;
            }
        }
        if let Some(TrackRef::Return(i)) = self.sel.track {
            if i >= self.project.returns.len() {
                self.sel.track = None;
            }
        }
        if self.selected_clip().is_none() {
            self.sel.clip = None;
        }
        self.sel.notes.clear();
    }

    // ------------------------------------------------------------ selection helpers

    pub fn selected_clip(&self) -> Option<&Clip> {
        match self.sel.clip? {
            ClipLoc::Slot { track, slot } => self.project.tracks.get(track)?.slots.get(slot)?.clip.as_ref(),
            ClipLoc::Arrangement { track, id } => self.project.tracks.get(track)?.arrangement.iter().find(|c| c.id == id).map(|c| &c.clip),
        }
    }

    pub fn selected_clip_mut(&mut self) -> Option<&mut Clip> {
        match self.sel.clip? {
            ClipLoc::Slot { track, slot } => self.project.tracks.get_mut(track)?.slots.get_mut(slot)?.clip.as_mut(),
            ClipLoc::Arrangement { track, id } => self.project.tracks.get_mut(track)?.arrangement.iter_mut().find(|c| c.id == id).map(|c| &mut c.clip),
        }
    }

    pub fn select_track(&mut self, r: TrackRef) {
        self.sel.track = Some(r);
        // Exclusive arm follows selection for MIDI tracks (computer keyboard / MIDI input).
        if let TrackRef::Track(i) = r {
            if self.project.tracks[i].kind == TrackKind::Midi && !self.project.tracks.iter().any(|t| t.mixer.arm && t.kind == TrackKind::Audio) {
                for (j, t) in self.project.tracks.iter_mut().enumerate() {
                    if t.kind == TrackKind::Midi {
                        t.mixer.arm = j == i;
                    }
                }
                // Selection-driven arming is not an undoable edit.
                for (j, t) in self.committed.tracks.iter_mut().enumerate() {
                    if t.kind == TrackKind::Midi && j < self.project.tracks.len() {
                        t.mixer.arm = self.project.tracks[j].mixer.arm;
                    }
                }
            }
        }
    }

    pub fn beat(&self) -> Beats {
        self.rt.status.beat.load()
    }

    pub fn playing(&self) -> bool {
        self.rt.status.playing.load(Ordering::Relaxed)
    }

    pub fn track_status(&self, t: usize) -> &xdaw_engine::protocol::TrackStatus {
        self.rt.status.track(TrackIx::Track(t as u16))
    }

    /// Smoothed meter value for a status slot (peak hold with decay).
    pub fn meter(&mut self, ix: TrackIx) -> (f32, f32) {
        let slot = ix.status_slot();
        if self.meter_peaks.len() <= slot {
            self.meter_peaks.resize(slot + 1, (0.0, 0.0));
        }
        let st = self.rt.status.track(ix);
        let (l, r) = (st.peak_l.take(), st.peak_r.take());
        let m = &mut self.meter_peaks[slot];
        m.0 = l.max(m.0 * 0.85);
        m.1 = r.max(m.1 * 0.85);
        *m
    }

    // ------------------------------------------------------------ engine events

    fn handle_engine_events(&mut self) {
        let mut events = vec![];
        if let Some(h) = self.rt.handle.as_mut() {
            h.poll_events(|e| events.push(e));
            h.collect_garbage();
        }
        for e in events {
            self.on_event(e);
        }
        if let Some(rec) = self.rt.recorder.as_ref() {
            let takes: Vec<_> = rec.finished.try_iter().collect();
            for take in takes {
                self.on_audio_take(take);
            }
        }
        if self.media.poll() {
            self.media_gen += 1;
        }
    }

    fn on_event(&mut self, e: EngineEvent) {
        match e {
            EngineEvent::ClipStarted { track, slot, at, origin } => {
                let t = track as usize;
                self.close_record(t, at);
                if self.arrangement_record {
                    if let Some(clip) = self.project.tracks.get(t).and_then(|tr| tr.slots.get(slot as usize)?.clip.clone()) {
                        self.open_records.insert(t, OpenRecord { start: at, origin, clip });
                    }
                }
            }
            EngineEvent::ClipStopped { track, at, .. } | EngineEvent::TrackReleased { track, at } => {
                self.close_record(track as usize, at);
                if self.arrangement_record {
                    // A stopped track records silence: clear what it overwrote.
                    let _ = at;
                }
            }
            EngineEvent::TransportStopped { at } => {
                let open: Vec<usize> = self.open_records.keys().copied().collect();
                for t in open {
                    self.close_record(t, at);
                }
                if self.arrangement_record {
                    self.arrangement_record = false;
                    self.label("Record Arrangement");
                }
            }
            EngineEvent::TransportStarted { .. } => {}
            EngineEvent::RecordingStarted { track, slot, at } => {
                let t = track as usize;
                match self.project.tracks.get(t).map(|tr| tr.kind) {
                    Some(TrackKind::Midi) => {
                        let has_clip = self.project.tracks[t].slots.get(slot as usize).is_some_and(|s| s.clip.is_some());
                        if !has_clip {
                            self.midi_takes.insert(t, MidiTake { slot: slot as usize, start: at, notes: vec![] });
                        }
                    }
                    Some(TrackKind::Audio) => {
                        self.audio_takes.insert(t, (slot as usize, at));
                    }
                    _ => {}
                }
            }
            EngineEvent::NoteRecorded { track, slot, pitch, velocity, start, end, local_start } => {
                let t = track as usize;
                if let Some(take) = self.midi_takes.get_mut(&t) {
                    take.notes.push(Note { pitch, start: start - take.start, length: (end - start).max(1.0 / 32.0), velocity });
                } else if let Some(clip) = self.project.tracks.get_mut(t).and_then(|tr| tr.slots.get_mut(slot as usize)?.clip.as_mut()) {
                    if let ClipContent::Midi { notes } = &mut clip.content {
                        notes.push(Note { pitch, start: local_start, length: (end - start).max(1.0 / 32.0), velocity });
                        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
                        self.pending_label = Some("Record MIDI".into());
                    }
                }
            }
            EngineEvent::RecordingStopped { track, slot, at } => {
                let t = track as usize;
                if let Some(take) = self.midi_takes.remove(&t) {
                    let bar = self.project.transport.time_signature.bar_beats();
                    let length = (((at - take.start) / bar).ceil() * bar).max(bar);
                    let id = self.project.alloc_id();
                    let color = self.project.tracks[t].color;
                    let clip = Clip {
                        id,
                        name: "Recorded".into(),
                        color,
                        content: ClipContent::Midi { notes: take.notes },
                        region: ClipRegion::looped(length),
                        launch_mode: Default::default(),
                        quantize: Default::default(),
                        legato: false,
                        gain_db: 0.0,
                        transpose: 0,
                        follow: Default::default(),
                    };
                    if let Some(s) = self.project.tracks[t].slots.get_mut(take.slot) {
                        s.clip = Some(clip);
                        self.sel.clip = Some(ClipLoc::Slot { track: t, slot: take.slot });
                        self.label("Record MIDI Clip");
                    }
                }
                let _ = slot;
            }
            EngineEvent::AutomationOverridden { .. } => self.automation_overridden = true,
            EngineEvent::Looped { .. } => {}
        }
    }

    fn on_audio_take(&mut self, take: xdaw_engine::recorder::FinishedTake) {
        let t = take.track as usize;
        let (slot, _) = self.audio_takes.remove(&t).unwrap_or((take.slot as usize, 0.0));
        if let Some(e) = take.error {
            self.status_line = format!("Recording failed: {e}");
            return;
        }
        if take.frames == 0 {
            return;
        }
        let path = match self.project_dir() {
            Some(d) if take.path.starts_with(&d) => io::relativize(&take.path, &d),
            _ => take.path.to_string_lossy().to_string(),
        };
        let bpm = self.project.transport.bpm;
        let beats = take.frames as f64 / self.rt.settings.sample_rate as f64 * bpm / 60.0;
        let id = self.project.alloc_id();
        let color = self.project.tracks.get(t).map(|tr| tr.color).unwrap_or([200, 80, 80]);
        let clip = Clip {
            id,
            name: "Audio Take".into(),
            color,
            content: ClipContent::Audio(AudioContent { media: MediaRef { path: path.clone() }, warp: WarpMode::RePitch, source_bpm: bpm, markers: vec![] }),
            region: ClipRegion::looped(beats.max(0.25)),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        };
        self.media.request(&path, io::resolve_media(&MediaRef { path: path.clone() }, self.project_dir().as_deref()));
        if let Some(s) = self.project.tracks.get_mut(t).and_then(|tr| tr.slots.get_mut(slot)) {
            s.clip = Some(clip);
            self.label("Record Audio");
            self.status_line = format!("Recorded {:.1} s to {}", take.frames as f64 / self.rt.settings.sample_rate as f64, take.path.display());
        }
    }

    /// Ends an Arrangement log entry for a track at `at`.
    fn close_record(&mut self, t: usize, at: Beats) {
        let Some(open) = self.open_records.remove(&t) else { return };
        if at <= open.start + 1e-6 {
            return;
        }
        let id = self.project.alloc_id();
        let mut ids = self.project.next_id;
        let track = &mut self.project.tracks[t];
        track.clear_arrangement_range(open.start, at, || {
            ids += 1;
            ids
        });
        track.arrangement.push(ArrangementClip {
            id,
            start: open.start,
            length: at - open.start,
            offset: open.start - open.origin,
            fade_in: 0.0,
            fade_out: 0.0,
            clip: open.clip,
        });
        track.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.project.next_id = ids + 1;
        self.label("Record Arrangement");
    }

    pub fn toggle_arrangement_record(&mut self) {
        let now = self.beat();
        if self.arrangement_record {
            self.arrangement_record = false;
            let open: Vec<usize> = self.open_records.keys().copied().collect();
            for t in open {
                self.close_record(t, now);
            }
        } else {
            self.arrangement_record = true;
            // Clips already running are logged from now on.
            for t in 0..self.project.tracks.len() {
                let st = self.track_status(t);
                let slot = st.playing_slot.load(Ordering::Relaxed);
                let pos = st.clip_position.load();
                if slot >= 0 && self.playing() {
                    if let Some(clip) = self.project.tracks[t].slots.get(slot as usize).and_then(|s| s.clip.clone()) {
                        let elapsed = (pos - clip.region.start).max(0.0);
                        self.open_records.insert(t, OpenRecord { start: now, origin: now - elapsed, clip });
                    }
                }
            }
            if !self.playing() {
                if self.project.transport.count_in_bars > 0 {
                    self.rt.send(Command::PlayWithCountIn);
                } else {
                    self.rt.send(Command::Play);
                }
            }
        }
    }

    /// Session Record: overdub running MIDI clips or record into the selected scene on armed tracks.
    pub fn session_record(&mut self) {
        let recording: Vec<usize> = (0..self.project.tracks.len())
            .filter(|&t| self.track_status(t).recording_slot.load(Ordering::Relaxed) >= 0)
            .collect();
        if !recording.is_empty() {
            for t in recording {
                let slot = self.track_status(t).recording_slot.load(Ordering::Relaxed) as u16;
                self.rt.send(Command::SessionRecord { track: t as u16, slot, on: false });
            }
            return;
        }
        let scene = self.sel.scene;
        for t in 0..self.project.tracks.len() {
            let tr = &self.project.tracks[t];
            if !tr.mixer.arm {
                continue;
            }
            let playing = self.track_status(t).playing_slot.load(Ordering::Relaxed);
            if tr.kind == TrackKind::Midi && playing >= 0 {
                self.rt.send(Command::SessionRecord { track: t as u16, slot: playing as u16, on: true });
            } else if let Some(slot) = (scene..tr.slots.len()).find(|&s| tr.slots[s].clip.is_none()) {
                self.rt.send(Command::SessionRecord { track: t as u16, slot: slot as u16, on: true });
            }
        }
    }

    // ------------------------------------------------------------ keyboard

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let egui::Event::Key { key, pressed, repeat, modifiers, .. } = ev else { continue };
            if repeat {
                continue;
            }
            if pressed && modifiers.command {
                match key {
                    Key::Z if modifiers.shift => self.redo(),
                    Key::Z => self.undo(),
                    Key::Y => self.redo(),
                    Key::S if modifiers.shift => {
                        self.save_as();
                    }
                    Key::S => {
                        self.save();
                    }
                    Key::O => self.dialogs.open_requested = true,
                    Key::N => self.dialogs.new_requested = true,
                    Key::E => arrangement::split_selected(self),
                    Key::D => self.duplicate_selection(),
                    Key::C => self.copy_selection(),
                    Key::V => self.paste(),
                    Key::A if self.detail == Detail::Clip => clip_editor::select_all(self),
                    Key::R if modifiers.shift => self.dialogs.export_open = true,
                    Key::Comma => self.dialogs.settings_open = true,
                    Key::Num1 => self.project.transport.global_quantize = xdaw_sequencer::Quantize::Bar1,
                    _ => {}
                }
                continue;
            }
            if typing {
                continue;
            }
            if pressed {
                match key {
                    Key::Space => {
                        if self.playing() {
                            self.rt.send(Command::Stop);
                        } else if modifiers.shift {
                            self.rt.send(Command::Continue);
                        } else {
                            self.rt.send(Command::Play);
                        }
                        continue;
                    }
                    Key::Tab => {
                        self.view = if self.view == View::Session { View::Arrangement } else { View::Session };
                        continue;
                    }
                    Key::F9 => {
                        self.toggle_arrangement_record();
                        continue;
                    }
                    Key::Delete | Key::Backspace => {
                        self.delete_selection();
                        continue;
                    }
                    Key::Enter => {
                        if let Some(ClipLoc::Slot { track, slot }) = self.sel.clip {
                            self.rt.send(Command::LaunchSlot { track: track as u16, slot: slot as u16, down: true });
                            self.rt.send(Command::LaunchSlot { track: track as u16, slot: slot as u16, down: false });
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if self.map_mode == MapMode::Key && pressed {
                if let Some(target) = self.map_target.clone() {
                    let name = format!("{key:?}");
                    self.project.key_mappings.retain(|m| m.target != target);
                    self.project.key_mappings.push(ControlMapping { source: MappingSource::Key { key: name.clone() }, target });
                    self.label("Key Map");
                    self.status_line = format!("Mapped key {name}");
                    continue;
                }
            }
            if self.map_mode == MapMode::Off && pressed && self.apply_key_mapping(&format!("{key:?}")) {
                continue;
            }
            if self.kb_enabled && modifiers == Modifiers::NONE || (self.kb_enabled && modifiers.shift) {
                self.computer_key(key, pressed);
            }
        }
    }

    fn computer_key(&mut self, key: Key, pressed: bool) {
        let name = format!("{key:?}");
        if pressed {
            match key {
                Key::Z => return self.kb.octave_down(),
                Key::X => return self.kb.octave_up(),
                Key::C => return self.kb.velocity_down(),
                Key::V => return self.kb.velocity_up(),
                _ => {}
            }
        }
        let note = if pressed { self.kb.note_for(&name) } else { self.held_keys.remove(&key) };
        let Some(note) = note else { return };
        if pressed {
            if self.held_keys.contains_key(&key) {
                return;
            }
            self.held_keys.insert(key, note);
        }
        let kind = if pressed { xdaw_dsp::NoteKind::On { pitch: note, velocity: self.kb.velocity } } else { xdaw_dsp::NoteKind::Off { pitch: note } };
        let targets: Vec<usize> = self.project.tracks.iter().enumerate().filter(|(_, t)| t.kind == TrackKind::Midi && t.mixer.arm).map(|(i, _)| i).collect();
        for t in targets {
            self.rt.send(Command::Note { track: t as u16, kind });
        }
    }

    fn apply_key_mapping(&mut self, key: &str) -> bool {
        let targets: Vec<MappingTarget> = self
            .project
            .key_mappings
            .iter()
            .filter(|m| matches!(&m.source, MappingSource::Key { key: k } if k == key))
            .map(|m| m.target.clone())
            .collect();
        for t in &targets {
            self.trigger_mapping(t, 127);
        }
        !targets.is_empty()
    }

    pub fn trigger_mapping(&mut self, target: &MappingTarget, value: u8) {
        match target {
            MappingTarget::LaunchSlot { track, scene } => {
                let t = self.project.tracks.iter().position(|t| t.id == *track);
                let s = self.project.scenes.iter().position(|s| s.id == *scene);
                if let (Some(t), Some(s)) = (t, s) {
                    self.rt.send(Command::LaunchSlot { track: t as u16, slot: s as u16, down: value > 0 });
                }
            }
            MappingTarget::LaunchScene { scene } => {
                if value > 0 {
                    if let Some(s) = self.project.scenes.iter().position(|x| x.id == *scene) {
                        self.rt.send(Command::LaunchScene { slot: s as u16 });
                    }
                }
            }
            MappingTarget::StopTrack { track } => {
                if value > 0 {
                    if let Some(t) = self.project.tracks.iter().position(|t| t.id == *track) {
                        self.rt.send(Command::StopTrack { track: t as u16 });
                    }
                }
            }
            MappingTarget::Param { track, target } => {
                let Some(r) = self.project.find_track(*track) else { return };
                let n = value as f32 / 127.0;
                let t = self.project.track_mut(r);
                match *target {
                    ParamTarget::Volume => t.mixer.volume_db = crate::mixer::fader_to_db(n),
                    ParamTarget::Pan => t.mixer.pan = n * 2.0 - 1.0,
                    ParamTarget::Send(i) => {
                        if let Some(s) = t.mixer.sends_db.get_mut(i) {
                            *s = crate::mixer::fader_to_db(n);
                        }
                    }
                    ParamTarget::Device { device, param } => {
                        if let Some(d) = t.devices.iter_mut().find(|d| d.id == device) {
                            if let Some(desc) = d.kind.params().get(param) {
                                let v = desc.from_normalized(n);
                                if let Some(p) = d.params.get_mut(param) {
                                    *p = v;
                                }
                            }
                        }
                    }
                }
                self.label("MIDI Control");
            }
            MappingTarget::TransportPlay => {
                if value > 0 {
                    self.rt.send(Command::Play);
                }
            }
            MappingTarget::TransportStop => {
                if value > 0 {
                    self.rt.send(Command::Stop);
                }
            }
        }
    }

    fn handle_midi_ui(&mut self) {
        let Some(rx) = self.rt.midi_rx.clone() else { return };
        for raw in rx.try_iter() {
            let msg = xdaw_midi::parse(&raw.bytes[..raw.len as usize]);
            let source = match msg {
                xdaw_midi::MidiMessage::NoteOn { channel, note, .. } | xdaw_midi::MidiMessage::NoteOff { channel, note, .. } => {
                    Some((MappingSource::MidiNote { channel, note }, if matches!(msg, xdaw_midi::MidiMessage::NoteOn { .. }) { 127 } else { 0 }))
                }
                xdaw_midi::MidiMessage::ControlChange { channel, cc, value } => Some((MappingSource::MidiCc { channel, cc }, value)),
                _ => None,
            };
            let Some((source, value)) = source else { continue };
            if self.map_mode == MapMode::Midi {
                if let Some(target) = self.map_target.clone() {
                    if value > 0 || matches!(source, MappingSource::MidiCc { .. }) {
                        self.project.midi_mappings.retain(|m| m.target != target && m.source != source);
                        self.project.midi_mappings.push(ControlMapping { source: source.clone(), target });
                        self.label("MIDI Map");
                        self.status_line = format!("Mapped {source:?}");
                        self.map_target = None;
                    }
                }
                continue;
            }
            let targets: Vec<MappingTarget> = self.project.midi_mappings.iter().filter(|m| m.source == source).map(|m| m.target.clone()).collect();
            for t in targets {
                self.trigger_mapping(&t, value);
            }
        }
    }

    // ------------------------------------------------------------ edit commands

    pub fn delete_selection(&mut self) {
        if self.detail == Detail::Clip && !self.sel.notes.is_empty() {
            clip_editor::delete_selected_notes(self);
            return;
        }
        if self.view == View::Arrangement && !self.sel.arr_clips.is_empty() {
            let sel = std::mem::take(&mut self.sel.arr_clips);
            for (t, id) in sel {
                if let Some(tr) = self.project.tracks.get_mut(t) {
                    tr.arrangement.retain(|c| c.id != id);
                }
            }
            self.sel.clip = None;
            self.label("Delete Clips");
            return;
        }
        if let Some(ClipLoc::Slot { track, slot }) = self.sel.clip {
            if let Some(s) = self.project.tracks.get_mut(track).and_then(|t| t.slots.get_mut(slot)) {
                s.clip = None;
                self.sel.clip = None;
                self.label("Delete Clip");
            }
        }
    }

    pub fn duplicate_selection(&mut self) {
        if self.view == View::Arrangement && !self.sel.arr_clips.is_empty() {
            arrangement::duplicate_selected(self);
            return;
        }
        if let Some(ClipLoc::Slot { track, slot }) = self.sel.clip {
            session::duplicate_clip(self, track, slot);
        }
    }

    pub fn copy_selection(&mut self) {
        if self.detail == Detail::Clip && !self.sel.notes.is_empty() {
            clip_editor::copy_notes(self);
            return;
        }
        if self.view == View::Arrangement && !self.sel.arr_clips.is_empty() {
            self.clipboard.arrangement = self
                .sel
                .arr_clips
                .iter()
                .filter_map(|&(t, id)| self.project.tracks.get(t)?.arrangement.iter().find(|c| c.id == id).map(|c| (t, c.clone())))
                .collect();
            self.status_line = format!("Copied {} clip(s)", self.clipboard.arrangement.len());
            return;
        }
        self.clipboard.clip = self.selected_clip().cloned();
    }

    pub fn paste(&mut self) {
        if self.detail == Detail::Clip && !self.clipboard.notes.is_empty() && self.sel.clip.is_some() {
            clip_editor::paste_notes(self);
            return;
        }
        if self.view == View::Arrangement && !self.clipboard.arrangement.is_empty() {
            arrangement::paste(self);
            return;
        }
        if let (Some(clip), Some(ClipLoc::Slot { track, slot })) = (self.clipboard.clip.clone(), self.sel.clip) {
            session::place_clip(self, track, slot, clip);
        }
    }

    pub fn track_ix(r: TrackRef) -> TrackIx {
        match r {
            TrackRef::Track(i) => TrackIx::Track(i as u16),
            TrackRef::Return(i) => TrackIx::Return(i as u16),
            TrackRef::Master => TrackIx::Master,
        }
    }

    /// Records an automation point while Arrangement Record runs (gesture on a parameter).
    pub fn record_automation(&mut self, r: TrackRef, target: ParamTarget, value: f32) {
        if !(self.arrangement_record && self.playing()) {
            return;
        }
        let beat = self.beat();
        let lane = self.project.track_mut(r).lane_mut(target);
        // Overwrite points the gesture passes over.
        lane.points.retain(|p| !(p.beat > beat - 0.25 && p.beat < beat));
        lane.insert(AutomationPoint { beat, value });
    }
}

impl eframe::App for XdawApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = self.last_frame.elapsed();
        self.last_frame = Instant::now();
        let _ = dt;
        self.handle_engine_events();
        if let Some(mut script) = self.script.take() {
            script.run(self, &ctx);
            self.script = Some(script);
        }
        self.handle_midi_ui();
        self.handle_keys(&ctx);

        transport::show(self, ui);
        egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| transport::status_bar(self, ui));
        egui::Panel::left("browser").resizable(true).default_size(210.0).min_size(140.0).show(ui, |ui| browser::show(self, ui));
        egui::Panel::bottom("detail").resizable(true).default_size(260.0).min_size(120.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.detail, Detail::Clip, "Clip");
                ui.selectable_value(&mut self.detail, Detail::Devices, "Devices");
            });
            ui.separator();
            match self.detail {
                Detail::Clip => clip_editor::show(self, ui),
                Detail::Devices => devices::show(self, ui),
            }
        });
        egui::CentralPanel::default().show(ui, |ui| match self.view {
            View::Session => session::show(self, ui),
            View::Arrangement => arrangement::show(self, ui),
        });
        dialogs::show(self, &ctx);

        // Sync the audio thread, then record undo once gestures finish.
        if self.rt.reconciler.needs_sync(&self.project, self.media_gen) {
            let mut cmds = vec![];
            let result = self.rt.reconciler.sync(&self.project, &self.media, self.media_gen, |c| cmds.push(c));
            if let Err(e) = result {
                self.status_line = format!("Engine sync failed: {e}");
            }
            for c in cmds {
                self.rt.send(c);
            }
        }
        if let Some(s) = self.pending_launch_scene.take() {
            self.rt.send(Command::LaunchScene { slot: s as u16 });
        }
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        self.commit_undo(pointer_down);
        self.autosave();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        ctx.request_repaint_after(Duration::from_millis(if self.playing() { 16 } else { 50 }));
    }

    fn on_exit(&mut self) {
        if self.is_dirty() {
            let _ = io::write_autosave(&self.project, self.path.as_deref());
        }
        self.rt.stop();
    }
}
