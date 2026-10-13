//! Audio device I/O through CPAL. The output callback owns the renderer and
//! measures its own processing time against the buffer deadline.

use crate::protocol::SharedStatus;
use crate::render::Renderer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub const SUPPORTED_RATES: [u32; 2] = [44_100, 48_000];
pub const BUFFER_SIZES: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

#[derive(Clone, Debug, PartialEq)]
pub struct AudioSettings {
    /// `None` = system default.
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub enable_input: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { output_device: None, input_device: None, sample_rate: 48_000, buffer_size: 256, enable_input: false }
    }
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "Unknown device".into())
}

pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices().map(|it| it.map(|d| device_name(&d)).collect()).unwrap_or_default()
}

pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|it| it.map(|d| device_name(&d)).collect()).unwrap_or_default()
}

fn find_output(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        None => host.default_output_device().ok_or_else(|| "no default output device".to_string()),
        Some(n) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| device_name(d) == n)
            .ok_or_else(|| format!("output device '{n}' not found")),
    }
}

fn find_input(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        None => host.default_input_device().ok_or_else(|| "no default input device".to_string()),
        Some(n) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| device_name(d) == n)
            .ok_or_else(|| format!("input device '{n}' not found")),
    }
}

/// Interleaved input frames from the input stream (channel count in `channels`).
pub struct InputTap {
    pub consumer: Consumer<f32>,
    pub channels: usize,
}

pub struct AudioOutput {
    _stream: cpal::Stream,
    _input: Option<cpal::Stream>,
    pub device: String,
    pub input_device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub channels: u16,
    pub input_channels: u16,
}

/// Opens the output (and optional input) stream and moves the renderer into the
/// output callback. Fails without side effects on unsupported configurations.
pub fn start(mut renderer: Renderer, settings: &AudioSettings, status: Arc<SharedStatus>) -> Result<AudioOutput, String> {
    let device = find_output(settings.output_device.as_deref())?;
    let name = device_name(&device);
    let default = device.default_output_config().map_err(|e| format!("{name}: {e}"))?;
    let channels = default.channels().max(1);
    if renderer.sample_rate() as u32 != settings.sample_rate {
        return Err(format!("renderer built for {} Hz but device requested {} Hz", renderer.sample_rate(), settings.sample_rate));
    }
    let config = cpal::StreamConfig {
        channels,
        sample_rate: settings.sample_rate,
        buffer_size: cpal::BufferSize::Fixed(settings.buffer_size),
    };

    // Optional input: separate stream feeding a ring the renderer drains.
    let mut input_stream = None;
    let mut input_name = None;
    let mut input_channels = 0;
    if settings.enable_input {
        match open_input(settings, status.clone()) {
            Ok((stream, tap, n, ch)) => {
                renderer.attach_input(tap);
                input_stream = Some(stream);
                input_name = Some(n);
                input_channels = ch;
            }
            Err(e) => {
                status.stream_errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("input disabled: {e}");
            }
        }
    }

    let err_status = status.clone();
    let cb_status = status.clone();
    let sr = settings.sample_rate as f64;
    let ch = channels as usize;
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], _info| {
                let t0 = Instant::now();
                renderer.process_interleaved(data, ch);
                let ns = t0.elapsed().as_nanos() as u64;
                cb_status.callback.record(ns, (data.len() / ch) as u32, sr);
            },
            move |err| {
                // Error callbacks run on a backend thread, not the render callback.
                err_status.stream_errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("audio output: {err}");
            },
            None,
        )
        .map_err(|e| format!("{name}: cannot open {} Hz / {} frames: {e}", settings.sample_rate, settings.buffer_size))?;
    stream.play().map_err(|e| format!("{name}: {e}"))?;
    if let Some(s) = &input_stream {
        let _ = s.play();
    }
    Ok(AudioOutput {
        _stream: stream,
        _input: input_stream,
        device: name,
        input_device: input_name,
        sample_rate: settings.sample_rate,
        buffer_size: settings.buffer_size,
        channels,
        input_channels,
    })
}

fn open_input(settings: &AudioSettings, status: Arc<SharedStatus>) -> Result<(cpal::Stream, InputTap, String, u16), String> {
    let device = find_input(settings.input_device.as_deref())?;
    let name = device_name(&device);
    let default = device.default_input_config().map_err(|e| format!("{name}: {e}"))?;
    let channels = default.channels().clamp(1, 2);
    let config = cpal::StreamConfig {
        channels,
        sample_rate: settings.sample_rate,
        buffer_size: cpal::BufferSize::Fixed(settings.buffer_size),
    };
    // Two seconds of headroom between input and output callbacks.
    let (mut tx, rx): (Producer<f32>, Consumer<f32>) = RingBuffer::new(settings.sample_rate as usize * 2 * channels as usize);
    let st = status.clone();
    let stream = device
        .build_input_stream::<f32, _, _>(
            config,
            move |data: &[f32], _| {
                let mut peak: f32 = 0.0;
                for &x in data {
                    peak = peak.max(x.abs());
                    if tx.push(x).is_err() {
                        st.input_overruns.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
                st.input_peak.store(st.input_peak.load().max(peak));
            },
            move |_e| {
                status.stream_errors.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("{name}: {e}"))?;
    Ok((stream, InputTap { consumer: rx, channels: channels as usize }, name, channels))
}
