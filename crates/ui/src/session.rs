//! Session View: tracks as columns, scenes as rows, mixer below.

use crate::app::{ClipLoc, Detail, MapMode, XdawApp};
use crate::browser::{self, BrowserItem};
use crate::dialogs::RenameTarget;
use crate::{mixer, theme};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui};
use std::sync::atomic::Ordering;
use xdaw_engine::protocol::{Command, NO_SLOT, QUEUED_STOP};
use xdaw_project::*;
use xdaw_sequencer::{ClipQuantize, ClipRegion, LaunchMode, Quantize};

pub const COL_W: f32 = 112.0;
pub const SLOT_H: f32 = 22.0;
const HEADER_H: f32 = 22.0;

#[derive(Clone, Copy, Debug)]
pub struct SlotDrag {
    pub track: usize,
    pub slot: usize,
}

pub fn show(app: &mut XdawApp, ui: &mut Ui) {
    let scenes = app.project.scenes.len();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for t in 0..app.project.tracks.len() {
                ui.vertical(|ui| {
                    ui.set_width(COL_W);
                    track_header(app, ui, TrackRef::Track(t));
                    for s in 0..scenes {
                        slot(app, ui, t, s);
                    }
                    track_status(app, ui, t);
                    ui.add_space(4.0);
                    mixer::strip(app, ui, TrackRef::Track(t), COL_W);
                });
            }
            // Drop zone to create tracks from browser items.
            ui.vertical(|ui| {
                ui.set_width(70.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(70.0, HEADER_H + SLOT_H * scenes as f32), Sense::hover());
                ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::GRID), StrokeKind::Inside);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Drop to\ncreate\ntrack", FontId::proportional(10.0), theme::TEXT_DIM);
                if let Some(item) = resp.dnd_release_payload::<BrowserItem>() {
                    create_track_from(app, (*item).clone());
                }
            });
            ui.separator();
            for r in 0..app.project.returns.len() {
                ui.vertical(|ui| {
                    ui.set_width(COL_W * 0.8);
                    track_header(app, ui, TrackRef::Return(r));
                    ui.add_space(SLOT_H * scenes as f32 + SLOT_H + 4.0 + 3.0 * scenes as f32);
                    mixer::strip(app, ui, TrackRef::Return(r), COL_W * 0.8);
                });
            }
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(COL_W);
                track_header(app, ui, TrackRef::Master);
                for s in 0..scenes {
                    scene_launcher(app, ui, s);
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click());
                ui.painter().rect_filled(rect.shrink(2.0), 2.0, theme::SLOT);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "■ Stop All Clips", FontId::proportional(11.0), theme::TEXT);
                if resp.clicked() {
                    app.rt.send(Command::StopAllClips);
                }
                ui.add_space(4.0);
                mixer::strip(app, ui, TrackRef::Master, COL_W);
            });
        });
    });
}

fn create_track_from(app: &mut XdawApp, item: BrowserItem) {
    match item {
        BrowserItem::Device(k) => {
            let kind = if k.is_instrument() { TrackKind::Midi } else { TrackKind::Audio };
            let n = app.project.tracks.len() + 1;
            let t = app.project.add_track(kind, &format!("{n} {}", k.name()));
            app.project.add_device(TrackRef::Track(t), k);
            app.label("Create Track");
        }
        BrowserItem::File(p) => {
            let kind = if browser::is_midi(&p) { TrackKind::Midi } else { TrackKind::Audio };
            let n = app.project.tracks.len() + 1;
            let t = app.project.add_track(kind, &format!("{n} {}", if kind == TrackKind::Midi { "MIDI" } else { "Audio" }));
            if kind == TrackKind::Midi {
                app.project.add_device(TrackRef::Track(t), xdaw_dsp::DeviceKind::PolySynth);
            }
            browser::drop_file_on_slot(app, t, 0, &p);
        }
        BrowserItem::Preset(_) => {}
    }
}

pub fn track_header(app: &mut XdawApp, ui: &mut Ui, r: TrackRef) {
    let w = ui.available_width().min(COL_W);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, HEADER_H), Sense::click());
    let t = app.project.track(r);
    let selected = app.sel.track == Some(r);
    let color = theme::rgb(t.color);
    let group_color = t.group.and_then(|g| app.project.tracks.iter().find(|x| x.id == g)).map(|g| theme::rgb(g.color));
    let is_group = t.kind == TrackKind::Group;
    let p = ui.painter();
    p.rect_filled(rect.shrink(1.0), 2.0, if selected { color } else { theme::dim(color, 0.55) });
    if let Some(gc) = group_color {
        p.rect_filled(Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(4.0, rect.height() - 2.0)), 0.0, gc);
    }
    let label = if is_group { format!("[{}]", t.name) } else { t.name.clone() };
    p.text(rect.left_center() + vec2(7.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(11.5), if selected { Color32::BLACK } else { theme::TEXT });
    if resp.clicked() {
        app.select_track(r);
        app.detail = Detail::Devices;
    }
    if let Some(item) = resp.dnd_release_payload::<BrowserItem>() {
        match (*item).clone() {
            BrowserItem::Device(k) => browser::add_device(app, r, k),
            BrowserItem::Preset(path) => browser::load_preset(app, r, &path),
            BrowserItem::File(_) => {}
        }
    }
    resp.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            let name = app.project.track(r).name.clone();
            app.dialogs.rename = Some((RenameTarget::Track(r), name));
            ui.close();
        }
        ui.menu_button("Color", |ui| color_menu(ui, &mut app.project.track_mut(r).color));
        if let TrackRef::Track(i) = r {
            if ui.button("Duplicate Track").clicked() {
                let mut copy = app.project.tracks[i].clone();
                copy.id = app.project.alloc_id();
                copy.name = format!("{} copy", copy.name);
                for d in copy.devices.iter_mut() {
                    let old = d.id;
                    d.id = app.project.alloc_id();
                    for lane in copy.automation.iter_mut() {
                        if let ParamTarget::Device { device, .. } = &mut lane.target {
                            if *device == old {
                                *device = d.id;
                            }
                        }
                    }
                }
                for s in copy.slots.iter_mut().filter_map(|s| s.clip.as_mut()) {
                    s.id = app.project.alloc_id();
                }
                for a in copy.arrangement.iter_mut() {
                    a.id = app.project.alloc_id();
                }
                app.project.tracks.insert(i + 1, copy);
                app.label("Duplicate Track");
                ui.close();
            }
            if app.project.tracks[i].kind != TrackKind::Group {
                let groups: Vec<(Id, String)> = app.project.tracks.iter().filter(|g| g.kind == TrackKind::Group).map(|g| (g.id, g.name.clone())).collect();
                if !groups.is_empty() || app.project.tracks[i].group.is_some() {
                    ui.menu_button("Group", |ui| {
                        for (gid, name) in groups {
                            if ui.button(format!("Add to {name}")).clicked() {
                                app.project.tracks[i].group = Some(gid);
                                app.label("Group Track");
                                ui.close();
                            }
                        }
                        if app.project.tracks[i].group.is_some() && ui.button("Remove from group").clicked() {
                            app.project.tracks[i].group = None;
                            app.label("Ungroup Track");
                            ui.close();
                        }
                    });
                }
            }
            if app.project.tracks[i].kind != TrackKind::Group && ui.button("Flatten Arrangement to Audio").clicked() {
                app.flatten_track(i);
                ui.close();
            }
            if ui.button("Delete Track").clicked() {
                app.rt.send(Command::StopTrack { track: i as u16 });
                let removed = app.project.tracks.remove(i);
                if removed.kind == TrackKind::Group {
                    for t in app.project.tracks.iter_mut().filter(|t| t.group == Some(removed.id)) {
                        t.group = None;
                    }
                }
                app.sel = Default::default();
                app.label("Delete Track");
                ui.close();
            }
        }
        if let TrackRef::Return(i) = r {
            if ui.button("Delete Return Track").clicked() {
                app.project.returns.remove(i);
                for t in app.project.tracks.iter_mut() {
                    if i < t.mixer.sends_db.len() {
                        t.mixer.sends_db.remove(i);
                    }
                    t.automation.retain(|l| !matches!(l.target, ParamTarget::Send(s) if s == i));
                    for l in t.automation.iter_mut() {
                        if let ParamTarget::Send(s) = &mut l.target {
                            if *s > i {
                                *s -= 1;
                            }
                        }
                    }
                }
                app.sel = Default::default();
                app.label("Delete Return");
                ui.close();
            }
        }
    });
}

pub fn color_menu(ui: &mut Ui, color: &mut [u8; 3]) {
    ui.horizontal_wrapped(|ui| {
        for c in TRACK_COLORS {
            let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
            ui.painter().rect_filled(rect, 2.0, theme::rgb(c));
            if resp.clicked() {
                *color = c;
            }
        }
    });
}

fn members(app: &XdawApp, g: usize) -> Vec<usize> {
    let gid = app.project.tracks[g].id;
    (0..app.project.tracks.len()).filter(|&i| app.project.tracks[i].group == Some(gid)).collect()
}

/// Group slot: launches every member clip in the scene (stop buttons stop the others).
fn group_slot(app: &mut XdawApp, ui: &mut Ui, g: usize, s: usize) {
    let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click());
    let m = members(app, g);
    let with_clip: Vec<usize> = m.iter().copied().filter(|&t| app.project.tracks[t].slots[s].clip.is_some()).collect();
    let playing = with_clip.iter().any(|&t| app.track_status(t).playing_slot.load(Ordering::Relaxed) == s as i32);
    let inner = rect.shrink2(vec2(2.0, 1.5));
    let p = ui.painter();
    let color = theme::rgb(app.project.tracks[g].color);
    p.rect_filled(inner, 2.0, if with_clip.is_empty() { theme::SLOT } else { theme::dim(color, if playing { 0.9 } else { 0.4 }) });
    let c0 = pos2(inner.left() + 9.0, inner.center().y);
    if with_clip.is_empty() {
        p.rect_filled(Rect::from_center_size(c0, vec2(7.0, 7.0)), 1.0, theme::GRID);
    } else {
        p.add(egui::Shape::convex_polygon(vec![c0 + vec2(-4.0, -5.0), c0 + vec2(5.0, 0.0), c0 + vec2(-4.0, 5.0)], if playing { theme::PLAYING } else { theme::TEXT }, Stroke::NONE));
    }
    if resp.clicked() {
        for t in m {
            if app.project.tracks[t].slots[s].clip.is_some() {
                app.rt.send(Command::LaunchSlot { track: t as u16, slot: s as u16, down: true });
                app.rt.send(Command::LaunchSlot { track: t as u16, slot: s as u16, down: false });
            } else if app.project.tracks[t].slots[s].has_stop_button && !with_clip.is_empty() {
                app.rt.send(Command::StopTrack { track: t as u16 });
            } else if with_clip.is_empty() {
                app.rt.send(Command::StopTrack { track: t as u16 });
            }
        }
    }
}

fn slot(app: &mut XdawApp, ui: &mut Ui, t: usize, s: usize) {
    if app.project.tracks[t].kind == TrackKind::Group {
        return group_slot(app, ui, t, s);
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click_and_drag());
    let st = app.track_status(t);
    let playing = st.playing_slot.load(Ordering::Relaxed) == s as i32;
    let queued = st.queued.load(Ordering::Relaxed) == s as i32;
    let recording = st.recording_slot.load(Ordering::Relaxed) == s as i32;
    let pos = st.clip_position.load();
    let track = &app.project.tracks[t];
    let armed = track.mixer.arm;
    let slot_data = &track.slots[s];
    let selected = app.sel.clip == Some(ClipLoc::Slot { track: t, slot: s });
    let inner = rect.shrink2(vec2(2.0, 1.5));
    let button = Rect::from_min_size(inner.min, vec2(SLOT_H - 3.0, inner.height()));
    let time = ui.input(|i| i.time);
    let blink = (time * 4.0).fract() < 0.5;
    let p = ui.painter();
    match &slot_data.clip {
        Some(c) => {
            let color = theme::rgb(c.color);
            p.rect_filled(inner, 2.0, if playing { color } else { theme::dim(color, 0.6) });
            if playing && pos >= 0.0 {
                let frac = ((pos - c.region.start) / c.length().max(1e-6)).clamp(0.0, 1.0) as f32;
                let bar = Rect::from_min_size(pos2(inner.left(), inner.bottom() - 2.0), vec2(inner.width() * frac, 2.0));
                p.rect_filled(bar, 0.0, Color32::WHITE);
            }
            let tri_color = if recording {
                theme::RECORD
            } else if playing {
                theme::PLAYING
            } else if queued && blink {
                theme::QUEUED
            } else {
                Color32::from_gray(30)
            };
            let c0 = button.center();
            if recording {
                p.circle_filled(c0, 5.0, tri_color);
            } else {
                p.add(egui::Shape::convex_polygon(
                    vec![c0 + vec2(-4.0, -5.0), c0 + vec2(5.0, 0.0), c0 + vec2(-4.0, 5.0)],
                    tri_color,
                    Stroke::NONE,
                ));
            }
            p.text(pos2(button.right() + 3.0, inner.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(11.0), if playing { Color32::BLACK } else { theme::TEXT });
        }
        None => {
            p.rect_filled(inner, 2.0, theme::SLOT);
            let c0 = button.center();
            if recording {
                p.circle_filled(c0, 5.0, if blink { theme::RECORD } else { theme::dim(theme::RECORD, 0.5) });
            } else if armed {
                p.circle_stroke(c0, 4.5, Stroke::new(1.5, theme::RECORD));
            } else if slot_data.has_stop_button {
                p.rect_filled(Rect::from_center_size(c0, vec2(7.0, 7.0)), 1.0, theme::GRID);
            }
        }
    }
    if selected {
        p.rect_stroke(inner, 2.0, Stroke::new(1.5, theme::SELECT), StrokeKind::Inside);
    }

    let has_clip = slot_data.clip.is_some();
    let on_button = resp.interact_pointer_pos().is_some_and(|q| button.contains(q)) || resp.hover_pos().is_some_and(|q| button.contains(q));
    let mapping = app.map_mode != MapMode::Off;
    if mapping && resp.clicked() {
        let (track_id, scene_id) = (app.project.tracks[t].id, app.project.scenes[s].id);
        app.map_target = Some(MappingTarget::LaunchSlot { track: track_id, scene: scene_id });
        return;
    }
    // Launch on press, release for Gate/Repeat.
    let held_id = egui::Id::new(("held_launch", t, s));
    if resp.is_pointer_button_down_on() && on_button && !ui.data(|d| d.get_temp::<bool>(held_id).unwrap_or(false)) {
        ui.data_mut(|d| d.insert_temp(held_id, true));
        if has_clip {
            app.rt.send(Command::LaunchSlot { track: t as u16, slot: s as u16, down: true });
            if app.project.tracks[t].kind == TrackKind::Midi || true {
                app.sel.clip = Some(ClipLoc::Slot { track: t, slot: s });
            }
        } else if armed {
            app.rt.send(Command::SessionRecord { track: t as u16, slot: s as u16, on: true });
        } else if app.project.tracks[t].slots[s].has_stop_button {
            app.rt.send(Command::StopTrack { track: t as u16 });
        }
    }
    if ui.data(|d| d.get_temp::<bool>(held_id).unwrap_or(false)) && !resp.is_pointer_button_down_on() {
        ui.data_mut(|d| d.insert_temp(held_id, false));
        if has_clip {
            app.rt.send(Command::LaunchSlot { track: t as u16, slot: s as u16, down: false });
        }
    }
    if resp.clicked() && !on_button {
        app.sel.clip = has_clip.then_some(ClipLoc::Slot { track: t, slot: s });
        app.sel.track = Some(TrackRef::Track(t));
        app.sel.scene = s;
        app.sel.notes.clear();
        if has_clip {
            app.detail = Detail::Clip;
        }
    }
    if resp.double_clicked() && !has_clip && app.project.tracks[t].kind == TrackKind::Midi {
        let bar = app.project.transport.time_signature.bar_beats();
        let clip = Clip {
            id: app.project.alloc_id(),
            name: "MIDI Clip".into(),
            color: app.project.tracks[t].color,
            content: ClipContent::Midi { notes: vec![] },
            region: ClipRegion::looped(bar),
            launch_mode: LaunchMode::Trigger,
            quantize: ClipQuantize::Global,
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        };
        app.project.tracks[t].slots[s].clip = Some(clip);
        app.sel.clip = Some(ClipLoc::Slot { track: t, slot: s });
        app.detail = Detail::Clip;
        app.label("Insert MIDI Clip");
    }

    // Drag & drop.
    if has_clip && resp.drag_started() && !on_button {
        resp.dnd_set_drag_payload(SlotDrag { track: t, slot: s });
    }
    if let Some(drag) = resp.dnd_release_payload::<SlotDrag>() {
        let copy = ui.input(|i| i.modifiers.alt || i.modifiers.command);
        move_clip(app, *drag, t, s, copy);
    }
    if let Some(item) = resp.dnd_release_payload::<BrowserItem>() {
        match (*item).clone() {
            BrowserItem::File(path) => browser::drop_file_on_slot(app, t, s, &path),
            BrowserItem::Device(k) => browser::add_device(app, TrackRef::Track(t), k),
            BrowserItem::Preset(path) => browser::load_preset(app, TrackRef::Track(t), &path),
        }
    }
    if resp.dnd_hover_payload::<SlotDrag>().is_some() || resp.dnd_hover_payload::<BrowserItem>().is_some() {
        ui.painter().rect_stroke(inner, 2.0, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
    }

    resp.context_menu(|ui| slot_menu(app, ui, t, s));
}

fn slot_menu(app: &mut XdawApp, ui: &mut Ui, t: usize, s: usize) {
    let has_clip = app.project.tracks[t].slots[s].clip.is_some();
    if has_clip {
        if ui.button("Rename").clicked() {
            let name = app.project.tracks[t].slots[s].clip.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            app.dialogs.rename = Some((RenameTarget::Clip(t, s), name));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            duplicate_clip(app, t, s);
            ui.close();
        }
        if ui.button("Delete").clicked() {
            app.project.tracks[t].slots[s].clip = None;
            app.label("Delete Clip");
            ui.close();
        }
        if let Some(c) = app.project.tracks[t].slots[s].clip.as_mut() {
            ui.menu_button("Color", |ui| color_menu(ui, &mut c.color));
            ui.menu_button("Launch Mode", |ui| {
                for m in LaunchMode::ALL {
                    ui.radio_value(&mut c.launch_mode, m, m.label());
                }
            });
            ui.menu_button("Quantization", |ui| {
                ui.radio_value(&mut c.quantize, ClipQuantize::Global, "Global");
                for q in Quantize::ALL {
                    ui.radio_value(&mut c.quantize, ClipQuantize::Fixed(q), q.label());
                }
            });
            ui.checkbox(&mut c.legato, "Legato");
        }
        if ui.button("Copy to Arrangement at Insert Marker").clicked() {
            crate::arrangement::copy_slot_to_arrangement(app, t, s);
            ui.close();
        }
    } else if app.project.tracks[t].kind == TrackKind::Midi && ui.button("Insert MIDI Clip").clicked() {
        let bar = app.project.transport.time_signature.bar_beats();
        let clip = Clip {
            id: app.project.alloc_id(),
            name: "MIDI Clip".into(),
            color: app.project.tracks[t].color,
            content: ClipContent::Midi { notes: vec![] },
            region: ClipRegion::looped(bar),
            launch_mode: LaunchMode::Trigger,
            quantize: ClipQuantize::Global,
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        };
        app.project.tracks[t].slots[s].clip = Some(clip);
        app.label("Insert MIDI Clip");
        ui.close();
    }
    let slot = &mut app.project.tracks[t].slots[s];
    let label = if slot.has_stop_button { "Remove Stop Button" } else { "Add Stop Button" };
    if ui.button(label).clicked() {
        slot.has_stop_button = !slot.has_stop_button;
        app.label(label);
        ui.close();
    }
}

fn move_clip(app: &mut XdawApp, from: SlotDrag, t: usize, s: usize, copy: bool) {
    if (from.track, from.slot) == (t, s) {
        return;
    }
    let Some(mut clip) = app.project.tracks[from.track].slots[from.slot].clip.clone() else { return };
    if clip.is_midi() != (app.project.tracks[t].kind == TrackKind::Midi) {
        app.status_line = "Clip type does not match the target track".into();
        return;
    }
    if copy {
        clip.id = app.project.alloc_id();
    } else {
        app.project.tracks[from.track].slots[from.slot].clip = None;
    }
    app.project.tracks[t].slots[s].clip = Some(clip);
    app.sel.clip = Some(ClipLoc::Slot { track: t, slot: s });
    app.label(if copy { "Copy Clip" } else { "Move Clip" });
}

pub fn place_clip(app: &mut XdawApp, t: usize, s: usize, mut clip: Clip) {
    if clip.is_midi() != (app.project.tracks[t].kind == TrackKind::Midi) {
        app.status_line = "Clip type does not match the target track".into();
        return;
    }
    clip.id = app.project.alloc_id();
    app.project.tracks[t].slots[s].clip = Some(clip);
    app.label("Paste Clip");
}

/// Duplicates into the next slot below, inserting a scene when needed.
pub fn duplicate_clip(app: &mut XdawApp, t: usize, s: usize) {
    let Some(mut clip) = app.project.tracks[t].slots[s].clip.clone() else { return };
    clip.id = app.project.alloc_id();
    let target = s + 1;
    let occupied = app.project.tracks[t].slots.get(target).is_none_or(|x| x.clip.is_some());
    if occupied {
        app.project.insert_scene(target, &format!("{}", app.project.scenes.len() + 1));
    }
    app.project.tracks[t].slots[target].clip = Some(clip);
    app.sel.clip = Some(ClipLoc::Slot { track: t, slot: target });
    app.label("Duplicate Clip");
}

/// Live 10 §7.4.3: copy the currently playing clips into a new scene below and launch it.
pub fn capture_scene(app: &mut XdawApp) {
    let at = app.sel.scene + 1;
    let playing: Vec<i32> = (0..app.project.tracks.len()).map(|t| app.track_status(t).playing_slot.load(Ordering::Relaxed)).collect();
    app.project.insert_scene(at, &format!("{}", app.project.scenes.len() + 1));
    for (t, &p) in playing.iter().enumerate() {
        if p == NO_SLOT {
            continue;
        }
        let src = if p as usize >= at { p as usize + 1 } else { p as usize };
        if let Some(mut c) = app.project.tracks[t].slots[src].clip.clone() {
            c.id = app.project.alloc_id();
            app.project.tracks[t].slots[at].clip = Some(c);
        }
    }
    app.label("Capture and Insert Scene");
    app.pending_launch_scene = Some(at);
}

fn scene_launcher(app: &mut XdawApp, ui: &mut Ui, s: usize) {
    let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click());
    let scene = &app.project.scenes[s];
    let inner = rect.shrink2(vec2(2.0, 1.5));
    let selected = app.sel.scene == s;
    let p = ui.painter();
    p.rect_filled(inner, 2.0, if selected { theme::dim(theme::rgb(scene.color), 0.9) } else { theme::dim(theme::rgb(scene.color), 0.5) });
    let c0 = pos2(inner.left() + 9.0, inner.center().y);
    p.add(egui::Shape::convex_polygon(vec![c0 + vec2(-4.0, -5.0), c0 + vec2(5.0, 0.0), c0 + vec2(-4.0, 5.0)], theme::TEXT, Stroke::NONE));
    p.text(pos2(inner.left() + 20.0, inner.center().y), Align2::LEFT_CENTER, &scene.name, FontId::proportional(11.0), theme::TEXT);
    if app.map_mode != MapMode::Off && resp.clicked() {
        app.map_target = Some(MappingTarget::LaunchScene { scene: scene.id });
        return;
    }
    if resp.clicked() {
        let on_button = resp.interact_pointer_pos().is_some_and(|q| q.x < inner.left() + 18.0);
        app.sel.scene = s;
        if on_button {
            launch_scene(app, s);
        }
    }
    if resp.double_clicked() {
        launch_scene(app, s);
    }
    resp.context_menu(|ui| {
        if ui.button("Launch").clicked() {
            launch_scene(app, s);
            ui.close();
        }
        if ui.button("Rename").clicked() {
            app.dialogs.rename = Some((RenameTarget::Scene(s), app.project.scenes[s].name.clone()));
            ui.close();
        }
        ui.menu_button("Color", |ui| color_menu(ui, &mut app.project.scenes[s].color));
        if ui.button("Insert Scene").clicked() {
            app.project.insert_scene(s + 1, &format!("{}", app.project.scenes.len() + 1));
            app.label("Insert Scene");
            ui.close();
        }
        if ui.button("Duplicate Scene").clicked() {
            app.project.insert_scene(s + 1, &format!("{} copy", app.project.scenes[s].name));
            for t in 0..app.project.tracks.len() {
                if let Some(mut c) = app.project.tracks[t].slots[s].clip.clone() {
                    c.id = app.project.alloc_id();
                    app.project.tracks[t].slots[s + 1].clip = Some(c);
                }
                app.project.tracks[t].slots[s + 1].has_stop_button = app.project.tracks[t].slots[s].has_stop_button;
            }
            app.label("Duplicate Scene");
            ui.close();
        }
        if ui.add_enabled(app.project.scenes.len() > 1, egui::Button::new("Delete Scene")).clicked() {
            app.project.remove_scene(s);
            app.sel.scene = app.sel.scene.min(app.project.scenes.len() - 1);
            app.sel.clip = None;
            app.label("Delete Scene");
            ui.close();
        }
    });
}

/// Launches a scene; names with "<n> BPM" or "x/y" change tempo/signature (Live 10 §7.2).
pub fn launch_scene(app: &mut XdawApp, s: usize) {
    let name = app.project.scenes[s].name.clone();
    if let Some(bpm) = parse_scene_tempo(&name) {
        app.project.transport.bpm = bpm;
    }
    if let Some(sig) = parse_scene_signature(&name) {
        app.project.transport.time_signature = sig;
    }
    app.rt.send(Command::LaunchScene { slot: s as u16 });
    // Live's default "Select Next Scene on Launch".
    app.sel.scene = (s + 1).min(app.project.scenes.len() - 1);
}

pub fn parse_scene_tempo(name: &str) -> Option<f64> {
    let lower = name.to_ascii_lowercase();
    let idx = lower.find("bpm")?;
    let head = lower[..idx].trim_end();
    let start = head.rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map_or(0, |i| i + 1);
    let v: f64 = head[start..].parse().ok()?;
    (20.0..=999.0).contains(&v).then_some(v)
}

pub fn parse_scene_signature(name: &str) -> Option<xdaw_sequencer::TimeSignature> {
    let bytes: Vec<char> = name.chars().collect();
    for (i, &c) in bytes.iter().enumerate() {
        if c != '/' {
            continue;
        }
        let num: String = bytes[..i].iter().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        let den: String = bytes[i + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if let (Ok(n), Ok(d)) = (num.parse(), den.parse()) {
            if let Some(sig) = xdaw_sequencer::TimeSignature::new(n, d) {
                return Some(sig);
            }
        }
    }
    None
}

fn track_status(app: &mut XdawApp, ui: &mut Ui, t: usize) {
    let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click());
    let st = app.track_status(t);
    let queued_stop = st.queued.load(Ordering::Relaxed) == QUEUED_STOP;
    let playing = st.playing_slot.load(Ordering::Relaxed);
    let arr = st.arrangement_active.load(Ordering::Relaxed);
    let p = ui.painter();
    let inner = rect.shrink2(vec2(2.0, 1.5));
    p.rect_filled(inner, 2.0, theme::PANEL_DARK);
    let sq = Rect::from_center_size(pos2(inner.left() + 10.0, inner.center().y), vec2(8.0, 8.0));
    let blink = (ui.input(|i| i.time) * 4.0).fract() < 0.5;
    p.rect_filled(sq, 1.0, if queued_stop && blink { theme::QUEUED } else { theme::TEXT_DIM });
    let info = if playing >= 0 {
        let pos = st.clip_position.load();
        format!("▶ {:.1}", pos.max(0.0))
    } else if arr {
        "Arrangement".into()
    } else {
        "—".into()
    };
    p.text(pos2(inner.left() + 20.0, inner.center().y), Align2::LEFT_CENTER, info, FontId::proportional(10.0), theme::TEXT_DIM);
    if resp.clicked() {
        if app.map_mode != MapMode::Off {
            app.map_target = Some(MappingTarget::StopTrack { track: app.project.tracks[t].id });
        } else if app.project.tracks[t].kind == TrackKind::Group {
            for m in members(app, t) {
                app.rt.send(Command::StopTrack { track: m as u16 });
            }
        } else {
            app.rt.send(Command::StopTrack { track: t as u16 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_names_parse_like_live_10() {
        assert_eq!(parse_scene_tempo("96 BPM"), Some(96.0));
        assert_eq!(parse_scene_tempo("72 BPM;7/8"), Some(72.0));
        assert_eq!(parse_scene_tempo("60 bpm Chorus 3/4"), Some(60.0));
        assert_eq!(parse_scene_tempo("Verse"), None);
        assert_eq!(parse_scene_tempo("5000 BPM"), None);
        assert_eq!(parse_scene_signature("2/4+108 BPM").map(|s| (s.numerator, s.denominator)), Some((2, 4)));
        assert_eq!(parse_scene_signature("72 BPM;7/8").map(|s| (s.numerator, s.denominator)), Some((7, 8)));
        assert_eq!(parse_scene_signature("7/3"), None);
    }
}
