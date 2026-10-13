//! Physical MIDI input through midir. The midir callback thread forwards raw
//! bytes to the audio thread through a lock-free ring and to the UI (for MIDI
//! mapping/learn) through a channel.

use crossbeam_channel::Sender;
use midir::{MidiInput, MidiInputConnection};

#[derive(Clone, Copy, Debug)]
pub struct RawMidi {
    pub bytes: [u8; 3],
    pub len: u8,
}

pub struct MidiInputs {
    connections: Vec<MidiInputConnection<()>>,
    pub port_names: Vec<String>,
}

pub fn list_ports() -> Result<Vec<String>, String> {
    let input = MidiInput::new("XDAW probe").map_err(|e| e.to_string())?;
    Ok(input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect())
}

impl MidiInputs {
    /// Connects to every available input port. `to_audio` must be realtime-safe
    /// (e.g. pushes into an rtrb ring); `to_ui` receives a copy for mapping.
    pub fn connect_all(
        to_audio: impl FnMut(RawMidi) + Send + Clone + 'static,
        to_ui: Sender<RawMidi>,
    ) -> Result<Self, String> {
        let probe = MidiInput::new("XDAW").map_err(|e| e.to_string())?;
        let ports = probe.ports();
        let mut connections = vec![];
        let mut port_names = vec![];
        for port in ports {
            let input = MidiInput::new("XDAW").map_err(|e| e.to_string())?;
            let name = input.port_name(&port).unwrap_or_else(|_| "MIDI".into());
            let mut audio = to_audio.clone();
            let ui = to_ui.clone();
            let conn = input.connect(
                &port,
                "xdaw-in",
                move |_stamp, bytes, _| {
                    if bytes.is_empty() || bytes.len() > 3 {
                        return;
                    }
                    let mut raw = RawMidi { bytes: [0; 3], len: bytes.len() as u8 };
                    raw.bytes[..bytes.len()].copy_from_slice(bytes);
                    audio(raw);
                    let _ = ui.try_send(raw);
                },
                (),
            );
            match conn {
                Ok(c) => {
                    connections.push(c);
                    port_names.push(name);
                }
                Err(e) => port_names.push(format!("{name} (failed: {e})")),
            }
        }
        Ok(Self { connections, port_names })
    }

    pub fn connected(&self) -> usize {
        self.connections.len()
    }
}
