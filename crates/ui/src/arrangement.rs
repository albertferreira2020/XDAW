//! Arrangement View: multitrack timeline with clips, ruler, loop and automation.

use crate::app::{ClipLoc, Detail, XdawApp};
use crate::browser::{self, BrowserItem};
use crate::theme;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use std::collections::HashMap;
use xdaw_engine::protocol::Command;
use xdaw_project::*;
use xdaw_sequencer::Beats;

const HEADER_W: f32 = 150.0;
const RULER_H: f32 = 34.0;
const EDGE: f32 = 6.0;

pub struct ArrangementState {
    pub px_per_beat: f32,
    pub scroll_beats: f64,
    pub snap: f64,
    pub cursor: Beats,
    pub lane_h: f32,
    /// Automation lane shown per track id.
    pub lane: HashMap<Id, ParamTarget>,
    drag: Option<Drag>,
}

impl Default for ArrangementState {
    fn default() -> Self {
        Self { px_per_beat: 18.0, scroll_beats: 0.0, snap: 1.0, cursor: 0.0, lane_h: 56.0, lane: HashMap::new(), drag: None }
    }
}

#[derive(Clone, Debug)]
enum DragKind {
    Move { origins: Vec<(usize, Id, Beats)> },
    TrimStart { track: usize, id: Id, start: Beats, offset: Beats, length: Beats },
    TrimEnd { track: usize, id: Id, length: Beats },
    FadeIn { track: usize, id: Id },
    FadeOut { track: usize, id: Id },
    Point { track: usize, target: ParamTarget, index: usize },
    Loop { anchor: Beats },
}

#[derive(Clone, Debug)]
struct Drag {
    kind: DragKind,
    start_beat: Beats,
}

impl ArrangementState {
    fn x(&self, rect: Rect, beat: Beats) -> f32 {
        rect.left() + ((beat - self.scroll_beats) as f32) * self.px_per_beat
    }

    fn beat(&self, rect: Rect, x: f32) -> Beats {
        self.scroll_beats + ((x - rect.left()) / self.px_per_beat) as f64
    }

    fn snap(&self, b: Beats, bypass: bool) -> Beats {
        if self.snap <= 0.0 || bypass { b } else { (b / self.snap).round() * self.snap }
    }
}

fn song_end(p: &Project) -> Beats {
    p.tracks.iter().flat_map(|t| t.arrangement.iter().map(|c| c.end())).fold(0.0, f64::max)
}

/// Value range of an automation target (native units) for drawing.
pub fn target_range(p: &Project, r: TrackRef, target: ParamTarget) -> (f32, f32) {
    match target {
        ParamTarget::Volume | ParamTarget::Send(_) => (SEND_OFF_DB, 6.0),
        ParamTarget::Pan => (-1.0, 1.0),
        ParamTarget::Device { device, param } => p
            .track(r)
            .devices
            .iter()
            .find(|d| d.id == device)
            .and_then(|d| d.kind.params().get(param))
            .map(|d| (d.min, d.max))
            .unwrap_or((0.0, 1.0)),
    }
}

fn target_norm(p: &Project, r: TrackRef, target: ParamTarget, v: f32) -> f32 {
    match target {
        ParamTarget::Volume | ParamTarget::Send(_) => crate::mixer::db_to_fader(v),
        ParamTarget::Device { device, param } => p
            .track(r)
            .devices
            .iter()
            .find(|d| d.id == device)
            .and_then(|d| d.kind.params().get(param))
            .map(|d| d.to_normalized(v))
            .unwrap_or(0.0),
        ParamTarget::Pan => {
            let (lo, hi) = target_range(p, r, target);
            (v - lo) / (hi - lo)
        }
    }
}

fn target_from_norm(p: &Project, r: TrackRef, target: ParamTarget, n: f32) -> f32 {
    match target {
        ParamTarget::Volume | ParamTarget::Send(_) => crate::mixer::fader_to_db(n),
        ParamTarget::Device { device, param } => p
            .track(r)
            .devices
            .iter()
            .find(|d| d.id == device)
            .and_then(|d| d.kind.params().get(param))
            .map(|d| d.from_normalized(n))
            .unwrap_or(n),
        ParamTarget::Pan => n * 2.0 - 1.0,
    }
}

pub fn target_name(p: &Project, r: TrackRef, target: ParamTarget) -> String {
    match target {
        ParamTarget::Volume => "Volume".into(),
        ParamTarget::Pan => "Pan".into(),
        ParamTarget::Send(i) => format!("Send {}", (b'A' + i as u8) as char),
        ParamTarget::Device { device, param } => p
            .track(r)
            .devices
            .iter()
            .find(|d| d.id == device)
            .map(|d| format!("{}: {}", d.kind.name(), d.kind.params().get(param).map(|x| x.name).unwrap_or("?")))
            .unwrap_or_else(|| "Missing device".into()),
    }
}

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label("Snap");
        egui::ComboBox::from_id_salt("snap").width(70.0).selected_text(snap_label(app.arr.snap)).show_ui(ui, |ui| {
            for s in [0.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0] {
                ui.selectable_value(&mut app.arr.snap, s, snap_label(s));
            }
        });
        if ui.small_button("−").on_hover_text("Zoom out").clicked() {
            app.arr.px_per_beat = (app.arr.px_per_beat / 1.25).max(2.0);
        }
        if ui.small_button("+").on_hover_text("Zoom in").clicked() {
            app.arr.px_per_beat = (app.arr.px_per_beat * 1.25).min(400.0);
        }
        ui.label(egui::RichText::new(format!("Insert marker {:.2}", app.arr.cursor)).color(theme::TEXT_DIM));
        if ui.small_button("Split at marker (Cmd+E)").clicked() {
            split_selected(app);
        }
        ui.checkbox(&mut app.follow, "Follow");
    });
    let rows = app.project.tracks.len();
    let total_h = RULER_H + rows as f32 * app.arr.lane_h + 40.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width();
        let (outer, resp) = ui.allocate_exact_size(vec2(width, total_h.max(ui.available_height())), Sense::click_and_drag());
        let timeline = Rect::from_min_max(pos2(outer.left() + HEADER_W, outer.top()), outer.max);
        handle_scroll_zoom(app, ui, &resp, timeline);
        follow_playhead(app, timeline);
        draw_ruler(app, ui, timeline);
        for t in 0..rows {
            let top = outer.top() + RULER_H + t as f32 * app.arr.lane_h;
            let header = Rect::from_min_size(pos2(outer.left(), top), vec2(HEADER_W, app.arr.lane_h));
            let lane = Rect::from_min_max(pos2(timeline.left(), top), pos2(timeline.right(), top + app.arr.lane_h));
            track_header(app, ui, t, header);
            track_lane(app, ui, t, lane);
        }
        // Playhead and insert marker over everything.
        let p = ui.painter_at(timeline);
        let x = app.arr.x(timeline, app.beat());
        p.line_segment([pos2(x, timeline.top()), pos2(x, timeline.bottom())], Stroke::new(1.5, theme::ACCENT));
        let cx = app.arr.x(timeline, app.arr.cursor);
        p.line_segment([pos2(cx, timeline.top() + RULER_H), pos2(cx, timeline.bottom())], Stroke::new(1.0, Color32::from_white_alpha(90)));
        if ui.input(|i| i.pointer.any_released()) {
            app.arr.drag = None;
        }
    });
}

fn snap_label(s: f64) -> String {
    match s {
        x if x <= 0.0 => "Off".into(),
        x if x >= 4.0 => "1 Bar".into(),
        x if x >= 2.0 => "1/2".into(),
        x if x >= 1.0 => "1/4".into(),
        x if x >= 0.5 => "1/8".into(),
        x if x >= 0.25 => "1/16".into(),
        _ => "1/32".into(),
    }
}

fn handle_scroll_zoom(app: &mut XdawApp, ui: &Ui, resp: &egui::Response, timeline: Rect) {
    if !resp.hovered() {
        return;
    }
    let (scroll, zoom, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
    if zoom != 1.0 || (mods.command && scroll.y != 0.0) {
        let factor = if zoom != 1.0 { zoom } else { (1.0 + scroll.y * 0.003).clamp(0.5, 2.0) };
        let anchor = hover.map(|p| app.arr.beat(timeline, p.x)).unwrap_or(app.arr.scroll_beats);
        app.arr.px_per_beat = (app.arr.px_per_beat * factor).clamp(2.0, 400.0);
        if let Some(p) = hover {
            app.arr.scroll_beats = (anchor - ((p.x - timeline.left()) / app.arr.px_per_beat) as f64).max(0.0);
        }
    } else if scroll.x != 0.0 || mods.shift {
        let dx = if scroll.x != 0.0 { scroll.x } else { scroll.y };
        app.arr.scroll_beats = (app.arr.scroll_beats - (dx / app.arr.px_per_beat) as f64).max(0.0);
    }
}

fn follow_playhead(app: &mut XdawApp, timeline: Rect) {
    if !(app.follow && app.playing()) {
        return;
    }
    let visible = (timeline.width() / app.arr.px_per_beat) as f64;
    let b = app.beat();
    if b < app.arr.scroll_beats || b > app.arr.scroll_beats + visible * 0.9 {
        app.arr.scroll_beats = (b - visible * 0.1).max(0.0);
    }
}

fn draw_ruler(app: &mut XdawApp, ui: &mut Ui, timeline: Rect) {
    let rect = Rect::from_min_size(timeline.min, vec2(timeline.width(), RULER_H));
    let loop_rect = Rect::from_min_size(rect.min, vec2(rect.width(), 12.0));
    let resp = ui.interact(rect, ui.id().with("ruler"), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::PANEL_DARK);
    let sig = app.project.transport.time_signature;
    let bar = sig.bar_beats();
    let visible = (timeline.width() / app.arr.px_per_beat) as f64;
    let step_bars = if app.arr.px_per_beat * bar as f32 > 40.0 { 1.0 } else if app.arr.px_per_beat * bar as f32 > 12.0 { 4.0 } else { 16.0 };
    let first = (app.arr.scroll_beats / (bar * step_bars)).floor() as i64;
    let last = ((app.arr.scroll_beats + visible) / (bar * step_bars)).ceil() as i64;
    for k in first..=last {
        let b = k as f64 * bar * step_bars;
        let x = app.arr.x(timeline, b);
        p.line_segment([pos2(x, rect.top() + 14.0), pos2(x, rect.bottom())], Stroke::new(1.0, theme::GRID));
        p.text(pos2(x + 3.0, rect.top() + 14.0), Align2::LEFT_TOP, format!("{}", (b / bar) as i64 + 1), FontId::proportional(10.0), theme::TEXT_DIM);
    }
    // Loop brace.
    let t = &app.project.transport;
    let (lx0, lx1) = (app.arr.x(timeline, t.loop_start), app.arr.x(timeline, t.loop_start + t.loop_length));
    let brace = Rect::from_min_max(pos2(lx0, loop_rect.top() + 2.0), pos2(lx1, loop_rect.bottom()));
    p.rect_filled(brace, 2.0, if t.loop_enabled { theme::QUEUED.linear_multiply(0.8) } else { theme::GRID });
    let alt = ui.input(|i| i.modifiers.alt);
    if let Some(pos) = resp.interact_pointer_pos() {
        let beat = app.arr.snap(app.arr.beat(timeline, pos.x), alt).max(0.0);
        if resp.drag_started() && pos.y < loop_rect.bottom() {
            app.arr.drag = Some(Drag { kind: DragKind::Loop { anchor: beat }, start_beat: beat });
        }
        if let Some(Drag { kind: DragKind::Loop { anchor }, .. }) = app.arr.drag.clone() {
            if resp.dragged() {
                let (a, b) = (anchor.min(beat), anchor.max(beat));
                if b - a >= 0.25 {
                    app.project.transport.loop_start = a;
                    app.project.transport.loop_length = b - a;
                    app.label("Set Loop");
                }
            }
        } else if resp.clicked() {
            app.arr.cursor = beat;
            app.rt.send(Command::Locate(beat));
        }
    }
    if resp.double_clicked() && resp.interact_pointer_pos().is_some_and(|p| p.y < loop_rect.bottom()) {
        app.project.transport.loop_enabled = !app.project.transport.loop_enabled;
    }
}

fn track_header(app: &mut XdawApp, ui: &mut Ui, t: usize, rect: Rect) {
    let r = TrackRef::Track(t);
    let resp = ui.interact(rect, ui.id().with(("arr_header", t)), Sense::click());
    let track = &app.project.tracks[t];
    let selected = app.sel.track == Some(r);
    let p = ui.painter_at(rect);
    p.rect_filled(rect.shrink(0.5), 0.0, if selected { theme::dim(theme::rgb(track.color), 0.8) } else { theme::dim(theme::rgb(track.color), 0.45) });
    p.text(rect.left_top() + vec2(5.0, 4.0), Align2::LEFT_TOP, &track.name, FontId::proportional(11.5), theme::TEXT);
    let id = track.id;
    if resp.clicked() {
        app.select_track(r);
    }
    let track = &app.project.tracks[t];
    // Automation lane chooser.
    let current = app.arr.lane.get(&id).copied();
    let mut choices: Vec<(Option<ParamTarget>, String)> = vec![(None, "No automation".into()), (Some(ParamTarget::Volume), "Volume".into()), (Some(ParamTarget::Pan), "Pan".into())];
    for i in 0..app.project.returns.len() {
        choices.push((Some(ParamTarget::Send(i)), format!("Send {}", (b'A' + i as u8) as char)));
    }
    for d in &track.devices {
        for (pi, desc) in d.kind.params().iter().enumerate() {
            choices.push((Some(ParamTarget::Device { device: d.id, param: pi }), format!("{}: {}", d.kind.name(), desc.name)));
        }
    }
    let label = current.map(|c| target_name(&app.project, r, c)).unwrap_or_else(|| "No automation".into());
    let combo_rect = Rect::from_min_size(rect.left_top() + vec2(4.0, 22.0), vec2(HEADER_W - 8.0, 18.0));
    let mut chosen = current;
    ui.scope_builder(egui::UiBuilder::new().max_rect(combo_rect), |ui| {
        egui::ComboBox::from_id_salt(("lane", t)).width(HEADER_W - 12.0).selected_text(egui::RichText::new(label).size(10.0)).show_ui(ui, |ui| {
            for (c, name) in &choices {
                let has = c.is_some_and(|c| app.project.tracks[t].lane(c).is_some_and(|l| !l.points.is_empty()));
                let text = if has { format!("* {name}") } else { name.clone() };
                ui.selectable_value(&mut chosen, *c, text);
            }
        });
    });
    match chosen {
        Some(c) => {
            app.arr.lane.insert(id, c);
        }
        None => {
            app.arr.lane.remove(&id);
        }
    }
}

fn track_lane(app: &mut XdawApp, ui: &mut Ui, t: usize, lane: Rect) {
    let r = TrackRef::Track(t);
    let id = ui.id().with(("lane", t));
    let resp = ui.interact(lane, id, Sense::click_and_drag());
    let painter = ui.painter_at(lane);
    painter.rect_filled(lane, 0.0, if t % 2 == 0 { theme::BG } else { theme::dim(theme::BG, 1.08) });
    // Beat grid.
    let bar = app.project.transport.time_signature.bar_beats();
    let visible = (lane.width() / app.arr.px_per_beat) as f64;
    let first = (app.arr.scroll_beats / bar).floor() as i64;
    for k in first..=first + (visible / bar).ceil() as i64 + 1 {
        let x = app.arr.x(lane, k as f64 * bar);
        painter.line_segment([pos2(x, lane.top()), pos2(x, lane.bottom())], Stroke::new(1.0, theme::dim(theme::GRID, 0.7)));
    }
    painter.line_segment([lane.left_bottom(), lane.right_bottom()], Stroke::new(1.0, theme::PANEL_DARK));
    let loop_t = &app.project.transport;
    if loop_t.loop_enabled {
        let x0 = app.arr.x(lane, loop_t.loop_start);
        let x1 = app.arr.x(lane, loop_t.loop_start + loop_t.loop_length);
        painter.rect_filled(Rect::from_min_max(pos2(x0, lane.top()), pos2(x1, lane.bottom())), 0.0, Color32::from_white_alpha(6));
    }

    let alt = ui.input(|i| i.modifiers.alt);
    let shift = ui.input(|i| i.modifiers.shift);
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let pointer_beat = pointer.map(|p| app.arr.beat(lane, p.x));

    // Clips.
    let clips: Vec<ArrangementClip> = app.project.tracks[t].arrangement.clone();
    let mut hit: Option<(Id, HitZone)> = None;
    for c in &clips {
        let x0 = app.arr.x(lane, c.start);
        let x1 = app.arr.x(lane, c.end());
        if x1 < lane.left() || x0 > lane.right() {
            continue;
        }
        let rect = Rect::from_min_max(pos2(x0, lane.top() + 2.0), pos2(x1, lane.bottom() - 2.0));
        let selected = app.sel.arr_clips.contains(&(t, c.id));
        draw_clip(app, &painter, c, rect);
        if selected {
            painter.rect_stroke(rect, 2.0, Stroke::new(1.5, theme::SELECT), StrokeKind::Inside);
        }
        if let Some(p) = pointer {
            if rect.contains(p) {
                let zone = if p.y < rect.top() + 8.0 && p.x < rect.left() + rect.width() * 0.5 {
                    HitZone::FadeIn
                } else if p.y < rect.top() + 8.0 {
                    HitZone::FadeOut
                } else if p.x < rect.left() + EDGE {
                    HitZone::Start
                } else if p.x > rect.right() - EDGE {
                    HitZone::End
                } else {
                    HitZone::Body
                };
                hit = Some((c.id, zone));
            }
        }
    }

    // Automation overlay.
    let lane_target = app.arr.lane.get(&app.project.tracks[t].id).copied();
    let mut point_hit = None;
    if let Some(target) = lane_target {
        point_hit = draw_automation(app, &painter, t, target, lane, pointer);
    }

    if let Some((_, zone)) = hit {
        let icon = match zone {
            HitZone::Start | HitZone::End => egui::CursorIcon::ResizeHorizontal,
            HitZone::FadeIn | HitZone::FadeOut => egui::CursorIcon::ResizeNeSw,
            HitZone::Body => egui::CursorIcon::Grab,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    // Start drags.
    if resp.drag_started() {
        let start_beat = pointer_beat.unwrap_or(0.0);
        if let (Some(target), Some(idx)) = (lane_target, point_hit) {
            app.arr.drag = Some(Drag { kind: DragKind::Point { track: t, target, index: idx }, start_beat });
        } else if let Some((cid, zone)) = hit {
            let c = clips.iter().find(|c| c.id == cid).unwrap();
            if !app.sel.arr_clips.contains(&(t, cid)) {
                app.sel.arr_clips = vec![(t, cid)];
            }
            let kind = match zone {
                HitZone::Body => {
                    let origins = app
                        .sel
                        .arr_clips
                        .iter()
                        .filter_map(|&(tt, id)| app.project.tracks.get(tt)?.arrangement.iter().find(|x| x.id == id).map(|x| (tt, id, x.start)))
                        .collect();
                    DragKind::Move { origins }
                }
                HitZone::Start => DragKind::TrimStart { track: t, id: cid, start: c.start, offset: c.offset, length: c.length },
                HitZone::End => DragKind::TrimEnd { track: t, id: cid, length: c.length },
                HitZone::FadeIn => DragKind::FadeIn { track: t, id: cid },
                HitZone::FadeOut => DragKind::FadeOut { track: t, id: cid },
            };
            app.arr.drag = Some(Drag { kind, start_beat });
        }
    }
    if resp.dragged() {
        if let (Some(drag), Some(pb)) = (app.arr.drag.clone(), pointer_beat) {
            apply_drag(app, &drag, pb, alt, lane, pointer);
        }
    }

    // Clicks.
    if resp.clicked() {
        let beat = app.arr.snap(pointer_beat.unwrap_or(0.0), alt).max(0.0);
        match (lane_target, point_hit, hit) {
            (Some(target), None, _) if shift || hit.is_none() && lane_target.is_some() && ui.input(|i| i.modifiers.command) => {
                // Cmd/Shift-click on the envelope adds a point.
                if let Some(p) = pointer {
                    let n = 1.0 - (p.y - lane.top()) / lane.height();
                    let v = target_from_norm(&app.project, r, target, n);
                    app.project.tracks[t].lane_mut(target).insert(AutomationPoint { beat, value: v });
                    app.label("Add Automation Point");
                }
            }
            (_, _, Some((cid, _))) => {
                if shift {
                    if let Some(i) = app.sel.arr_clips.iter().position(|x| *x == (t, cid)) {
                        app.sel.arr_clips.remove(i);
                    } else {
                        app.sel.arr_clips.push((t, cid));
                    }
                } else {
                    app.sel.arr_clips = vec![(t, cid)];
                }
                app.sel.clip = Some(ClipLoc::Arrangement { track: t, id: cid });
                app.sel.track = Some(r);
                app.arr.cursor = beat;
            }
            _ => {
                app.sel.arr_clips.clear();
                app.arr.cursor = beat;
                app.select_track(r);
                if !app.playing() {
                    app.rt.send(Command::Locate(beat));
                }
            }
        }
    }
    if resp.double_clicked() {
        if let (Some(target), Some(idx)) = (lane_target, point_hit) {
            let lane = app.project.tracks[t].lane_mut(target);
            if idx < lane.points.len() {
                lane.points.remove(idx);
                app.label("Delete Automation Point");
            }
        } else if let Some((cid, _)) = hit {
            app.sel.clip = Some(ClipLoc::Arrangement { track: t, id: cid });
            app.detail = Detail::Clip;
        } else if let (Some(target), Some(p)) = (lane_target, pointer) {
            let beat = app.arr.snap(pointer_beat.unwrap_or(0.0), alt).max(0.0);
            let n = 1.0 - (p.y - lane.top()) / lane.height();
            let v = target_from_norm(&app.project, r, target, n);
            app.project.tracks[t].lane_mut(target).insert(AutomationPoint { beat, value: v });
            app.label("Add Automation Point");
        }
    }

    // Drops: browser files and Session clips.
    if let Some(item) = resp.dnd_release_payload::<BrowserItem>() {
        if let (BrowserItem::File(path), Some(b)) = ((*item).clone(), pointer_beat) {
            drop_file(app, t, app.arr.snap(b, alt).max(0.0), &path);
        }
    }
    if let Some(drag) = resp.dnd_release_payload::<crate::session::SlotDrag>() {
        if let Some(b) = pointer_beat {
            let at = app.arr.snap(b, alt).max(0.0);
            let saved = app.arr.cursor;
            app.arr.cursor = at;
            if drag.track == t {
                copy_slot_to_arrangement(app, drag.track, drag.slot);
            }
            app.arr.cursor = saved;
        }
    }

    resp.context_menu(|ui| {
        if ui.button("Delete").clicked() {
            app.delete_selection();
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            duplicate_selected(app);
            ui.close();
        }
        if ui.button("Split at Insert Marker").clicked() {
            split_selected(app);
            ui.close();
        }
        if ui.button("Copy").clicked() {
            app.copy_selection();
            ui.close();
        }
        if ui.button("Paste at Insert Marker").clicked() {
            paste(app);
            ui.close();
        }
        if ui.button("Insert MIDI clip (1 bar) at marker").clicked() && app.project.tracks[t].kind == TrackKind::Midi {
            let bar = app.project.transport.time_signature.bar_beats();
            let clip = Clip {
                id: app.project.alloc_id(),
                name: "MIDI Clip".into(),
                color: app.project.tracks[t].color,
                content: ClipContent::Midi { notes: vec![] },
                region: xdaw_sequencer::ClipRegion::looped(bar),
                launch_mode: Default::default(),
                quantize: Default::default(),
                legato: false,
                gain_db: 0.0,
                transpose: 0,
            };
            let id = app.project.alloc_id();
            let start = app.arr.cursor;
            app.project.tracks[t].arrangement.push(ArrangementClip { id, start, length: bar, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
            app.label("Insert MIDI Clip");
            ui.close();
        }
        if let Some(target) = lane_target {
            if ui.button(format!("Clear {} automation", target_name(&app.project, r, target))).clicked() {
                app.project.tracks[t].automation.retain(|l| l.target != target);
                app.label("Clear Automation");
                ui.close();
            }
        }
    });
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum HitZone {
    Body,
    Start,
    End,
    FadeIn,
    FadeOut,
}

fn apply_drag(app: &mut XdawApp, drag: &Drag, pb: Beats, alt: bool, lane: Rect, pointer: Option<Pos2>) {
    let delta = pb - drag.start_beat;
    match &drag.kind {
        DragKind::Move { origins, .. } => {
            let snapped = |b: Beats| app.arr.snap(b, alt).max(0.0);
            let first = origins.iter().map(|o| o.2).fold(f64::INFINITY, f64::min);
            let shift = snapped(first + delta) - first;
            for &(t, id, start) in origins {
                if let Some(c) = app.project.tracks[t].arrangement.iter_mut().find(|c| c.id == id) {
                    c.start = (start + shift).max(0.0);
                }
            }
            app.label("Move Clips");
        }
        DragKind::TrimStart { track, id, start, offset, length } => {
            let new_start = app.arr.snap(start + delta, alt).clamp(start - offset, start + length - 0.0625);
            if let Some(c) = app.project.tracks[*track].arrangement.iter_mut().find(|c| c.id == *id) {
                let d = new_start - start;
                c.start = new_start;
                c.offset = offset + d;
                c.length = length - d;
            }
            app.label("Trim Clip");
        }
        DragKind::TrimEnd { track, id, length } => {
            let snapped_len = app.arr.snap(length + delta, alt).max(0.0625);
            if let Some(c) = app.project.tracks[*track].arrangement.iter_mut().find(|c| c.id == *id) {
                // A non-looping clip cannot extend past its region end.
                let max = if c.clip.region.looping { f64::INFINITY } else { c.clip.region.end - c.offset };
                c.length = snapped_len.min(max);
            }
            app.label("Trim Clip");
        }
        DragKind::FadeIn { track, id } => {
            if let Some(c) = app.project.tracks[*track].arrangement.iter_mut().find(|c| c.id == *id) {
                c.fade_in = (pb - c.start).clamp(0.0, c.length - c.fade_out);
            }
            app.label("Fade In");
        }
        DragKind::FadeOut { track, id } => {
            if let Some(c) = app.project.tracks[*track].arrangement.iter_mut().find(|c| c.id == *id) {
                c.fade_out = (c.end() - pb).clamp(0.0, c.length - c.fade_in);
            }
            app.label("Fade Out");
        }
        DragKind::Point { track, target, index } => {
            let Some(p) = pointer else { return };
            let n = (1.0 - (p.y - lane.top()) / lane.height()).clamp(0.0, 1.0);
            let r = TrackRef::Track(*track);
            let v = target_from_norm(&app.project, r, *target, n);
            let beat = app.arr.snap(pb, alt).max(0.0);
            let lane = app.project.tracks[*track].lane_mut(*target);
            if let Some(pt) = lane.points.get_mut(*index) {
                pt.beat = beat;
                pt.value = v;
            }
            lane.points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            app.label("Move Automation Point");
        }
        DragKind::Loop { .. } => {}
    }
}

fn draw_clip(app: &XdawApp, p: &egui::Painter, c: &ArrangementClip, rect: Rect) {
    let color = theme::rgb(c.clip.color);
    p.rect_filled(rect, 2.0, theme::dim(color, 0.55));
    let title = Rect::from_min_size(rect.min, vec2(rect.width(), 12.0));
    p.rect_filled(title, 2.0, color);
    p.text(title.left_center() + vec2(3.0, 0.0), Align2::LEFT_CENTER, &c.clip.name, FontId::proportional(10.0), Color32::BLACK);
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 12.0), rect.max);
    let ppb = app.arr.px_per_beat as f64;
    match &c.clip.content {
        ClipContent::Audio(a) => {
            if let (Some(ov), Some(data)) = (app.media.overviews.get(&a.media.path), app.media.loaded.get(&a.media.path)) {
                let dur = data.duration_seconds().max(1e-6);
                let mid = body.center().y;
                let h = body.height() * 0.5;
                let x0 = body.left().max(p.clip_rect().left());
                let x1 = body.right().min(p.clip_rect().right());
                let mut x = x0;
                while x < x1 {
                    let beat_in = (x - rect.left()) as f64 / ppb;
                    if let Some(local) = c.clip.region.position(c.offset + beat_in) {
                        let secs = local * 60.0 / a.source_bpm;
                        let idx = ((secs / dur) * ov.len() as f64) as usize;
                        if let Some(&(lo, hi)) = ov.get(idx) {
                            p.line_segment([pos2(x, mid - hi * h), pos2(x, mid - lo * h)], Stroke::new(1.0, theme::dim(color, 1.4)));
                        }
                    }
                    x += 1.0;
                }
            } else {
                p.text(body.center(), Align2::CENTER_CENTER, "media offline", FontId::proportional(10.0), theme::RECORD);
            }
        }
        ClipContent::Midi { notes } => {
            let (lo, hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
            let range = (hi.saturating_sub(lo) as f32 + 1.0).max(12.0);
            let region = c.clip.region;
            // Draw each loop pass visible in the clip.
            let mut e = c.offset;
            let end = c.offset + c.length;
            let mut guard = 0;
            while e < end && guard < 512 {
                guard += 1;
                let local = region.position(e).unwrap_or(region.end);
                let span = (region.end - local).min(end - e);
                for n in notes.iter().filter(|n| n.start >= local && n.start < local + span) {
                    let x = rect.left() + ((e - c.offset + n.start - local) * ppb) as f32;
                    let w = ((n.length.min(local + span - n.start)) * ppb).max(1.0) as f32;
                    let y = body.bottom() - (n.pitch - lo) as f32 / range * body.height() - 2.0;
                    p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(w, 2.0)), 0.0, theme::dim(color, 1.5));
                }
                if span <= 0.0 || !region.looping {
                    break;
                }
                e += span;
            }
        }
    }
    // Fades.
    let fx0 = rect.left() + (c.fade_in * ppb) as f32;
    let fx1 = rect.right() - (c.fade_out * ppb) as f32;
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(180));
    if c.fade_in > 0.0 {
        p.line_segment([pos2(rect.left(), rect.bottom()), pos2(fx0, rect.top() + 12.0)], stroke);
    }
    if c.fade_out > 0.0 {
        p.line_segment([pos2(fx1, rect.top() + 12.0), pos2(rect.right(), rect.bottom())], stroke);
    }
}

/// Draws the envelope; returns the index of the point under the pointer.
fn draw_automation(app: &XdawApp, p: &egui::Painter, t: usize, target: ParamTarget, lane: Rect, pointer: Option<Pos2>) -> Option<usize> {
    let r = TrackRef::Track(t);
    let track = &app.project.tracks[t];
    let stroke = Stroke::new(1.5, Color32::from_rgb(255, 120, 120));
    let y = |v: f32| lane.bottom() - target_norm(&app.project, r, target, v).clamp(0.0, 1.0) * lane.height();
    let mut hit = None;
    match track.lane(target).filter(|l| !l.points.is_empty()) {
        Some(l) => {
            let pts: Vec<Pos2> = l.points.iter().map(|pt| pos2(app.arr.x(lane, pt.beat), y(pt.value))).collect();
            p.line_segment([pos2(lane.left(), pts[0].y), pts[0]], stroke);
            for w in pts.windows(2) {
                p.line_segment([w[0], w[1]], stroke);
            }
            p.line_segment([*pts.last().unwrap(), pos2(lane.right(), pts.last().unwrap().y)], stroke);
            for (i, pt) in pts.iter().enumerate() {
                let near = pointer.is_some_and(|q| q.distance(*pt) < 6.0);
                if near {
                    hit = Some(i);
                }
                p.circle_filled(*pt, if near { 4.5 } else { 3.0 }, Color32::from_rgb(255, 150, 150));
            }
        }
        None => {
            // Flat line at the current static value.
            let v = match target {
                ParamTarget::Volume => track.mixer.volume_db,
                ParamTarget::Pan => track.mixer.pan,
                ParamTarget::Send(i) => track.mixer.sends_db.get(i).copied().unwrap_or(SEND_OFF_DB),
                ParamTarget::Device { device, param } => track.devices.iter().find(|d| d.id == device).map(|d| d.param(param)).unwrap_or(0.0),
            };
            p.line_segment([pos2(lane.left(), y(v)), pos2(lane.right(), y(v))], Stroke::new(1.0, Color32::from_rgb(200, 110, 110)));
        }
    }
    p.text(lane.right_top() + vec2(-4.0, 2.0), Align2::RIGHT_TOP, format!("{} — Cmd-click/double-click: add · drag: move · double-click point: delete", target_name(&app.project, r, target)), FontId::proportional(9.5), theme::TEXT_DIM);
    hit
}

fn drop_file(app: &mut XdawApp, t: usize, at: Beats, path: &std::path::Path) {
    // Reuse slot import logic through a temporary clip.
    let scene_count = app.project.scenes.len();
    if scene_count == 0 {
        return;
    }
    let before = app.project.tracks[t].slots[0].clip.clone();
    browser::drop_file_on_slot(app, t, 0, path);
    let made = app.project.tracks[t].slots[0].clip.take();
    app.project.tracks[t].slots[0].clip = before;
    if let Some(clip) = made {
        let id = app.project.alloc_id();
        let length = clip.length();
        app.project.tracks[t].arrangement.push(ArrangementClip { id, start: at, length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
        app.sel.arr_clips = vec![(t, id)];
        app.label("Import to Arrangement");
    }
}

pub fn copy_slot_to_arrangement(app: &mut XdawApp, t: usize, s: usize) {
    let Some(clip) = app.project.tracks[t].slots[s].clip.clone() else { return };
    let id = app.project.alloc_id();
    let start = app.arr.cursor;
    let length = clip.length();
    let mut next = app.project.next_id;
    app.project.tracks[t].clear_arrangement_range(start, start + length, || {
        next += 1;
        next
    });
    app.project.next_id = next + 1;
    app.project.tracks[t].arrangement.push(ArrangementClip { id, start, length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
    app.project.tracks[t].arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
    app.label("Copy to Arrangement");
}

pub fn split_selected(app: &mut XdawApp) {
    let at = app.arr.cursor;
    let sel = app.sel.arr_clips.clone();
    let mut new_sel = vec![];
    for (t, id) in sel {
        let Some(track) = app.project.tracks.get(t) else { continue };
        let Some(c) = track.arrangement.iter().find(|c| c.id == id).cloned() else { continue };
        if at <= c.start + 1e-6 || at >= c.end() - 1e-6 {
            new_sel.push((t, id));
            continue;
        }
        let right_id = app.project.alloc_id();
        let track = &mut app.project.tracks[t];
        let left = track.arrangement.iter_mut().find(|x| x.id == id).unwrap();
        left.length = at - c.start;
        left.fade_out = 0.0;
        let mut right = c.clone();
        right.id = right_id;
        right.start = at;
        right.length = c.end() - at;
        right.offset = c.offset + (at - c.start);
        right.fade_in = 0.0;
        track.arrangement.push(right);
        track.arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
        new_sel.push((t, id));
        new_sel.push((t, right_id));
    }
    app.sel.arr_clips = new_sel;
    app.label("Split");
}

pub fn duplicate_selected(app: &mut XdawApp) {
    let sel: Vec<ArrangementClip> = app
        .sel
        .arr_clips
        .iter()
        .filter_map(|&(t, id)| app.project.tracks.get(t)?.arrangement.iter().find(|c| c.id == id).cloned())
        .collect();
    if sel.is_empty() {
        return;
    }
    let start = sel.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let end = sel.iter().map(|c| c.end()).fold(0.0, f64::max);
    let shift = end - start;
    let pairs = app.sel.arr_clips.clone();
    let mut new_sel = vec![];
    for ((t, _), c) in pairs.into_iter().zip(sel) {
        let mut copy = c;
        copy.id = app.project.alloc_id();
        copy.start += shift;
        new_sel.push((t, copy.id));
        let mut next = app.project.next_id;
        app.project.tracks[t].clear_arrangement_range(copy.start, copy.end(), || {
            next += 1;
            next
        });
        app.project.next_id = next + 1;
        app.project.tracks[t].arrangement.push(copy);
        app.project.tracks[t].arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    app.sel.arr_clips = new_sel;
    app.label("Duplicate");
}

pub fn paste(app: &mut XdawApp) {
    let items = app.clipboard.arrangement.clone();
    let first = items.iter().map(|(_, c)| c.start).fold(f64::INFINITY, f64::min);
    let mut new_sel = vec![];
    for (t, mut c) in items {
        if t >= app.project.tracks.len() || c.clip.is_midi() != (app.project.tracks[t].kind == TrackKind::Midi) {
            continue;
        }
        c.id = app.project.alloc_id();
        c.start = app.arr.cursor + (c.start - first);
        new_sel.push((t, c.id));
        app.project.tracks[t].arrangement.push(c);
        app.project.tracks[t].arrangement.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    app.sel.arr_clips = new_sel;
    app.label("Paste");
    let _ = song_end(&app.project);
}
