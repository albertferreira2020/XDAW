//! Device View: the selected track's ordered device chain.

use crate::app::{MapMode, XdawApp};
use crate::browser::{self, BrowserItem};
use crate::{mixer, theme};
use egui::{vec2, RichText, Ui};
use xdaw_dsp::{DeviceKind, ParamKind};
use xdaw_project::*;

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    let Some(r) = app.sel.track else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Select a track to see its devices").color(theme::TEXT_DIM)));
        return;
    };
    let track_name = app.project.track(r).name.clone();
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(track_name).strong());
                ui.label(RichText::new("Drop devices here →").size(10.0).color(theme::TEXT_DIM));
            });
            if !app.project.track(r).devices.is_empty() {
                macros(app, ui, r);
            }
            let ids: Vec<Id> = app.project.track(r).devices.iter().map(|d| d.id).collect();
            let mut action = None;
            for id in ids {
                if let Some(a) = device_box(app, ui, r, id) {
                    action = Some(a);
                }
            }
            // Drop target at the end of the chain.
            let (rect, resp) = ui.allocate_exact_size(vec2(90.0, 180.0), egui::Sense::hover());
            ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::GRID), egui::StrokeKind::Inside);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Drop\ndevice", egui::FontId::proportional(11.0), theme::TEXT_DIM);
            if let Some(item) = crate::browser::dnd_release::<BrowserItem>(&resp) {
                match (*item).clone() {
                    BrowserItem::Device(k) => browser::add_device(app, r, k),
                    BrowserItem::Preset(p) => browser::load_preset(app, r, &p),
                    BrowserItem::Factory(k, i) => browser::load_factory(app, r, k, i),
                    BrowserItem::File(_) => {}
                }
            }
            if let Some(a) = action {
                apply(app, r, a);
            }
        });
    });
}

/// Rack-style macro controls: 8 knobs, each mapped to any parameter of the
/// track's devices, rack chains and chained devices.
fn macros(app: &mut XdawApp, ui: &mut Ui, r: TrackRef) {
    let frame = egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(6.0).corner_radius(3.0).stroke(egui::Stroke::new(1.0, theme::ACCENT));
    frame.show(ui, |ui| {
        ui.set_min_height(180.0);
        ui.vertical(|ui| {
            ui.label(RichText::new("Macros").strong());
            egui::Grid::new(("macros", app.project.track(r).id)).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
                for i in 0..xdaw_project::MACROS {
                    let m = app.project.track(r).macros.get(i).cloned().unwrap_or(xdaw_project::Macro { name: format!("Macro {}", i + 1), value: 0.0, mappings: vec![] });
                    ui.vertical(|ui| {
                        ui.set_width(58.0);
                        ui.label(RichText::new(&m.name).size(9.5).color(if m.mappings.is_empty() { theme::TEXT_DIM } else { theme::ACCENT }));
                        let mut v = m.value;
                        let resp = mixer::knob(ui, &mut v, 0.0, false, theme::ACCENT).on_hover_text(format!("{} mapping(s); right-click to map", m.mappings.len()));
                        if resp.changed() {
                            let t = app.project.track_mut(r);
                            t.ensure_macros();
                            t.macros[i].value = v;
                            t.apply_macros();
                            app.label("Macro");
                        }
                        resp.context_menu(|ui| macro_menu(app, ui, r, i, &m));
                        ui.label(RichText::new(format!("{:.0}%", m.value * 100.0)).size(9.5));
                    });
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        });
    });
}

fn macro_menu(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, i: usize, m: &xdaw_project::Macro) {
    let track = app.project.track(r);
    // One submenu per parameter owner (device, chain, chained device).
    let mut owners: Vec<(Id, String)> = vec![];
    for (t, _) in track.param_targets() {
        if let ParamTarget::Device { device, .. } = t {
            if !owners.iter().any(|o| o.0 == device) {
                owners.push((device, track.param_owner(device).map(|o| o.label).unwrap_or_default()));
            }
        }
    }
    for (id, label) in owners {
        ui.menu_button(label, |ui| {
            let Some(descs) = app.project.track(r).param_owner(id).map(|o| o.descs) else { return };
            for (pi, desc) in descs.iter().enumerate() {
                if ui.button(desc.name).clicked() {
                    let t = app.project.track_mut(r);
                    t.ensure_macros();
                    t.macros[i].mappings.push(xdaw_project::MacroMapping { device: id, param: pi, min: desc.min, max: desc.max });
                    if t.macros[i].name.starts_with("Macro ") {
                        t.macros[i].name = desc.name.to_string();
                    }
                    app.label("Map Macro");
                    ui.close();
                }
            }
        });
    }
    if !m.mappings.is_empty() && ui.button("Clear mappings").clicked() {
        app.project.track_mut(r).macros[i].mappings.clear();
        app.label("Clear Macro");
        ui.close();
    }
}

enum Action {
    Remove(Id),
    Move(Id, bool),
    AddChain(Id),
    RemoveChain(Id),
    AddToChain(Id, DeviceKind),
}

fn apply(app: &mut XdawApp, r: TrackRef, a: Action) {
    match a {
        Action::Remove(id) => {
            if app.project.track_mut(r).remove_device(id) {
                app.label("Delete Device");
            }
        }
        // Devices move only within their section (MIDI effects, instrument, audio effects).
        Action::Move(id, right) => {
            if app.project.track_mut(r).move_device(id, right) {
                app.label("Move Device");
            }
        }
        Action::AddChain(rack) => {
            if app.project.add_chain(r, rack).is_some() {
                app.label("Add Chain");
            }
        }
        Action::RemoveChain(id) => {
            if app.project.track_mut(r).remove_chain(id) {
                app.label("Delete Chain");
            }
        }
        Action::AddToChain(chain, kind) => match app.project.add_device_to_chain(r, chain, kind) {
            Ok(_) => app.label(&format!("Add {}", kind.name())),
            Err(e) => app.status_line = e,
        },
    }
}

fn device_box(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, id: Id) -> Option<Action> {
    let mut action = None;
    let d = app.project.track(r).device(id)?.clone();
    let track_id = app.project.track(r).id;
    let frame = egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(6.0).corner_radius(3.0).stroke(egui::Stroke::new(1.0, theme::GRID));
    frame.show(ui, |ui| {
        ui.set_min_height(180.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if mixer::toggle(ui, !d.bypass, "On", theme::ACCENT, 22.0).on_hover_text("Device on/off").clicked() {
                    if let Some(x) = app.project.track_mut(r).device_mut(id) {
                        x.bypass = !d.bypass;
                    }
                    app.label("Device On/Off");
                }
                ui.label(RichText::new(d.kind.name()).strong());
                if let Some(p) = &d.preset_name {
                    ui.label(RichText::new(p).size(10.0).color(theme::TEXT_DIM));
                }
                if ui.small_button("◀").clicked() {
                    action = Some(Action::Move(id, false));
                }
                if ui.small_button("▶").clicked() {
                    action = Some(Action::Move(id, true));
                }
                if d.kind.is_rack() && ui.small_button("+ Chain").clicked() {
                    action = Some(Action::AddChain(id));
                }
                let factory = xdaw_dsp::presets::factory_presets(d.kind);
                if !factory.is_empty() {
                    ui.menu_button("Presets", |ui| {
                        for (pi, p) in factory.iter().enumerate() {
                            if ui.selectable_label(d.preset_name.as_deref() == Some(p.name), p.name).clicked() {
                                browser::load_factory_into(app, r, id, pi);
                                ui.close();
                            }
                        }
                    });
                }
                ui.menu_button("...", |ui| {
                    let key = egui::Id::new(("preset_name", d.id));
                    let mut name: String = ui.data(|m| m.get_temp(key)).unwrap_or_else(|| d.preset_name.clone().unwrap_or_else(|| "My Preset".into()));
                    if !d.kind.is_rack() {
                        ui.text_edit_singleline(&mut name);
                        ui.data_mut(|m| m.insert_temp(key, name.clone()));
                        if ui.button("Save Preset").clicked() {
                            browser::save_preset(app, &d, &name);
                            if let Some(x) = app.project.track_mut(r).device_mut(id) {
                                x.preset_name = Some(name);
                            }
                            ui.close();
                        }
                        if ui.button("Reset to defaults").clicked() {
                            if let Some(x) = app.project.track_mut(r).device_mut(id) {
                                x.params = d.kind.params().iter().map(|p| p.default).collect();
                            }
                            app.label("Reset Device");
                            ui.close();
                        }
                    }
                    if ui.button("Delete").clicked() {
                        action = Some(Action::Remove(id));
                        ui.close();
                    }
                });
            });
            if d.kind == DeviceKind::Sampler {
                sample_slot(app, ui, r, id, None, d.sample.as_ref());
            }
            if d.kind == DeviceKind::DrumKit {
                drum_pads(app, ui, r, &d);
                return;
            }
            if d.kind.supports_sidechain() && app.project.track(r).devices.iter().any(|x| x.id == id) {
                sidechain_chooser(app, ui, r, &d);
            }
            if d.kind.is_rack() {
                if let Some(a) = rack_body(app, ui, r, &d) {
                    action = Some(a);
                }
                return;
            }
            if d.kind == DeviceKind::ParamEq {
                param_eq(app, ui, r, &d, track_id);
                return;
            }
            let all: Vec<usize> = (0..d.kind.params().len()).collect();
            param_grid(app, ui, r, id, track_id, &all);
        });
    });
    action
}

/// Chain list (left) and the selected chain's devices (right) of a rack (spec D6).
fn rack_body(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, d: &DeviceState) -> Option<Action> {
    let mut action = None;
    let sel_key = egui::Id::new(("rack_chain", d.id));
    let mut sel: usize = ui.data(|m| m.get_temp(sel_key)).unwrap_or(0).min(d.chains.len().saturating_sub(1));
    let instrument = d.kind == DeviceKind::InstrumentRack;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(if instrument { 250.0 } else { 170.0 });
            if d.chains.is_empty() {
                ui.label(RichText::new("No chains: press + Chain").size(10.0).color(theme::TEXT_DIM));
            }
            for (ci, c) in d.chains.iter().enumerate() {
                ui.horizontal(|ui| {
                    let resp = ui.selectable_label(sel == ci, RichText::new(&c.name).size(10.0));
                    if resp.clicked() {
                        sel = ci;
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Delete chain").clicked() {
                            action = Some(Action::RemoveChain(c.id));
                            ui.close();
                        }
                    });
                    let set = |app: &mut XdawApp, k: usize, v: f32, label: &str| {
                        app.project.track_mut(r).set_param(c.id, k, v);
                        app.label(label);
                        app.record_automation(r, ParamTarget::Device { device: c.id, param: k }, v);
                    };
                    let (mute, solo) = (c.param(2) >= 0.5, c.param(3) >= 0.5);
                    if mixer::toggle(ui, !mute, "A", theme::ACCENT, 16.0).on_hover_text("Chain on (mute)").clicked() {
                        set(app, 2, if mute { 0.0 } else { 1.0 }, "Chain Mute");
                    }
                    if mixer::toggle(ui, solo, "S", theme::QUEUED, 16.0).on_hover_text("Solo chain").clicked() {
                        set(app, 3, if solo { 0.0 } else { 1.0 }, "Chain Solo");
                    }
                    let mut vol = c.param(0);
                    if ui.add(egui::DragValue::new(&mut vol).range(-70.0..=6.0).speed(0.2).suffix(" dB")).on_hover_text("Chain volume").changed() {
                        set(app, 0, vol, "Chain Volume");
                    }
                    if instrument {
                        let (mut lo, mut hi) = (c.param(4), c.param(5));
                        let lo_txt = xdaw_midi::note_name(lo as u8);
                        if ui.add(egui::DragValue::new(&mut lo).range(0.0..=127.0).speed(0.3).custom_formatter(move |_, _| lo_txt.clone())).on_hover_text("Key zone low").changed() {
                            set(app, 4, lo.min(hi), "Key Zone");
                        }
                        let hi_txt = xdaw_midi::note_name(hi as u8);
                        if ui.add(egui::DragValue::new(&mut hi).range(0.0..=127.0).speed(0.3).custom_formatter(move |_, _| hi_txt.clone())).on_hover_text("Key zone high").changed() {
                            set(app, 5, hi.max(lo), "Key Zone");
                        }
                    }
                });
            }
        });
        ui.separator();
        ui.data_mut(|m| m.insert_temp(sel_key, sel));
        let Some(chain) = d.chains.get(sel) else { return };
        let ids: Vec<Id> = chain.devices.iter().map(|x| x.id).collect();
        for id in ids {
            if let Some(a) = device_box(app, ui, r, id) {
                action = Some(a);
            }
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(80.0, 160.0), egui::Sense::hover());
        ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::GRID), egui::StrokeKind::Inside);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, format!("Drop into\n{}", chain.name), egui::FontId::proportional(10.0), theme::TEXT_DIM);
        if let Some(item) = crate::browser::dnd_release::<BrowserItem>(&resp) {
            if let BrowserItem::Device(k) = (*item).clone() {
                action = Some(Action::AddToChain(chain.id, k));
            }
        }
    });
    action
}

/// Detector source of a Compressor, Bus Compressor or Gate (spec D3.1).
fn sidechain_chooser(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, d: &DeviceState) {
    let own = app.project.track(r).id;
    let sources: Vec<(Id, String)> = app.project.tracks.iter().filter(|t| t.id != own && t.kind != TrackKind::Group).map(|t| (t.id, t.name.clone())).collect();
    let current = d.sidechain.and_then(|id| sources.iter().find(|s| s.0 == id)).map_or("Self".to_string(), |s| s.1.clone());
    ui.horizontal(|ui| {
        ui.label(RichText::new("Sidechain").size(9.5).color(theme::TEXT_DIM));
        let mut pick = d.sidechain;
        egui::ComboBox::from_id_salt(("sidechain", d.id)).width(110.0).selected_text(RichText::new(current).size(10.0)).show_ui(ui, |ui| {
            ui.selectable_value(&mut pick, None, "Self");
            for (id, name) in &sources {
                ui.selectable_value(&mut pick, Some(*id), name);
            }
        });
        if pick != d.sidechain {
            if let Some(x) = app.project.track_mut(r).device_mut(d.id) {
                x.sidechain = pick;
            }
            app.label("Sidechain");
        }
    });
}

/// Parametric EQ: response curve, band selector, then the selected band's controls.
fn param_eq(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, d: &DeviceState, track_id: Id) {
    use xdaw_dsp::tone::{peq_response_db, PEQ_BANDS, PEQ_STRIDE};
    let values: Vec<f32> = (0..d.kind.params().len()).map(|k| d.param(k)).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(8.0 * 44.0, 64.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, theme::BG);
    let (lo, hi, range_db) = (20f64.ln(), 20_000f64.ln(), 18.0f32);
    let y_of = |db: f32| rect.center().y - (db / range_db).clamp(-1.0, 1.0) * rect.height() * 0.5;
    for f in [100.0f64, 1_000.0, 10_000.0] {
        let x = rect.left() + ((f.ln() - lo) / (hi - lo)) as f32 * rect.width();
        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], (1.0, theme::GRID));
    }
    p.line_segment([egui::pos2(rect.left(), y_of(0.0)), egui::pos2(rect.right(), y_of(0.0))], (1.0, theme::GRID));
    let pts: Vec<egui::Pos2> = (0..=rect.width() as usize)
        .map(|k| {
            let x = rect.left() + k as f32;
            let f = (lo + (k as f64 / rect.width() as f64) * (hi - lo)).exp();
            egui::pos2(x, y_of(peq_response_db(&values, 48_000.0, f) as f32))
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, theme::ACCENT)));
    let sel_id = egui::Id::new(("peq_band", d.id));
    let mut band: usize = ui.data(|m| m.get_temp(sel_id)).unwrap_or(0);
    ui.horizontal(|ui| {
        for b in 0..PEQ_BANDS {
            let on = values[b * PEQ_STRIDE] >= 0.5;
            let label = RichText::new(format!("{}", b + 1)).size(10.0).color(if on { theme::ACCENT } else { theme::TEXT_DIM });
            if ui.selectable_label(band == b, label).on_hover_text("Select band (its On switch is below)").clicked() {
                band = b;
            }
        }
    });
    ui.data_mut(|m| m.insert_temp(sel_id, band));
    let mut shown: Vec<usize> = (band * PEQ_STRIDE..band * PEQ_STRIDE + PEQ_STRIDE).collect();
    shown.push(PEQ_BANDS * PEQ_STRIDE);
    param_grid(app, ui, r, d.id, track_id, &shown);
}

/// Controls for parameters `shown` of the device or chain `owner`.
fn param_grid(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, owner: Id, track_id: Id, shown: &[usize]) {
    let Some((label, descs)) = app.project.track(r).param_owner(owner).map(|o| (o.label, o.descs)) else { return };
    let per_row = 6;
    egui::Grid::new(("params", owner)).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
        for (n, &pi) in shown.iter().enumerate() {
            let Some(desc) = descs.get(pi) else { continue };
            let v = app.project.track(r).param_value(owner, pi).unwrap_or(desc.default);
            ui.vertical(|ui| {
                ui.set_width(56.0);
                ui.label(RichText::new(desc.name).size(9.5).color(theme::TEXT_DIM));
                let new = match desc.kind {
                    ParamKind::Choice(opts) => {
                        let mut idx = v as usize;
                        egui::ComboBox::from_id_salt(("choice", owner, pi)).width(54.0).selected_text(RichText::new(opts.get(idx).copied().unwrap_or("?")).size(10.0)).show_ui(ui, |ui| {
                            for (k, o) in opts.iter().enumerate() {
                                ui.selectable_value(&mut idx, k, *o);
                            }
                        });
                        idx as f32
                    }
                    ParamKind::Toggle => {
                        let on = v >= 0.5;
                        if mixer::toggle(ui, on, if on { "On" } else { "Off" }, theme::ACCENT, 40.0).clicked() {
                            if on { 0.0 } else { 1.0 }
                        } else {
                            v
                        }
                    }
                    ParamKind::Continuous { .. } | ParamKind::Int => {
                        let mut n = desc.to_normalized(v);
                        let bipolar = desc.min < 0.0 && desc.max > 0.0;
                        let resp = mixer::knob(ui, &mut n, desc.to_normalized(desc.default), bipolar, theme::ACCENT);
                        let target = ParamTarget::Device { device: owner, param: pi };
                        if app.map_mode != MapMode::Off && resp.clicked() {
                            app.map_target = Some(MappingTarget::Param { track: track_id, target });
                        }
                        if resp.changed() { desc.from_normalized(n) } else { v }
                    }
                };
                ui.label(RichText::new(desc.format(new)).size(9.5));
                if new != v {
                    app.project.track_mut(r).set_param(owner, pi, new);
                    app.label(&format!("{label} {}", desc.name));
                    app.record_automation(r, ParamTarget::Device { device: owner, param: pi }, new);
                }
            });
            if (n + 1) % per_row == 0 {
                ui.end_row();
            }
        }
    });
}

fn sample_slot(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, device: Id, pad: Option<usize>, current: Option<&MediaRef>) {
    let label = current.map(|m| std::path::Path::new(&m.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).unwrap_or_else(|| "Drop sample".into());
    let missing = current.is_some_and(|m| app.media.failed.contains_key(&m.path));
    let text = RichText::new(label).size(10.0).color(if missing { theme::RECORD } else { theme::TEXT });
    let resp = ui.add(egui::Button::new(text).min_size(vec2(if pad.is_some() { 60.0 } else { 160.0 }, 18.0)));
    if resp.clicked() {
        if let Some(p) = rfd::FileDialog::new().add_filter("Audio", &["wav", "aif", "aiff", "flac"]).pick_file() {
            browser::assign_sample(app, r, device, pad, &p);
        }
    }
    if let Some(item) = crate::browser::dnd_release::<BrowserItem>(&resp) {
        if let BrowserItem::File(p) = (*item).clone() {
            if browser::is_audio(&p) {
                browser::assign_sample(app, r, device, pad, &p);
            }
        }
    }
}

fn drum_pads(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, d: &DeviceState) {
    let track_index = match r {
        TrackRef::Track(t) => Some(t),
        _ => None,
    };
    egui::Grid::new(("pads", d.id)).spacing(vec2(3.0, 3.0)).show(ui, |ui| {
        for row in (0..4).rev() {
            for col in 0..4 {
                let pad = row * 4 + col;
                ui.vertical(|ui| {
                    let note = xdaw_dsp::instruments::FIRST_PAD_NOTE + pad as u8;
                    let resp = ui.add(egui::Button::new(RichText::new(xdaw_midi::note_name(note)).size(9.0)).min_size(vec2(60.0, 14.0)));
                    if let Some(t) = track_index {
                        if resp.is_pointer_button_down_on() && !ui.data(|m| m.get_temp::<bool>(resp.id).unwrap_or(false)) {
                            ui.data_mut(|m| m.insert_temp(resp.id, true));
                            app.rt.send(xdaw_engine::protocol::Command::Note { track: t as u16, kind: xdaw_dsp::NoteKind::On { pitch: note, velocity: 110 } });
                        } else if !resp.is_pointer_button_down_on() && ui.data(|m| m.get_temp::<bool>(resp.id).unwrap_or(false)) {
                            ui.data_mut(|m| m.insert_temp(resp.id, false));
                            app.rt.send(xdaw_engine::protocol::Command::Note { track: t as u16, kind: xdaw_dsp::NoteKind::Off { pitch: note } });
                        }
                    }
                    sample_slot(app, ui, r, d.id, Some(pad), d.pads.get(pad).and_then(Option::as_ref));
                    let base = pad * xdaw_dsp::instruments::PAD_PARAMS_EACH;
                    let mut gain = d.param(base);
                    if ui.add(egui::DragValue::new(&mut gain).range(-36.0..=6.0).speed(0.2).suffix(" dB")).changed() {
                        app.project.track_mut(r).set_param(d.id, base, gain);
                        app.label("Pad Gain");
                    }
                });
            }
            ui.end_row();
        }
    });
}
