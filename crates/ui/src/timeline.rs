//! Horizontal zoom and scrolling shared by the Arrangement, the piano roll and
//! the sample editor.
//!
//! Gestures (own spec, modelled on Live 10 §6.2 "Navigating the Arrangement"):
//! - pinch or Cmd+wheel: zoom around the pointer;
//! - horizontal wheel or Shift+wheel: scroll;
//! - scroll bar: drag the thumb to scroll, drag its ends to zoom, click the
//!   trough to jump, wheel over it to scroll;
//! - beat ruler: drag vertically to zoom (down = in), horizontally to scroll.

use crate::theme;
use egui::{pos2, vec2, Color32, CursorIcon, Rect, Sense, Ui};

/// Horizontal view: zoom in pixels per beat and the first visible beat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HView {
    pub px_per_beat: f32,
    pub scroll_beats: f64,
    pub min_ppb: f32,
    pub max_ppb: f32,
}

impl HView {
    pub const fn new(px_per_beat: f32, min_ppb: f32, max_ppb: f32) -> Self {
        Self { px_per_beat, scroll_beats: 0.0, min_ppb, max_ppb }
    }

    pub fn x(&self, rect: Rect, beat: f64) -> f32 {
        rect.left() + ((beat - self.scroll_beats) as f32) * self.px_per_beat
    }

    pub fn beat(&self, rect: Rect, x: f32) -> f64 {
        self.scroll_beats + ((x - rect.left()) / self.px_per_beat) as f64
    }

    pub fn visible_beats(&self, width: f32) -> f64 {
        (width / self.px_per_beat) as f64
    }

    /// Zooms by `factor`, keeping the beat under `anchor_px` (offset from the
    /// view's left edge) in place.
    pub fn zoom_at(&mut self, factor: f32, anchor_px: f32) {
        let anchor = self.scroll_beats + (anchor_px / self.px_per_beat) as f64;
        self.px_per_beat = (self.px_per_beat * factor).clamp(self.min_ppb, self.max_ppb);
        self.scroll_beats = (anchor - (anchor_px / self.px_per_beat) as f64).max(0.0);
    }

    /// Shows `[from, to)` across `width` pixels.
    pub fn fit(&mut self, from: f64, to: f64, width: f32) {
        let span = (to - from).max(1e-3);
        self.px_per_beat = (width / span as f32).clamp(self.min_ppb, self.max_ppb);
        let shown = self.visible_beats(width);
        self.scroll_beats = (from - (shown - span).max(0.0) * 0.5).max(0.0);
    }

    pub fn scroll_by_px(&mut self, dx: f32) {
        self.scroll_beats = (self.scroll_beats - (dx / self.px_per_beat) as f64).max(0.0);
    }

    /// Wheel and pinch gestures over `area` while it is hovered. Returns the
    /// vertical wheel delta that was not used (for vertical scrolling).
    pub fn wheel(&mut self, ui: &Ui, hovered: bool, area: Rect) -> f32 {
        if !hovered {
            return 0.0;
        }
        let (scroll, zoom, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
        let anchor = hover.map(|p| (p.x - area.left()).clamp(0.0, area.width())).unwrap_or(0.0);
        if zoom != 1.0 {
            self.zoom_at(zoom, anchor);
            return 0.0;
        }
        if mods.command && scroll.y != 0.0 {
            self.zoom_at((1.0 + scroll.y * 0.003).clamp(0.5, 2.0), anchor);
            return 0.0;
        }
        if scroll.x != 0.0 {
            self.scroll_by_px(scroll.x);
            return scroll.y;
        }
        if mods.shift && scroll.y != 0.0 {
            self.scroll_by_px(scroll.y);
            return 0.0;
        }
        scroll.y
    }
}

/// Live-style ruler gesture: vertical drag zooms around the press point,
/// horizontal drag scrolls. Call with the ruler's drag response.
pub fn ruler_zoom(view: &mut HView, ui: &Ui, resp: &egui::Response, area: Rect) {
    let id = resp.id.with("ruler_zoom");
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            ui.data_mut(|d| d.insert_temp(id, p.x - area.left()));
        }
    }
    if resp.dragged() {
        let anchor = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
        let delta = resp.drag_delta();
        if delta.y != 0.0 {
            view.zoom_at((1.0 + delta.y * 0.01).clamp(0.5, 2.0), anchor);
        }
        if delta.x != 0.0 {
            view.scroll_by_px(delta.x);
        }
        ui.ctx().set_cursor_icon(CursorIcon::ZoomIn);
    }
}

pub const SCROLLBAR_H: f32 = 14.0;

/// Horizontal scroll bar for a view of `width` pixels over `content` beats.
/// The thumb's ends resize the visible range (zoom). Returns true if changed.
pub fn scrollbar(ui: &mut Ui, rect: Rect, id_salt: &str, view: &mut HView, width: f32, content: f64) -> bool {
    let before = *view;
    let visible = view.visible_beats(width);
    let total = content.max(view.scroll_beats + visible).max(visible).max(1e-3);
    let track_w = rect.width().max(1.0);
    let to_x = |b: f64| rect.left() + (b / total) as f32 * track_w;
    let thumb = Rect::from_min_max(
        pos2(to_x(view.scroll_beats), rect.top() + 2.0),
        pos2(to_x(view.scroll_beats + visible).max(to_x(view.scroll_beats) + 16.0).min(rect.right()), rect.bottom() - 2.0),
    );
    let id = ui.id().with(("hscroll", id_salt));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::PANEL_DARK);
    let hot = resp.hovered() || resp.dragged();
    p.rect_filled(thumb, 3.0, if hot { theme::dim(theme::ACCENT, 0.9) } else { theme::GRID });
    for x in [thumb.left() + 3.0, thumb.right() - 3.0] {
        p.line_segment([pos2(x, thumb.top() + 3.0), pos2(x, thumb.bottom() - 3.0)], (1.0, Color32::from_white_alpha(70)));
    }

    // 0 = move, 1 = left edge, 2 = right edge.
    let mode_id = id.with("mode");
    let edge = 5.0;
    let zone_at = |x: f32| {
        if (x - thumb.left()).abs() <= edge {
            1u8
        } else if (x - thumb.right()).abs() <= edge {
            2
        } else {
            0
        }
    };
    if let Some(h) = resp.hover_pos() {
        if thumb.expand(edge).contains(h) && zone_at(h.x) != 0 {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
    }
    if resp.drag_started() {
        if let Some(pt) = resp.interact_pointer_pos() {
            let mode = if thumb.expand(edge).contains(pt) { zone_at(pt.x) } else { 0 };
            if mode == 0 && !thumb.contains(pt) {
                // Pressed in the trough: centre the thumb there, then drag it.
                let b = ((pt.x - rect.left()) / track_w) as f64 * total;
                view.scroll_beats = (b - visible * 0.5).max(0.0);
            }
            ui.data_mut(|d| d.insert_temp(mode_id, mode));
        }
    }
    if resp.dragged() {
        let mode = ui.data(|d| d.get_temp::<u8>(mode_id)).unwrap_or(0);
        let db = (resp.drag_delta().x / track_w) as f64 * total;
        match mode {
            1 => {
                let end = view.scroll_beats + visible;
                let start = (view.scroll_beats + db).clamp(0.0, end - 0.25);
                view.px_per_beat = (width / (end - start) as f32).clamp(view.min_ppb, view.max_ppb);
                view.scroll_beats = (end - view.visible_beats(width)).max(0.0);
            }
            2 => {
                let end = (view.scroll_beats + visible + db).max(view.scroll_beats + 0.25);
                view.px_per_beat = (width / (end - view.scroll_beats) as f32).clamp(view.min_ppb, view.max_ppb);
            }
            _ => view.scroll_beats = (view.scroll_beats + db).max(0.0),
        }
    } else if resp.clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            if !thumb.contains(pt) {
                let b = ((pt.x - rect.left()) / track_w) as f64 * total;
                view.scroll_beats = (b - visible * 0.5).max(0.0);
            }
        }
    }
    if resp.hovered() {
        let s = ui.input(|i| i.smooth_scroll_delta);
        let d = if s.x != 0.0 { s.x } else { s.y };
        if d != 0.0 {
            view.scroll_by_px(d);
        }
    }
    let resp = resp.on_hover_text("Drag to scroll · drag the ends to zoom · click to jump");
    let _ = resp;
    *view != before
}

/// Allocates a full-width scroll bar row at the current cursor.
pub fn scrollbar_row(ui: &mut Ui, id_salt: &str, view: &mut HView, view_left: f32, view_width: f32, content: f64) -> bool {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), SCROLLBAR_H), Sense::hover());
    let rect = Rect::from_min_max(pos2(view_left.max(row.left()), row.top()), pos2(row.right(), row.bottom()));
    scrollbar(ui, rect, id_salt, view, view_width, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor_beat() {
        let mut v = HView::new(20.0, 2.0, 400.0);
        v.scroll_beats = 8.0;
        let anchor_px = 100.0; // beat 13
        v.zoom_at(2.0, anchor_px);
        assert_eq!(v.px_per_beat, 40.0);
        assert!((v.scroll_beats + (anchor_px / v.px_per_beat) as f64 - 13.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_clamped_and_never_scrolls_negative() {
        let mut v = HView::new(20.0, 2.0, 400.0);
        v.zoom_at(1000.0, 0.0);
        assert_eq!(v.px_per_beat, 400.0);
        v.zoom_at(0.0001, 500.0);
        assert_eq!(v.px_per_beat, 2.0);
        assert!(v.scroll_beats >= 0.0);
    }

    #[test]
    fn fit_shows_range() {
        let mut v = HView::new(20.0, 2.0, 400.0);
        v.fit(16.0, 32.0, 800.0);
        assert_eq!(v.px_per_beat, 50.0);
        assert_eq!(v.scroll_beats, 16.0);
    }
}
