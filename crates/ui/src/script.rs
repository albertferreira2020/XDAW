//! Scripted UI driver for repeatable end-to-end checks (`XDAW_SCRIPT`).
//!
//! Commands are separated by `;`, e.g.
//! `wait 2; scene 2; wait 3; shot /tmp/a.png; view arrangement; quit`.
//! Results are printed to stdout prefixed with `[script]`.

use crate::app::{ClipLoc, Detail, View, XdawApp};
use std::sync::atomic::Ordering;
use xdaw_engine::protocol::Command;

pub struct Script {
    steps: Vec<String>,
    next: usize,
    wait_until: f64,
    pending_shot: bool,
    /// Synthetic input, one batch per frame, injected through `raw_input_hook`.
    pub inject: std::collections::VecDeque<Vec<egui::Event>>,
}

fn pointer(x: f32, y: f32) -> egui::Event {
    egui::Event::PointerMoved(egui::pos2(x, y))
}

fn button(x: f32, y: f32, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers }
}

fn parse_key(spec: &str) -> Option<(egui::Key, egui::Modifiers)> {
    let mut mods = egui::Modifiers::NONE;
    let mut key = None;
    for part in spec.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "cmd" => {
                mods.command = true;
                mods.mac_cmd = true;
            }
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            other => key = egui::Key::from_name(if other.len() == 1 { part } else { part }).or_else(|| egui::Key::from_name(&other.to_ascii_uppercase())),
        }
    }
    key.map(|k| (k, mods))
}

impl Script {
    pub fn from_env() -> Option<Self> {
        let s = std::env::var("XDAW_SCRIPT").ok()?;
        Some(Self {
            steps: s.split(';').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            next: 0,
            wait_until: 0.0,
            pending_shot: false,
            inject: Default::default(),
        })
    }

    pub fn run(&mut self, app: &mut XdawApp, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        // Collect screenshots requested earlier.
        let shots: Vec<(String, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => user_data.data.as_ref().and_then(|d| d.downcast_ref::<String>()).map(|p| (p.clone(), image.clone())),
                    _ => None,
                })
                .collect()
        });
        for (path, img) in shots {
            match write_png(&path, &img) {
                Ok(()) => println!("[script] shot {path} {}x{}", img.size[0], img.size[1]),
                Err(e) => println!("[script] shot {path} FAILED {e}"),
            }
            self.pending_shot = false;
        }
        if self.pending_shot || now < self.wait_until || !self.inject.is_empty() {
            ctx.request_repaint();
            return;
        }
        while self.next < self.steps.len() {
            let step = self.steps[self.next].clone();
            self.next += 1;
            let mut parts = step.split_whitespace();
            let cmd = parts.next().unwrap_or("");
            let args: Vec<&str> = parts.collect();
            let num = |i: usize| args.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            println!("[script] {step}");
            match cmd {
                "wait" => {
                    self.wait_until = now + num(0);
                    ctx.request_repaint();
                    return;
                }
                "shot" => {
                    let path = args.join(" ");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(path)));
                    self.pending_shot = true;
                    ctx.request_repaint();
                    return;
                }
                "click" | "dclick" | "rclick" => {
                    let (x, y) = (num(0) as f32, num(1) as f32);
                    let mods = egui::Modifiers { shift: args.contains(&"shift"), alt: args.contains(&"alt"), command: args.contains(&"cmd"), mac_cmd: args.contains(&"cmd"), ..Default::default() };
                    let times = if cmd == "dclick" { 2 } else { 1 };
                    self.inject.push_back(vec![pointer(x, y)]);
                    for _ in 0..times {
                        if cmd == "rclick" {
                            self.inject.push_back(vec![egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Secondary, pressed: true, modifiers: mods }]);
                            self.inject.push_back(vec![egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Secondary, pressed: false, modifiers: mods }]);
                        } else {
                            self.inject.push_back(vec![button(x, y, true, mods)]);
                            self.inject.push_back(vec![button(x, y, false, mods)]);
                        }
                    }
                    self.inject.push_back(vec![]);
                    ctx.request_repaint();
                    return;
                }
                "drag" => {
                    let (x0, y0, x1, y1) = (num(0) as f32, num(1) as f32, num(2) as f32, num(3) as f32);
                    let steps = 12;
                    self.inject.push_back(vec![pointer(x0, y0)]);
                    self.inject.push_back(vec![button(x0, y0, true, egui::Modifiers::NONE)]);
                    for i in 1..=steps {
                        let t = i as f32 / steps as f32;
                        self.inject.push_back(vec![pointer(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)]);
                    }
                    self.inject.push_back(vec![button(x1, y1, false, egui::Modifiers::NONE)]);
                    self.inject.push_back(vec![]);
                    ctx.request_repaint();
                    return;
                }
                "key" => {
                    if let Some((key, modifiers)) = args.first().and_then(|k| parse_key(k)) {
                        self.inject.push_back(vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
                        self.inject.push_back(vec![egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
                        self.inject.push_back(vec![]);
                    } else {
                        println!("[script] bad key {:?}", args);
                    }
                    ctx.request_repaint();
                    return;
                }
                "notes" => {
                    let n = app.selected_clip().map(|c| match &c.content {
                        xdaw_project::ClipContent::Midi { notes } => format!("{:?}", notes.iter().map(|n| (n.pitch, n.start, n.length)).collect::<Vec<_>>()),
                        _ => "audio".into(),
                    });
                    println!("[script] notes {}", n.unwrap_or_else(|| "no clip".into()));
                }
                "arr" => {
                    let t = (num(0) as usize).saturating_sub(1);
                    if let Some(tr) = app.project.tracks.get(t) {
                        println!("[script] arr track {} {:?}", t + 1, tr.arrangement.iter().map(|c| (c.start, c.length, c.offset)).collect::<Vec<_>>());
                        println!("[script] automation track {} {:?}", t + 1, tr.automation.iter().map(|l| (l.target, l.points.len())).collect::<Vec<_>>());
                    }
                }
                "view" => app.view = if args.first() == Some(&"arrangement") { View::Arrangement } else { View::Session },
                "detail" => app.detail = if args.first() == Some(&"clip") { Detail::Clip } else { Detail::Devices },
                "scene" => crate::session::launch_scene(app, (num(0) as usize).saturating_sub(1)),
                "launch" => {
                    let (t, s) = ((num(0) as usize).saturating_sub(1) as u16, (num(1) as usize).saturating_sub(1) as u16);
                    app.rt.send(Command::LaunchSlot { track: t, slot: s, down: true });
                    app.rt.send(Command::LaunchSlot { track: t, slot: s, down: false });
                }
                "select" => {
                    let (t, s) = ((num(0) as usize).saturating_sub(1), (num(1) as usize).saturating_sub(1));
                    app.select_track(xdaw_project::TrackRef::Track(t));
                    app.sel.clip = Some(ClipLoc::Slot { track: t, slot: s });
                }
                "play" => app.rt.send(Command::Play),
                "capture" => app.capture_midi(),
                "flatten" => {
                    app.flatten_track((num(0) as usize).saturating_sub(1));
                    println!("[script] flatten {}", app.status_line);
                }
                "stop" => app.rt.send(Command::Stop),
                "record" => app.toggle_arrangement_record(),
                "back" => app.rt.send(Command::BackToArrangement),
                "undo" => app.undo(),
                "redo" => app.redo(),
                "save" => {
                    let ok = if args.is_empty() { app.save() } else { app.save_as_path(&std::path::PathBuf::from(args.join(" "))) };
                    println!("[script] save ok={ok} {}", app.status_line);
                }
                "open" => app.open_path(&std::path::PathBuf::from(args.join(" "))),
                "export" => {
                    let end = app.project.tracks.iter().flat_map(|t| t.arrangement.iter().map(|c| c.end())).fold(0.0, f64::max);
                    let settings = xdaw_engine::export::ExportSettings {
                        start: 0.0,
                        end,
                        tail_seconds: 2.0,
                        sample_rate: 48_000,
                        format: xdaw_engine::export::SampleFormat::Pcm24,
                        dither: true,
                    };
                    let media = app.media.loaded.clone();
                    match xdaw_engine::export::export_wav(&app.project, &media, &settings, std::path::Path::new(&args.join(" "))) {
                        Ok(r) => println!("[script] export frames={} peak={:.3} clipped={}", r.frames, r.peak, r.clipped_samples),
                        Err(e) => println!("[script] export FAILED {e}"),
                    }
                }
                "state" => {
                    let playing: Vec<i32> = (0..app.project.tracks.len()).map(|t| app.track_status(t).playing_slot.load(Ordering::Relaxed)).collect();
                    let arr: Vec<usize> = app.project.tracks.iter().map(|t| t.arrangement.len()).collect();
                    println!(
                        "[script] state beat={:.3} playing={} slots={:?} arrangement_clips={:?} undo={:?} dirty={}",
                        app.beat(),
                        app.playing(),
                        playing,
                        arr,
                        app.history.undo_label(),
                        app.is_dirty()
                    );
                }
                "tempo" => app.project.transport.bpm = num(0),
                "mastervol" => {
                    app.project.master.mixer.volume_db = num(0) as f32;
                    app.label("Master Volume");
                }
                "stats" => {
                    let cb = &app.rt.status.callback;
                    let n = cb.count.load(Ordering::Relaxed);
                    println!(
                        "[script] stats callbacks={} mean_ms={:.4} p50_ms<={:.1} p99_ms<={:.1} p999_ms<={:.1} max_ms={:.4} budget_ms={:.4} deadline_misses={} stream_errors={} dropped_cmds={} dropped_events={} undo_depth_label={:?}",
                        n,
                        cb.total_ns.load(Ordering::Relaxed) as f64 / n.max(1) as f64 / 1e6,
                        cb.percentile_ms(0.5),
                        cb.percentile_ms(0.99),
                        cb.percentile_ms(0.999),
                        cb.max_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        cb.budget_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        cb.deadline_misses.load(Ordering::Relaxed),
                        app.rt.status.stream_errors.load(Ordering::Relaxed),
                        app.rt.status.dropped_commands.load(Ordering::Relaxed),
                        app.rt.status.dropped_events.load(Ordering::Relaxed),
                        app.history.undo_label()
                    );
                }
                "volume" => {
                    let t = (num(0) as usize).saturating_sub(1);
                    if let Some(tr) = app.project.tracks.get_mut(t) {
                        tr.mixer.volume_db = num(1) as f32;
                        app.label("Volume");
                    }
                }
                "quit" => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                other => println!("[script] unknown command {other}"),
            }
        }
    }
}

/// Minimal PNG writer (stored deflate blocks) to avoid an image dependency.
pub fn write_png(path: &str, img: &egui::ColorImage) -> std::io::Result<()> {
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        for x in 0..w as usize {
            let c = img.pixels[y * w as usize + x];
            raw.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let crc_table: Vec<u32> = (0..256u32)
        .map(|n| (0..8).fold(n, |c, _| if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }))
        .collect();
    let crc = |data: &[u8]| !data.iter().fold(!0u32, |c, &x| crc_table[((c ^ x as u32) & 0xFF) as usize] ^ (c >> 8));
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut ihdr = vec![];
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::write(path, out)
}
