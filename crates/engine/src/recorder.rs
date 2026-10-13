//! Disk writer for audio recording. The audio thread pushes frames into a ring;
//! this worker writes 32-bit float WAV files and reports finished takes.

use crossbeam_channel::{Receiver, Sender};
use rtrb::{Consumer, Producer, RingBuffer};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub enum RecMsg {
    Start { track: u16, slot: u16 },
    Frame(f32, f32),
    Stop,
}

#[derive(Debug, Clone)]
pub struct FinishedTake {
    pub track: u16,
    pub slot: u16,
    pub path: PathBuf,
    pub frames: usize,
    pub error: Option<String>,
}

pub struct Recorder {
    pub finished: Receiver<FinishedTake>,
    dir_tx: Sender<PathBuf>,
}

/// Seconds of audio buffered between the audio thread and the disk writer.
pub const RING_SECONDS: usize = 10;

impl Recorder {
    /// Spawns the writer. Returns the recorder and the producer for the renderer.
    pub fn spawn(sample_rate: u32, dir: PathBuf) -> (Self, Producer<RecMsg>) {
        let (tx, mut rx): (Producer<RecMsg>, Consumer<RecMsg>) = RingBuffer::new(sample_rate as usize * RING_SECONDS);
        let (done_tx, finished) = crossbeam_channel::unbounded();
        let (dir_tx, dir_rx) = crossbeam_channel::unbounded::<PathBuf>();
        std::thread::Builder::new()
            .name("xdaw-recorder".into())
            .spawn(move || {
                let mut dir = dir;
                let mut current: Option<(hound::WavWriter<std::io::BufWriter<std::fs::File>>, FinishedTake)> = None;
                loop {
                    while let Ok(d) = dir_rx.try_recv() {
                        dir = d;
                    }
                    let mut idle = true;
                    while let Ok(msg) = rx.pop() {
                        idle = false;
                        match msg {
                            RecMsg::Start { track, slot } => {
                                let stamp = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis())
                                    .unwrap_or(0);
                                let path = dir.join(format!("Rec-{stamp}-T{}.wav", track + 1));
                                let spec = hound::WavSpec { channels: 2, sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
                                let _ = std::fs::create_dir_all(&dir);
                                let take = FinishedTake { track, slot, path: path.clone(), frames: 0, error: None };
                                match hound::WavWriter::create(&path, spec) {
                                    Ok(w) => current = Some((w, take)),
                                    Err(e) => {
                                        let _ = done_tx.send(FinishedTake { error: Some(e.to_string()), ..take });
                                    }
                                }
                            }
                            RecMsg::Frame(l, r) => {
                                if let Some((w, take)) = current.as_mut() {
                                    if take.error.is_none() {
                                        if let Err(e) = w.write_sample(l).and_then(|_| w.write_sample(r)) {
                                            take.error = Some(e.to_string());
                                        }
                                        take.frames += 1;
                                    }
                                }
                            }
                            RecMsg::Stop => {
                                if let Some((w, mut take)) = current.take() {
                                    if let Err(e) = w.finalize() {
                                        take.error.get_or_insert(e.to_string());
                                    }
                                    let _ = done_tx.send(take);
                                }
                            }
                        }
                    }
                    if idle {
                        if done_tx.is_empty() && rx.is_abandoned() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            })
            .expect("spawn recorder");
        (Self { finished, dir_tx }, tx)
    }

    pub fn set_directory(&self, dir: PathBuf) {
        let _ = self.dir_tx.send(dir);
    }
}
