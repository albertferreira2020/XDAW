//! XDAW's own visual identity: graphite panels, teal accent, compact controls.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(38, 40, 44);
pub const PANEL: Color32 = Color32::from_rgb(48, 50, 55);
pub const PANEL_DARK: Color32 = Color32::from_rgb(30, 31, 34);
pub const SLOT: Color32 = Color32::from_rgb(58, 60, 66);
pub const GRID: Color32 = Color32::from_rgb(70, 72, 78);
pub const TEXT: Color32 = Color32::from_rgb(220, 222, 226);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 152, 158);
pub const ACCENT: Color32 = Color32::from_rgb(63, 193, 176);
pub const PLAYING: Color32 = Color32::from_rgb(110, 220, 110);
pub const QUEUED: Color32 = Color32::from_rgb(240, 200, 80);
pub const RECORD: Color32 = Color32::from_rgb(235, 70, 70);
pub const SELECT: Color32 = Color32::from_rgb(255, 255, 255);
pub const METER_LO: Color32 = Color32::from_rgb(80, 200, 120);
pub const METER_HI: Color32 = Color32::from_rgb(240, 190, 60);

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn dim(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb((c.r() as f32 * f) as u8, (c.g() as f32 * f) as u8, (c.b() as f32 * f) as u8)
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = PANEL_DARK;
    v.faint_bg_color = BG;
    v.selection.bg_fill = ACCENT.linear_multiply(0.6);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.corner_radius = CornerRadius::same(2);
    v.widgets.hovered.corner_radius = CornerRadius::same(2);
    v.widgets.active.corner_radius = CornerRadius::same(2);
    v.override_text_color = Some(TEXT);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(5.0, 3.0);
        s.spacing.button_padding = egui::vec2(5.0, 1.0);
        s.spacing.interact_size.y = 18.0;
    });
}
