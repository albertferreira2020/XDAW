//! Offline audio edits: Freeze/Flatten, Reverse, Consolidate and Slice to MIDI.
//!
//! Each operation writes new WAV files (never touching the originals), points the
//! project at them and records one undo step. Files go to `Samples/<kind>` of the
//! saved project, or to the application folder while the project is unsaved.

use crate::app::{ClipLoc, XdawApp};
use crate::browser::media_ref_for;
use std::path::PathBuf;
use xdaw_dsp::instruments::{FIRST_PAD_NOTE, PADS};
use xdaw_dsp::SampleData;
use xdaw_engine::export::{render_track_offline, write_wav, ExportSettings, SampleFormat};
use xdaw_project::*;
use xdaw_sequencer::{Beats, ClipRegion};

/// Seconds rendered after the last clip of a frozen track so delays and reverbs ring out.
pub const FREEZE_TAIL_SECONDS: f64 = 3.0;

fn edit_dir(app: &XdawApp, kind: &str) -> PathBuf {
    match app.project_dir() {
        Some(d) => d.join("Samples").join(kind),
        None => io::app_support_dir().join(kind),
    }
}

fn clean(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' { c } else { '_' }).collect()
}

/// Writes planar audio as a stereo float WAV and registers it with the media cache.
fn write_media(app: &mut XdawApp, dir: &str, name: &str, l: &[f32], r: &[f32], sr: u32) -> Result<MediaRef, String> {
    let dir = edit_dir(app, dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{}.wav", clean(name), app.project.next_id));
    app.project.next_id += 1;
    write_wav(&path, l, r, sr, SampleFormat::Float32, false).map_err(|e| e.to_string())?;
    let media = media_ref_for(app, &path);
    app.media.request(&media.path, io::resolve_media(&media, app.project_dir().as_deref()));
    Ok(media)
}

fn planar(data: &SampleData, from: usize, to: usize, reversed: bool) -> (Vec<f32>, Vec<f32>) {
    let to = to.min(data.frames());
    let from = from.min(to);
    let ch = |c: usize| -> Vec<f32> {
        let src = &data.channels[c.min(data.channels.len() - 1)][from..to];
        if reversed { src.iter().rev().copied().collect() } else { src.to_vec() }
    };
    (ch(0), ch(1))
}

fn song_length(t: &Track, bar: Beats) -> Beats {
    let end = t.arrangement.iter().map(|c| c.end()).fold(0.0, f64::max);
    (end / bar).ceil() * bar
}

// ------------------------------------------------------------ freeze / flatten

/// Renders the track through its devices and plays the result instead of them.
pub fn freeze_track(app: &mut XdawApp, t: usize) -> bool {
    let Some(track) = app.project.tracks.get(t) else { return false };
    if track.kind == TrackKind::Group {
        app.status_line = "Group tracks cannot be frozen".into();
        return false;
    }
    let bar = app.project.transport.time_signature.bar_beats();
    let end = song_length(track, bar);
    if end <= 0.0 {
        app.status_line = "Freeze: the track has no Arrangement clips".into();
        return false;
    }
    let name = track.name.clone();
    let sr = app.rt.settings.sample_rate;
    let settings = ExportSettings { start: 0.0, end, tail_seconds: FREEZE_TAIL_SECONDS, sample_rate: sr, format: SampleFormat::Float32, dither: false };
    let media = app.media.loaded.clone();
    match render_track_offline(&app.project, &media, t, &settings, true) {
        Ok((l, r)) => match write_media(app, "Frozen", &name, &l, &r, sr) {
            Ok(m) => {
                let bpm = app.project.transport.bpm;
                let length = l.len() as f64 / sr as f64 * bpm / 60.0;
                app.project.tracks[t].frozen = Some(Frozen { media: m, bpm, length });
                app.label("Freeze Track");
                app.status_line = format!("Froze '{name}' ({:.1} s)", l.len() as f64 / sr as f64);
                true
            }
            Err(e) => {
                app.status_line = format!("Freeze failed: {e}");
                false
            }
        },
        Err(e) => {
            app.status_line = format!("Freeze failed: {e}");
            false
        }
    }
}

pub fn unfreeze_track(app: &mut XdawApp, t: usize) {
    if let Some(tr) = app.project.tracks.get_mut(t) {
        if tr.frozen.take().is_some() {
            app.label("Unfreeze Track");
        }
    }
}

/// Freezes if needed, then replaces the track's clips and devices by the audio.
pub fn flatten_track_in_place(app: &mut XdawApp, t: usize) {
    if app.project.tracks.get(t).is_some_and(|tr| tr.frozen.is_none()) && !freeze_track(app, t) {
        return;
    }
    let Some(f) = app.project.tracks.get_mut(t).and_then(|tr| tr.frozen.take()) else { return };
    let (color, name) = (app.project.tracks[t].color, app.project.tracks[t].name.clone());
    let (clip_id, arr_id) = (app.project.alloc_id(), app.project.alloc_id());
    let clip = Clip::rendered_audio(clip_id, &format!("{name} (Flattened)"), color, f.media, f.bpm, f.length);
    app.project.tracks[t].flatten_to_audio(clip, f.length, arr_id);
    app.sel.arr_clips = vec![(t, arr_id)];
    app.label("Flatten Track");
    app.status_line = format!("Flattened '{name}' to audio");
}

// ------------------------------------------------------------ reverse

/// Reverses the selected audio clips (Arrangement selection, else the open Session clip).
pub fn reverse_selection(app: &mut XdawApp) {
    let mut done = 0;
    let mut sel = app.sel.arr_clips.clone();
    if sel.is_empty() {
        if let Some(ClipLoc::Arrangement { track, id }) = app.sel.clip {
            sel.push((track, id));
        }
    }
    for (t, id) in sel {
        let Some(c) = app.project.tracks.get(t).and_then(|tr| tr.arrangement.iter().find(|c| c.id == id)).cloned() else { continue };
        if let Some(media) = reverse_media(app, &c.clip) {
            let file_beats = media.1;
            if let Some(c) = app.project.tracks[t].arrangement.iter_mut().find(|c| c.id == id) {
                c.reverse(file_beats);
                if let ClipContent::Audio(a) = &mut c.clip.content {
                    a.media = media.0;
                }
                done += 1;
            }
        }
    }
    if done == 0 {
        if let Some(ClipLoc::Slot { track, slot }) = app.sel.clip {
            let clip = app.project.tracks.get(track).and_then(|t| t.slots.get(slot)).and_then(|s| s.clip.clone());
            if let Some(clip) = clip {
                if let Some((m, file_beats)) = reverse_media(app, &clip) {
                    if let Some(c) = app.project.tracks[track].slots[slot].clip.as_mut() {
                        c.reverse_region(file_beats);
                        if let ClipContent::Audio(a) = &mut c.content {
                            a.media = m;
                        }
                        done += 1;
                    }
                }
            }
        }
    }
    if done > 0 {
        app.label("Reverse");
        app.status_line = format!("Reversed {done} clip(s); warp markers were cleared");
    } else if app.status_line.is_empty() || !app.status_line.starts_with("Reverse") {
        app.status_line = "Reverse: select an audio clip whose media is loaded".into();
    }
}

/// Writes a reversed copy of the clip's media; returns it with the file length in beats.
fn reverse_media(app: &mut XdawApp, clip: &Clip) -> Option<(MediaRef, Beats)> {
    let ClipContent::Audio(a) = &clip.content else { return None };
    let data = app.media.loaded.get(&a.media.path)?.clone();
    let (l, r) = planar(&data, 0, data.frames(), true);
    let stem = std::path::Path::new(&a.media.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "clip".into());
    match write_media(app, "Reversed", &format!("{stem}-reversed"), &l, &r, data.sample_rate as u32) {
        Ok(m) => Some((m, data.duration_seconds() * a.source_bpm / 60.0)),
        Err(e) => {
            app.status_line = format!("Reverse failed: {e}");
            None
        }
    }
}

// ------------------------------------------------------------ consolidate

/// Merges the selected Arrangement clips of each track into one clip.
pub fn consolidate_selection(app: &mut XdawApp) {
    let mut ranges: Vec<(usize, Beats, Beats)> = vec![];
    for &(t, id) in &app.sel.arr_clips {
        let Some(c) = app.project.tracks.get(t).and_then(|tr| tr.arrangement.iter().find(|c| c.id == id)) else { continue };
        match ranges.iter_mut().find(|r| r.0 == t) {
            Some(r) => {
                r.1 = r.1.min(c.start);
                r.2 = r.2.max(c.end());
            }
            None => ranges.push((t, c.start, c.end())),
        }
    }
    if ranges.is_empty() {
        app.status_line = "Consolidate: select clips in the Arrangement first".into();
        return;
    }
    let mut new_sel = vec![];
    for (t, from, to) in ranges {
        let mut next = app.project.next_id;
        let mut ids = || {
            next += 1;
            next - 1
        };
        if app.project.tracks[t].kind == TrackKind::Midi {
            if app.project.tracks[t].consolidate_midi(from, to, &mut ids) {
                app.project.next_id = next;
                if let Some(c) = app.project.tracks[t].arrangement.iter().find(|c| c.start == from) {
                    new_sel.push((t, c.id));
                }
            }
            continue;
        }
        app.project.next_id = next;
        let sr = app.rt.settings.sample_rate;
        let settings = ExportSettings { start: from, end: to, tail_seconds: 0.0, sample_rate: sr, format: SampleFormat::Float32, dither: false };
        let media = app.media.loaded.clone();
        let rendered = render_track_offline(&app.project, &media, t, &settings, false).map_err(|e| e.to_string());
        let name = app.project.tracks[t].name.clone();
        match rendered.and_then(|(l, r)| write_media(app, "Consolidated", &name, &l, &r, sr)) {
            Ok(m) => {
                let bpm = app.project.transport.bpm;
                let clip = Clip::rendered_audio(app.project.alloc_id(), "Consolidated", app.project.tracks[t].color, m, bpm, to - from);
                let mut next = app.project.next_id;
                app.project.tracks[t].replace_with_audio(from, to, clip, || {
                    next += 1;
                    next - 1
                });
                app.project.next_id = next;
                if let Some(c) = app.project.tracks[t].arrangement.iter().find(|c| c.start == from) {
                    new_sel.push((t, c.id));
                }
            }
            Err(e) => {
                app.status_line = format!("Consolidate failed: {e}");
                return;
            }
        }
    }
    app.sel.arr_clips = new_sel;
    app.label("Consolidate");
}

// ------------------------------------------------------------ slice to MIDI

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliceMode {
    /// Every `beats` beats of the clip.
    Grid(f64),
    /// Transients; sensitivity 0..1.
    Transients(f32),
}

impl SliceMode {
    pub const ALL: [(SliceMode, &'static str); 6] = [
        (SliceMode::Grid(1.0), "Every beat"),
        (SliceMode::Grid(0.5), "Every 1/8"),
        (SliceMode::Grid(0.25), "Every 1/16"),
        (SliceMode::Grid(4.0), "Every bar"),
        (SliceMode::Transients(0.5), "Transients"),
        (SliceMode::Transients(0.85), "Transients (more)"),
    ];
}

/// Slice points (seconds in the file) for the part of `data` the clip plays.
pub fn slice_points(data: &SampleData, from_s: f64, to_s: f64, bpm: f64, mode: SliceMode) -> Vec<f64> {
    match mode {
        SliceMode::Grid(beats) => xdaw_dsp::slicing::grid_points(from_s, to_s, beats * 60.0 / bpm),
        SliceMode::Transients(s) => xdaw_dsp::slicing::detect_onsets(data, from_s, to_s, s),
    }
}

/// Builds a MIDI track with a Drum Rack of the slices and a clip that replays them.
/// Works on the selected Arrangement clip, else on the open Session clip.
pub fn slice_to_midi(app: &mut XdawApp, mode: SliceMode) {
    enum Src {
        Arr { start: Beats, length: Beats, offset: Beats },
        Slot(usize),
    }
    let arr_target = match app.sel.clip {
        Some(ClipLoc::Arrangement { track, id }) => Some((track, id)),
        Some(ClipLoc::Slot { .. }) => None,
        None => app.sel.arr_clips.first().copied(),
    };
    let (clip, src) = if let Some((t, id)) = arr_target {
        match app.project.tracks.get(t).and_then(|tr| tr.arrangement.iter().find(|c| c.id == id)) {
            Some(c) => (c.clip.clone(), Src::Arr { start: c.start, length: c.length, offset: c.offset }),
            None => return,
        }
    } else if let Some(ClipLoc::Slot { track, slot }) = app.sel.clip {
        match app.project.tracks.get(track).and_then(|t| t.slots.get(slot)).and_then(|s| s.clip.clone()) {
            Some(c) => (c, Src::Slot(slot)),
            None => return,
        }
    } else {
        app.status_line = "Slice: select an audio clip first".into();
        return;
    };
    let ClipContent::Audio(a) = &clip.content else {
        app.status_line = "Slice: only audio clips can be sliced".into();
        return;
    };
    let Some(data) = app.media.loaded.get(&a.media.path).cloned() else {
        app.status_line = "Slice: the clip's media is not loaded".into();
        return;
    };
    let spb = 60.0 / a.source_bpm;
    let r = clip.region;
    let (from_s, to_s) = (r.start * spb, (r.end * spb).min(data.duration_seconds()));
    let mut points = slice_points(&data, from_s, to_s, a.source_bpm, mode);
    if points.is_empty() {
        points.push(from_s);
    }
    let capped = points.len() > PADS;
    points.truncate(PADS);
    let sr = data.sample_rate;
    let stem = clean(&clip.name);
    let mut pads: Vec<Option<MediaRef>> = vec![None; PADS];
    let mut notes = vec![];
    for (i, &p) in points.iter().enumerate() {
        let end = points.get(i + 1).copied().unwrap_or(to_s);
        let (l, rr) = planar(&data, (p * sr) as usize, (end * sr) as usize, false);
        if l.is_empty() {
            continue;
        }
        match write_media(app, "Slices", &format!("{stem}-slice{:02}", i + 1), &l, &rr, sr as u32) {
            Ok(m) => pads[i] = Some(m),
            Err(e) => {
                app.status_line = format!("Slice failed: {e}");
                return;
            }
        }
        notes.push(Note { pitch: FIRST_PAD_NOTE + i as u8, start: p / spb - r.start, length: ((end - p) / spb).max(1.0 / 16.0), velocity: 100 });
    }
    let idx = app.project.add_track(TrackKind::Midi, &format!("{} Slices", clip.name));
    app.project.tracks[idx].color = clip.color;
    let dev = app.project.add_device(TrackRef::Track(idx), xdaw_dsp::DeviceKind::DrumRack);
    if let Some(d) = app.project.tracks[idx].devices.iter_mut().find(|d| d.id == dev) {
        d.pads = pads;
        d.preset_name = Some("Slices".into());
    }
    let len = (r.end - r.start).max(1.0 / 16.0);
    let midi = Clip {
        id: app.project.alloc_id(),
        name: format!("{} Slices", clip.name),
        color: clip.color,
        content: ClipContent::Midi { notes },
        region: ClipRegion::looped(len),
        launch_mode: clip.launch_mode,
        quantize: clip.quantize,
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    };
    match src {
        Src::Arr { start, length, offset } => {
            let id = app.project.alloc_id();
            app.project.tracks[idx].arrangement.push(ArrangementClip { id, start, length, offset, fade_in: 0.0, fade_out: 0.0, clip: midi });
        }
        Src::Slot(s) => {
            if let Some(slot) = app.project.tracks[idx].slots.get_mut(s) {
                slot.clip = Some(midi);
            }
        }
    }
    app.select_track(TrackRef::Track(idx));
    app.label("Slice to New MIDI Track");
    app.status_line = format!("Sliced into {} pads{}", points.len(), if capped { " (limited to the Drum Rack's 16 pads)" } else { "" });
}
