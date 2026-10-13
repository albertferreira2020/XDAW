//! Files dragged in from the OS file manager (Finder, Explorer).
//!
//! They become ordinary `BrowserItem::File` drag payloads, so every target that
//! accepts browser files (Session slots, Arrangement lanes, track-creation
//! zones, Sampler and Drum Rack pads) also accepts OS drops, with the same
//! hover highlights. Further files of a multi-file drop go to
//! `XdawApp::extra_drop` and are placed after the first one by the target.
//!
//! winit 0.30 reports hovered/dropped files without a position and macOS sends
//! no mouse moves during a drag session, so while files hover the window the
//! pointer position is read from the OS (macOS: Core Graphics) and injected as
//! pointer moves; the drop becomes a pointer release at that position.

use crate::app::XdawApp;
use crate::browser::{self, BrowserItem};
use egui::{Event, Pos2, RawInput};
use std::path::PathBuf;

#[derive(Default)]
pub struct OsDrop {
    active: bool,
    /// Pointer position supplied by the script driver instead of the OS.
    pub pos_override: Option<Pos2>,
}

fn importable(p: &std::path::Path) -> bool {
    browser::is_audio(p) || browser::is_midi(p)
}

pub fn hook(app: &mut XdawApp, ctx: &egui::Context, raw: &mut RawInput) {
    let hovered: Vec<PathBuf> = raw.hovered_files.iter().filter_map(|f| f.path.clone()).filter(|p| importable(p)).collect();
    let dropped: Vec<PathBuf> = raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
    if hovered.is_empty() && dropped.is_empty() {
        if app.os_drop.active {
            app.os_drop.active = false;
            egui::DragAndDrop::clear_payload(ctx);
        }
        return;
    }
    let pos = app.os_drop.pos_override.take().or_else(|| cursor_in_window(ctx, raw));
    if let Some(pos) = pos {
        raw.events.push(Event::PointerMoved(pos));
    }
    if !hovered.is_empty() && !app.os_drop.active {
        app.os_drop.active = true;
        egui::DragAndDrop::set_payload(ctx, BrowserItem::File(hovered[0].clone()));
    }
    if dropped.is_empty() {
        return;
    }
    app.os_drop.active = false;
    let (ok, skipped): (Vec<PathBuf>, Vec<PathBuf>) = dropped.into_iter().partition(|p| importable(p));
    if !skipped.is_empty() {
        app.status_line = format!("Skipped {} unsupported file(s) (WAV, AIFF, FLAC, MIDI only)", skipped.len());
    }
    let Some(first) = ok.first().cloned() else {
        egui::DragAndDrop::clear_payload(ctx);
        return;
    };
    egui::DragAndDrop::set_payload(ctx, BrowserItem::File(first));
    app.extra_drop = ok[1..].to_vec();
    let at = pos.or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(Pos2::ZERO);
    raw.events.push(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: ctx.input(|i| i.modifiers) });
}

/// True while OS files hover the window (keeps frames coming to track the pointer).
pub fn hovering(app: &XdawApp) -> bool {
    app.os_drop.active
}

/// Pointer position in window points, read from the OS.
fn cursor_in_window(ctx: &egui::Context, raw: &RawInput) -> Option<Pos2> {
    let (gx, gy) = global_cursor_points()?;
    let vp = raw.viewports.get(&raw.viewport_id)?;
    let inner = vp.inner_rect?;
    // `inner_rect` is the window's physical origin divided by egui's pixels-per-point.
    let native = vp.native_pixels_per_point.unwrap_or(1.0);
    let ppp = ctx.pixels_per_point();
    let scale = native / ppp;
    Some(Pos2::new(gx as f32 * scale - inner.min.x, gy as f32 * scale - inner.min.y))
}

#[cfg(target_os = "macos")]
fn global_cursor_points() -> Option<(f64, f64)> {
    use std::ffi::c_void;
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    // SAFETY: CGEventCreate(NULL) returns a new event (or NULL) owned by us; it is
    // only read and then released once.
    unsafe {
        let e = CGEventCreate(std::ptr::null());
        if e.is_null() {
            return None;
        }
        let p = CGEventGetLocation(e);
        CFRelease(e);
        Some((p.x, p.y))
    }
}

#[cfg(not(target_os = "macos"))]
fn global_cursor_points() -> Option<(f64, f64)> {
    // Other platforms deliver pointer moves during OS drags; egui's own pointer
    // position is used.
    None
}
