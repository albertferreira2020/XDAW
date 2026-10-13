//! Compact mixer column beside the Arrangement lanes: one strip per track row.

use crate::app::XdawApp;
use crate::mixer::{db_label, db_to_fader, fader_to_db, meter, toggle};
use crate::theme;
use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Ui};
use xdaw_project::{ParamTarget, TrackKind, TrackRef};

pub const STRIP_W: f32 = 156.0;

/// Horizontal bar editing a normalized value; double-click resets. Returns true when changed.
fn hbar(ui: &mut Ui, rect: Rect, id: egui::Id, value: &mut f32, default: f32, bipolar: bool, color: Color32) -> bool {
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let mut changed = false;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        *value = (*value + resp.drag_delta().x / rect.width() * fine).clamp(0.0, 1.0);
        changed = true;
    }
    if resp.double_clicked() {
        *value = default;
        changed = true;
    }
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, theme::PANEL_DARK);
    let x = rect.left() + rect.width() * *value;
    let from = if bipolar { rect.center().x } else { rect.left() };
    p.rect_filled(Rect::from_min_max(pos2(from.min(x), rect.top()), pos2(from.max(x), rect.bottom())), 2.0, color.linear_multiply(0.7));
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, theme::TEXT));
    changed
}

/// Draws the strip of track `t` inside `rect` (the row's height decides how much fits).
pub fn cell(app: &mut XdawApp, ui: &mut Ui, t: usize, rect: Rect) {
    let r = TrackRef::Track(t);
    let tr = app.project.tracks[t].clone();
    let ix = XdawApp::track_ix(r);
    let peaks = app.meter(ix);
    let clipped = app.rt.status.track(ix).clipped.load(std::sync::atomic::Ordering::Relaxed);
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::dim(theme::PANEL_DARK, 1.15));
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, theme::PANEL_DARK));
    let inner = rect.shrink2(vec2(4.0, 2.0));
    let meter_w = 10.0;
    let ctrl = Rect::from_min_max(inner.min, pos2(inner.right() - meter_w - 3.0, inner.bottom()));
    let meter_rect = Rect::from_min_max(pos2(inner.right() - meter_w, inner.top()), inner.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(meter_rect), |ui| {
        meter(ui, peaks, clipped, meter_rect.size());
    });
    let tall = rect.height() >= 40.0;
    // Row 1: activator, solo, arm, sends.
    let row1 = Rect::from_min_size(ctrl.min, vec2(ctrl.width(), 16.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(row1).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if toggle(ui, !tr.mixer.mute, &(t + 1).to_string(), theme::QUEUED, 20.0).on_hover_text("Track Activator").clicked() {
            app.project.tracks[t].mixer.mute = !tr.mixer.mute;
            app.label("Mute");
        }
        if toggle(ui, tr.mixer.solo, "S", theme::ACCENT, 16.0).on_hover_text("Solo (Cmd: add to solo)").clicked() {
            let exclusive = !ui.input(|i| i.modifiers.command);
            if exclusive {
                for x in app.project.tracks.iter_mut() {
                    x.mixer.solo = false;
                }
            }
            app.project.tracks[t].mixer.solo = !tr.mixer.solo;
            app.label("Solo");
        }
        if matches!(tr.kind, TrackKind::Audio | TrackKind::Midi) && toggle(ui, tr.mixer.arm, "R", theme::RECORD, 16.0).on_hover_text("Arm recording").clicked() {
            app.project.tracks[t].mixer.arm = !tr.mixer.arm;
            app.label("Arm");
        }
        if !app.project.returns.is_empty() && tr.kind != TrackKind::Master {
            ui.menu_button(egui::RichText::new("Snd").size(10.0), |ui| {
                for i in 0..app.project.returns.len() {
                    let mut db = app.project.tracks[t].mixer.sends_db[i];
                    ui.horizontal(|ui| {
                        ui.label(format!("Send {}", (b'A' + i as u8) as char));
                        if ui.add(egui::Slider::new(&mut db, xdaw_project::SEND_OFF_DB..=6.0).suffix(" dB")).changed() {
                            app.project.tracks[t].mixer.sends_db[i] = db;
                            app.label("Send");
                            app.record_automation(r, ParamTarget::Send(i), db);
                        }
                    });
                }
            });
        }
        if !tall {
            ui.label(egui::RichText::new(db_label(tr.mixer.volume_db)).size(9.0).color(theme::TEXT_DIM));
        }
    });
    if !tall {
        return;
    }
    // Row 2: volume.
    let row2 = Rect::from_min_size(pos2(ctrl.left(), ctrl.top() + 19.0), vec2(ctrl.width() - 34.0, 11.0));
    let mut n = db_to_fader(tr.mixer.volume_db);
    if hbar(ui, row2, ui.id().with(("strip_vol", t)), &mut n, db_to_fader(0.0), false, theme::rgb(tr.color)) {
        let db = fader_to_db(n);
        app.project.tracks[t].mixer.volume_db = db;
        app.label("Volume");
        app.record_automation(r, ParamTarget::Volume, db);
    }
    p.text(pos2(row2.right() + 3.0, row2.center().y), egui::Align2::LEFT_CENTER, db_label(tr.mixer.volume_db), egui::FontId::proportional(9.5), theme::TEXT_DIM);
    // Row 3: pan.
    if rect.height() >= 52.0 {
        let row3 = Rect::from_min_size(pos2(ctrl.left(), ctrl.top() + 33.0), vec2(ctrl.width() - 34.0, 11.0));
        let mut pan = (tr.mixer.pan + 1.0) / 2.0;
        if hbar(ui, row3, ui.id().with(("strip_pan", t)), &mut pan, 0.5, true, theme::TEXT_DIM) {
            let v = pan * 2.0 - 1.0;
            app.project.tracks[t].mixer.pan = v;
            app.label("Pan");
            app.record_automation(r, ParamTarget::Pan, v);
        }
        let label = if tr.mixer.pan.abs() < 0.01 { "C".to_string() } else if tr.mixer.pan < 0.0 { format!("{:.0}L", -tr.mixer.pan * 50.0) } else { format!("{:.0}R", tr.mixer.pan * 50.0) };
        p.text(pos2(row3.right() + 3.0, row3.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(9.5), theme::TEXT_DIM);
    }
}
