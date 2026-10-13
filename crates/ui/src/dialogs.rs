//! Modal and floating windows: rename, export, audio settings, recovery, missing media.

use crate::app::XdawApp;
use crate::theme;
use egui::RichText;
use std::path::PathBuf;
use std::sync::Arc;
use xdaw_engine::audio_io::{self, BUFFER_SIZES, SUPPORTED_RATES};
use xdaw_engine::export::{ExportReport, ExportSettings, SampleFormat};
use xdaw_project::*;

#[derive(Clone, Copy, Debug)]
pub enum RenameTarget {
    Track(TrackRef),
    Clip(usize, usize),
    Scene(usize),
}

pub struct ExportState {
    pub range_loop: bool,
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub dither: bool,
    pub tail: f64,
    pub running: Option<crossbeam_channel::Receiver<Result<ExportReport, String>>>,
    pub result: Option<Result<(PathBuf, ExportReport), String>>,
    pub path: Option<PathBuf>,
}

impl Default for ExportState {
    fn default() -> Self {
        Self { range_loop: false, sample_rate: 48_000, format: SampleFormat::Pcm24, dither: true, tail: 2.0, running: None, result: None, path: None }
    }
}

#[derive(Default)]
pub struct Dialogs {
    pub rename: Option<(RenameTarget, String)>,
    pub export_open: bool,
    pub export: ExportState,
    pub settings_open: bool,
    pub settings_draft: Option<audio_io::AudioSettings>,
    pub recovery: Option<(PathBuf, Option<PathBuf>)>,
    pub missing: Vec<MediaRef>,
    pub open_requested: bool,
    pub new_requested: bool,
    pub confirm_discard: Option<PendingFileAction>,
}

#[derive(Clone, Debug)]
pub enum PendingFileAction {
    New,
    Open(PathBuf),
}

pub fn show(app: &mut XdawApp, ctx: &egui::Context) {
    rename(app, ctx);
    export(app, ctx);
    settings(app, ctx);
    recovery(app, ctx);
    missing(app, ctx);
    file_actions(app, ctx);
}

fn rename(app: &mut XdawApp, ctx: &egui::Context) {
    let Some((target, mut text)) = app.dialogs.rename.clone() else { return };
    let mut open = true;
    let mut done = false;
    egui::Window::new("Rename").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        let resp = ui.text_edit_singleline(&mut text);
        resp.request_focus();
        if ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            done = true;
        }
    });
    if done {
        match target {
            RenameTarget::Track(r) => app.project.track_mut(r).name = text.clone(),
            RenameTarget::Clip(t, s) => {
                if let Some(c) = app.project.tracks.get_mut(t).and_then(|t| t.slots.get_mut(s)?.clip.as_mut()) {
                    c.name = text.clone();
                }
            }
            RenameTarget::Scene(s) => {
                if let Some(sc) = app.project.scenes.get_mut(s) {
                    sc.name = text.clone();
                }
            }
        }
        app.label("Rename");
        app.dialogs.rename = None;
    } else if !open {
        app.dialogs.rename = None;
    } else {
        app.dialogs.rename = Some((target, text));
    }
}

fn export(app: &mut XdawApp, ctx: &egui::Context) {
    if !app.dialogs.export_open {
        return;
    }
    let mut open = true;
    let song_end = app.project.tracks.iter().flat_map(|t| t.arrangement.iter().map(|c| c.end())).fold(0.0, f64::max);
    egui::Window::new("Export Audio").collapsible(false).open(&mut open).show(ctx, |ui| {
        let st = &mut app.dialogs.export;
        ui.label(RichText::new("Renders the Arrangement through the same engine used for playback (metronome excluded).").size(10.5).color(theme::TEXT_DIM));
        ui.radio_value(&mut st.range_loop, false, format!("Whole Arrangement (0 – {song_end:.1} beats)"));
        let t = &app.project.transport;
        ui.radio_value(&mut st.range_loop, true, format!("Loop region ({:.1} – {:.1} beats)", t.loop_start, t.loop_start + t.loop_length));
        egui::ComboBox::from_label("Sample rate").selected_text(format!("{} Hz", st.sample_rate)).show_ui(ui, |ui| {
            for r in SUPPORTED_RATES {
                ui.selectable_value(&mut st.sample_rate, r, format!("{r} Hz"));
            }
        });
        egui::ComboBox::from_label("Format").selected_text(st.format.label()).show_ui(ui, |ui| {
            for f in SampleFormat::ALL {
                ui.selectable_value(&mut st.format, f, f.label());
            }
        });
        ui.add_enabled(st.format != SampleFormat::Float32, egui::Checkbox::new(&mut st.dither, "TPDF dither"));
        ui.horizontal(|ui| {
            ui.label("Tail (effects ring-out)");
            ui.add(egui::DragValue::new(&mut st.tail).range(0.0..=30.0).suffix(" s"));
        });
        if st.running.is_some() {
            ui.spinner();
        } else if ui.button("Export…").clicked() {
            let (start, end) = if st.range_loop { (t.loop_start, t.loop_start + t.loop_length) } else { (0.0, song_end) };
            if end <= start {
                st.result = Some(Err("The Arrangement is empty; record or place clips first, or export the loop region".into()));
            } else if let Some(path) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).set_file_name("Export.wav").save_file() {
                let settings = ExportSettings { start, end, tail_seconds: st.tail, sample_rate: st.sample_rate, format: st.format, dither: st.dither };
                let project = app.project.clone();
                let media: std::collections::HashMap<String, Arc<xdaw_dsp::SampleData>> = app.media.loaded.clone();
                let (tx, rx) = crossbeam_channel::bounded(1);
                let p2 = path.clone();
                std::thread::spawn(move || {
                    let r = xdaw_engine::export::export_wav(&project, &media, &settings, &p2).map_err(|e| e.to_string());
                    let _ = tx.send(r);
                });
                st.running = Some(rx);
                st.path = Some(path);
            }
        }
        if let Some(rx) = &st.running {
            if let Ok(r) = rx.try_recv() {
                st.result = Some(r.map(|rep| (st.path.clone().unwrap_or_default(), rep)));
                st.running = None;
            }
        }
        match &st.result {
            Some(Ok((p, r))) => {
                ui.label(format!("Wrote {}", p.display()));
                ui.label(format!("{:.1} s · peak {:.1} dBFS · clipped samples {}", r.frames as f64 / st.sample_rate as f64, 20.0 * r.peak.max(1e-9).log10(), r.clipped_samples));
                if r.clipped_samples > 0 {
                    ui.colored_label(theme::RECORD, "Output clipped: lower the master volume and export again.");
                }
            }
            Some(Err(e)) => {
                ui.colored_label(theme::RECORD, e);
            }
            None => {}
        }
    });
    app.dialogs.export_open = open;
}

fn settings(app: &mut XdawApp, ctx: &egui::Context) {
    if !app.dialogs.settings_open {
        app.dialogs.settings_draft = None;
        return;
    }
    let mut open = true;
    let mut apply = false;
    let draft = app.dialogs.settings_draft.get_or_insert_with(|| app.rt.settings.clone());
    egui::Window::new("Audio / MIDI Settings").collapsible(false).open(&mut open).show(ctx, |ui| {
        let outputs = audio_io::output_devices();
        let inputs = audio_io::input_devices();
        egui::ComboBox::from_label("Output device").selected_text(draft.output_device.clone().unwrap_or_else(|| "System default".into())).show_ui(ui, |ui| {
            ui.selectable_value(&mut draft.output_device, None, "System default");
            for d in outputs {
                ui.selectable_value(&mut draft.output_device, Some(d.clone()), d);
            }
        });
        ui.checkbox(&mut draft.enable_input, "Enable audio input (recording/monitoring)");
        ui.add_enabled_ui(draft.enable_input, |ui| {
            egui::ComboBox::from_label("Input device").selected_text(draft.input_device.clone().unwrap_or_else(|| "System default".into())).show_ui(ui, |ui| {
                ui.selectable_value(&mut draft.input_device, None, "System default");
                for d in inputs {
                    ui.selectable_value(&mut draft.input_device, Some(d.clone()), d);
                }
            });
        });
        egui::ComboBox::from_label("Sample rate").selected_text(format!("{} Hz", draft.sample_rate)).show_ui(ui, |ui| {
            for r in SUPPORTED_RATES {
                ui.selectable_value(&mut draft.sample_rate, r, format!("{r} Hz"));
            }
        });
        egui::ComboBox::from_label("Buffer size").selected_text(format!("{} samples ({:.2} ms)", draft.buffer_size, draft.buffer_size as f64 / draft.sample_rate as f64 * 1000.0)).show_ui(ui, |ui| {
            for b in BUFFER_SIZES {
                ui.selectable_value(&mut draft.buffer_size, b, format!("{b}"));
            }
        });
        ui.label(RichText::new("Applying restarts the audio engine and stops playback.").size(10.0).color(theme::TEXT_DIM));
        if ui.button("Apply").clicked() {
            apply = true;
        }
        ui.separator();
        let cb = &app.rt.status.callback;
        let count = cb.count.load(std::sync::atomic::Ordering::Relaxed);
        ui.label(format!(
            "Callbacks {count} · mean {:.3} ms · max {:.3} ms · p99 ≤ {:.1} ms · budget {:.3} ms",
            cb.total_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / count.max(1) as f64 / 1e6,
            cb.max_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6,
            cb.percentile_ms(0.99),
            cb.budget_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
        ));
        ui.label(format!(
            "Deadline misses {} · stream errors {} · input overruns {}",
            cb.deadline_misses.load(std::sync::atomic::Ordering::Relaxed),
            app.rt.status.stream_errors.load(std::sync::atomic::Ordering::Relaxed),
            app.rt.status.input_overruns.load(std::sync::atomic::Ordering::Relaxed)
        ));
        if ui.small_button("Reset statistics").clicked() {
            cb.reset();
        }
        ui.separator();
        match (&app.rt.midi, &app.rt.midi_error) {
            (Some(m), _) if !m.port_names.is_empty() => {
                ui.label("MIDI inputs:");
                for n in &m.port_names {
                    ui.label(format!("  • {n}"));
                }
            }
            (_, Some(e)) => {
                ui.colored_label(theme::RECORD, format!("MIDI unavailable: {e}"));
            }
            _ => {
                ui.label("No MIDI input ports found. The computer keyboard can play armed MIDI tracks.");
            }
        }
    });
    if apply {
        app.rt.settings = draft.clone();
        app.dialogs.settings_draft = None;
        app.start_audio();
    }
    app.dialogs.settings_open = open;
}

fn recovery(app: &mut XdawApp, ctx: &egui::Context) {
    let Some((auto, project)) = app.dialogs.recovery.clone() else { return };
    egui::Window::new("Recover unsaved work?").collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label("XDAW found an autosave newer than the saved Set (the previous session may have ended unexpectedly).");
        ui.label(RichText::new(auto.display().to_string()).size(10.0).color(theme::TEXT_DIM));
        ui.horizontal(|ui| {
            if ui.button("Recover").clicked() {
                match io::load(&auto) {
                    Ok(p) => {
                        app.project = p;
                        app.path = project.clone();
                        app.status_line = "Recovered autosave — save to keep it".into();
                        app.request_media();
                        app.media_gen += 1;
                    }
                    Err(e) => app.status_line = format!("Recovery failed: {e}"),
                }
                app.dialogs.recovery = None;
            }
            if ui.button("Discard").clicked() {
                io::discard_autosave(project.as_deref());
                app.dialogs.recovery = None;
            }
        });
    });
}

fn missing(app: &mut XdawApp, ctx: &egui::Context) {
    if app.dialogs.missing.is_empty() {
        return;
    }
    let mut open = true;
    egui::Window::new("Missing media").open(&mut open).show(ctx, |ui| {
        ui.label("These files could not be found. Their clips stay in the Set and play silence.");
        for m in app.dialogs.missing.clone() {
            ui.horizontal(|ui| {
                ui.label(&m.path);
                if ui.small_button("Locate…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        let new = crate::browser::media_ref_for(app, &p);
                        app.project.for_each_media_mut(|x| {
                            if *x == m {
                                *x = new.clone();
                            }
                        });
                        app.dialogs.missing.retain(|x| *x != m);
                        app.request_media();
                        app.label("Relink Media");
                    }
                }
            });
        }
    });
    if !open {
        app.dialogs.missing.clear();
    }
}

fn file_actions(app: &mut XdawApp, ctx: &egui::Context) {
    if std::mem::take(&mut app.dialogs.new_requested) {
        app.dialogs.confirm_discard = Some(PendingFileAction::New);
    }
    if std::mem::take(&mut app.dialogs.open_requested) {
        if let Some(p) = rfd::FileDialog::new().add_filter("XDAW project", &[io::EXTENSION]).pick_file() {
            app.dialogs.confirm_discard = Some(PendingFileAction::Open(p));
        }
    }
    let Some(action) = app.dialogs.confirm_discard.clone() else { return };
    if !app.is_dirty() {
        run(app, action);
        app.dialogs.confirm_discard = None;
        return;
    }
    egui::Window::new("Unsaved changes").collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label("Save changes to the current Set first?");
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() && app.save() {
                run(app, action.clone());
                app.dialogs.confirm_discard = None;
            }
            if ui.button("Don't Save").clicked() {
                io::discard_autosave(app.path.as_deref());
                run(app, action.clone());
                app.dialogs.confirm_discard = None;
            }
            if ui.button("Cancel").clicked() {
                app.dialogs.confirm_discard = None;
            }
        });
    });
}

fn run(app: &mut XdawApp, action: PendingFileAction) {
    match action {
        PendingFileAction::New => app.new_project(),
        PendingFileAction::Open(p) => app.open_path(&p),
    }
}
