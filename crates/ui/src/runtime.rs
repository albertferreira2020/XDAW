//! Owns the audio device, engine handle, MIDI inputs and recorder for the UI.

use crossbeam_channel::Receiver;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use xdaw_engine::audio_io::{self, AudioOutput, AudioSettings};
use xdaw_engine::media::MediaCache;
use xdaw_engine::protocol::{Command, SharedStatus};
use xdaw_engine::recorder::Recorder;
use xdaw_engine::sync::Reconciler;
use xdaw_engine::{build_graph, create, EngineHandle, MidiInput};
use xdaw_midi::input::{MidiInputs, RawMidi};
use xdaw_project::Project;

pub struct AudioRuntime {
    pub settings: AudioSettings,
    pub handle: Option<EngineHandle>,
    pub output: Option<AudioOutput>,
    pub error: Option<String>,
    pub reconciler: Reconciler,
    pub recorder: Option<Recorder>,
    pub midi: Option<MidiInputs>,
    pub midi_error: Option<String>,
    pub midi_rx: Option<Receiver<RawMidi>>,
    pub status: Arc<SharedStatus>,
}

impl AudioRuntime {
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            reconciler: Reconciler::new(settings.sample_rate as f64),
            settings,
            handle: None,
            output: None,
            error: None,
            recorder: None,
            midi: None,
            midi_error: None,
            midi_rx: None,
            status: Arc::new(SharedStatus::new()),
        }
    }

    pub fn send(&mut self, cmd: Command) {
        if let Some(h) = self.handle.as_mut() {
            h.send(cmd);
        }
    }

    /// (Re)starts the engine. Transport stops; the project is rebuilt for the new rate.
    pub fn start(&mut self, project: &Project, media: &MediaCache, recordings: PathBuf) {
        self.stop();
        self.reconciler = Reconciler::new(self.settings.sample_rate as f64);
        let graph = match build_graph(project, media, self.settings.sample_rate as f64) {
            Ok(g) => g,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let (handle, mut renderer, midi_tx) = create(graph, project.transport.bpm, true);
        let (recorder, rec_tx) = Recorder::spawn(self.settings.sample_rate, recordings);
        renderer.attach_recorder(rec_tx);
        self.recorder = Some(recorder);
        // MIDI: the midir thread feeds the audio ring (mutex only on the MIDI thread).
        if let Some(tx) = midi_tx {
            let tx = Arc::new(Mutex::new(tx));
            let (ui_tx, ui_rx) = crossbeam_channel::bounded(1024);
            let audio = move |raw: RawMidi| {
                if let Ok(mut p) = tx.lock() {
                    let _ = p.push(MidiInput { bytes: raw.bytes, len: raw.len });
                }
            };
            match MidiInputs::connect_all(audio, ui_tx) {
                Ok(m) => {
                    self.midi = Some(m);
                    self.midi_error = None;
                }
                Err(e) => self.midi_error = Some(e),
            }
            self.midi_rx = Some(ui_rx);
        }
        self.status = handle.status.clone();
        // The next UI frame re-syncs everything (graph + transport) through the reconciler.
        match audio_io::start(renderer, &self.settings, self.status.clone()) {
            Ok(out) => {
                self.output = Some(out);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.handle = Some(handle);
        self.reconciler.invalidate();
    }

    pub fn stop(&mut self) {
        self.output = None;
        self.handle = None;
        self.midi = None;
        self.recorder = None;
    }
}
