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
    jobs: Sender<(String, PathBuf)>,
    results: Receiver<LoadedMedia>,
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
        let (jobs, rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
        let (tx, results) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("xdaw-media-loader".into())
            .spawn(move || {
                for (key, path) in rx {
                    let result = decode_file(&path).map(Arc::new);
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
        let _ = self.jobs.send((key.to_string(), path));
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
