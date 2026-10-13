//! UI-side handle: sends commands, receives events and drops retired payloads.

use crate::graph::Graph;
use crate::protocol::*;
use crate::render::{MidiInput, Renderer, Rings};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub const COMMAND_CAPACITY: usize = 4096;
pub const EVENT_CAPACITY: usize = 4096;
pub const GARBAGE_CAPACITY: usize = 1024;
pub const MIDI_CAPACITY: usize = 1024;

pub struct EngineHandle {
    commands: Producer<Command>,
    events: Consumer<EngineEvent>,
    garbage: Consumer<Garbage>,
    pub status: Arc<SharedStatus>,
}

impl EngineHandle {
    /// Sends a command; returns false (and counts it) if the queue is full.
    pub fn send(&mut self, cmd: Command) -> bool {
        match self.commands.push(cmd) {
            Ok(()) => true,
            Err(_) => {
                self.status.dropped_commands.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn poll_events(&mut self, mut f: impl FnMut(EngineEvent)) {
        while let Ok(e) = self.events.pop() {
            f(e);
        }
    }

    /// Drops payloads retired by the audio thread (call regularly from the UI thread).
    pub fn collect_garbage(&mut self) -> usize {
        let mut n = 0;
        while let Ok(g) = self.garbage.pop() {
            drop(g);
            n += 1;
        }
        n
    }
}

/// Creates a connected handle/renderer pair. The optional MIDI producer feeds
/// physical MIDI input into the renderer.
pub fn create(graph: Graph, bpm: f64, with_midi: bool) -> (EngineHandle, Renderer, Option<Producer<MidiInput>>) {
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (ev_tx, ev_rx) = RingBuffer::new(EVENT_CAPACITY);
    let (gb_tx, gb_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let (midi_tx, midi_rx) = if with_midi {
        let (a, b) = RingBuffer::new(MIDI_CAPACITY);
        (Some(a), Some(b))
    } else {
        (None, None)
    };
    let status = Arc::new(SharedStatus::new());
    let renderer = Renderer::new(
        Box::new(graph),
        bpm,
        Rings { commands: cmd_rx, garbage: gb_tx, events: ev_tx, midi_in: midi_rx },
        status.clone(),
    );
    (EngineHandle { commands: cmd_tx, events: ev_rx, garbage: gb_rx, status }, renderer, midi_tx)
}
