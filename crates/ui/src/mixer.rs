//! Mixer widgets: fader, knob, meter and the per-track mixer strip.

use crate::app::XdawApp;
use crate::theme;
use egui::{pos2, vec2, Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui};
use xdaw_project::{Monitor, ParamTarget, TrackKind, TrackRef, SEND_OFF_DB};

/// Fader law: 0..1 → -70..+6 dB with more resolution near 0 dB.
pub fn fader_to_db(n: f32) -> f32 {
    let n = n.clamp(0.0, 1.0);
    if n <= 0.0 { SEND_OFF_DB } else { -70.0 + 76.0 * n.powf(0.5) }
}

pub fn db_to_fader(db: f32) -> f32 {
    if db <= SEND_OFF_DB { 0.0 } else { ((db + 70.0) / 76.0).clamp(0.0, 1.0).powi(2) }
}

pub fn db_label(db: f32) -> String {
    if db <= SEND_OFF_DB + 0.01 { "-inf".into() } else { format!("{db:.1}") }
}

/// Vertical fader editing a normalized value; double-click resets.
pub fn fader(ui: &mut Ui, value: &mut f32, size: egui::Vec2, default: f32, color: Color32) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if resp.dragged() {
        let d = -resp.drag_delta().y / rect.height() * if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        *value = (*value + d).clamp(0.0, 1.0);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        *value = default;
        resp.mark_changed();
    }
    let p = ui.painter();
    p.rect_filled(rect, 2.0, theme::PANEL_DARK);
    let fill = Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * *value), rect.max);
    p.rect_filled(fill, 2.0, color.linear_multiply(0.7));
    let y = rect.bottom() - rect.height() * *value;
    p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, theme::TEXT));
    resp
}

/// Rotary knob on a normalized value; vertical drag, double-click resets.
pub fn knob(ui: &mut Ui, value: &mut f32, default: f32, bipolar: bool, color: Color32) -> Response {
    let size = vec2(26.0, 26.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        *value = (*value - resp.drag_delta().y / 120.0 * fine).clamp(0.0, 1.0);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        *value = default;
        resp.mark_changed();
    }
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width() * 0.42;
    let a0 = std::f32::consts::PI * 0.75;
    let span = std::f32::consts::PI * 1.5;
    let pt = |a: f32, rr: f32| pos2(c.x + a.cos() * rr, c.y + a.sin() * rr);
    let n = 24;
    for i in 0..n {
        let (t0, t1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        p.line_segment([pt(a0 + span * t0, r), pt(a0 + span * t1, r)], Stroke::new(2.0, theme::GRID));
    }
    let (from, to) = if bipolar { (0.5f32.min(*value), 0.5f32.max(*value)) } else { (0.0, *value) };
    let steps = ((to - from) * n as f32).ceil() as i32;
    for i in 0..steps {
        let t0 = from + (to - from) * i as f32 / steps as f32;
        let t1 = from + (to - from) * (i + 1) as f32 / steps as f32;
        p.line_segment([pt(a0 + span * t0, r), pt(a0 + span * t1, r)], Stroke::new(2.5, color));
    }
    p.line_segment([c, pt(a0 + span * *value, r)], Stroke::new(1.5, theme::TEXT));
    resp
}

pub fn meter(ui: &mut Ui, peaks: (f32, f32), clipped: bool, size: egui::Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 1.0, theme::PANEL_DARK);
    let w = (rect.width() - 1.0) / 2.0;
    for (i, v) in [peaks.0, peaks.1].into_iter().enumerate() {
        let db = 20.0 * v.max(1e-6).log10();
        let n = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
        let x = rect.left() + i as f32 * (w + 1.0);
        let bar = Rect::from_min_max(pos2(x, rect.bottom() - rect.height() * n), pos2(x + w, rect.bottom()));
        let color = if db > -6.0 { theme::METER_HI } else { theme::METER_LO };
        p.rect_filled(bar, 0.0, color);
    }
    if clipped {
        p.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 3.0)), 0.0, theme::RECORD);
    }
    resp
}

pub fn toggle(ui: &mut Ui, on: bool, label: &str, on_color: Color32, w: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, if on { on_color } else { theme::SLOT });
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, theme::GRID), StrokeKind::Inside);
    p.text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), if on { Color32::BLACK } else { theme::TEXT });
    resp
}

/// One mixer strip (sends, pan, volume, meter, activator/solo/arm).
pub fn strip(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, width: f32) {
    let ix = XdawApp::track_ix(r);
    let peaks = app.meter(ix);
    let clipped = app.rt.status.track(ix).clipped.load(std::sync::atomic::Ordering::Relaxed);
    let returns = app.project.returns.len();
    let mapping = app.map_mode != crate::app::MapMode::Off;
    let track_id = app.project.track(r).id;
    ui.vertical(|ui| {
        ui.set_width(width);
        let t = app.project.track(r).clone();
        // Sends.
        if matches!(t.kind, TrackKind::Audio | TrackKind::Midi) {
            ui.horizontal_wrapped(|ui| {
                for i in 0..returns {
                    let mut n = db_to_fader(t.mixer.sends_db[i]);
                    let resp = knob(ui, &mut n, 0.0, false, theme::ACCENT).on_hover_text(format!(
                        "Send {} {}",
                        (b'A' + i as u8) as char,
                        db_label(fader_to_db(n))
                    ));
                    if resp.changed() {
                        let db = fader_to_db(n);
                        app.project.track_mut(r).mixer.sends_db[i] = db;
                        app.label("Send");
                        app.record_automation(r, ParamTarget::Send(i), db);
                    }
                    if mapping && resp.clicked() {
                        app.map_target = Some(xdaw_project::MappingTarget::Param { track: track_id, target: ParamTarget::Send(i) });
                    }
                }
            });
        }
        // Pan.
        let mut pan = (t.mixer.pan + 1.0) / 2.0;
        let resp = knob(ui, &mut pan, 0.5, true, theme::TEXT_DIM).on_hover_text(format!("Pan {:.0}", t.mixer.pan * 50.0));
        if resp.changed() {
            let v = pan * 2.0 - 1.0;
            app.project.track_mut(r).mixer.pan = v;
            app.label("Pan");
            app.record_automation(r, ParamTarget::Pan, v);
        }
        if mapping && resp.clicked() {
            app.map_target = Some(xdaw_project::MappingTarget::Param { track: track_id, target: ParamTarget::Pan });
        }
        // Fader + meter.
        ui.horizontal(|ui| {
            let mut n = db_to_fader(t.mixer.volume_db);
            let resp = fader(ui, &mut n, vec2(14.0, 90.0), db_to_fader(0.0), theme::rgb(t.color));
            if resp.changed() {
                let db = fader_to_db(n);
                app.project.track_mut(r).mixer.volume_db = db;
                app.label("Volume");
                app.record_automation(r, ParamTarget::Volume, db);
            }
            if mapping && resp.clicked() {
                app.map_target = Some(xdaw_project::MappingTarget::Param { track: track_id, target: ParamTarget::Volume });
            }
            if meter(ui, peaks, clipped, vec2(10.0, 90.0)).on_hover_text("Click to reset clip indicator").clicked() {
                app.rt.status.track(ix).clipped.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        });
        ui.label(egui::RichText::new(db_label(t.mixer.volume_db)).size(10.0).color(theme::TEXT_DIM));
        if r != TrackRef::Master {
            ui.horizontal(|ui| {
                if toggle(ui, !t.mixer.mute, &format!("{}", match r { TrackRef::Track(i) => (i + 1).to_string(), TrackRef::Return(i) => ((b'A' + i as u8) as char).to_string(), TrackRef::Master => "M".into() }), theme::QUEUED, 20.0)
                    .on_hover_text("Track Activator")
                    .clicked()
                {
                    let m = &mut app.project.track_mut(r).mixer;
                    m.mute = !m.mute;
                    app.label("Mute");
                }
                if toggle(ui, t.mixer.solo, "S", theme::ACCENT, 16.0).on_hover_text("Solo").clicked() {
                    let exclusive = !ui.input(|i| i.modifiers.command);
                    let new = !t.mixer.solo;
                    if exclusive {
                        for tr in app.project.tracks.iter_mut() {
                            tr.mixer.solo = false;
                        }
                    }
                    app.project.track_mut(r).mixer.solo = new;
                    app.label("Solo");
                }
                if matches!(t.kind, TrackKind::Audio | TrackKind::Midi)
                    && toggle(ui, t.mixer.arm, "R", theme::RECORD, 16.0).on_hover_text("Arm recording").clicked()
                {
                    let m = &mut app.project.track_mut(r).mixer;
                    m.arm = !m.arm;
                    app.label("Arm");
                }
            });
            if t.kind == TrackKind::Audio {
                ui.horizontal(|ui| {
                    for (m, l) in [(Monitor::In, "In"), (Monitor::Auto, "Auto"), (Monitor::Off, "Off")] {
                        if toggle(ui, t.mixer.monitor == m, l, theme::QUEUED, 24.0).on_hover_text("Monitoring").clicked() {
                            app.project.track_mut(r).mixer.monitor = m;
                            app.label("Monitor");
                        }
                    }
                });
            }
        }
    });
}
