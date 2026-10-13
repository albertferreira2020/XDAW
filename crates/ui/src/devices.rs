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
            let n = app.project.track(r).devices.len();
            let mut action = None;
            for i in 0..n {
                if let Some(a) = device_box(app, ui, r, i) {
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

/// Rack-style macro controls: 8 knobs, each mapped to any device parameters.
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
                        resp.context_menu(|ui| {
                            let devices = app.project.track(r).devices.clone();
                            for d in &devices {
                                ui.menu_button(d.kind.name(), |ui| {
                                    for (pi, desc) in d.kind.params().iter().enumerate() {
                                        if ui.button(desc.name).clicked() {
                                            let t = app.project.track_mut(r);
                                            t.ensure_macros();
                                            t.macros[i].mappings.push(xdaw_project::MacroMapping { device: d.id, param: pi, min: desc.min, max: desc.max });
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
                        });
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

enum Action {
    Remove(usize),
    MoveLeft(usize),
    MoveRight(usize),
}

fn apply(app: &mut XdawApp, r: TrackRef, a: Action) {
    let devices = &mut app.project.track_mut(r).devices;
    match a {
        Action::Remove(i) => {
            let id = devices.remove(i).id;
            app.project.track_mut(r).automation.retain(|l| !matches!(l.target, ParamTarget::Device { device, .. } if device == id));
            app.label("Delete Device");
        }
        Action::MoveLeft(i) if i > 0 => {
            // Instruments stay first in the chain.
            if !(devices[i - 1].kind.is_instrument()) {
                devices.swap(i, i - 1);
                app.label("Move Device");
            }
        }
        Action::MoveRight(i) if i + 1 < devices.len() => {
            if !devices[i].kind.is_instrument() {
                devices.swap(i, i + 1);
                app.label("Move Device");
            }
        }
        _ => {}
    }
}

fn device_box(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, i: usize) -> Option<Action> {
    let mut action = None;
    let d = app.project.track(r).devices[i].clone();
    let track_id = app.project.track(r).id;
    let frame = egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(6.0).corner_radius(3.0).stroke(egui::Stroke::new(1.0, theme::GRID));
    frame.show(ui, |ui| {
        ui.set_min_height(180.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if mixer::toggle(ui, !d.bypass, "On", theme::ACCENT, 22.0).on_hover_text("Device on/off").clicked() {
                    app.project.track_mut(r).devices[i].bypass = !d.bypass;
                    app.label("Device On/Off");
                }
                ui.label(RichText::new(d.kind.name()).strong());
                if let Some(p) = &d.preset_name {
                    ui.label(RichText::new(p).size(10.0).color(theme::TEXT_DIM));
                }
                if ui.small_button("◀").clicked() {
                    action = Some(Action::MoveLeft(i));
                }
                if ui.small_button("▶").clicked() {
                    action = Some(Action::MoveRight(i));
                }
                let factory = xdaw_dsp::presets::factory_presets(d.kind);
                if !factory.is_empty() {
                    ui.menu_button("Presets", |ui| {
                        for (pi, p) in factory.iter().enumerate() {
                            if ui.selectable_label(d.preset_name.as_deref() == Some(p.name), p.name).clicked() {
                                browser::load_factory(app, r, d.kind, pi);
                                ui.close();
                            }
                        }
                    });
                }
                ui.menu_button("...", |ui| {
                    let id = egui::Id::new(("preset_name", d.id));
                    let mut name: String = ui.data(|m| m.get_temp(id)).unwrap_or_else(|| d.preset_name.clone().unwrap_or_else(|| "My Preset".into()));
                    ui.text_edit_singleline(&mut name);
                    ui.data_mut(|m| m.insert_temp(id, name.clone()));
                    if ui.button("Save Preset").clicked() {
                        browser::save_preset(app, &d, &name);
                        app.project.track_mut(r).devices[i].preset_name = Some(name);
                        ui.close();
                    }
                    if ui.button("Reset to defaults").clicked() {
                        app.project.track_mut(r).devices[i].params = d.kind.params().iter().map(|p| p.default).collect();
                        app.label("Reset Device");
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        action = Some(Action::Remove(i));
                        ui.close();
                    }
                });
            });
            if d.kind == DeviceKind::Sampler {
                sample_slot(app, ui, r, i, None, d.sample.as_ref());
            }
            if d.kind == DeviceKind::DrumRack {
                drum_pads(app, ui, r, i, &d);
                return;
            }
            params(app, ui, r, i, &d, track_id);
            if d.kind == DeviceKind::Compressor {
                ui.label(RichText::new("GR meter: see master/track meters").size(9.0).color(theme::TEXT_DIM));
            }
        });
    });
    action
}

fn params(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, i: usize, d: &DeviceState, track_id: Id) {
    let descs = d.kind.params();
    let per_row = 6;
    egui::Grid::new(("params", d.id)).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
        for (pi, desc) in descs.iter().enumerate() {
            let v = d.param(pi);
            ui.vertical(|ui| {
                ui.set_width(56.0);
                ui.label(RichText::new(desc.name).size(9.5).color(theme::TEXT_DIM));
                let new = match desc.kind {
                    ParamKind::Choice(opts) => {
                        let mut idx = v as usize;
                        egui::ComboBox::from_id_salt(("choice", d.id, pi)).width(54.0).selected_text(RichText::new(opts.get(idx).copied().unwrap_or("?")).size(10.0)).show_ui(ui, |ui| {
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
                    ParamKind::Continuous { .. } => {
                        let mut n = desc.to_normalized(v);
                        let bipolar = desc.min < 0.0 && desc.max > 0.0;
                        let resp = mixer::knob(ui, &mut n, desc.to_normalized(desc.default), bipolar, theme::ACCENT);
                        let target = ParamTarget::Device { device: d.id, param: pi };
                        if app.map_mode != MapMode::Off && resp.clicked() {
                            app.map_target = Some(MappingTarget::Param { track: track_id, target });
                        }
                        if resp.changed() { desc.from_normalized(n) } else { v }
                    }
                };
                ui.label(RichText::new(desc.format(new)).size(9.5));
                if new != v {
                    let dev = &mut app.project.track_mut(r).devices[i];
                    if dev.params.len() <= pi {
                        dev.params.resize(descs.len(), 0.0);
                    }
                    dev.params[pi] = new;
                    app.label(&format!("{} {}", d.kind.name(), desc.name));
                    app.record_automation(r, ParamTarget::Device { device: d.id, param: pi }, new);
                }
            });
            if (pi + 1) % per_row == 0 {
                ui.end_row();
            }
        }
    });
}

fn sample_slot(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, device: usize, pad: Option<usize>, current: Option<&MediaRef>) {
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

fn drum_pads(app: &mut XdawApp, ui: &mut Ui, r: TrackRef, device: usize, d: &DeviceState) {
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
                    sample_slot(app, ui, r, device, Some(pad), d.pads.get(pad).and_then(Option::as_ref));
                    let base = pad * xdaw_dsp::instruments::PAD_PARAMS_EACH;
                    let mut gain = d.param(base);
                    if ui.add(egui::DragValue::new(&mut gain).range(-36.0..=6.0).speed(0.2).suffix(" dB")).changed() {
                        app.project.track_mut(r).devices[device].params[base] = gain;
                        app.label("Pad Gain");
                    }
                });
            }
            ui.end_row();
        }
    });
}
