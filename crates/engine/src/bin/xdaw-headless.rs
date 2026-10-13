//! Headless driver for verification on real hardware.
//!
//! `xdaw-headless play <project.xdaw> [seconds] [buffer]` plays through the
//! default output device, launching each scene in turn, and prints callback
//! timing statistics.
//! `xdaw-headless export <project.xdaw> <out.wav> [16|24|32]` renders the Arrangement.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use xdaw_engine::audio_io::{self, AudioSettings};
use xdaw_engine::export::{export_wav, ExportSettings, SampleFormat};
use xdaw_engine::media::MediaCache;
use xdaw_engine::protocol::*;
use xdaw_engine::{build_graph, create};
use xdaw_project::{io, Project};

fn load(path: &Path) -> (Project, MediaCache) {
    let project = io::load(path).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let dir = path.parent().map(Path::to_path_buf);
    let mut media = MediaCache::new();
    for m in project.media_refs() {
        media.request(&m.path, io::resolve_media(&m, dir.as_deref()));
    }
    media.wait();
    for (k, e) in &media.failed {
        eprintln!("media {k}: {e}");
    }
    (project, media)
}

fn peak_rss_mb() -> f64 {
    // ru_maxrss is bytes on macOS, kilobytes on Linux.
    let mut usage = std::mem::MaybeUninit::<libc_rusage>::zeroed();
    unsafe {
        getrusage(0, usage.as_mut_ptr());
        let r = usage.assume_init();
        if cfg!(target_os = "macos") { r.ru_maxrss as f64 / 1_048_576.0 } else { r.ru_maxrss as f64 / 1024.0 }
    }
}

#[repr(C)]
struct libc_rusage {
    ru_utime: [i64; 2],
    ru_stime: [i64; 2],
    ru_maxrss: i64,
    rest: [i64; 13],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut libc_rusage) -> i32;
}

fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc_rusage>::zeroed();
    unsafe {
        getrusage(0, usage.as_mut_ptr());
        let r = usage.assume_init();
        (r.ru_utime[0] + r.ru_stime[0]) as f64 + (r.ru_utime[1] + r.ru_stime[1]) as f64 / 1e6
    }
}

fn play(path: &Path, seconds: f64, buffer: u32) {
    let (project, media) = load(path);
    let settings = AudioSettings { buffer_size: buffer, ..Default::default() };
    let graph = build_graph(&project, &media, settings.sample_rate as f64).expect("graph");
    let (mut handle, renderer, _) = create(graph, project.transport.bpm, false);
    let status = handle.status.clone();
    handle.send(Command::SetGlobalQuantize(project.transport.global_quantize));
    let out = match audio_io::start(renderer, &settings, status.clone()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("audio device error: {e}");
            std::process::exit(2);
        }
    };
    println!("device: {} | {} Hz | buffer {} | {} ch", out.device, out.sample_rate, out.buffer_size, out.channels);
    let scenes = project.scenes.len().max(1);
    let per_scene = seconds / scenes as f64;
    let t0 = Instant::now();
    let cpu0 = cpu_seconds();
    let mut scene = 0;
    let mut next_launch = 0.0;
    let mut master_peak: f32 = 0.0;
    while t0.elapsed().as_secs_f64() < seconds {
        if t0.elapsed().as_secs_f64() >= next_launch && scene < scenes {
            handle.send(Command::LaunchScene { slot: scene as u16 });
            println!("{:6.2}s launch scene {} ({})", t0.elapsed().as_secs_f64(), scene + 1, project.scenes[scene].name);
            scene += 1;
            next_launch += per_scene;
        }
        std::thread::sleep(Duration::from_millis(50));
        handle.collect_garbage();
        handle.poll_events(|_| {});
        let m = status.track(TrackIx::Master);
        master_peak = master_peak.max(m.peak_l.take()).max(m.peak_r.take());
    }
    handle.send(Command::Stop);
    std::thread::sleep(Duration::from_millis(300));
    let cb = &status.callback;
    let count = cb.count.load(Ordering::Relaxed);
    let wall = t0.elapsed().as_secs_f64();
    println!("callbacks: {count}");
    println!("budget per callback: {:.3} ms", cb.budget_ns.load(Ordering::Relaxed) as f64 / 1e6);
    println!(
        "callback time: mean {:.3} ms | p50 <= {:.1} ms | p99 <= {:.1} ms | p99.9 <= {:.1} ms | max {:.3} ms",
        cb.total_ns.load(Ordering::Relaxed) as f64 / count.max(1) as f64 / 1e6,
        cb.percentile_ms(0.5),
        cb.percentile_ms(0.99),
        cb.percentile_ms(0.999),
        cb.max_ns.load(Ordering::Relaxed) as f64 / 1e6
    );
    println!("deadline misses (processing > buffer duration): {}", cb.deadline_misses.load(Ordering::Relaxed));
    println!("stream errors reported by backend: {}", status.stream_errors.load(Ordering::Relaxed));
    println!("dropped commands/events: {}/{}", status.dropped_commands.load(Ordering::Relaxed), status.dropped_events.load(Ordering::Relaxed));
    println!("master peak: {:.3} ({:.1} dBFS)", master_peak, 20.0 * master_peak.max(1e-9).log10());
    println!("process CPU: {:.2} s over {:.1} s wall ({:.1}%)", cpu_seconds() - cpu0, wall, (cpu_seconds() - cpu0) / wall * 100.0);
    println!("peak RSS: {:.1} MB", peak_rss_mb());
    drop(out);
}

fn export(path: &Path, out: &Path, bits: u32) {
    let (project, media) = load(path);
    let end = project.tracks.iter().flat_map(|t| t.arrangement.iter().map(|c| c.end())).fold(0.0, f64::max);
    let format = match bits {
        16 => SampleFormat::Pcm16,
        32 => SampleFormat::Float32,
        _ => SampleFormat::Pcm24,
    };
    let settings = ExportSettings { start: 0.0, end, tail_seconds: 2.0, sample_rate: 48_000, format, dither: true };
    let t0 = Instant::now();
    match export_wav(&project, &media, &settings, out) {
        Ok(r) => println!(
            "exported {} frames ({:.1} s) to {} in {:.2} s | peak {:.3} | clipped samples {} | non-finite blocks {}",
            r.frames,
            r.frames as f64 / 48_000.0,
            out.display(),
            t0.elapsed().as_secs_f64(),
            r.peak,
            r.clipped_samples,
            r.nonfinite_blocks
        ),
        Err(e) => {
            eprintln!("export failed: {e}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("play") if args.len() >= 3 => play(
            &PathBuf::from(&args[2]),
            args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20.0),
            args.get(4).and_then(|s| s.parse().ok()).unwrap_or(256),
        ),
        Some("export") if args.len() >= 4 => export(&PathBuf::from(&args[2]), &PathBuf::from(&args[3]), args.get(4).and_then(|s| s.parse().ok()).unwrap_or(24)),
        _ => {
            eprintln!("usage: xdaw-headless play <project.xdaw> [seconds] [buffer] | export <project.xdaw> <out.wav> [16|24|32]");
            std::process::exit(64);
        }
    }
}
