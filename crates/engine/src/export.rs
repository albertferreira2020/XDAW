//! Offline export through the same renderer used for playback.

use crate::graph::{build_graph, GraphError, MediaSource};
use crate::handle::create;
use crate::protocol::Command;
use std::path::Path;
use xdaw_dsp::util::Dither;
use xdaw_project::Project;
use xdaw_sequencer::Beats;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Pcm16,
    Pcm24,
    Float32,
}

impl SampleFormat {
    pub const ALL: [SampleFormat; 3] = [SampleFormat::Pcm16, SampleFormat::Pcm24, SampleFormat::Float32];

    pub fn label(self) -> &'static str {
        match self {
            SampleFormat::Pcm16 => "16-bit PCM",
            SampleFormat::Pcm24 => "24-bit PCM",
            SampleFormat::Float32 => "32-bit float",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub start: Beats,
    pub end: Beats,
    /// Extra time after `end` during which sources are silent and effects ring out.
    pub tail_seconds: f64,
    pub sample_rate: u32,
    pub format: SampleFormat,
    /// TPDF dither for PCM formats (ignored for float).
    pub dither: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportReport {
    pub frames: usize,
    pub peak: f32,
    /// Samples beyond full scale before conversion (PCM output is clipped to range).
    pub clipped_samples: usize,
    pub nonfinite_blocks: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error("export range is empty")]
    EmptyRange,
    #[error("unsupported sample rate {0} (use 44100 or 48000, or 22050–192000)")]
    SampleRate(u32),
    #[error("cannot write {path}: {reason}")]
    Write { path: String, reason: String },
}

/// Renders `[start, end)` of the Arrangement plus tail to planar stereo.
pub fn render_offline(
    project: &Project,
    media: &dyn MediaSource,
    settings: &ExportSettings,
) -> Result<(Vec<f32>, Vec<f32>, u32), ExportError> {
    if settings.end <= settings.start {
        return Err(ExportError::EmptyRange);
    }
    if !(22_050..=192_000).contains(&settings.sample_rate) {
        return Err(ExportError::SampleRate(settings.sample_rate));
    }
    let sr = settings.sample_rate as f64;
    let graph = build_graph(project, media, sr)?;
    let bpm = project.transport.bpm;
    let (mut handle, mut renderer, _) = create(graph, bpm, false);
    renderer.include_metronome = false;
    handle.send(Command::SetTimeSignature(project.transport.time_signature));
    handle.send(Command::SetGlobalQuantize(project.transport.global_quantize));
    handle.send(Command::Locate(settings.start));
    handle.send(Command::Play);
    let body = ((settings.end - settings.start) * 60.0 / bpm * sr).round() as usize;
    let tail = (settings.tail_seconds.max(0.0) * sr).round() as usize;
    let mut l = vec![0.0f32; body + tail];
    let mut r = vec![0.0f32; body + tail];
    const CHUNK: usize = 4096;
    let mut pos = 0;
    while pos < body {
        let n = CHUNK.min(body - pos);
        renderer.process_planar(&mut l[pos..pos + n], &mut r[pos..pos + n]);
        pos += n;
        handle.collect_garbage();
    }
    renderer.release_all_sources();
    while pos < body + tail {
        let n = CHUNK.min(body + tail - pos);
        renderer.process_planar(&mut l[pos..pos + n], &mut r[pos..pos + n]);
        pos += n;
    }
    let nonfinite = handle.status.nonfinite_blocks.load(std::sync::atomic::Ordering::Relaxed);
    Ok((l, r, nonfinite))
}

/// Renders one track on its own, without the mixer, returns or master chain:
/// `with_devices` keeps its devices (freeze), otherwise only its clips (consolidate).
/// Mixer volume, pan, sends and their automation are left out, so the result is unity gain.
pub fn render_track_offline(
    project: &Project,
    media: &dyn MediaSource,
    track: usize,
    settings: &ExportSettings,
    with_devices: bool,
) -> Result<(Vec<f32>, Vec<f32>), ExportError> {
    use xdaw_project::{ParamTarget, TrackKind};
    let src = project.tracks.get(track).ok_or(ExportError::EmptyRange)?;
    if src.kind == TrackKind::Group {
        return Err(ExportError::Write { path: String::new(), reason: "group tracks cannot be rendered on their own".into() });
    }
    let mut t = src.clone();
    t.frozen = None;
    t.group = None;
    t.slots.clear();
    t.take_lanes.clear();
    t.mixer = xdaw_project::Mixer::default();
    t.mixer.input_channel = None;
    t.automation.retain(|l| matches!(l.target, ParamTarget::Device { .. }));
    if !with_devices {
        t.devices.clear();
        t.automation.clear();
    }
    let mut p = Project::new("render");
    p.transport = project.transport.clone();
    p.transport.loop_enabled = false;
    p.transport.count_in_bars = 0;
    p.transport.metronome = false;
    p.tracks.push(t);
    let (l, r, _) = render_offline(&p, media, settings)?;
    Ok((l, r))
}

pub fn write_wav(path: &Path, l: &[f32], r: &[f32], sample_rate: u32, format: SampleFormat, dither: bool) -> Result<ExportReport, ExportError> {
    let werr = |e: hound::Error| ExportError::Write { path: path.display().to_string(), reason: e.to_string() };
    let (bits, sample_format) = match format {
        SampleFormat::Pcm16 => (16, hound::SampleFormat::Int),
        SampleFormat::Pcm24 => (24, hound::SampleFormat::Int),
        SampleFormat::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec { channels: 2, sample_rate, bits_per_sample: bits, sample_format };
    // Write to a temporary file and rename, so a failed export never leaves a truncated WAV.
    let tmp = path.with_extension("wav.part");
    let mut w = hound::WavWriter::create(&tmp, spec).map_err(werr)?;
    let mut peak: f32 = 0.0;
    let mut clipped = 0;
    let mut dl = Dither::new(bits as u32, 0x1234_5678);
    let mut dr = Dither::new(bits as u32, 0x8765_4321);
    let scale = match format {
        SampleFormat::Pcm16 => i16::MAX as f32,
        SampleFormat::Pcm24 => 8_388_607.0,
        SampleFormat::Float32 => 1.0,
    };
    for (&a, &b) in l.iter().zip(r) {
        for (x, d) in [(a, &mut dl), (b, &mut dr)] {
            peak = peak.max(x.abs());
            if x.abs() > 1.0 {
                clipped += 1;
            }
            match format {
                SampleFormat::Float32 => w.write_sample(x).map_err(werr)?,
                _ => {
                    let y = if dither { d.apply(x) } else { x };
                    let v = (y.clamp(-1.0, 1.0) * scale).round() as i32;
                    w.write_sample(v).map_err(werr)?
                }
            }
        }
    }
    w.finalize().map_err(werr)?;
    std::fs::rename(&tmp, path).map_err(|e| ExportError::Write { path: path.display().to_string(), reason: e.to_string() })?;
    Ok(ExportReport { frames: l.len(), peak, clipped_samples: clipped, nonfinite_blocks: 0 })
}

pub fn export_wav(project: &Project, media: &dyn MediaSource, settings: &ExportSettings, path: &Path) -> Result<ExportReport, ExportError> {
    let (l, r, nonfinite) = render_offline(project, media, settings)?;
    let mut report = write_wav(path, &l, &r, settings.sample_rate, settings.format, settings.dither)?;
    report.nonfinite_blocks = nonfinite;
    Ok(report)
}
