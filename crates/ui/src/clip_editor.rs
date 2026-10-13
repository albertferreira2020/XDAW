//! Clip View: clip properties, MIDI note editor and audio clip display.

use crate::app::{ClipLoc, XdawApp};
use crate::theme;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use xdaw_engine::protocol::Command;
use xdaw_project::*;
use xdaw_sequencer::{ClipQuantize, LaunchMode, Quantize};

pub struct PianoState {
    pub key_h: f32,
    pub top_pitch: i32,
    pub px_per_beat: f32,
    pub scroll_beats: f64,
    pub grid: f64,
    pub last_length: f64,
    drag: Option<NoteDrag>,
    preview: Option<u8>,
}

impl Default for PianoState {
    fn default() -> Self {
        Self { key_h: 9.0, top_pitch: 84, px_per_beat: 60.0, scroll_beats: 0.0, grid: 0.25, last_length: 0.25, drag: None, preview: None }
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
            }).show_ui(ui, |ui| {
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

pub fn select_all(app: &mut XdawApp) {
    let n = app.selected_clip().map(|c| if let ClipContent::Midi { notes } = &c.content { notes.len() } else { 0 }).unwrap_or(0);
    app.sel.notes = (0..n).collect();
}

pub fn delete_selected_notes(app: &mut XdawApp) {
    let mut sel = std::mem::take(&mut app.sel.notes);
    sel.sort_unstable();
    sel.dedup();
    if let Some(notes) = notes_mut(app) {
        for i in sel.into_iter().rev() {
            if i < notes.len() {
                notes.remove(i);
            }
        }
    }
    app.label("Delete Notes");
}

pub fn copy_notes(app: &mut XdawApp) {
    let Some(ClipContent::Midi { notes }) = app.selected_clip().map(|c| &c.content) else { return };
    let mut copied: Vec<Note> = app.sel.notes.iter().filter_map(|&i| notes.get(i).copied()).collect();
    let first = copied.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
    for n in copied.iter_mut() {
        n.start -= first;
    }
    app.clipboard.notes = copied;
}

pub fn paste_notes(app: &mut XdawApp) {
    let clip_beat = app.piano.scroll_beats.max(0.0);
    let pasted = app.clipboard.notes.clone();
    let at = {
        let Some(ClipContent::Midi { notes }) = app.selected_clip().map(|c| &c.content) else { return };
        // Paste after the current selection, or at the visible start.
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

fn quantize_notes(app: &mut XdawApp) {
    let grid = app.piano.grid;
    let sel = app.sel.notes.clone();
    let Some(notes) = notes_mut(app) else { return };
    let all = sel.is_empty();
    for (i, n) in notes.iter_mut().enumerate() {
        if all || sel.contains(&i) {
            n.start = (n.start / grid).round() * grid;
        }
    }
    app.label("Quantize");
}

fn transpose_notes(app: &mut XdawApp, semis: i32) {
    let sel = app.sel.notes.clone();
    let Some(notes) = notes_mut(app) else { return };
    let all = sel.is_empty();
    for (i, n) in notes.iter_mut().enumerate() {
        if all || sel.contains(&i) {
            n.pitch = (n.pitch as i32 + semis).clamp(0, 127) as u8;
        }
    }
    app.label("Transpose");
}

fn duplicate_loop(app: &mut XdawApp) {
    let Some(c) = app.selected_clip_mut() else { return };
    let len = c.region.end - c.region.loop_start;
    let (ls, end) = (c.region.loop_start, c.region.end);
    if let ClipContent::Midi { notes } = &mut c.content {
        let copies: Vec<Note> = notes.iter().filter(|n| n.start >= ls && n.start < end).map(|n| Note { start: n.start + len, ..*n }).collect();
        notes.extend(copies);
    }
    c.region.end += len;
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
            if ui.small_button("Fit").clicked() {
                if let Some(c) = app.selected_clip() {
                    let w = ui.available_width().max(200.0) - 40.0;
                    app.piano.px_per_beat = (w / c.region.end.max(1.0) as f32).clamp(8.0, 400.0);
                    app.piano.scroll_beats = 0.0;
                }
            }
            ui.label(egui::RichText::new("double-click: add/delete · drag: move · edge: resize · shift: fine/extend").size(10.0).color(theme::TEXT_DIM));
        });
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let vel_h = 44.0;
        let keys = Rect::from_min_size(rect.min, vec2(40.0, rect.height() - vel_h));
        let grid = Rect::from_min_max(pos2(keys.right(), rect.top()), pos2(rect.right(), rect.bottom() - vel_h));
        let vel = Rect::from_min_max(pos2(keys.right(), grid.bottom()), rect.max);
        roll(app, ui, &resp, keys, grid, vel, loc);
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
    if resp.hovered() {
        let (scroll, mods) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers));
        if mods.command {
            st.px_per_beat = (st.px_per_beat * (1.0 + scroll.y * 0.003)).clamp(8.0, 600.0);
        } else {
            st.scroll_beats = (st.scroll_beats - (scroll.x / st.px_per_beat) as f64).max(0.0);
            st.top_pitch = (st.top_pitch as f32 + scroll.y / st.key_h).round().clamp(12.0, 127.0) as i32;
        }
    }
    let (ppb, key_h, top, sb, gsize) = (st.px_per_beat, st.key_h, st.top_pitch, st.scroll_beats, st.grid);
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
    let ClipContent::Midi { notes } = &clip.content else { return };
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
    let color = theme::rgb(clip.color);
    let mut hovered: Option<(usize, bool)> = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    for (i, n) in notes.iter().enumerate() {
        let rect = Rect::from_min_max(pos2(x_of(n.start), y_of(n.pitch as i32) + 0.5), pos2(x_of(n.start + n.length), y_of(n.pitch as i32) + key_h - 0.5));
        let selected = app.sel.notes.contains(&i);
        let shade = 0.45 + 0.55 * n.velocity as f32 / 127.0;
        p.rect_filled(rect, 1.5, if selected { theme::SELECT } else { theme::dim(color, shade * 1.3) });
        p.rect_stroke(rect, 1.5, Stroke::new(1.0, Color32::from_black_alpha(140)), StrokeKind::Inside);
        let vx = rect.left();
        let vh = (n.velocity as f32 / 127.0) * (vel.height() - 4.0);
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
                let origin = app.sel.notes.iter().filter_map(|&i| notes.get(i).map(|n| (i, n.velocity))).collect();
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
                    if let Some(ns) = notes_mut(app) {
                        for (i, s, pch) in origin {
                            if let Some(n) = ns.get_mut(i) {
                                n.start = (s + db).max(0.0);
                                n.pitch = (pch as i32 + dp).clamp(0, 127) as u8;
                            }
                        }
                    }
                    app.label("Move Notes");
                }
                NoteDrag::Resize { origin, start_beat } => {
                    let db = beat_of(q.x) - start_beat;
                    let db = if shift { db } else { (db / gsize).round() * gsize };
                    let mut last = None;
                    if let Some(ns) = notes_mut(app) {
                        for (i, l) in origin {
                            if let Some(n) = ns.get_mut(i) {
                                n.length = (l + db).max(1.0 / 64.0);
                                last = Some(n.length);
                            }
                        }
                    }
                    if let Some(l) = last {
                        app.piano.last_length = l;
                    }
                    app.label("Resize Notes");
                }
                NoteDrag::Velocity { origin, start_y } => {
                    let dv = ((start_y - q.y) / (vel.height().max(1.0)) * 127.0) as i32;
                    if let Some(ns) = notes_mut(app) {
                        for (i, v) in origin {
                            if let Some(n) = ns.get_mut(i) {
                                n.velocity = (v as i32 + dv).clamp(1, 127) as u8;
                            }
                        }
                    }
                    app.label("Velocity");
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
                    if let Some(ns) = notes_mut(app) {
                        if i < ns.len() {
                            ns.remove(i);
                        }
                    }
                    app.sel.notes.clear();
                    app.label("Delete Note");
                }
                None => {
                    let start = snap(beat_of(q.x)).max(0.0);
                    let pitch = pitch_of(q.y).clamp(0, 127) as u8;
                    let length = app.piano.last_length.max(gsize);
                    if let Some(ns) = notes_mut(app) {
                        ns.push(Note { pitch, start, length, velocity: 100 });
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
            if let Some(ns) = notes_mut(app) {
                for i in sel {
                    if let Some(n) = ns.get_mut(i) {
                        n.start = (n.start + shift_b).max(0.0);
                    }
                }
            }
            app.label("Move Notes");
        }
    }
}

// ---------------------------------------------------------------- audio

fn audio_view(app: &mut XdawApp, ui: &mut Ui) {
    let Some(clip) = app.selected_clip().cloned() else { return };
    let ClipContent::Audio(a) = &clip.content else { return };
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
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
    let total_beats = data.duration_seconds() * a.source_bpm / 60.0;
    let x_of = |b: f64| rect.left() + (b / total_beats.max(1e-6)) as f32 * rect.width();
    let beat_of = |x: f32| ((x - rect.left()) / rect.width()) as f64 * total_beats;
    let mid = rect.center().y;
    let h = rect.height() * 0.45;
    let color = theme::rgb(clip.color);
    for px in 0..rect.width() as usize {
        let idx = px * ov.len() / rect.width().max(1.0) as usize;
        if let Some(&(lo, hi)) = ov.get(idx) {
            let x = rect.left() + px as f32;
            p.line_segment([pos2(x, mid - hi * h), pos2(x, mid - lo * h)], Stroke::new(1.0, color));
        }
    }
    let r = clip.region;
    p.rect_filled(Rect::from_min_max(pos2(x_of(r.end), rect.top()), rect.max), 0.0, Color32::from_black_alpha(120));
    p.rect_filled(Rect::from_min_max(rect.min, pos2(x_of(r.start), rect.bottom())), 0.0, Color32::from_black_alpha(120));
    for (b, label, c) in [(r.start, "S", theme::TEXT), (r.loop_start, "L", theme::QUEUED), (r.end, "E", theme::TEXT)] {
        let x = x_of(b);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, c));
        p.text(pos2(x + 2.0, rect.top() + 2.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), c);
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
