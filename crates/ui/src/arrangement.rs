//! Arrangement View: multitrack timeline with clips, ruler, loop and automation.

use crate::app::{ClipLoc, Detail, XdawApp};
use crate::browser::{self, BrowserItem};
use crate::arr_strip;
use crate::theme;
use crate::timeline::{self, HView};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use std::collections::HashMap;
use xdaw_engine::protocol::Command;
use xdaw_project::*;
use xdaw_sequencer::Beats;

const HEADER_W: f32 = 150.0;
const RULER_H: f32 = 46.0;
const TIME_H: f32 = 12.0;
const FOLD_H: f32 = 22.0;

fn ruler_h(app: &XdawApp) -> f32 {
    RULER_H + if app.arr.time_ruler { TIME_H } else { 0.0 }
}
const EDGE: f32 = 6.0;

pub struct ArrangementState {
    pub view: HView,
    pub snap: f64,
    pub cursor: Beats,
    pub lane_h: f32,
    /// Automation lane shown per track id.
    pub lane: HashMap<Id, ParamTarget>,
    /// Timeline width in the last frame (for keyboard zoom and the scroll bar).
    pub width: f32,
    /// The view moved this frame because of Follow or zoom, not a manual scroll.
    following: bool,
    drag: Option<Drag>,
    /// Mixer column beside the lanes.
    pub show_mixer: bool,
    /// Second ruler in minutes:seconds.
    pub time_ruler: bool,
    /// Range being dragged on a take lane `(track, lane, anchor)`.
    comp_drag: Option<(usize, usize, Beats)>,
    /// Drawing tool for automation lanes and clip envelopes.
    pub tool: Tool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    /// Click to add points, drag to move them.
    #[default]
    Select,
    /// Pencil: paint stair steps on the grid.
    Draw,
    /// Drag a straight ramp between two points.
    Line,
    /// Drag over points to delete them.
    Erase,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Draw, Tool::Line, Tool::Erase];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Draw => "Draw",
            Tool::Line => "Line",
            Tool::Erase => "Erase",
        }
    }
}

impl Default for ArrangementState {
    fn default() -> Self {
        Self { view: HView::new(18.0, 0.5, 600.0), snap: 1.0, cursor: 0.0, lane_h: 56.0, lane: HashMap::new(), width: 800.0, following: false, drag: None, show_mixer: true, time_ruler: true, comp_drag: None, tool: Tool::Select }
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
    /// Automation drawing: pencil, line or eraser over a lane.
    Draw { track: usize, target: ParamTarget, tool: Tool, anchor: Beats, anchor_val: f32, last: Beats, before: Vec<AutomationPoint> },
}

#[derive(Clone, Debug)]
struct Drag {
    kind: DragKind,
    start_beat: Beats,
}

impl ArrangementState {
    fn x(&self, rect: Rect, beat: Beats) -> f32 {
        self.view.x(rect, beat)
    }

    fn beat(&self, rect: Rect, x: f32) -> Beats {
        self.view.beat(rect, x)
    }

    /// Grid step in beats: the chosen value, or (Auto) the finest power-of-two
    /// division that keeps lines at least 12 px apart.
    pub fn grid_step(&self) -> f64 {
        if self.snap >= 0.0 {
            return self.snap;
        }
        let mut step = 1.0 / 16.0;
        while (step * self.view.px_per_beat as f64) < 12.0 && step < 64.0 {
            step *= 2.0;
        }
        step
    }

    fn snap(&self, b: Beats, bypass: bool) -> Beats {
        let step = self.grid_step();
        if step <= 0.0 || bypass { b } else { (b / step).round() * step }
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
        ParamTarget::Device { device, param } => p.track(r).param_desc(device, param).map(|d| (d.min, d.max)).unwrap_or((0.0, 1.0)),
    }
}

fn target_norm(p: &Project, r: TrackRef, target: ParamTarget, v: f32) -> f32 {
    match target {
        ParamTarget::Volume | ParamTarget::Send(_) => crate::mixer::db_to_fader(v),
        ParamTarget::Device { device, param } => p.track(r).param_desc(device, param).map(|d| d.to_normalized(v)).unwrap_or(0.0),
        ParamTarget::Pan => {
            let (lo, hi) = target_range(p, r, target);
            (v - lo) / (hi - lo)
        }
    }
}

fn target_from_norm(p: &Project, r: TrackRef, target: ParamTarget, n: f32) -> f32 {
    match target {
        ParamTarget::Volume | ParamTarget::Send(_) => crate::mixer::fader_to_db(n),
        ParamTarget::Device { device, param } => p.track(r).param_desc(device, param).map(|d| d.from_normalized(n)).unwrap_or(n),
        ParamTarget::Pan => n * 2.0 - 1.0,
    }
}

pub fn target_name(p: &Project, r: TrackRef, target: ParamTarget) -> String {
    match target {
        ParamTarget::Volume => "Volume".into(),
        ParamTarget::Pan => "Pan".into(),
        ParamTarget::Send(i) => format!("Send {}", (b'A' + i as u8) as char),
        ParamTarget::Device { device, param } => p.track(r).param_label(device, param).unwrap_or_else(|| "Missing device".into()),
    }
}

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Snap");
        egui::ComboBox::from_id_salt("snap").width(70.0).selected_text(snap_label(app.arr.snap)).show_ui(ui, |ui| {
            for (v, l) in SNAPS {
                ui.selectable_value(&mut app.arr.snap, *v, *l);
            }
        });
        ui.separator();
        ui.label("Zoom");
        if ui.small_button("−").on_hover_text("Zoom out (−)").clicked() {
            zoom_step(app, 0.8);
        }
        let mut z = app.arr.view.px_per_beat;
        let slider = egui::Slider::new(&mut z, app.arr.view.min_ppb..=app.arr.view.max_ppb).logarithmic(true).show_value(false);
        if ui.add_sized(vec2(90.0, 16.0), slider).on_hover_text("Horizontal zoom · pinch or Cmd+wheel over the timeline").changed() {
            zoom_step(app, z / app.arr.view.px_per_beat);
        }
        if ui.small_button("+").on_hover_text("Zoom in (+)").clicked() {
            zoom_step(app, 1.25);
        }
        if ui.small_button("Sel").on_hover_text("Zoom to selection, or to the loop (Z)").clicked() {
            zoom_to_selection(app);
        }
        if ui.small_button("All").on_hover_text("Zoom to the whole song (Shift+Z)").clicked() {
            zoom_to_song(app);
        }
        ui.separator();
        ui.label("Height");
        ui.add_sized(vec2(60.0, 16.0), egui::Slider::new(&mut app.arr.lane_h, LANE_MIN..=LANE_MAX).show_value(false))
            .on_hover_text("Track height · Alt+wheel over the timeline");
        ui.separator();
        ui.label(egui::RichText::new(format!("Insert marker {:.2}", app.arr.cursor)).color(theme::TEXT_DIM));
        if ui.small_button("Split at marker (Cmd+E)").clicked() {
            split_selected(app);
        }
        ui.checkbox(&mut app.follow, "Follow");
        ui.separator();
        ui.checkbox(&mut app.arr.show_mixer, "Mixer").on_hover_text("Mixer column beside the tracks");
        ui.checkbox(&mut app.arr.time_ruler, "Time").on_hover_text("Second ruler in minutes:seconds");
        ui.separator();
        ui.label("Draw");
        for t in Tool::ALL {
            ui.selectable_value(&mut app.arr.tool, t, t.label()).on_hover_text(match t {
                Tool::Select => "Automation: Cmd/double-click adds a point, drag moves it",
                Tool::Draw => "Automation: paint steps on the grid",
                Tool::Line => "Automation: drag a straight ramp",
                Tool::Erase => "Automation: drag over points to delete them",
            });
        }
        if ui.small_button("+ Locator").on_hover_text("Add a locator at the insert marker (L)").clicked() {
            add_locator(app);
        }
    });
    let before = app.arr.view;
    let (rows, rows_h) = layout(app);
    let rh = ruler_h(app);
    let total_h = rh + rows_h + DROP_H + 8.0;
    let left = ui.max_rect().left();
    let area_h = (ui.available_height() - timeline::SCROLLBAR_H - 4.0).max(60.0);
    let strip_w = if app.arr.show_mixer { arr_strip::STRIP_W } else { 0.0 };
    egui::ScrollArea::vertical().id_salt("arr_scroll").max_height(area_h).auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width();
        let (outer, resp) = ui.allocate_exact_size(vec2(width, total_h.max(area_h)), Sense::click_and_drag());
        let timeline = Rect::from_min_max(pos2(outer.left() + HEADER_W, outer.top()), pos2(outer.right() - strip_w, outer.bottom()));
        app.arr.width = timeline.width();
        handle_wheel(app, ui, &resp, timeline);
        follow_playhead(app, timeline);
        draw_ruler(app, ui, timeline);
        for row in &rows {
            let top = outer.top() + rh + row.top;
            let header = Rect::from_min_size(pos2(outer.left(), top), vec2(HEADER_W, row.h));
            let lane = Rect::from_min_max(pos2(timeline.left(), top), pos2(timeline.right(), top + row.h));
            match row.kind {
                RowKind::Main => {
                    track_header(app, ui, row.t, header);
                    track_lane(app, ui, row.t, lane);
                    if strip_w > 0.0 {
                        arr_strip::cell(app, ui, row.t, Rect::from_min_max(pos2(timeline.right(), top), pos2(outer.right(), top + row.h)));
                    }
                }
                RowKind::Take(li) => {
                    take_header(app, ui, row.t, li, header);
                    take_lane(app, ui, row.t, li, lane);
                }
            }
        }
        let drop_top = outer.top() + rh + rows_h + 4.0;
        drop_zone(app, ui, Rect::from_min_max(pos2(outer.left(), drop_top), pos2(timeline.right(), drop_top + DROP_H)), timeline);
        // Playhead and insert marker over everything.
        let p = ui.painter_at(timeline);
        let x = app.arr.x(timeline, app.beat());
        p.line_segment([pos2(x, timeline.top()), pos2(x, timeline.bottom())], Stroke::new(1.5, theme::ACCENT));
        let cx = app.arr.x(timeline, app.arr.cursor);
        p.line_segment([pos2(cx, timeline.top() + rh), pos2(cx, timeline.bottom())], Stroke::new(1.0, Color32::from_white_alpha(90)));
        if ui.input(|i| i.pointer.any_released()) {
            app.arr.drag = None;
            app.arr.comp_drag = None;
        }
    });
    let content = content_end(app) + 8.0 * app.project.transport.time_signature.bar_beats();
    let width = app.arr.width;
    let bar = Rect::from_min_size(pos2(left + HEADER_W, ui.cursor().top()), vec2(width, timeline::SCROLLBAR_H));
    ui.allocate_rect(Rect::from_min_size(pos2(left, bar.top()), vec2(ui.available_width(), timeline::SCROLLBAR_H)), Sense::hover());
    timeline::scrollbar(ui, bar, "arr", &mut app.arr.view, width, content);
    // Scrolling by hand while playing stops Follow, as in Live.
    let v = app.arr.view;
    if app.playing() && app.follow && v.px_per_beat == before.px_per_beat && v.scroll_beats != before.scroll_beats && !app.arr.following {
        app.follow = false;
    }
    app.arr.following = false;
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum RowKind {
    Main,
    Take(usize),
}

#[derive(Clone, Copy, Debug)]
struct Row {
    t: usize,
    kind: RowKind,
    top: f32,
    h: f32,
}

/// True when an ancestor group of track `t` is folded.
fn hidden(app: &XdawApp, t: usize) -> bool {
    let mut cur = app.project.tracks[t].group;
    for _ in 0..8 {
        let Some(g) = cur else { return false };
        let Some(gt) = app.project.tracks.iter().find(|x| x.id == g) else { return false };
        if gt.folded {
            return true;
        }
        cur = gt.group;
    }
    false
}

/// Nesting depth of a track (number of ancestor groups).
fn depth(app: &XdawApp, t: usize) -> usize {
    let mut d = 0;
    let mut cur = app.project.tracks[t].group;
    while let Some(g) = cur {
        let Some(gt) = app.project.tracks.iter().find(|x| x.id == g) else { break };
        d += 1;
        cur = gt.group;
        if d >= 8 {
            break;
        }
    }
    d
}

fn take_h(app: &XdawApp) -> f32 {
    (app.arr.lane_h * 0.6).max(28.0)
}

/// Visible rows (folded groups hide their members; take lanes follow their track).
fn layout(app: &XdawApp) -> (Vec<Row>, f32) {
    let mut rows = vec![];
    let mut y = 0.0;
    for (t, tr) in app.project.tracks.iter().enumerate() {
        if hidden(app, t) {
            continue;
        }
        let h = if tr.folded { FOLD_H } else { app.arr.lane_h };
        rows.push(Row { t, kind: RowKind::Main, top: y, h });
        y += h;
        if tr.show_takes && !tr.folded {
            for li in (0..tr.take_lanes.len()).rev() {
                let h = take_h(app);
                rows.push(Row { t, kind: RowKind::Take(li), top: y, h });
                y += h;
            }
        }
    }
    (rows, y)
}

const SNAPS: &[(f64, &str)] = &[
    (-1.0, "Auto"),
    (0.0, "Off"),
    (4.0, "1 Bar"),
    (2.0, "1/2"),
    (1.0, "1/4"),
    (2.0 / 3.0, "1/4T"),
    (0.5, "1/8"),
    (1.0 / 3.0, "1/8T"),
    (0.25, "1/16"),
    (1.0 / 6.0, "1/16T"),
    (0.125, "1/32"),
];

const DROP_H: f32 = 48.0;
const LANE_MIN: f32 = 24.0;
const LANE_MAX: f32 = 200.0;

/// End of the material the view should be able to reach.
fn content_end(app: &XdawApp) -> Beats {
    let t = &app.project.transport;
    song_end(&app.project).max(t.loop_start + t.loop_length).max(app.arr.cursor).max(app.beat())
}

/// Zooms around the playhead when it is visible, else around the view centre.
pub fn zoom_step(app: &mut XdawApp, factor: f32) {
    let w = app.arr.width.max(1.0);
    let px = (app.arr.view.x(Rect::from_min_size(Pos2::ZERO, vec2(w, 1.0)), app.beat())).clamp(-1.0, w + 1.0);
    let anchor = if (0.0..=w).contains(&px) { px } else { w * 0.5 };
    app.arr.view.zoom_at(factor, anchor);
}

pub fn zoom_to_song(app: &mut XdawApp) {
    let end = content_end(app).max(app.project.transport.time_signature.bar_beats() * 4.0);
    app.arr.view.fit(0.0, end, app.arr.width.max(1.0));
}

pub fn zoom_to_selection(app: &mut XdawApp) {
    let spans: Vec<(Beats, Beats)> = app
        .sel
        .arr_clips
        .iter()
        .filter_map(|&(t, id)| app.project.tracks.get(t)?.arrangement.iter().find(|c| c.id == id).map(|c| (c.start, c.end())))
        .collect();
    let range = if spans.is_empty() {
        let t = &app.project.transport;
        if t.loop_length > 0.0 { Some((t.loop_start, t.loop_start + t.loop_length)) } else { None }
    } else {
        Some((spans.iter().map(|s| s.0).fold(f64::INFINITY, f64::min), spans.iter().map(|s| s.1).fold(0.0, f64::max)))
    };
    match range {
        Some((a, b)) => app.arr.view.fit(a, b, app.arr.width.max(1.0)),
        None => zoom_to_song(app),
    }
}

fn snap_label(s: f64) -> String {
    SNAPS.iter().find(|(v, _)| (*v - s).abs() < 1e-9).map(|(_, l)| l.to_string()).unwrap_or_else(|| format!("{s:.3}"))
}

fn handle_wheel(app: &mut XdawApp, ui: &mut Ui, resp: &egui::Response, timeline: Rect) {
    let hovered = resp.contains_pointer();
    if hovered && ui.input(|i| i.modifiers.alt && i.smooth_scroll_delta.y != 0.0) {
        // Alt+wheel: track height; keep the vertical scroll area still.
        let dy = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y));
        app.arr.lane_h = (app.arr.lane_h * (1.0 + dy * 0.004)).clamp(LANE_MIN, LANE_MAX);
        return;
    }
    let before = app.arr.view;
    app.arr.view.wheel(ui, hovered, timeline);
    if app.arr.view != before && app.arr.view.px_per_beat != before.px_per_beat {
        app.arr.following = true; // zooming is not a manual scroll
    }
}

fn follow_playhead(app: &mut XdawApp, timeline: Rect) {
    if !(app.follow && app.playing()) {
        return;
    }
    let visible = app.arr.view.visible_beats(timeline.width());
    let b = app.beat();
    if b < app.arr.view.scroll_beats || b > app.arr.view.scroll_beats + visible * 0.9 {
        app.arr.view.scroll_beats = (b - visible * 0.1).max(0.0);
        app.arr.following = true;
    }
}

pub fn add_locator(app: &mut XdawApp) {
    let beat = app.arr.cursor.max(0.0);
    app.project.add_locator(beat);
    app.label("Add Locator");
}

fn draw_ruler(app: &mut XdawApp, ui: &mut Ui, timeline: Rect) {
    let rect = Rect::from_min_size(timeline.min, vec2(timeline.width(), ruler_h(app)));
    let loop_rect = Rect::from_min_size(rect.min, vec2(rect.width(), 12.0));
    let loc_rect = Rect::from_min_size(pos2(rect.left(), rect.top() + 34.0), vec2(rect.width(), 12.0));
    let resp = ui.interact(Rect::from_min_size(rect.min, vec2(rect.width(), 34.0)), ui.id().with("ruler"), Sense::click_and_drag());
    let strip = ui.interact(loc_rect, ui.id().with("locator_strip"), Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::PANEL_DARK);
    let sig = app.project.transport.time_signature;
    let bar = sig.bar_beats();
    let visible = (timeline.width() / app.arr.view.px_per_beat) as f64;
    let step_bars = if app.arr.view.px_per_beat * bar as f32 > 40.0 { 1.0 } else if app.arr.view.px_per_beat * bar as f32 > 12.0 { 4.0 } else { 16.0 };
    let first = (app.arr.view.scroll_beats / (bar * step_bars)).floor() as i64;
    let last = ((app.arr.view.scroll_beats + visible) / (bar * step_bars)).ceil() as i64;
    for k in first..=last {
        let b = k as f64 * bar * step_bars;
        let x = app.arr.x(timeline, b);
        p.line_segment([pos2(x, rect.top() + 14.0), pos2(x, rect.top() + 34.0)], Stroke::new(1.0, theme::GRID));
        p.text(pos2(x + 3.0, rect.top() + 14.0), Align2::LEFT_TOP, format!("{}", (b / bar) as i64 + 1), FontId::proportional(10.0), theme::TEXT_DIM);
    }
    // Beat ticks once zoomed in enough.
    if app.arr.view.px_per_beat >= 14.0 {
        let b0 = app.arr.view.scroll_beats.floor() as i64;
        for b in b0..=b0 + visible.ceil() as i64 + 1 {
            let x = app.arr.x(timeline, b as f64);
            p.line_segment([pos2(x, rect.top() + 28.0), pos2(x, rect.top() + 34.0)], Stroke::new(1.0, theme::GRID));
        }
    }
    // Loop brace.
    let t = &app.project.transport;
    let (lx0, lx1) = (app.arr.x(timeline, t.loop_start), app.arr.x(timeline, t.loop_start + t.loop_length));
    let brace = Rect::from_min_max(pos2(lx0, loop_rect.top() + 2.0), pos2(lx1, loop_rect.bottom()));
    p.rect_filled(brace, 2.0, if t.loop_enabled { theme::QUEUED.linear_multiply(0.8) } else { theme::GRID });
    if t.punch_in || t.punch_out {
        let c = theme::RECORD;
        if t.punch_in {
            p.line_segment([pos2(lx0, loop_rect.top()), pos2(lx0, rect.top() + 34.0)], Stroke::new(2.0, c));
        }
        if t.punch_out {
            p.line_segment([pos2(lx1, loop_rect.top()), pos2(lx1, rect.top() + 34.0)], Stroke::new(2.0, c));
        }
    }
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
    // Below the loop strip the ruler zooms (vertical drag) and scrolls (horizontal drag).
    if !matches!(app.arr.drag, Some(Drag { kind: DragKind::Loop { .. }, .. })) {
        timeline::ruler_zoom(&mut app.arr.view, ui, &resp, timeline);
    }
    if resp.hovered() && resp.hover_pos().is_some_and(|p| p.y >= loop_rect.bottom()) && !resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
    }
    if resp.double_clicked() && resp.interact_pointer_pos().is_some_and(|p| p.y < loop_rect.bottom()) {
        app.project.transport.loop_enabled = !app.project.transport.loop_enabled;
    }

    // Locators.
    p.rect_filled(loc_rect, 0.0, theme::dim(theme::PANEL_DARK, 0.85));
    if strip.double_clicked() {
        if let Some(pos) = strip.interact_pointer_pos() {
            app.arr.cursor = app.arr.snap(app.arr.beat(timeline, pos.x), alt).max(0.0);
            add_locator(app);
        }
    }
    let mut moved: Option<(Id, Beats)> = None;
    let mut goto: Option<Beats> = None;
    let mut remove: Option<Id> = None;
    let mut rename: Option<(Id, String)> = None;
    let locators: Vec<Locator> = app.project.locators.clone();
    for l in &locators {
        let x = app.arr.x(timeline, l.beat);
        let w = (l.name.chars().count() as f32 * 5.6 + 10.0).min(160.0);
        let flag = Rect::from_min_size(pos2(x, loc_rect.top()), vec2(w, loc_rect.height())).intersect(loc_rect.expand2(vec2(200.0, 0.0)));
        if x + w < rect.left() || x > rect.right() {
            continue;
        }
        let fr = ui.interact(flag, ui.id().with(("locator", l.id)), Sense::click_and_drag());
        let col = if fr.hovered() { theme::ACCENT } else { theme::dim(theme::ACCENT, 0.7) };
        let pp = p.with_clip_rect(rect);
        pp.line_segment([pos2(x, loc_rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, col));
        pp.rect_filled(flag, 1.0, col);
        pp.text(pos2(x + 4.0, flag.center().y), Align2::LEFT_CENTER, &l.name, FontId::proportional(9.5), Color32::BLACK);
        if fr.clicked() {
            goto = Some(l.beat);
        }
        if fr.dragged() {
            if let Some(pos) = fr.interact_pointer_pos() {
                moved = Some((l.id, app.arr.snap(app.arr.beat(timeline, pos.x), alt).max(0.0)));
            }
        }
        fr.context_menu(|ui| {
            let mut name = l.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                rename = Some((l.id, name));
            }
            if ui.button("Delete locator").clicked() {
                remove = Some(l.id);
                ui.close();
            }
        });
    }
    if let Some(b) = goto {
        app.arr.cursor = b;
        app.rt.send(Command::Locate(b));
    }
    if let Some((id, b)) = moved {
        if let Some(l) = app.project.locators.iter_mut().find(|l| l.id == id) {
            l.beat = b;
        }
        app.project.locators.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        app.label("Move Locator");
    }
    if let Some((id, name)) = rename {
        if let Some(l) = app.project.locators.iter_mut().find(|l| l.id == id) {
            l.name = name;
        }
        app.label("Rename Locator");
    }
    if let Some(id) = remove {
        app.project.remove_locator(id);
        app.label("Delete Locator");
    }

    // Time ruler (minutes:seconds).
    if app.arr.time_ruler {
        let tr = Rect::from_min_size(pos2(rect.left(), rect.top() + 46.0), vec2(rect.width(), TIME_H));
        p.rect_filled(tr, 0.0, theme::dim(theme::PANEL_DARK, 0.7));
        let sec_per_beat = 60.0 / app.project.transport.bpm;
        let ppsec = app.arr.view.px_per_beat as f64 / sec_per_beat;
        let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0].into_iter().find(|s| s * ppsec >= 60.0).unwrap_or(1200.0);
        let t0 = app.arr.view.scroll_beats * sec_per_beat;
        let t1 = t0 + visible * sec_per_beat;
        let mut k = (t0 / step).floor() as i64;
        while (k as f64) * step <= t1 + step {
            let secs = k as f64 * step;
            let x = app.arr.x(timeline, secs / sec_per_beat);
            p.line_segment([pos2(x, tr.top()), pos2(x, tr.bottom())], Stroke::new(1.0, theme::GRID));
            p.text(pos2(x + 3.0, tr.center().y), Align2::LEFT_CENTER, format_time(secs), FontId::proportional(9.5), theme::TEXT_DIM);
            k += 1;
        }
    }
}

fn format_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn track_header(app: &mut XdawApp, ui: &mut Ui, t: usize, rect: Rect) {
    let r = TrackRef::Track(t);
    let resp = ui.interact(rect, ui.id().with(("arr_header", t)), Sense::click());
    let track = &app.project.tracks[t];
    let selected = app.sel.track == Some(r);
    let p = ui.painter_at(rect);
    let indent = depth(app, t) as f32 * 8.0;
    p.rect_filled(rect.shrink(0.5), 0.0, if selected { theme::dim(theme::rgb(track.color), 0.8) } else { theme::dim(theme::rgb(track.color), 0.45) });
    if indent > 0.0 {
        p.rect_filled(Rect::from_min_size(rect.min, vec2(indent, rect.height())), 0.0, theme::dim(theme::rgb(track.color), 0.25));
    }
    let name = if track.kind == TrackKind::Group { format!("[{}]", track.name) } else { track.name.clone() };
    p.text(rect.left_top() + vec2(5.0 + indent, 4.0), Align2::LEFT_TOP, name, FontId::proportional(11.5), theme::TEXT);
    let id = track.id;
    let (folded, has_takes, shows_takes, frozen) = (track.folded, !track.take_lanes.is_empty(), track.show_takes, track.frozen.is_some());
    // Fold arrow, take-lane toggle and freeze badge, right-aligned in the title line.
    let mut bx = rect.right() - 12.0;
    let fold_rect = Rect::from_center_size(pos2(bx, rect.top() + 11.0), vec2(14.0, 14.0));
    let fold = ui.interact(fold_rect, ui.id().with(("fold", t)), Sense::click()).on_hover_text("Fold track (groups hide their members)");
    p.text(fold_rect.center(), Align2::CENTER_CENTER, if folded { "▸" } else { "▾" }, FontId::proportional(12.0), theme::TEXT);
    if fold.clicked() {
        app.project.tracks[t].folded = !folded;
        app.label("Fold Track");
    }
    if has_takes {
        bx -= 16.0;
        let tr = Rect::from_center_size(pos2(bx, rect.top() + 11.0), vec2(14.0, 14.0));
        let tk = ui.interact(tr, ui.id().with(("takes", t)), Sense::click()).on_hover_text("Show take lanes");
        p.rect_filled(tr, 2.0, if shows_takes { theme::RECORD } else { theme::SLOT });
        p.text(tr.center(), Align2::CENTER_CENTER, "T", FontId::proportional(10.0), if shows_takes { Color32::BLACK } else { theme::TEXT });
        if tk.clicked() {
            app.project.tracks[t].show_takes = !shows_takes;
            app.label("Show Takes");
        }
    }
    if frozen {
        bx -= 16.0;
        let fr = Rect::from_center_size(pos2(bx, rect.top() + 11.0), vec2(14.0, 14.0));
        p.rect_filled(fr, 2.0, theme::ACCENT);
        p.text(fr.center(), Align2::CENTER_CENTER, "F", FontId::proportional(10.0), Color32::BLACK);
        ui.interact(fr, ui.id().with(("frozen", t)), Sense::hover()).on_hover_text("Frozen: devices are rendered to audio");
    }
    if resp.clicked() {
        app.select_track(r);
    }
    track_menu(app, &resp, t);
    if rect.height() < 44.0 {
        return;
    }
    let track = &app.project.tracks[t];
    // Automation lane chooser.
    let current = app.arr.lane.get(&id).copied();
    let mut choices: Vec<(Option<ParamTarget>, String)> = vec![(None, "No automation".into()), (Some(ParamTarget::Volume), "Volume".into()), (Some(ParamTarget::Pan), "Pan".into())];
    for i in 0..app.project.returns.len() {
        choices.push((Some(ParamTarget::Send(i)), format!("Send {}", (b'A' + i as u8) as char)));
    }
    choices.extend(track.param_targets().into_iter().map(|(t, name)| (Some(t), name)));
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
    let visible = (lane.width() / app.arr.view.px_per_beat) as f64;
    let first = (app.arr.view.scroll_beats / bar).floor() as i64;
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
        if let (Some(target), true) = (lane_target, app.arr.tool != Tool::Select) {
            let (anchor, val) = (start_beat, pointer.map(|p| value_at_y(app, r, target, lane, p.y)).unwrap_or(0.0));
            let before = app.project.tracks[t].lane(target).map(|l| l.points.clone()).unwrap_or_default();
            app.arr.drag = Some(Drag { kind: DragKind::Draw { track: t, target, tool: app.arr.tool, anchor, anchor_val: val, last: anchor, before }, start_beat });
        } else if let (Some(target), Some(idx)) = (lane_target, point_hit) {
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

    // Drops: browser/Finder files, devices and Session clips.
    if let Some(item) = resp.dnd_hover_payload::<BrowserItem>() {
        painter.rect_stroke(lane.shrink(1.0), 0.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
        if let (BrowserItem::File(_), Some(b)) = (&*item, pointer_beat) {
            let x = app.arr.x(lane, app.arr.snap(b, alt).max(0.0));
            painter.line_segment([pos2(x, lane.top()), pos2(x, lane.bottom())], Stroke::new(2.0, theme::ACCENT));
        }
    }
    if let Some(item) = crate::browser::dnd_release::<BrowserItem>(&resp) {
        match ((*item).clone(), pointer_beat) {
            (BrowserItem::File(path), Some(b)) => drop_files(app, t, app.arr.snap(b, alt).max(0.0), &path),
            (BrowserItem::Device(k), _) => browser::add_device(app, r, k),
            (BrowserItem::Preset(path), _) => browser::load_preset(app, r, &path),
            (BrowserItem::Factory(k, i), _) => browser::load_factory(app, r, k, i),
            _ => {}
        }
    }
    if let Some(drag) = crate::browser::dnd_release::<crate::session::SlotDrag>(&resp) {
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
                follow: Default::default(),
            };
            let id = app.project.alloc_id();
            let start = app.arr.cursor;
            app.project.tracks[t].arrangement.push(ArrangementClip { id, start, length: bar, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
            app.label("Insert MIDI Clip");
            ui.close();
        }
        ui.separator();
        if ui.button("Consolidate (Cmd+J)").clicked() {
            crate::audio_ops::consolidate_selection(app);
            ui.close();
        }
        if ui.button("Reverse").clicked() {
            crate::audio_ops::reverse_selection(app);
            ui.close();
        }
        if ui.button("Crossfade selected neighbours").clicked() {
            crossfade_selected(app);
            ui.close();
        }
        ui.menu_button("Slice to New MIDI Track", |ui| {
            for (mode, label) in crate::audio_ops::SliceMode::ALL {
                if ui.button(label).clicked() {
                    crate::audio_ops::slice_to_midi(app, mode);
                    ui.close();
                }
            }
        });
        if let Some(target) = lane_target {
            if ui.button(format!("Clear {} automation", target_name(&app.project, r, target))).clicked() {
                app.project.tracks[t].automation.retain(|l| l.target != target);
                app.label("Clear Automation");
                ui.close();
            }
        }
    });
}

/// Header of a take lane: its name and a delete button.
fn take_header(app: &mut XdawApp, ui: &mut Ui, t: usize, li: usize, rect: Rect) {
    let Some(lane) = app.project.tracks[t].take_lanes.get(li) else { return };
    let color = theme::rgb(app.project.tracks[t].color);
    let p = ui.painter_at(rect);
    p.rect_filled(rect.shrink(0.5), 0.0, theme::dim(color, 0.3));
    p.text(rect.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, &lane.name, FontId::proportional(10.0), theme::TEXT);
    let x_rect = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(14.0, 14.0));
    let del = ui.interact(x_rect, ui.id().with(("take_del", t, li)), Sense::click()).on_hover_text("Delete this take");
    p.text(x_rect.center(), Align2::CENTER_CENTER, "×", FontId::proportional(13.0), if del.hovered() { theme::RECORD } else { theme::TEXT_DIM });
    if del.clicked() {
        app.project.tracks[t].remove_take_lane(li);
        app.label("Delete Take");
    }
}

/// A take lane: drag across it to make that range the comp (click a clip for its whole range).
fn take_lane(app: &mut XdawApp, ui: &mut Ui, t: usize, li: usize, lane: Rect) {
    let Some(tl) = app.project.tracks[t].take_lanes.get(li).cloned() else { return };
    let resp = ui.interact(lane, ui.id().with(("take_lane", t, li)), Sense::click_and_drag());
    let painter = ui.painter_at(lane);
    painter.rect_filled(lane, 0.0, theme::dim(theme::BG, 0.8));
    painter.line_segment([lane.left_bottom(), lane.right_bottom()], Stroke::new(1.0, theme::PANEL_DARK));
    let comp = app.project.tracks[t].arrangement.clone();
    for c in &tl.clips {
        let (x0, x1) = (app.arr.x(lane, c.start), app.arr.x(lane, c.end()));
        if x1 < lane.left() || x0 > lane.right() {
            continue;
        }
        let rect = Rect::from_min_max(pos2(x0, lane.top() + 2.0), pos2(x1, lane.bottom() - 2.0));
        draw_clip(app, &painter, c, rect);
        // Mark the parts of this take that currently play.
        for k in comp.iter().filter(|k| k.clip.id == c.clip.id && k.start < c.end() && k.end() > c.start) {
            let (a, b) = (app.arr.x(lane, k.start.max(c.start)), app.arr.x(lane, k.end().min(c.end())));
            painter.rect_stroke(Rect::from_min_max(pos2(a, rect.top()), pos2(b, rect.bottom())), 2.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
        }
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let pointer_beat = resp.interact_pointer_pos().or(resp.hover_pos()).map(|p| app.arr.snap(app.arr.beat(lane, p.x), alt).max(0.0));
    if resp.drag_started() {
        if let Some(b) = pointer_beat {
            app.arr.comp_drag = Some((t, li, b));
        }
    }
    if let (Some((ct, cl, anchor)), Some(b)) = (app.arr.comp_drag, pointer_beat) {
        if (ct, cl) == (t, li) {
            let (x0, x1) = (app.arr.x(lane, anchor.min(b)), app.arr.x(lane, anchor.max(b)));
            painter.rect_filled(Rect::from_min_max(pos2(x0, lane.top()), pos2(x1, lane.bottom())), 0.0, theme::ACCENT.linear_multiply(0.25));
            if resp.drag_stopped() {
                comp_range(app, t, li, anchor.min(b), anchor.max(b));
            }
        }
    }
    if resp.clicked() {
        if let Some(b) = pointer_beat {
            if let Some(c) = tl.clips.iter().find(|c| c.start <= b && b < c.end()) {
                comp_range(app, t, li, c.start, c.end());
            }
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
}

/// Makes `[from, to)` of take lane `li` the active comp.
pub fn comp_range(app: &mut XdawApp, t: usize, li: usize, from: Beats, to: Beats) {
    if to - from < 1.0 / 64.0 {
        return;
    }
    let mut next = app.project.next_id;
    let ok = app.project.tracks[t].comp_from_take(li, from, to, || {
        next += 1;
        next - 1
    });
    app.project.next_id = next;
    if ok {
        app.label("Comp Take");
    }
}

/// Track-level edit commands (right-click on the header).
fn track_menu(app: &mut XdawApp, resp: &egui::Response, t: usize) {
    resp.context_menu(|ui| {
        let tr = &app.project.tracks[t];
        let name = tr.name.clone();
        ui.label(egui::RichText::new(name).strong());
        ui.separator();
        if ui.button(if tr.folded { "Unfold" } else { "Fold" }).clicked() {
            app.project.tracks[t].folded = !app.project.tracks[t].folded;
            app.label("Fold Track");
            ui.close();
        }
        let frozen = app.project.tracks[t].frozen.is_some();
        if app.project.tracks[t].kind != TrackKind::Group {
            if frozen {
                if ui.button("Unfreeze Track").clicked() {
                    crate::audio_ops::unfreeze_track(app, t);
                    ui.close();
                }
            } else if ui.button("Freeze Track").on_hover_text("Render the track with its devices; playback uses the audio").clicked() {
                crate::audio_ops::freeze_track(app, t);
                ui.close();
            }
            if ui.button("Flatten Track").on_hover_text("Replace clips and devices by the rendered audio (undoable)").clicked() {
                crate::audio_ops::flatten_track_in_place(app, t);
                ui.close();
            }
        }
        if !app.project.tracks[t].take_lanes.is_empty() && ui.button("Delete all takes").clicked() {
            app.project.tracks[t].take_lanes.clear();
            app.project.tracks[t].show_takes = false;
            app.label("Delete Takes");
            ui.close();
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
        DragKind::Draw { track, target, tool, anchor, anchor_val, last, before } => {
            let Some(p) = pointer else { return };
            let r = TrackRef::Track(*track);
            let v = value_at_y(app, r, *target, lane, p.y);
            let step = match app.arr.grid_step() {
                s if s > 0.0 => s,
                _ => 1.0 / 16.0,
            };
            let beat = pb.max(0.0);
            let l = app.project.tracks[*track].lane_mut(*target);
            match tool {
                Tool::Draw => {
                    // Paint every grid cell between the previous and the current pointer position.
                    let (a, b) = (last.min(beat), last.max(beat));
                    let (c0, c1) = ((a / step).floor() as i64, (b / step).floor() as i64);
                    for c in c0..=c1 {
                        l.draw_cell(c as f64 * step, (c + 1) as f64 * step, v);
                    }
                    app.label("Draw Automation");
                }
                Tool::Line => {
                    l.points = before.clone();
                    l.draw_line(*anchor, *anchor_val, app.arr.snap(beat, alt), v, step);
                    app.label("Draw Automation Line");
                }
                Tool::Erase => {
                    let slack = 3.0 / app.arr.view.px_per_beat as f64;
                    l.erase_range(last.min(beat) - slack, last.max(beat) + slack);
                    app.label("Erase Automation");
                }
                Tool::Select => {}
            }
            if let Some(Drag { kind: DragKind::Draw { last, .. }, .. }) = app.arr.drag.as_mut() {
                *last = beat;
            }
        }
    }
}

/// Value of an automation target for a pointer height inside a lane.
fn value_at_y(app: &XdawApp, r: TrackRef, target: ParamTarget, lane: Rect, y: f32) -> f32 {
    let n = (1.0 - (y - lane.top()) / lane.height()).clamp(0.0, 1.0);
    target_from_norm(&app.project, r, target, n)
}

fn draw_clip(app: &XdawApp, p: &egui::Painter, c: &ArrangementClip, rect: Rect) {
    let color = theme::rgb(c.clip.color);
    p.rect_filled(rect, 2.0, theme::dim(color, 0.55));
    let title = Rect::from_min_size(rect.min, vec2(rect.width(), 12.0));
    p.rect_filled(title, 2.0, color);
    // Keep the name readable when the clip starts left of the view.
    let name_x = title.left().max(p.clip_rect().left()).min(title.right() - 20.0).max(title.left());
    p.with_clip_rect(title.intersect(p.clip_rect())).text(pos2(name_x + 3.0, title.center().y), Align2::LEFT_CENTER, &c.clip.name, FontId::proportional(10.0), Color32::BLACK);
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 12.0), rect.max);
    let ppb = app.arr.view.px_per_beat as f64;
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
                ParamTarget::Device { device, param } => track.param_value(device, param).unwrap_or(0.0),
            };
            p.line_segment([pos2(lane.left(), y(v)), pos2(lane.right(), y(v))], Stroke::new(1.0, Color32::from_rgb(200, 110, 110)));
        }
    }
    p.text(lane.right_top() + vec2(-4.0, 2.0), Align2::RIGHT_TOP, format!("{} — Cmd-click/double-click: add · drag: move · double-click point: delete", target_name(&app.project, r, target)), FontId::proportional(9.5), theme::TEXT_DIM);
    hit
}

/// Drops a file (plus any further files of the same Finder drop) one after another from `at`.
fn drop_files(app: &mut XdawApp, t: usize, at: Beats, first: &std::path::Path) {
    let mut paths = vec![first.to_path_buf()];
    paths.extend(std::mem::take(&mut app.extra_drop));
    let mut at = at;
    for p in &paths {
        if let Some(end) = drop_file(app, t, at, p) {
            at = end;
        }
    }
}

/// Drop area under the last track: creates a track from the dropped item.
fn drop_zone(app: &mut XdawApp, ui: &mut Ui, rect: Rect, timeline: Rect) {
    let resp = ui.interact(rect, ui.id().with("arr_drop_zone"), Sense::hover());
    let hovering = resp.dnd_hover_payload::<BrowserItem>().is_some();
    let p = ui.painter_at(rect);
    p.rect_stroke(rect.shrink(2.0), 3.0, Stroke::new(1.0, if hovering { theme::ACCENT } else { theme::GRID }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, "Drop samples, MIDI files or devices here to create a track", FontId::proportional(11.0), if hovering { theme::ACCENT } else { theme::TEXT_DIM });
    if let Some(item) = crate::browser::dnd_release::<BrowserItem>(&resp) {
        let at = resp
            .hover_pos()
            .or(ui.input(|i| i.pointer.latest_pos()))
            .filter(|p| p.x >= timeline.left())
            .map(|p| app.arr.snap(app.arr.beat(timeline, p.x), false).max(0.0))
            .unwrap_or(app.arr.cursor);
        match (*item).clone() {
            BrowserItem::File(path) => {
                let t = crate::session::new_track_for_file(app, &path);
                drop_files(app, t, at, &path);
            }
            other => crate::session::create_track_from(app, other),
        }
    }
}

/// Places one file on track `t` at `at`; returns the end of the new clip.
fn drop_file(app: &mut XdawApp, t: usize, at: Beats, path: &std::path::Path) -> Option<Beats> {
    // Reuse slot import logic through a temporary clip.
    if app.project.scenes.is_empty() {
        app.project.add_scene("1");
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
        return Some(at + length);
    }
    None
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

/// Crossfades every pair of selected neighbouring audio clips on a track.
pub fn crossfade_selected(app: &mut XdawApp) {
    let len = app.arr.grid_step().clamp(1.0 / 16.0, 0.5).max(0.125);
    let sel = app.sel.arr_clips.clone();
    let mut n = 0;
    let mut err = None;
    for t in 0..app.project.tracks.len() {
        let ids: Vec<Id> = sel.iter().filter(|(tt, _)| *tt == t).map(|(_, id)| *id).collect();
        let mut i = 0;
        while i + 1 < app.project.tracks[t].arrangement.len() {
            let (a, b) = (app.project.tracks[t].arrangement[i].id, app.project.tracks[t].arrangement[i + 1].id);
            if ids.contains(&a) && ids.contains(&b) {
                match app.project.tracks[t].crossfade(i, len) {
                    Ok(_) => n += 1,
                    Err(e) => err = Some(e),
                }
            }
            i += 1;
        }
    }
    if n > 0 {
        app.label("Crossfade");
        app.status_line = format!("Crossfaded {n} join(s)");
    } else {
        app.status_line = format!("Crossfade: {}", err.unwrap_or("select two neighbouring audio clips that touch"));
    }
}
