//! Menu bar, transport/control bar and status bar.

use crate::app::{MapMode, View, XdawApp};
use crate::mixer::toggle;
use crate::theme;
use egui::{RichText, Ui};
use std::sync::atomic::Ordering;
use xdaw_engine::protocol::Command;
use xdaw_sequencer::time::beats_to_bbs;
use xdaw_sequencer::{Quantize, TimeSignature};

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    egui::Panel::top("menu").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| menus(app, ui));
    });
    egui::Panel::top("transport").exact_size(30.0).show(ui, |ui| {
        ui.horizontal_centered(|ui| control_bar(app, ui));
    });
}

fn menus(app: &mut XdawApp, ui: &mut Ui) {
    ui.menu_button("File", |ui| {
        if ui.button("New Set").clicked() {
            app.dialogs.new_requested = true;
        }
        if ui.button("Open Set…").clicked() {
            app.dialogs.open_requested = true;
        }
        if ui.button("Save Set").clicked() {
            app.save();
        }
        if ui.button("Save Set As…").clicked() {
            app.save_as();
        }
        if ui.button("Collect All and Save").clicked() {
            app.collect_and_save();
        }
        ui.separator();
        if ui.button("Import Audio/MIDI File…").clicked() {
            crate::browser::import_dialog(app);
        }
        if ui.button("Export Audio…").clicked() {
            app.dialogs.export_open = true;
        }
        if ui.button("Export Selected MIDI Clip…").clicked() {
            crate::clip_editor::export_midi(app);
        }
        ui.separator();
        if ui.button("Audio/MIDI Settings…").clicked() {
            app.dialogs.settings_open = true;
        }
    });
    ui.menu_button("Edit", |ui| {
        let undo = app.history.undo_label().map(|l| format!("Undo {l}")).unwrap_or_else(|| "Undo".into());
        if ui.add_enabled(app.history.undo_label().is_some(), egui::Button::new(undo)).clicked() {
            app.undo();
        }
        let redo = app.history.redo_label().map(|l| format!("Redo {l}")).unwrap_or_else(|| "Redo".into());
        if ui.add_enabled(app.history.redo_label().is_some(), egui::Button::new(redo)).clicked() {
            app.redo();
        }
        ui.separator();
        if ui.button("Copy").clicked() {
            app.copy_selection();
        }
        if ui.button("Paste").clicked() {
            app.paste();
        }
        if ui.button("Duplicate").clicked() {
            app.duplicate_selection();
        }
        if ui.button("Delete").clicked() {
            app.delete_selection();
        }
        if ui.button("Split").clicked() {
            crate::arrangement::split_selected(app);
        }
    });
    ui.menu_button("Create", |ui| {
        if ui.button("Insert Audio Track").clicked() {
            let n = app.project.tracks.len() + 1;
            app.project.add_track(xdaw_project::TrackKind::Audio, &format!("{n} Audio"));
            app.label("Insert Audio Track");
        }
        if ui.button("Insert MIDI Track").clicked() {
            let n = app.project.tracks.len() + 1;
            let t = app.project.add_track(xdaw_project::TrackKind::Midi, &format!("{n} MIDI"));
            app.project.add_device(xdaw_project::TrackRef::Track(t), xdaw_dsp::DeviceKind::PolySynth);
            app.label("Insert MIDI Track");
        }
        if ui.add_enabled(app.project.returns.len() < xdaw_engine::protocol::MAX_RETURNS, egui::Button::new("Insert Return Track")).clicked() {
            let n = app.project.returns.len();
            app.project.add_return(&format!("{} Return", (b'A' + n as u8) as char));
            app.label("Insert Return Track");
        }
        if ui.button("Insert Scene").clicked() {
            let at = app.sel.scene + 1;
            app.project.insert_scene(at, &format!("{}", app.project.scenes.len() + 1));
            app.label("Insert Scene");
        }
        if ui.button("Capture and Insert Scene").clicked() {
            crate::session::capture_scene(app);
        }
    });
    ui.menu_button("View", |ui| {
        ui.selectable_value(&mut app.view, View::Session, "Session (Tab)");
        ui.selectable_value(&mut app.view, View::Arrangement, "Arrangement (Tab)");
        ui.checkbox(&mut app.follow, "Follow playhead");
    });
    ui.menu_button("Help", |ui| {
        ui.label("XDAW — independent DAW. See docs/USER_GUIDE.md.");
        ui.label("Space play/stop · Tab switch view · F9 record · Cmd+Z undo");
    });
}

fn control_bar(app: &mut XdawApp, ui: &mut Ui) {
    let t = &mut app.project.transport;
    // Tempo.
    ui.label(RichText::new("BPM").color(theme::TEXT_DIM));
    let before = t.bpm;
    ui.add(egui::DragValue::new(&mut t.bpm).range(20.0..=999.0).speed(0.1).max_decimals(2));
    if t.bpm != before {
        app.pending_label = Some("Tempo".into());
    }
    if ui.small_button("Tap").on_hover_text("Tap tempo").clicked() {
        tap_tempo(app, ui);
    }
    let t = &mut app.project.transport;
    let mut num = t.time_signature.numerator;
    let mut den = t.time_signature.denominator;
    ui.add(egui::DragValue::new(&mut num).range(1..=99));
    ui.label("/");
    egui::ComboBox::from_id_salt("den").width(36.0).selected_text(den.to_string()).show_ui(ui, |ui| {
        for d in [1, 2, 4, 8, 16] {
            ui.selectable_value(&mut den, d, d.to_string());
        }
    });
    if let Some(sig) = TimeSignature::new(num, den) {
        if sig != t.time_signature {
            t.time_signature = sig;
            app.pending_label = Some("Time Signature".into());
        }
    }
    let t = &mut app.project.transport;
    if toggle(ui, t.metronome, "Met", theme::ACCENT, 30.0).on_hover_text("Metronome").clicked() {
        t.metronome = !t.metronome;
    }
    egui::ComboBox::from_id_salt("countin").width(70.0).selected_text(if t.count_in_bars == 0 { "No count-in".to_string() } else { format!("{} bar in", t.count_in_bars) }).show_ui(ui, |ui| {
        for b in [0, 1, 2, 4] {
            ui.selectable_value(&mut t.count_in_bars, b, if b == 0 { "None".to_string() } else { format!("{b} Bar(s)") });
        }
    });
    ui.separator();
    // Position.
    let beat = app.beat();
    let bbs = beats_to_bbs(beat, app.project.transport.time_signature);
    let counting = app.rt.status.counting_in.load(Ordering::Relaxed);
    ui.label(RichText::new(format!("{:>4}.{}.{}", bbs.bar, bbs.beat, bbs.sixteenth)).monospace().size(15.0).color(if counting { theme::ACCENT } else { theme::TEXT }));
    // Transport buttons.
    let playing = app.playing();
    if toggle(ui, playing, "▶", theme::PLAYING, 26.0).on_hover_text("Play (Space)").clicked() {
        app.rt.send(Command::Play);
    }
    if toggle(ui, false, "■", theme::TEXT, 26.0).on_hover_text("Stop (twice: return to start)").clicked() {
        app.rt.send(Command::Stop);
    }
    if toggle(ui, app.arrangement_record, "Rec", theme::RECORD, 30.0).on_hover_text("Arrangement Record (F9)").clicked() {
        app.toggle_arrangement_record();
    }
    let session_rec = (0..app.project.tracks.len()).any(|t| app.track_status(t).recording_slot.load(Ordering::Relaxed) >= 0);
    if toggle(ui, session_rec, "S.Rec", theme::RECORD, 38.0).on_hover_text("Session Record: record/overdub on armed tracks").clicked() {
        app.session_record();
    }
    if toggle(ui, false, "Capture", theme::ACCENT, 50.0).on_hover_text("Capture MIDI: turn what you just played into a clip").clicked() {
        app.capture_midi();
    }
    let released = (0..app.project.tracks.len()).any(|t| !app.track_status(t).arrangement_active.load(Ordering::Relaxed));
    let lit = released || app.automation_overridden;
    if toggle(ui, lit, "↺ Arr", theme::QUEUED, 40.0).on_hover_text("Back to Arrangement").clicked() {
        app.rt.send(Command::BackToArrangement);
        app.automation_overridden = false;
    }
    let t = &mut app.project.transport;
    if toggle(ui, t.loop_enabled, "Loop", theme::QUEUED, 34.0).clicked() {
        t.loop_enabled = !t.loop_enabled;
    }
    ui.separator();
    ui.label(RichText::new("Quantize").color(theme::TEXT_DIM));
    egui::ComboBox::from_id_salt("gq").width(64.0).selected_text(t.global_quantize.label()).show_ui(ui, |ui| {
        for q in Quantize::ALL {
            ui.selectable_value(&mut t.global_quantize, q, q.label());
        }
    });
    ui.separator();
    if toggle(ui, app.kb_enabled, "KB", theme::ACCENT, 24.0).on_hover_text(format!("Computer MIDI keyboard (octave {}, velocity {})", app.kb.octave, app.kb.velocity)).clicked() {
        app.kb_enabled = !app.kb_enabled;
    }
    if toggle(ui, app.map_mode == MapMode::Key, "KEY", theme::ACCENT, 30.0).on_hover_text("Key Map mode").clicked() {
        app.map_mode = if app.map_mode == MapMode::Key { MapMode::Off } else { MapMode::Key };
        app.map_target = None;
    }
    if toggle(ui, app.map_mode == MapMode::Midi, "MIDI", theme::ACCENT, 32.0).on_hover_text("MIDI Map mode").clicked() {
        app.map_mode = if app.map_mode == MapMode::Midi { MapMode::Off } else { MapMode::Midi };
        app.map_target = None;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if toggle(ui, app.view == View::Arrangement, "Arrangement", theme::ACCENT, 76.0).clicked() {
            app.view = View::Arrangement;
        }
        if toggle(ui, app.view == View::Session, "Session", theme::ACCENT, 56.0).clicked() {
            app.view = View::Session;
        }
        cpu_meter(app, ui);
    });
}

fn tap_tempo(app: &mut XdawApp, ui: &Ui) {
    let now = ui.input(|i| i.time);
    let id = egui::Id::new("tap_times");
    let mut taps: Vec<f64> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    taps.retain(|&t| now - t < 3.0);
    taps.push(now);
    if taps.len() >= 2 {
        let span = taps.last().unwrap() - taps.first().unwrap();
        let bpm = 60.0 * (taps.len() - 1) as f64 / span;
        app.project.transport.bpm = xdaw_sequencer::time::clamp_bpm((bpm * 100.0).round() / 100.0);
        app.label("Tap Tempo");
    }
    ui.data_mut(|d| d.insert_temp(id, taps));
}

fn cpu_meter(app: &mut XdawApp, ui: &mut Ui) {
    let cb = &app.rt.status.callback;
    let last = cb.last_ns.load(Ordering::Relaxed) as f64;
    let budget = cb.budget_ns.load(Ordering::Relaxed).max(1) as f64;
    let misses = cb.deadline_misses.load(Ordering::Relaxed);
    let pct = last / budget * 100.0;
    let color = if misses > 0 { theme::RECORD } else { theme::TEXT_DIM };
    ui.label(RichText::new(format!("CPU {pct:4.1}%")).monospace().color(color)).on_hover_text(format!(
        "Callback {:.3} ms of {:.3} ms budget\nmax {:.3} ms · p99 ≤ {:.1} ms\ndeadline misses: {misses}\nstream errors: {}",
        last / 1e6,
        budget / 1e6,
        cb.max_ns.load(Ordering::Relaxed) as f64 / 1e6,
        cb.percentile_ms(0.99),
        app.rt.status.stream_errors.load(Ordering::Relaxed)
    ));
}

pub fn status_bar(app: &mut XdawApp, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        ui.label(RichText::new(&app.status_line).size(11.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let audio = match (&app.rt.output, &app.rt.error) {
                (Some(o), _) => format!("{} · {} Hz · {} smp{}", o.device, o.sample_rate, o.buffer_size, o.input_device.as_ref().map(|i| format!(" · in: {i}")).unwrap_or_default()),
                (None, Some(e)) => format!("Audio off: {e}"),
                (None, None) => "Audio off".into(),
            };
            ui.label(RichText::new(audio).size(11.0).color(theme::TEXT_DIM));
            let midi = app.rt.midi.as_ref().map(|m| m.connected()).unwrap_or(0);
            ui.label(RichText::new(format!("MIDI in: {midi}")).size(11.0).color(theme::TEXT_DIM));
            if app.map_mode != MapMode::Off {
                ui.label(RichText::new(match &app.map_target {
                    Some(t) => format!("Mapping target: {t:?} — press a key / move a control"),
                    None => "Click a control to map".into(),
                }).size(11.0).color(theme::ACCENT));
            }
        });
    });
}
