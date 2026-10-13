//! Left-hand browser: instruments, effects, presets, sample folders.
//! Items can be dragged onto Session slots, tracks and the device chain.

use crate::app::{ClipLoc, XdawApp};
use crate::theme;
use egui::{RichText, Ui};
use std::path::{Path, PathBuf};
use xdaw_dsp::DeviceKind;
use xdaw_project::*;
use xdaw_sequencer::ClipRegion;

#[derive(Clone, Debug)]
pub enum BrowserItem {
    Device(DeviceKind),
    File(PathBuf),
    Preset(PathBuf),
}

pub struct BrowserState {
    pub folders: Vec<PathBuf>,
    pub filter: String,
}

impl BrowserState {
    pub fn new() -> Self {
        let mut folders = vec![];
        if let Ok(cwd) = std::env::current_dir() {
            let demo = cwd.join("demo");
            if demo.is_dir() {
                folders.push(demo);
            }
        }
        Self { folders, filter: String::new() }
    }
}

pub fn is_audio(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("wav" | "wave" | "aif" | "aiff" | "flac"))
}

pub fn is_midi(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("mid" | "midi"))
}

pub fn presets_dir() -> PathBuf {
    io::app_support_dir().join("Presets")
}

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    ui.add(egui::TextEdit::singleline(&mut app.browser.filter).hint_text("Search").desired_width(f32::INFINITY));
    let filter = app.browser.filter.to_lowercase();
    let matches = |s: &str| filter.is_empty() || s.to_lowercase().contains(&filter);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::CollapsingHeader::new("Instruments").default_open(true).show(ui, |ui| {
            for k in DeviceKind::ALL.into_iter().filter(|k| k.is_instrument() && matches(k.name())) {
                item(app, ui, k.name(), BrowserItem::Device(k));
            }
        });
        egui::CollapsingHeader::new("Audio Effects").default_open(true).show(ui, |ui| {
            for k in DeviceKind::ALL.into_iter().filter(|k| !k.is_instrument() && matches(k.name())) {
                item(app, ui, k.name(), BrowserItem::Device(k));
            }
        });
        egui::CollapsingHeader::new("Presets").show(ui, |ui| {
            let dir = presets_dir();
            for kind in DeviceKind::ALL {
                let kdir = dir.join(format!("{kind:?}"));
                let Ok(entries) = std::fs::read_dir(&kdir) else { continue };
                let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
                files.sort();
                for f in files {
                    let name = format!("{} · {}", kind.name(), f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    if matches(&name) {
                        item(app, ui, &name, BrowserItem::Preset(f));
                    }
                }
            }
        });
        let mut places: Vec<(String, PathBuf)> = vec![];
        if let Some(d) = app.project_dir() {
            places.push(("Current Project".into(), d));
        }
        for f in &app.browser.folders {
            places.push((f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.display().to_string()), f.clone()));
        }
        for (name, dir) in places {
            egui::CollapsingHeader::new(name).id_salt(dir.display().to_string()).show(ui, |ui| folder(app, ui, &dir, &filter, 0));
        }
        if ui.button("Add Folder…").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                app.browser.folders.push(d);
            }
        }
    });
}

fn folder(app: &mut XdawApp, ui: &mut Ui, dir: &Path, filter: &str, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() && depth < 4 {
            egui::CollapsingHeader::new(&name).id_salt(p.display().to_string()).show(ui, |ui| folder(app, ui, &p, filter, depth + 1));
        } else if (is_audio(&p) || is_midi(&p)) && (filter.is_empty() || name.to_lowercase().contains(filter)) {
            item(app, ui, &name, BrowserItem::File(p.clone()));
        }
    }
}

fn item(app: &mut XdawApp, ui: &mut Ui, label: &str, payload: BrowserItem) {
    let id = egui::Id::new(("browser", label, format!("{payload:?}")));
    let resp = ui
        .dnd_drag_source(id, payload.clone(), |ui| {
            ui.label(RichText::new(label).color(theme::TEXT));
        })
        .response;
    if resp.double_clicked() {
        load_on_selection(app, payload);
    }
    resp.on_hover_text("Drag to a track/slot, or double-click to load on the selected track");
}

/// Double-click behaviour: devices/presets to the selected track, files into the selected slot.
pub fn load_on_selection(app: &mut XdawApp, payload: BrowserItem) {
    match payload {
        BrowserItem::Device(k) => {
            if let Some(r) = app.sel.track {
                add_device(app, r, k);
            } else {
                app.status_line = "Select a track first".into();
            }
        }
        BrowserItem::Preset(p) => {
            if let Some(r) = app.sel.track {
                load_preset(app, r, &p);
            }
        }
        BrowserItem::File(p) => {
            if let Some(ClipLoc::Slot { track, slot }) = app.sel.clip {
                drop_file_on_slot(app, track, slot, &p);
            } else if let Some(TrackRef::Track(t)) = app.sel.track {
                let slot = app.sel.scene;
                drop_file_on_slot(app, t, slot, &p);
            }
        }
    }
}

pub fn add_device(app: &mut XdawApp, r: TrackRef, kind: DeviceKind) {
    let t = app.project.track(r);
    if kind.is_instrument() && t.kind != TrackKind::Midi {
        app.status_line = format!("{} needs a MIDI track", kind.name());
        return;
    }
    app.project.add_device(r, kind);
    app.label(&format!("Add {}", kind.name()));
    app.detail = crate::app::Detail::Devices;
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PresetFile {
    format: String,
    kind: DeviceKind,
    name: String,
    params: Vec<(String, f32)>,
}

pub fn save_preset(app: &mut XdawApp, d: &DeviceState, name: &str) {
    let file = PresetFile {
        format: "xdaw-preset-1".into(),
        kind: d.kind,
        name: name.into(),
        params: d.kind.params().iter().enumerate().map(|(i, p)| (p.id.to_string(), d.param(i))).collect(),
    };
    let dir = presets_dir().join(format!("{:?}", d.kind));
    let path = dir.join(format!("{}.json", sanitize(name)));
    let r = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string()).and_then(|b| io::atomic_write(&path, &b).map_err(|e| e.to_string()));
    app.status_line = match r {
        Ok(()) => format!("Saved preset {}", path.display()),
        Err(e) => format!("Preset save failed: {e}"),
    };
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect()
}

pub fn load_preset(app: &mut XdawApp, r: TrackRef, path: &Path) {
    let parsed: Result<PresetFile, String> = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
    let preset = match parsed {
        Ok(p) => p,
        Err(e) => {
            app.status_line = format!("Invalid preset {}: {e}", path.display());
            return;
        }
    };
    let track = app.project.track(r);
    let existing = track.devices.iter().position(|d| d.kind == preset.kind);
    let idx = match existing {
        Some(i) => i,
        None => {
            if preset.kind.is_instrument() && track.kind != TrackKind::Midi {
                app.status_line = "Instrument presets need a MIDI track".into();
                return;
            }
            let id = app.project.add_device(r, preset.kind);
            app.project.track(r).devices.iter().position(|d| d.id == id).unwrap_or(0)
        }
    };
    let d = &mut app.project.track_mut(r).devices[idx];
    // Parameters are matched by id so presets survive parameter reordering.
    for (pid, v) in &preset.params {
        if let Some(i) = d.kind.params().iter().position(|p| p.id == pid) {
            if d.params.len() <= i {
                d.params.resize(i + 1, 0.0);
            }
            d.params[i] = d.kind.params()[i].clamp(*v);
        }
    }
    d.preset_name = Some(preset.name.clone());
    app.label("Load Preset");
}

/// Creates a clip from an audio or MIDI file in a Session slot.
pub fn drop_file_on_slot(app: &mut XdawApp, track: usize, slot: usize, path: &Path) {
    let Some(kind) = app.project.tracks.get(track).map(|t| t.kind) else { return };
    if slot >= app.project.scenes.len() {
        return;
    }
    let clip = if is_midi(path) {
        if kind != TrackKind::Midi {
            app.status_line = "MIDI files go on MIDI tracks".into();
            return;
        }
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| xdaw_midi::file::import(&b)) {
            Ok(m) => make_clip(
                app,
                path,
                ClipContent::Midi { notes: m.notes.iter().map(|n| Note { pitch: n.pitch, start: n.start, length: n.length, velocity: n.velocity }).collect() },
                m.length_beats,
                track,
            ),
            Err(e) => {
                app.status_line = format!("MIDI import failed: {e}");
                return;
            }
        }
    } else if is_audio(path) {
        if kind != TrackKind::Audio {
            app.status_line = "Audio files go on audio tracks (or drag onto a Sampler/Drum Rack)".into();
            return;
        }
        let seconds = match xdaw_engine::media::probe_duration(path) {
            Ok(s) => s,
            Err(e) => {
                app.status_line = format!("Cannot import: {e}");
                return;
            }
        };
        let bpm = app.project.transport.bpm;
        let beats = (seconds * bpm / 60.0).max(0.25);
        let media = media_ref_for(app, path);
        app.media.request(&media.path, io::resolve_media(&media, app.project_dir().as_deref()));
        make_clip(app, path, ClipContent::Audio(AudioContent { media, warp: WarpMode::RePitch, source_bpm: bpm, markers: vec![] }), beats, track)
    } else {
        return;
    };
    app.project.tracks[track].slots[slot].clip = Some(clip);
    app.sel.clip = Some(ClipLoc::Slot { track, slot });
    app.label("Import Clip");
}

pub fn media_ref_for(app: &XdawApp, path: &Path) -> MediaRef {
    match app.project_dir() {
        Some(d) if path.starts_with(&d) => MediaRef { path: io::relativize(path, &d) },
        _ => MediaRef { path: path.to_string_lossy().to_string() },
    }
}

fn make_clip(app: &mut XdawApp, path: &Path, content: ClipContent, beats: f64, track: usize) -> Clip {
    Clip {
        id: app.project.alloc_id(),
        name: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Clip".into()),
        color: app.project.tracks[track].color,
        content,
        region: ClipRegion::looped(beats),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    }
}

/// Assigns a sample to a Sampler, or to a Drum Rack pad.
pub fn assign_sample(app: &mut XdawApp, r: TrackRef, device: usize, pad: Option<usize>, path: &Path) {
    let media = media_ref_for(app, path);
    app.media.request(&media.path, io::resolve_media(&media, app.project_dir().as_deref()));
    let d = &mut app.project.track_mut(r).devices[device];
    match pad {
        Some(p) if p < d.pads.len() => d.pads[p] = Some(media),
        _ => d.sample = Some(media),
    }
    app.label("Assign Sample");
}

pub fn import_dialog(app: &mut XdawApp) {
    let Some(path) = rfd::FileDialog::new().add_filter("Audio/MIDI", &["wav", "aif", "aiff", "flac", "mid", "midi"]).pick_file() else { return };
    let track = match app.sel.track {
        Some(TrackRef::Track(t)) => t,
        _ => {
            app.status_line = "Select a track first".into();
            return;
        }
    };
    let slot = match app.sel.clip {
        Some(ClipLoc::Slot { slot, .. }) => slot,
        _ => (0..app.project.scenes.len()).find(|&s| app.project.tracks[track].slots[s].clip.is_none()).unwrap_or(0),
    };
    drop_file_on_slot(app, track, slot, &path);
}
