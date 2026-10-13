//! Clip View: clip properties, MIDI note editor and audio clip display.

use crate::app::{ClipLoc, XdawApp};
use crate::theme;
use crate::timeline::{self, HView};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use xdaw_engine::protocol::Command;
use xdaw_project::*;
use xdaw_sequencer::{ClipQuantize, LaunchMode, Quantize};

pub struct PianoState {
    pub key_h: f32,
    pub top_pitch: i32,
    pub view: HView,
    /// Note-grid width in the last frame (keyboard zoom).
    pub width: f32,
    pub grid: f64,
    pub last_length: f64,
    drag: Option<NoteDrag>,
    preview: Option<u8>,
    /// Edit every selected Arrangement MIDI clip together.
    pub multi: bool,
    /// Property shown and edited in the lane under the notes.
    pub lane: NoteLane,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NoteLane {
    #[default]
    Velocity,
    Chance,
}

impl Default for PianoState {
    fn default() -> Self {
        Self { key_h: 9.0, top_pitch: 84, view: HView::new(60.0, 4.0, 800.0), width: 600.0, grid: 0.25, last_length: 0.25, drag: None, preview: None, multi: true, lane: NoteLane::Velocity }
    }
}

/// Sample editor view; refits when another clip is shown.
pub struct WaveState {
    pub view: HView,
    pub width: f32,
    shown: Option<Id>,
    /// Sample length in beats and the waveform's left edge, for the scroll bar.
    total: f64,
    left: f32,
    /// Clip envelope being edited over the waveform.
    pub env: Option<ClipEnvTarget>,
}

impl Default for WaveState {
    fn default() -> Self {
        Self { view: HView::new(40.0, 0.5, 4000.0), width: 600.0, shown: None, total: 0.0, left: 0.0, env: None }
    }
}

#[derive(Clone, Debug)]
enum NoteDrag {
    Move { origin: Vec<(usize, f64, u8)>, start_beat: f64, start_pitch: i32 },
    Resize { origin: Vec<(usize, f64)>, start_beat: f64 },
    Select { from: Pos2 },
    Velocity { origin: Vec<(usize, u8)>, start_y: f32 },
}

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    let Some(loc) = app.sel.clip else {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new("Select a clip to edit it").color(theme::TEXT_DIM)));
        return;
    };
    if app.selected_clip().is_none() {
        app.sel.clip = None;
        return;
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(200.0);
            egui::ScrollArea::vertical().id_salt("clipprops").show(ui, |ui| properties(app, ui, loc));
        });
        ui.separator();
        let is_midi = app.selected_clip().is_some_and(Clip::is_midi);
        if is_midi {
            piano_roll(app, ui, loc);
        } else {
            audio_view(app, ui);
        }
    });
}

fn properties(app: &mut XdawApp, ui: &mut Ui, loc: ClipLoc) {
    let bpm = app.project.transport.bpm;
    let Some(c) = app.selected_clip_mut() else { return };
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, theme::rgb(c.color));
        changed |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(170.0)).changed();
    });
    ui.label(egui::RichText::new("Region (beats)").color(theme::TEXT_DIM));
    egui::Grid::new("region").num_columns(2).show(ui, |ui| {
        ui.label("Start");
        changed |= ui.add(egui::DragValue::new(&mut c.region.start).speed(0.05).range(0.0..=c.region.end - 0.0625)).changed();
        ui.end_row();
        ui.label("Loop start");
        changed |= ui.add(egui::DragValue::new(&mut c.region.loop_start).speed(0.05).range(0.0..=c.region.end - 0.0625)).changed();
        ui.end_row();
        ui.label("End");
        changed |= ui.add(egui::DragValue::new(&mut c.region.end).speed(0.05).range(c.region.loop_start + 0.0625..=4096.0)).changed();
        ui.end_row();
    });
    changed |= ui.checkbox(&mut c.region.looping, "Loop").changed();
    ui.separator();
    if let ClipLoc::Slot { .. } = loc {
        ui.label(egui::RichText::new("Launch").color(theme::TEXT_DIM));
        egui::ComboBox::from_label("Mode").selected_text(c.launch_mode.label()).show_ui(ui, |ui| {
            for m in LaunchMode::ALL {
                changed |= ui.selectable_value(&mut c.launch_mode, m, m.label()).changed();
            }
        });
        let qlabel = match c.quantize {
            ClipQuantize::Global => "Global".to_string(),
            ClipQuantize::Fixed(q) => q.label().to_string(),
        };
        egui::ComboBox::from_label("Quantize").selected_text(qlabel).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut c.quantize, ClipQuantize::Global, "Global").changed();
            for q in Quantize::ALL {
                changed |= ui.selectable_value(&mut c.quantize, ClipQuantize::Fixed(q), q.label()).changed();
            }
        });
        changed |= ui.checkbox(&mut c.legato, "Legato").changed();
        ui.label(egui::RichText::new("Follow Actions").color(theme::TEXT_DIM));
        let f = &mut c.follow;
        ui.horizontal(|ui| {
            ui.label("Time");
            changed |= ui.add(egui::DragValue::new(&mut f.time).range(0.0625..=256.0).speed(0.05).suffix(" beats")).changed();
        });
        for (label, action, chance) in [("A", &mut f.a, &mut f.chance_a), ("B", &mut f.b, &mut f.chance_b)] {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("follow", label)).width(90.0).selected_text(action.label()).show_ui(ui, |ui| {
                    for a in xdaw_sequencer::FollowAction::ALL {
                        changed |= ui.selectable_value(action, a, a.label()).changed();
                    }
                });
                ui.label("Chance");
                changed |= ui.add(egui::DragValue::new(chance).range(0..=100)).changed();
            });
        }
        ui.separator();
    }
    match &mut c.content {
        ClipContent::Audio(a) => {
            ui.label(egui::RichText::new("Audio").color(theme::TEXT_DIM));
            egui::ComboBox::from_label("Warp").selected_text(match a.warp {
                WarpMode::RePitch => "Re-Pitch",
                WarpMode::Off => "Off",
                WarpMode::Stretch => "Stretch",
            }).show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut a.warp, WarpMode::Stretch, "Stretch")
                    .on_hover_text("Follows tempo with pitch preserved (WSOLA, rendered in the background; plays as Re-Pitch until ready)")
                    .changed();
                changed |= ui.selectable_value(&mut a.warp, WarpMode::RePitch, "Re-Pitch").on_hover_text("Follows tempo by resampling: pitch changes with tempo").changed();
                changed |= ui.selectable_value(&mut a.warp, WarpMode::Off, "Off").on_hover_text("Plays at original speed").changed();
            });
            ui.horizontal(|ui| {
                ui.label("Source BPM");
                changed |= ui.add(egui::DragValue::new(&mut a.source_bpm).range(20.0..=999.0).speed(0.1)).changed();
            });
            if ui.small_button("Fit loop to 1/2/4/8 bars").on_hover_text("Sets the source tempo so the region spans whole bars at the project tempo").clicked() {
                let len_secs = (c.region.end - c.region.start) * 60.0 / a.source_bpm;
                let bars_beats = [4.0, 8.0, 16.0, 32.0];
                let best = bars_beats.iter().copied().min_by(|x, y| ((len_secs * bpm / 60.0) - x).abs().total_cmp(&((len_secs * bpm / 60.0) - y).abs())).unwrap();
                a.source_bpm = best * 60.0 / len_secs;
                c.region.start = 0.0;
                c.region.loop_start = 0.0;
                c.region.end = best;
                changed = true;
            }
            ui.label(egui::RichText::new("Warp markers (beat → source s)").color(theme::TEXT_DIM));
            let mut remove = None;
            for (i, m) in a.markers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut m.beat).speed(0.0625).range(0.0..=4096.0).max_decimals(3)).changed();
                    ui.label("→");
                    changed |= ui.add(egui::DragValue::new(&mut m.source_seconds).speed(0.001).range(0.0..=36_000.0).max_decimals(4).suffix(" s")).changed();
                    if ui.small_button("x").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                a.markers.remove(i);
                changed = true;
            }
            if changed {
                a.markers.sort_by(|x, y| x.beat.total_cmp(&y.beat));
            }
            ui.label(egui::RichText::new("Double-click the waveform to add a marker").size(9.5).color(theme::TEXT_DIM));
            ui.label(egui::RichText::new(&a.media.path).size(9.5).color(theme::TEXT_DIM));
        }
        ClipContent::Midi { notes } => {
            ui.label(egui::RichText::new(format!("{} notes", notes.len())).color(theme::TEXT_DIM));
            ui.horizontal(|ui| {
                ui.label("Transpose");
                changed |= ui.add(egui::DragValue::new(&mut c.transpose).range(-48..=48)).changed();
            });
        }
    }
    ui.horizontal(|ui| {
        ui.label("Gain dB");
        changed |= ui.add(egui::DragValue::new(&mut c.gain_db).range(-70.0..=24.0).speed(0.1)).changed();
    });
    ui.horizontal(|ui| {
        ui.label("Color");
        crate::session::color_menu(ui, &mut c.color);
    });
    if changed {
        app.label("Clip Properties");
    }
}

// ---------------------------------------------------------------- MIDI

fn notes_mut(app: &mut XdawApp) -> Option<&mut Vec<Note>> {
    match &mut app.selected_clip_mut()?.content {
        ClipContent::Midi { notes } => Some(notes),
        _ => None,
    }
}

fn clip_at(app: &XdawApp, loc: ClipLoc) -> Option<&Clip> {
    match loc {
        ClipLoc::Slot { track, slot } => app.project.tracks.get(track)?.slots.get(slot)?.clip.as_ref(),
        ClipLoc::Arrangement { track, id } => app.project.tracks.get(track)?.arrangement.iter().find(|c| c.id == id).map(|c| &c.clip),
    }
}

fn clip_at_mut(app: &mut XdawApp, loc: ClipLoc) -> Option<&mut Clip> {
    match loc {
        ClipLoc::Slot { track, slot } => app.project.tracks.get_mut(track)?.slots.get_mut(slot)?.clip.as_mut(),
        ClipLoc::Arrangement { track, id } => app.project.tracks.get_mut(track)?.arrangement.iter_mut().find(|c| c.id == id).map(|c| &mut c.clip),
    }
}

fn midi_notes_at_mut(app: &mut XdawApp, loc: ClipLoc) -> Option<&mut Vec<Note>> {
    match &mut clip_at_mut(app, loc)?.content {
        ClipContent::Midi { notes } => Some(notes),
        _ => None,
    }
}

/// The MIDI clips edited together: the open clip first, then (multi-clip mode)
/// the other selected Arrangement clips. Notes are numbered across all of them
/// (the open clip's keep their own numbers), so one selection can span clips.
pub struct NoteSpace {
    pub locs: Vec<ClipLoc>,
    map: Vec<(usize, usize)>,
}

impl NoteSpace {
    pub fn new(app: &XdawApp) -> Self {
        let is_midi = |l: ClipLoc| clip_at(app, l).is_some_and(Clip::is_midi);
        let mut locs: Vec<ClipLoc> = vec![];
        if let Some(l) = app.sel.clip.filter(|l| is_midi(*l)) {
            locs.push(l);
        }
        if app.piano.multi {
            for &(track, id) in &app.sel.arr_clips {
                let l = ClipLoc::Arrangement { track, id };
                if !locs.contains(&l) && is_midi(l) {
                    locs.push(l);
                }
            }
        }
        let mut map = vec![];
        for (ci, l) in locs.iter().enumerate() {
            if let Some(Clip { content: ClipContent::Midi { notes }, .. }) = clip_at(app, *l) {
                map.extend((0..notes.len()).map(|ni| (ci, ni)));
            }
        }
        Self { locs, map }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Every note with the colour of its clip.
    pub fn notes(&self, app: &XdawApp) -> Vec<(Note, [u8; 3])> {
        self.map
            .iter()
            .filter_map(|&(ci, ni)| {
                let c = clip_at(app, self.locs[ci])?;
                match &c.content {
                    ClipContent::Midi { notes } => notes.get(ni).map(|n| (*n, c.color)),
                    _ => None,
                }
            })
            .collect()
    }

    pub fn edit(&self, app: &mut XdawApp, gi: usize, f: impl FnOnce(&mut Note)) {
        if let Some(&(ci, ni)) = self.map.get(gi) {
            if let Some(n) = midi_notes_at_mut(app, self.locs[ci]).and_then(|ns| ns.get_mut(ni)) {
                f(n);
            }
        }
    }

    pub fn remove(&self, app: &mut XdawApp, gis: &[usize]) {
        let mut at: Vec<(usize, usize)> = gis.iter().filter_map(|&g| self.map.get(g).copied()).collect();
        at.sort_unstable();
        at.dedup();
        for (ci, ni) in at.into_iter().rev() {
            if let Some(ns) = midi_notes_at_mut(app, self.locs[ci]) {
                if ni < ns.len() {
                    ns.remove(ni);
                }
            }
        }
    }
}

pub fn select_all(app: &mut XdawApp) {
    app.sel.notes = (0..NoteSpace::new(app).len()).collect();
}

pub fn delete_selected_notes(app: &mut XdawApp) {
    let sel = std::mem::take(&mut app.sel.notes);
    NoteSpace::new(app).remove(app, &sel);
    app.label("Delete Notes");
}

pub fn copy_notes(app: &mut XdawApp) {
    let all = NoteSpace::new(app).notes(app);
    let mut copied: Vec<Note> = app.sel.notes.iter().filter_map(|&i| all.get(i).map(|n| n.0)).collect();
    if copied.is_empty() {
        return;
    }
    let first = copied.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
    for n in copied.iter_mut() {
        n.start -= first;
    }
    app.clipboard.notes = copied;
}

pub fn paste_notes(app: &mut XdawApp) {
    let clip_beat = app.piano.view.scroll_beats.max(0.0);
    let pasted = app.clipboard.notes.clone();
    let at = {
        let Some(ClipContent::Midi { notes }) = app.selected_clip().map(|c| &c.content) else { return };
        // Paste after the current selection (the open clip's notes), or at the visible start.
        app.sel.notes.iter().filter_map(|&i| notes.get(i)).map(|n| n.start + n.length).fold(clip_beat, f64::max)
    };
    let Some(notes) = notes_mut(app) else { return };
    let base = notes.len();
    for mut n in pasted.iter().copied() {
        n.start += at;
        notes.push(n);
    }
    app.sel.notes = (base..base + pasted.len()).collect();
    app.label("Paste Notes");
}

/// Applies `f` to the selected notes of every edited clip (all notes when none is selected).
fn edit_selected(app: &mut XdawApp, f: impl Fn(&mut Note)) {
    let space = NoteSpace::new(app);
    let sel = app.sel.notes.clone();
    for gi in 0..space.len() {
        if sel.is_empty() || sel.contains(&gi) {
            space.edit(app, gi, &f);
        }
    }
}

pub fn quantize_notes(app: &mut XdawApp) {
    let grid = app.piano.grid;
    edit_selected(app, |n| n.start = (n.start / grid).round() * grid);
    app.label("Quantize");
}

pub fn transpose_notes(app: &mut XdawApp, semis: i32) {
    edit_selected(app, |n| n.pitch = (n.pitch as i32 + semis).clamp(0, 127) as u8);
    app.label("Transpose");
}

fn duplicate_loop(app: &mut XdawApp) {
    for loc in NoteSpace::new(app).locs {
        let Some(c) = clip_at_mut(app, loc) else { continue };
        let len = c.region.end - c.region.loop_start;
        let (ls, end) = (c.region.loop_start, c.region.end);
        if let ClipContent::Midi { notes } = &mut c.content {
            let copies: Vec<Note> = notes.iter().filter(|n| n.start >= ls && n.start < end).map(|n| Note { start: n.start + len, ..*n }).collect();
            notes.extend(copies);
        }
        c.region.end += len;
    }
    app.label("Duplicate Loop");
}

pub fn export_midi(app: &mut XdawApp) {
    let Some(ClipContent::Midi { notes }) = app.selected_clip().map(|c| c.content.clone()) else {
        app.status_line = "Select a MIDI clip to export".into();
        return;
    };
    let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid"]).set_file_name("clip.mid").save_file() else { return };
    let fnotes: Vec<_> = notes.iter().map(|n| xdaw_midi::file::FileNote { pitch: n.pitch, start: n.start, length: n.length, velocity: n.velocity }).collect();
    let bytes = xdaw_midi::file::export(&fnotes, app.project.transport.bpm);
    app.status_line = match io::atomic_write(&path, &bytes) {
        Ok(()) => format!("Exported {}", path.display()),
        Err(e) => format!("MIDI export failed: {e}"),
    };
}

fn piano_roll(app: &mut XdawApp, ui: &mut Ui, loc: ClipLoc) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label("Grid");
            egui::ComboBox::from_id_salt("pgrid").width(56.0).selected_text(grid_label(app.piano.grid)).show_ui(ui, |ui| {
                for g in [1.0, 0.5, 0.25, 0.125, 1.0 / 3.0, 1.0 / 6.0] {
                    ui.selectable_value(&mut app.piano.grid, g, grid_label(g));
                }
            });
            if ui.small_button("Quantize").on_hover_text("Selected notes (or all) to the grid").clicked() {
                quantize_notes(app);
            }
            if ui.small_button("−12").clicked() {
                transpose_notes(app, -12);
            }
            if ui.small_button("−1").clicked() {
                transpose_notes(app, -1);
            }
            if ui.small_button("+1").clicked() {
                transpose_notes(app, 1);
            }
            if ui.small_button("+12").clicked() {
                transpose_notes(app, 12);
            }
            if ui.small_button("×2 loop").on_hover_text("Duplicate the loop and its notes").clicked() {
                duplicate_loop(app);
            }
            ui.separator();
            ui.label("Lane");
            ui.selectable_value(&mut app.piano.lane, NoteLane::Velocity, "Velocity");
            ui.selectable_value(&mut app.piano.lane, NoteLane::Chance, "Chance").on_hover_text("Probability that each note plays on every pass (percent)");
            let n_clips = NoteSpace::new(app).locs.len();
            let selected_midi = app.sel.arr_clips.iter().filter(|&&(t, id)| clip_at(app, ClipLoc::Arrangement { track: t, id }).is_some_and(Clip::is_midi)).count();
            if selected_midi > 1 {
                ui.checkbox(&mut app.piano.multi, format!("Multi-clip ({n_clips})")).on_hover_text("Edit all selected Arrangement MIDI clips together (Shift-click clips to select several)");
            }
            ui.separator();
            if ui.small_button("−").on_hover_text("Zoom out (−)").clicked() {
                let w = app.piano.width;
                app.piano.view.zoom_at(0.8, w * 0.5);
            }
            if ui.small_button("+").on_hover_text("Zoom in (+)").clicked() {
                let w = app.piano.width;
                app.piano.view.zoom_at(1.25, w * 0.5);
            }
            if ui.small_button("Fit").on_hover_text("Show the whole clip").clicked() {
                if let Some(c) = app.selected_clip() {
                    let end = c.region.end.max(1.0);
                    let w = app.piano.width;
                    app.piano.view.fit(0.0, end, w);
                }
            }
            ui.label(egui::RichText::new("double-click: add/delete · drag: move · edge: resize · shift: fine/extend").size(10.0).color(theme::TEXT_DIM));
        });
        let avail = ui.available_size() - vec2(0.0, timeline::SCROLLBAR_H + 2.0);
        let (rect, resp) = ui.allocate_exact_size(avail.max(vec2(100.0, 80.0)), Sense::click_and_drag());
        let vel_h = 44.0;
        let keys = Rect::from_min_size(rect.min, vec2(40.0, rect.height() - vel_h));
        let grid = Rect::from_min_max(pos2(keys.right(), rect.top()), pos2(rect.right(), rect.bottom() - vel_h));
        let vel = Rect::from_min_max(pos2(keys.right(), grid.bottom()), rect.max);
        app.piano.width = grid.width();
        roll(app, ui, &resp, keys, grid, vel, loc);
        let content = app
            .selected_clip()
            .map(|c| match &c.content {
                ClipContent::Midi { notes } => notes.iter().map(|n| n.start + n.length).fold(c.region.end, f64::max),
                _ => c.region.end,
            })
            .unwrap_or(4.0)
            + 4.0;
        let w = grid.width();
        timeline::scrollbar_row(ui, "piano", &mut app.piano.view, grid.left(), w, content);
    });
}

fn grid_label(g: f64) -> String {
    match g {
        x if (x - 1.0).abs() < 1e-9 => "1/4".into(),
        x if (x - 0.5).abs() < 1e-9 => "1/8".into(),
        x if (x - 0.25).abs() < 1e-9 => "1/16".into(),
        x if (x - 0.125).abs() < 1e-9 => "1/32".into(),
        x if (x - 1.0 / 3.0).abs() < 1e-9 => "1/8T".into(),
        _ => "1/16T".into(),
    }
}

fn is_black(p: i32) -> bool {
    matches!(p.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

fn roll(app: &mut XdawApp, ui: &mut Ui, resp: &egui::Response, keys: Rect, grid: Rect, vel: Rect, loc: ClipLoc) {
    let st = &mut app.piano;
    // Scroll/zoom.
    // Pinch/Cmd+wheel zoom, horizontal wheel scrolls time, vertical wheel scrolls pitch.
    let dy = st.view.wheel(ui, resp.hovered(), grid);
    if dy != 0.0 {
        st.top_pitch = (st.top_pitch as f32 + dy / st.key_h).round().clamp(12.0, 127.0) as i32;
    }
    let (ppb, key_h, top, sb, gsize) = (st.view.px_per_beat, st.key_h, st.top_pitch, st.view.scroll_beats, st.grid);
    let x_of = |b: f64| grid.left() + ((b - sb) as f32) * ppb;
    let beat_of = |x: f32| sb + ((x - grid.left()) / ppb) as f64;
    let y_of = |pitch: i32| grid.top() + (top - pitch) as f32 * key_h;
    let pitch_of = |y: f32| top - ((y - grid.top()) / key_h).floor() as i32;
    let p = ui.painter_at(keys.union(grid).union(vel));
    // Rows.
    let mut pitch = top;
    while y_of(pitch) < grid.bottom() && pitch >= 0 {
        let y = y_of(pitch);
        let row = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + key_h));
        p.rect_filled(row, 0.0, if is_black(pitch) { theme::dim(theme::BG, 0.85) } else { theme::BG });
        let key = Rect::from_min_max(pos2(keys.left(), y), pos2(keys.right(), y + key_h));
        p.rect_filled(key.shrink2(vec2(0.0, 0.5)), 0.0, if is_black(pitch) { Color32::from_gray(40) } else { Color32::from_gray(200) });
        if pitch.rem_euclid(12) == 0 {
            p.text(key.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, xdaw_midi::note_name(pitch as u8), FontId::proportional(8.5), Color32::BLACK);
            p.line_segment([pos2(grid.left(), y + key_h), pos2(grid.right(), y + key_h)], Stroke::new(1.0, theme::GRID));
        }
        pitch -= 1;
    }
    let Some(clip) = app.selected_clip().cloned() else { return };
    if !clip.is_midi() {
        return;
    }
    let space = NoteSpace::new(app);
    let gnotes = space.notes(app);
    let notes: Vec<Note> = gnotes.iter().map(|g| g.0).collect();
    let lane = app.piano.lane;
    // Loop region shading and beat lines.
    let r = clip.region;
    p.rect_filled(Rect::from_min_max(pos2(x_of(r.end), grid.top()), grid.max), 0.0, Color32::from_black_alpha(90));
    p.rect_filled(Rect::from_min_max(pos2(grid.left(), grid.top()), pos2(x_of(r.loop_start).max(grid.left()), grid.bottom())), 0.0, Color32::from_black_alpha(40));
    let mut b = (sb / gsize).floor() * gsize;
    while x_of(b) < grid.right() {
        let x = x_of(b);
        let strong = (b % 4.0).abs() < 1e-6;
        let beat = (b % 1.0).abs() < 1e-6;
        let c = if strong { theme::TEXT_DIM } else if beat { theme::GRID } else { theme::dim(theme::GRID, 0.6) };
        p.line_segment([pos2(x, grid.top()), pos2(x, vel.bottom())], Stroke::new(1.0, c));
        b += gsize;
    }
    // Playhead within the clip (Session clips).
    if let ClipLoc::Slot { track, slot } = loc {
        let st = app.track_status(track);
        if st.playing_slot.load(std::sync::atomic::Ordering::Relaxed) == slot as i32 {
            let x = x_of(st.clip_position.load());
            p.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.5, theme::ACCENT));
        }
    }
    // Notes.
    let mut hovered: Option<(usize, bool)> = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    for (i, n) in notes.iter().enumerate() {
        let color = theme::rgb(gnotes[i].1);
        let rect = Rect::from_min_max(pos2(x_of(n.start), y_of(n.pitch as i32) + 0.5), pos2(x_of(n.start + n.length), y_of(n.pitch as i32) + key_h - 0.5));
        let selected = app.sel.notes.contains(&i);
        let shade = 0.45 + 0.55 * n.velocity as f32 / 127.0;
        p.rect_filled(rect, 1.5, if selected { theme::SELECT } else { theme::dim(color, shade * 1.3) });
        p.rect_stroke(rect, 1.5, Stroke::new(1.0, Color32::from_black_alpha(140)), StrokeKind::Inside);
        let vx = rect.left();
        let frac = match lane {
            NoteLane::Velocity => n.velocity as f32 / 127.0,
            NoteLane::Chance => n.chance as f32 / 100.0,
        };
        let vh = frac * (vel.height() - 4.0);
        p.line_segment([pos2(vx + 1.0, vel.bottom()), pos2(vx + 1.0, vel.bottom() - vh)], Stroke::new(3.0, if selected { theme::SELECT } else { color }));
        if let Some(q) = pointer {
            if rect.contains(q) {
                hovered = Some((i, q.x > rect.right() - 5.0));
            }
        }
    }
    p.line_segment([pos2(grid.left(), vel.top()), pos2(grid.right(), vel.top())], Stroke::new(1.0, theme::GRID));
    if hovered.is_some_and(|h| h.1) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    let shift = ui.input(|i| i.modifiers.shift);
    let snap = |b: f64| if shift { b } else { (b / gsize).floor() * gsize };
    let track_index = match loc {
        ClipLoc::Slot { track, .. } | ClipLoc::Arrangement { track, .. } => track,
    };

    // Key preview.
    if let Some(q) = pointer {
        if keys.contains(q) && resp.is_pointer_button_down_on() && app.piano.preview.is_none() {
            let pitch = pitch_of(q.y).clamp(0, 127) as u8;
            app.piano.preview = Some(pitch);
            app.rt.send(Command::Note { track: track_index as u16, kind: xdaw_dsp::NoteKind::On { pitch, velocity: 100 } });
        }
    }
    if !resp.is_pointer_button_down_on() {
        if let Some(pitch) = app.piano.preview.take() {
            app.rt.send(Command::Note { track: track_index as u16, kind: xdaw_dsp::NoteKind::Off { pitch } });
        }
    }

    // Drag start.
    if resp.drag_started() {
        if let Some(q) = pointer {
            if vel.contains(q) {
                let origin = app.sel.notes.iter().filter_map(|&i| notes.get(i).map(|n| (i, if lane == NoteLane::Chance { n.chance } else { n.velocity }))).collect();
                app.piano.drag = Some(NoteDrag::Velocity { origin, start_y: q.y });
            } else if grid.contains(q) {
                match hovered {
                    Some((i, edge)) => {
                        if !app.sel.notes.contains(&i) {
                            if shift { app.sel.notes.push(i) } else { app.sel.notes = vec![i] }
                        }
                        let b = beat_of(q.x);
                        app.piano.drag = Some(if edge {
                            NoteDrag::Resize { origin: app.sel.notes.iter().filter_map(|&j| notes.get(j).map(|n| (j, n.length))).collect(), start_beat: b }
                        } else {
                            NoteDrag::Move {
                                origin: app.sel.notes.iter().filter_map(|&j| notes.get(j).map(|n| (j, n.start, n.pitch))).collect(),
                                start_beat: b,
                                start_pitch: pitch_of(q.y),
                            }
                        });
                    }
                    None => app.piano.drag = Some(NoteDrag::Select { from: q }),
                }
            }
        }
    }
    if resp.dragged() {
        if let (Some(drag), Some(q)) = (app.piano.drag.clone(), pointer) {
            match drag {
                NoteDrag::Move { origin, start_beat, start_pitch } => {
                    let db = beat_of(q.x) - start_beat;
                    let db = if shift { db } else { (db / gsize).round() * gsize };
                    let dp = pitch_of(q.y) - start_pitch;
                    for (i, s, pch) in origin {
                        space.edit(app, i, |n| {
                            n.start = (s + db).max(0.0);
                            n.pitch = (pch as i32 + dp).clamp(0, 127) as u8;
                        });
                    }
                    app.label("Move Notes");
                }
                NoteDrag::Resize { origin, start_beat } => {
                    let db = beat_of(q.x) - start_beat;
                    let db = if shift { db } else { (db / gsize).round() * gsize };
                    let mut last = None;
                    for (i, l) in origin {
                        let new = (l + db).max(1.0 / 64.0);
                        space.edit(app, i, |n| n.length = new);
                        last = Some(new);
                    }
                    if let Some(l) = last {
                        app.piano.last_length = l;
                    }
                    app.label("Resize Notes");
                }
                NoteDrag::Velocity { origin, start_y } => {
                    let span = if lane == NoteLane::Chance { 100.0 } else { 127.0 };
                    let dv = ((start_y - q.y) / (vel.height().max(1.0)) * span) as i32;
                    for (i, v) in origin {
                        let new = (v as i32 + dv).clamp(1, span as i32) as u8;
                        space.edit(app, i, |n| if lane == NoteLane::Chance { n.chance = new } else { n.velocity = new });
                    }
                    app.label(if lane == NoteLane::Chance { "Note Chance" } else { "Velocity" });
                }
                NoteDrag::Select { from } => {
                    let sel_rect = Rect::from_two_pos(from, q);
                    ui.painter().rect_stroke(sel_rect, 0.0, Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
                    let (b0, b1) = (beat_of(sel_rect.left()), beat_of(sel_rect.right()));
                    let (p_hi, p_lo) = (pitch_of(sel_rect.top()), pitch_of(sel_rect.bottom()));
                    app.sel.notes = notes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| n.start < b1 && n.start + n.length > b0 && (n.pitch as i32) <= p_hi && (n.pitch as i32) >= p_lo)
                        .map(|(i, _)| i)
                        .collect();
                }
            }
        }
    }
    if resp.drag_stopped() {
        app.piano.drag = None;
    }
    if resp.clicked() {
        match hovered {
            Some((i, _)) => {
                if shift {
                    if let Some(k) = app.sel.notes.iter().position(|&x| x == i) {
                        app.sel.notes.remove(k);
                    } else {
                        app.sel.notes.push(i);
                    }
                } else {
                    app.sel.notes = vec![i];
                }
            }
            None => app.sel.notes.clear(),
        }
    }
    if resp.double_clicked() {
        if let Some(q) = pointer.filter(|q| grid.contains(*q)) {
            match hovered {
                Some((i, _)) => {
                    space.remove(app, &[i]);
                    app.sel.notes.clear();
                    app.label("Delete Note");
                }
                None => {
                    let start = snap(beat_of(q.x)).max(0.0);
                    let pitch = pitch_of(q.y).clamp(0, 127) as u8;
                    let length = app.piano.last_length.max(gsize);
                    if let Some(ns) = notes_mut(app) {
                        ns.push(Note { pitch, start, length, velocity: 100, chance: 100 });
                        let idx = ns.len() - 1;
                        app.sel.notes = vec![idx];
                    }
                    app.label("Add Note");
                }
            }
        }
    }
    // Arrow keys on selection.
    if resp.hovered() && !app.sel.notes.is_empty() {
        let (up, down, left, right, big) = ui.input(|i| {
            (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.modifiers.shift)
        });
        let semis = if up { if big { 12 } else { 1 } } else if down { if big { -12 } else { -1 } } else { 0 };
        if semis != 0 {
            transpose_notes(app, semis);
        }
        let shift_b = if right { gsize } else if left { -gsize } else { 0.0 };
        if shift_b != 0.0 {
            let sel = app.sel.notes.clone();
            for i in sel {
                space.edit(app, i, |n| n.start = (n.start + shift_b).max(0.0));
            }
            app.label("Move Notes");
        }
    }
}

// ---------------------------------------------------------------- audio

fn audio_view(app: &mut XdawApp, ui: &mut Ui) {
    let Some(clip) = app.selected_clip().cloned() else { return };
    let ClipContent::Audio(a) = &clip.content else { return };
    ui.vertical(|ui| {
        app.wave.total = 0.0;
        audio_view_inner(app, ui, &clip, a);
        if app.wave.total > 0.0 {
            let (left, width, total) = (app.wave.left, app.wave.width, app.wave.total);
            timeline::scrollbar_row(ui, "wave", &mut app.wave.view, left, width, total);
        }
    });
}

fn audio_view_inner(app: &mut XdawApp, ui: &mut Ui, clip: &Clip, a: &AudioContent) {
    ui.horizontal(|ui| {
        if ui.small_button("−").on_hover_text("Zoom out (−)").clicked() {
            let w = app.wave.width;
            app.wave.view.zoom_at(0.8, w * 0.5);
        }
        if ui.small_button("+").on_hover_text("Zoom in (+)").clicked() {
            let w = app.wave.width;
            app.wave.view.zoom_at(1.25, w * 0.5);
        }
        if ui.small_button("Fit").on_hover_text("Show the whole sample").clicked() {
            app.wave.shown = None;
        }
        ui.separator();
        if ui.small_button("Reverse").on_hover_text("Write a reversed copy of the sample and use it").clicked() {
            crate::audio_ops::reverse_selection(app);
        }
        ui.menu_button("Slice to MIDI", |ui| {
            for (mode, label) in crate::audio_ops::SliceMode::ALL {
                if ui.button(label).clicked() {
                    crate::audio_ops::slice_to_midi(app, mode);
                    ui.close();
                }
            }
        });
        ui.separator();
        ui.label("Envelope");
        ui.selectable_value(&mut app.wave.env, None, "Off");
        for t in ClipEnvTarget::ALL {
            ui.selectable_value(&mut app.wave.env, Some(t), t.name());
        }
        if app.wave.env.is_some() {
            for t in crate::arrangement::Tool::ALL {
                ui.selectable_value(&mut app.arr.tool, t, t.label());
            }
        } else {
            ui.label(egui::RichText::new("pinch/Cmd+wheel: zoom · double-click: warp marker · drag: S/L/E").size(10.0).color(theme::TEXT_DIM));
        }
    });
    let avail = ui.available_size() - vec2(0.0, timeline::SCROLLBAR_H + 2.0);
    let (rect, resp) = ui.allocate_exact_size(avail.max(vec2(100.0, 40.0)), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::PANEL_DARK);
    let (Some(ov), Some(data)) = (app.media.overviews.get(&a.media.path).cloned(), app.media.loaded.get(&a.media.path).cloned()) else {
        let msg = match app.media.failed.get(&a.media.path) {
            Some(e) => format!("Media unavailable: {e}"),
            None => "Loading…".into(),
        };
        p.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(12.0), theme::RECORD);
        return;
    };
    let total_beats = (data.duration_seconds() * a.source_bpm / 60.0).max(1e-6);
    app.wave.width = rect.width();
    app.wave.left = rect.left();
    app.wave.total = total_beats;
    if app.wave.shown != Some(clip.id) {
        app.wave.shown = Some(clip.id);
        app.wave.view.fit(0.0, total_beats, rect.width());
    }
    app.wave.view.wheel(ui, resp.hovered(), rect);
    let view = app.wave.view;
    let x_of = |b: f64| view.x(rect, b);
    let beat_of = |x: f32| view.beat(rect, x);
    let mid = rect.center().y;
    let h = rect.height() * 0.45;
    let color = theme::rgb(clip.color);
    // Overview bins cover the whole file; each pixel takes the min/max of its bins.
    let bins = ov.len().max(1) as f64;
    for px in 0..rect.width() as usize {
        let b0 = beat_of(rect.left() + px as f32);
        let b1 = beat_of(rect.left() + px as f32 + 1.0);
        if b1 <= 0.0 || b0 >= total_beats {
            continue;
        }
        let i0 = ((b0 / total_beats) * bins).floor().max(0.0) as usize;
        let i1 = (((b1 / total_beats) * bins).ceil() as usize).clamp(i0 + 1, ov.len());
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &(l, h) in &ov[i0.min(ov.len().saturating_sub(1))..i1] {
            lo = lo.min(l);
            hi = hi.max(h);
        }
        if lo <= hi {
            let x = rect.left() + px as f32;
            p.line_segment([pos2(x, mid - hi * h), pos2(x, mid - lo * h)], Stroke::new(1.0, color));
        }
    }
    // Beat grid (bars brighter).
    let step = if view.px_per_beat > 24.0 { 1.0 } else if view.px_per_beat > 6.0 { 4.0 } else { 16.0 };
    let mut gb = (view.scroll_beats / step).floor() * step;
    while x_of(gb) < rect.right() {
        let x = x_of(gb);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.top() + 6.0)], Stroke::new(1.0, theme::GRID));
        gb += step;
    }
    let r = clip.region;
    p.rect_filled(Rect::from_min_max(pos2(x_of(r.end), rect.top()), rect.max), 0.0, Color32::from_black_alpha(120));
    p.rect_filled(Rect::from_min_max(rect.min, pos2(x_of(r.start), rect.bottom())), 0.0, Color32::from_black_alpha(120));
    for (b, label, c) in [(r.start, "S", theme::TEXT), (r.loop_start, "L", theme::QUEUED), (r.end, "E", theme::TEXT)] {
        let x = x_of(b);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, c));
        p.text(pos2(x + 2.0, rect.top() + 2.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), c);
    }
    // Warp markers at their source positions.
    let secs_to_x = |s: f64| x_of(s * a.source_bpm / 60.0);
    for m in &a.markers {
        let x = secs_to_x(m.source_seconds);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, theme::ACCENT));
        p.text(pos2(x + 2.0, rect.bottom() - 12.0), Align2::LEFT_TOP, format!("{:.2}", m.beat), FontId::proportional(9.5), theme::ACCENT);
    }
    if let Some(target) = app.wave.env {
        envelope_edit(app, ui, &p, &resp, rect, view, target, a);
        return;
    }
    if resp.double_clicked() {
        if let Some(q) = resp.interact_pointer_pos() {
            let secs = beat_of(q.x) * 60.0 / a.source_bpm;
            // Pin this source position to the beat it currently plays at (snapped to 1/16).
            let pairs = a.marker_pairs();
            let mut beat = secs * a.source_bpm / 60.0;
            for _ in 0..40 {
                let err = xdaw_sequencer::warp::source_seconds(&pairs, a.source_bpm, beat) - secs;
                beat -= err * a.source_bpm / 60.0;
            }
            let beat = (beat * 4.0).round() / 4.0;
            if let Some(Clip { content: ClipContent::Audio(a), .. }) = app.selected_clip_mut() {
                a.markers.push(WarpMarker { beat, source_seconds: secs });
                a.markers.sort_by(|x, y| x.beat.total_cmp(&y.beat));
            }
            app.label("Add Warp Marker");
            return;
        }
    }
    // Drag the nearest marker.
    let id = egui::Id::new("audio_marker");
    if resp.drag_started() {
        if let Some(q) = resp.interact_pointer_pos() {
            let d = |b: f64| (x_of(b) - q.x).abs();
            let which = [(0u8, d(r.start)), (1, d(r.loop_start)), (2, d(r.end))].into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0).unwrap_or(2);
            ui.data_mut(|m| m.insert_temp(id, which));
        }
    }
    if resp.dragged() {
        if let (Some(q), Some(which)) = (resp.interact_pointer_pos(), ui.data(|m| m.get_temp::<u8>(id))) {
            let shift = ui.input(|i| i.modifiers.shift);
            let mut b = beat_of(q.x).clamp(0.0, total_beats);
            if !shift {
                b = (b * 4.0).round() / 4.0;
            }
            if let Some(c) = app.selected_clip_mut() {
                match which {
                    0 => c.region.start = b.min(c.region.end - 0.0625),
                    1 => c.region.loop_start = b.min(c.region.end - 0.0625),
                    _ => c.region.end = b.max(c.region.loop_start.max(c.region.start) + 0.0625),
                }
            }
            app.label("Clip Region");
        }
    }
}

/// Runs `f` on the selected audio clip's envelope for `target` (created on demand).
fn with_envelope(app: &mut XdawApp, target: ClipEnvTarget, f: impl FnOnce(&mut ClipEnvelope)) {
    if let Some(Clip { content: ClipContent::Audio(a), .. }) = app.selected_clip_mut() {
        let i = match a.envelopes.iter().position(|e| e.target == target) {
            Some(i) => i,
            None => {
                a.envelopes.push(ClipEnvelope::new(target));
                a.envelopes.len() - 1
            }
        };
        f(&mut a.envelopes[i]);
        if a.envelopes[i].points.is_empty() {
            a.envelopes.remove(i);
        }
    }
}

/// Edits a clip envelope over the waveform with the Arrangement's drawing tools.
#[allow(clippy::too_many_arguments)]
fn envelope_edit(app: &mut XdawApp, ui: &mut Ui, p: &egui::Painter, resp: &egui::Response, rect: Rect, view: HView, target: ClipEnvTarget, a: &AudioContent) {
    use crate::arrangement::Tool;
    let (lo, hi, neutral) = target.range();
    let y_of = |v: f32| rect.bottom() - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * rect.height();
    let v_of = |y: f32| lo + (1.0 - (y - rect.top()) / rect.height()).clamp(0.0, 1.0) * (hi - lo);
    let pts: Vec<AutomationPoint> = a.envelopes.iter().find(|e| e.target == target).map(|e| e.points.clone()).unwrap_or_default();
    let stroke = Stroke::new(1.5, Color32::from_rgb(255, 120, 120));
    p.line_segment([pos2(rect.left(), y_of(neutral)), pos2(rect.right(), y_of(neutral))], Stroke::new(1.0, Color32::from_white_alpha(40)));
    let pos: Vec<Pos2> = pts.iter().map(|q| pos2(view.x(rect, q.beat), y_of(q.value))).collect();
    if let (Some(f), Some(l)) = (pos.first(), pos.last()) {
        p.line_segment([pos2(rect.left(), f.y), *f], stroke);
        for w in pos.windows(2) {
            p.line_segment([w[0], w[1]], stroke);
        }
        p.line_segment([*l, pos2(rect.right(), l.y)], stroke);
    }
    let hover = resp.hover_pos();
    let near = hover.and_then(|h| pos.iter().position(|q| q.distance(h) < 7.0));
    for (i, q) in pos.iter().enumerate() {
        p.circle_filled(*q, if near == Some(i) { 4.5 } else { 3.0 }, Color32::from_rgb(255, 150, 150));
    }
    p.text(rect.right_top() + vec2(-4.0, 2.0), Align2::RIGHT_TOP, format!("{} · {} tool", target.name(), app.arr.tool.label()), FontId::proportional(9.5), theme::TEXT_DIM);
    let step = match app.arr.grid_step() {
        s if s > 0.0 => s,
        _ => 1.0 / 16.0,
    };
    let beat_at = |x: f32| view.beat(rect, x).max(0.0);
    let id = egui::Id::new(("env_drag", target));
    let tool = app.arr.tool;
    if resp.drag_started() {
        if let Some(q) = resp.interact_pointer_pos() {
            let idx = pos.iter().position(|c| c.distance(q) < 7.0);
            let b = beat_at(q.x);
            ui.data_mut(|m| m.insert_temp(id, (idx, b, v_of(q.y), b, pts.clone())));
        }
    }
    if resp.dragged() {
        if let (Some(q), Some((idx, anchor, anchor_val, last, before))) = (resp.interact_pointer_pos(), ui.data(|m| m.get_temp::<(Option<usize>, f64, f32, f64, Vec<AutomationPoint>)>(id))) {
            let (b, v) = (beat_at(q.x), v_of(q.y));
            match (tool, idx) {
                (Tool::Select, Some(i)) => with_envelope(app, target, |e| {
                    if let Some(pt) = e.points.get_mut(i) {
                        pt.beat = b;
                        pt.value = v;
                    }
                    e.points.sort_by(|x, y| x.beat.total_cmp(&y.beat));
                }),
                (Tool::Select, None) => {}
                (Tool::Draw, _) => with_envelope(app, target, |e| {
                    let (c0, c1) = ((last.min(b) / step).floor() as i64, (last.max(b) / step).floor() as i64);
                    for c in c0..=c1 {
                        e.draw_cell(c as f64 * step, (c + 1) as f64 * step, v);
                    }
                }),
                (Tool::Line, _) => with_envelope(app, target, |e| {
                    e.points = before.clone();
                    e.draw_line(anchor, anchor_val, b, v, step);
                }),
                (Tool::Erase, _) => {
                    let slack = 3.0 / view.px_per_beat as f64;
                    with_envelope(app, target, |e| e.erase_range(last.min(b) - slack, last.max(b) + slack));
                }
            }
            ui.data_mut(|m| m.insert_temp(id, (idx, anchor, anchor_val, b, before)));
            app.label("Edit Clip Envelope");
        }
    }
    if tool == Tool::Select && resp.double_clicked() {
        if let Some(q) = resp.interact_pointer_pos() {
            match pos.iter().position(|c| c.distance(q) < 7.0) {
                Some(i) => with_envelope(app, target, |e| {
                    e.points.remove(i);
                }),
                None => {
                    let b = (beat_at(q.x) / step).round() * step;
                    with_envelope(app, target, |e| insert_point(&mut e.points, AutomationPoint { beat: b, value: v_of(q.y) }));
                }
            }
            app.label("Edit Clip Envelope");
        }
    }
}
