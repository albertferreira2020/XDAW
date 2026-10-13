//! Audio file decoding (WAV, AIFF, FLAC via symphonia) and a media cache with
//! a background loader so the UI and audio threads never wait on disk.

use crate::graph::MediaSource;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xdaw_dsp::SampleData;
use xdaw_project::MediaRef;

#[derive(Debug, Clone, thiserror::Error)]
pub enum MediaError {
    #[error("{0}: file not found")]
    Missing(PathBuf),
    #[error("{path}: {reason}")]
    Decode { path: PathBuf, reason: String },
}

/// Decodes a whole file into memory as planar f32 (1 or 2 channels; extra channels are dropped).
pub fn decode_file(path: &Path) -> Result<SampleData, MediaError> {
    use symphonia::core::audio::GenericAudioBufferRef;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let err = |reason: String| MediaError::Decode { path: path.to_path_buf(), reason };
    let file = std::fs::File::open(path).map_err(|_| MediaError::Missing(path.to_path_buf()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| err(format!("unsupported or invalid audio file ({e})")))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| err("no audio track".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| err("not an audio track".into()))?
        .clone();
    let sample_rate = params.sample_rate.ok_or_else(|| err("unknown sample rate".into()))? as f64;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| err(format!("no decoder ({e})")))?;
    let mut channels: Vec<Vec<f32>> = vec![];
    let mut scratch: Vec<f32> = vec![];
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(err(format!("read error ({e})"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded: GenericAudioBufferRef = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(err(format!("decode error ({e})"))),
        };
        let nch = decoded.num_planes().max(1);
        if channels.is_empty() {
            channels = vec![vec![]; nch.min(2)];
        }
        scratch.clear();
        decoded.copy_to_vec_interleaved(&mut scratch);
        for frame in scratch.chunks(nch) {
            for (c, ch) in channels.iter_mut().enumerate() {
                ch.push(frame[c.min(frame.len() - 1)]);
            }
        }
    }
    if channels.is_empty() || channels[0].is_empty() {
        return Err(err("file contains no audio".into()));
    }
    Ok(SampleData { channels, sample_rate })
}

/// Peak overview for waveform drawing: (min, max) per bucket of `bucket` frames.
pub fn overview(data: &SampleData, buckets: usize) -> Vec<(f32, f32)> {
    let n = data.frames();
    if n == 0 || buckets == 0 {
        return vec![];
    }
    let per = n.div_ceil(buckets).max(1);
    (0..buckets)
        .map(|b| {
            let range = (b * per).min(n)..((b + 1) * per).min(n);
            let mut lo: f32 = 0.0;
            let mut hi: f32 = 0.0;
            for ch in &data.channels {
                for &x in &ch[range.clone()] {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            (lo, hi)
        })
        .collect()
}

pub struct LoadedMedia {
    pub key: String,
    pub result: Result<Arc<SampleData>, MediaError>,
}

/// Decoded media keyed by project media path, with a worker thread for loading.
pub struct MediaCache {
    pub loaded: HashMap<String, Arc<SampleData>>,
    pub failed: HashMap<String, MediaError>,
    pub overviews: HashMap<String, Arc<Vec<(f32, f32)>>>,
    pending: HashMap<String, ()>,
    jobs: Sender<Job>,
    results: Receiver<LoadedMedia>,
}

enum Job {
    Decode(String, PathBuf),
    Stretch(String, Arc<SampleData>, Vec<(f64, f64, f64, f64)>, f64),
}

/// Renders `(beat_from, beat_to, sec_from, sec_to)` segments of `src` at `bpm` with pitch preserved.
pub fn render_warped(src: &SampleData, segments: &[(f64, f64, f64, f64)], bpm: f64) -> SampleData {
    let sr = src.sample_rate;
    let n = src.frames();
    let mut out: Vec<Vec<f32>> = vec![vec![]; src.channels.len().max(1)];
    for &(b0, b1, s0, s1) in segments {
        let target = ((b1 - b0) * 60.0 / bpm * sr).round() as usize;
        let (f0, f1) = (((s0 * sr).round().max(0.0) as usize).min(n), ((s1 * sr).round().max(0.0) as usize).min(n));
        if target == 0 {
            continue;
        }
        if f1 <= f0 {
            for ch in out.iter_mut() {
                ch.extend(std::iter::repeat_n(0.0, target));
            }
            continue;
        }
        let piece = SampleData { channels: src.channels.iter().map(|c| c[f0..f1].to_vec()).collect(), sample_rate: sr };
        let stretched = xdaw_dsp::stretch::wsola(&piece, target as f64 / (f1 - f0) as f64);
        for (ch, s) in out.iter_mut().zip(&stretched.channels) {
            ch.extend(s.iter().take(target));
            ch.resize(ch.len().max(ch.len() + target.saturating_sub(s.len())), 0.0);
        }
    }
    SampleData { channels: out, sample_rate: sr }
}

impl MediaSource for MediaCache {
    fn get(&self, media: &MediaRef) -> Option<Arc<SampleData>> {
        self.loaded.get(&media.path).cloned()
    }
}

impl Default for MediaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaCache {
    pub fn new() -> Self {
        let (jobs, rx) = crossbeam_channel::unbounded::<Job>();
        let (tx, results) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("xdaw-media-loader".into())
            .spawn(move || {
                for job in rx {
                    let (key, result) = match job {
                        Job::Decode(key, path) => (key, decode_file(&path).map(Arc::new)),
                        Job::Stretch(key, src, segments, bpm) => (key, Ok(Arc::new(render_warped(&src, &segments, bpm)))),
                    };
                    if tx.send(LoadedMedia { key, result }).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn media loader");
        Self { loaded: HashMap::new(), failed: HashMap::new(), overviews: HashMap::new(), pending: HashMap::new(), jobs, results }
    }

    /// Queues a load if the key is neither loaded, failed nor pending.
    pub fn request(&mut self, key: &str, path: PathBuf) {
        if self.loaded.contains_key(key) || self.pending.contains_key(key) || self.failed.contains_key(key) {
            return;
        }
        self.pending.insert(key.to_string(), ());
        let _ = self.jobs.send(Job::Decode(key.to_string(), path));
    }

    /// Queues a pitch-preserving rendering of `base` for warp `segments` at `bpm`.
    pub fn request_stretch(&mut self, key: &str, base: &str, segments: Vec<(f64, f64, f64, f64)>, bpm: f64) -> bool {
        if self.loaded.contains_key(key) || self.pending.contains_key(key) {
            return false;
        }
        let Some(src) = self.loaded.get(base).cloned() else { return false };
        self.pending.insert(key.to_string(), ());
        let _ = self.jobs.send(Job::Stretch(key.to_string(), src, segments, bpm));
        true
    }

    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Collects finished loads; returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.results.try_recv() {
            self.pending.remove(&done.key);
            match done.result {
                Ok(d) => {
                    self.overviews.insert(done.key.clone(), Arc::new(overview(&d, 2048)));
                    self.loaded.insert(done.key, d);
                }
                Err(e) => {
                    self.failed.insert(done.key, e);
                }
            }
            changed = true;
        }
        changed
    }

    /// Blocking load of everything requested (tools/tests).
    pub fn wait(&mut self) {
        while self.is_pending() {
            if let Ok(done) = self.results.recv() {
                self.pending.remove(&done.key);
                match done.result {
                    Ok(d) => {
                        self.overviews.insert(done.key.clone(), Arc::new(overview(&d, 2048)));
                        self.loaded.insert(done.key, d);
                    }
                    Err(e) => {
                        self.failed.insert(done.key, e);
                    }
                }
            }
        }
    }

    /// Inserts already decoded data (recording, generated content).
    pub fn insert(&mut self, key: String, data: Arc<SampleData>) {
        self.overviews.insert(key.clone(), Arc::new(overview(&data, 2048)));
        self.failed.remove(&key);
        self.loaded.insert(key, data);
    }
}

/// Duration in seconds; reads only the header for WAV, decodes other formats.
pub fn probe_duration(path: &Path) -> Result<f64, MediaError> {
    if !path.is_file() {
        return Err(MediaError::Missing(path.to_path_buf()));
    }
    if let Ok(r) = hound::WavReader::open(path) {
        let spec = r.spec();
        return Ok(r.duration() as f64 / spec.sample_rate as f64);
    }
    decode_file(path).map(|d| d.duration_seconds())
}
