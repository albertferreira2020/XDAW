//! Reconciles the UI-owned project with the audio thread.
//!
//! Structural changes (tracks, devices, media assignment, scene count, stop
//! buttons) rebuild the graph; everything else becomes fine-grained commands,
//! so editing a clip or moving a fader never re-creates devices.

use crate::graph::{build_graph, compile_arrangement, compile_automation, compile_clip, GraphError, MediaSource};
use crate::protocol::{Command, MixParam, TrackIx};
use xdaw_project::{Monitor, Project, Track, TrackRef};

#[derive(Clone, Debug, PartialEq)]
struct DeviceKey {
    id: u64,
    kind: xdaw_dsp::DeviceKind,
    sample: Option<String>,
    pads: Vec<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
struct TrackKey {
    id: u64,
    kind: xdaw_project::TrackKind,
    devices: Vec<DeviceKey>,
    stops: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
struct StructureKey {
    /// Tempo, when any clip uses pitch-preserving stretch (its rendering depends on tempo).
    stretch_bpm: Option<u64>,
    tracks: Vec<TrackKey>,
    returns: Vec<TrackKey>,
    master: TrackKey,
}

fn track_key(t: &Track) -> TrackKey {
    TrackKey {
        id: t.id,
        kind: t.kind,
        devices: t
            .devices
            .iter()
            .map(|d| DeviceKey {
                id: d.id,
                kind: d.kind,
                sample: d.sample.as_ref().map(|m| m.path.clone()),
                pads: d.pads.iter().map(|p| p.as_ref().map(|m| m.path.clone())).collect(),
            })
            .collect(),
        stops: t.slots.iter().map(|s| s.has_stop_button).collect(),
    }
}

fn structure(p: &Project) -> StructureKey {
    let uses_stretch = p.tracks.iter().any(|t| {
        t.slots.iter().filter_map(|s| s.clip.as_ref()).chain(t.arrangement.iter().map(|a| &a.clip)).any(|c| {
            matches!(&c.content, xdaw_project::ClipContent::Audio(a) if a.warp == xdaw_project::WarpMode::Stretch)
        })
    });
    StructureKey {
        stretch_bpm: uses_stretch.then_some(p.transport.bpm.to_bits()),
        tracks: p.tracks.iter().map(track_key).collect(),
        returns: p.returns.iter().map(track_key).collect(),
        master: track_key(&p.master),
    }
}

fn ix(r: TrackRef) -> TrackIx {
    match r {
        TrackRef::Track(i) => TrackIx::Track(i as u16),
        TrackRef::Return(i) => TrackIx::Return(i as u16),
        TrackRef::Master => TrackIx::Master,
    }
}

pub struct Reconciler {
    synced: Option<Project>,
    media_generation: u64,
    pub sample_rate: f64,
    pub full_rebuilds: u64,
}

impl Reconciler {
    pub fn new(sample_rate: f64) -> Self {
        Self { synced: None, media_generation: 0, sample_rate, full_rebuilds: 0 }
    }

    /// Forces the next `sync` to rebuild (device restart, media arrival).
    pub fn invalidate(&mut self) {
        self.synced = None;
    }

    pub fn needs_sync(&self, p: &Project, media_generation: u64) -> bool {
        media_generation != self.media_generation || self.synced.as_ref() != Some(p)
    }

    /// Emits the commands that bring the audio thread to `p`.
    pub fn sync(&mut self, p: &Project, media: &dyn MediaSource, media_generation: u64, mut send: impl FnMut(Command)) -> Result<(), GraphError> {
        let rebuild = media_generation != self.media_generation
            || self.synced.as_ref().is_none_or(|old| structure(old) != structure(p) || old.scenes.len() != p.scenes.len());
        if rebuild {
            let graph = build_graph(p, media, self.sample_rate)?;
            send(Command::SetGraph(Box::new(graph)));
            self.full_rebuilds += 1;
            self.send_transport(None, p, &mut send);
            self.media_generation = media_generation;
            self.synced = Some(p.clone());
            return Ok(());
        }
        let old = self.synced.take().expect("checked above");
        self.send_transport(Some(&old), p, &mut send);
        for (r, t) in p.all_tracks() {
            let o = old.track(r);
            let tix = ix(r);
            if let TrackRef::Track(ti) = r {
                for (si, (a, b)) in o.slots.iter().zip(&t.slots).enumerate() {
                    if a.clip != b.clip {
                        let clip = b.clip.as_ref().map(|c| compile_clip(c, media, p.transport.bpm));
                        send(Command::SetClip { track: ti as u16, slot: si as u16, clip: Box::new(clip) });
                    }
                }
            }
            if o.arrangement != t.arrangement {
                send(Command::SetArrangement { track: tix, clips: Box::new(compile_arrangement(t, media, p.transport.bpm)) });
            }
            if o.automation != t.automation {
                send(Command::SetAutomation { track: tix, lanes: Box::new(compile_automation(t)) });
            }
            let (a, b) = (&o.mixer, &t.mixer);
            let mut mix = |param, value| send(Command::SetMixer { track: tix, param, value });
            if a.volume_db != b.volume_db {
                mix(MixParam::Volume, b.volume_db);
            }
            if a.pan != b.pan {
                mix(MixParam::Pan, b.pan);
            }
            if a.mute != b.mute {
                mix(MixParam::Mute, b.mute as u8 as f32);
            }
            if a.solo != b.solo {
                mix(MixParam::Solo, b.solo as u8 as f32);
            }
            if a.arm != b.arm {
                mix(MixParam::Arm, b.arm as u8 as f32);
            }
            if a.monitor != b.monitor {
                mix(MixParam::Monitor, match b.monitor {
                    Monitor::In => 0.0,
                    Monitor::Auto => 1.0,
                    Monitor::Off => 2.0,
                });
            }
            for (i, (x, y)) in a.sends_db.iter().zip(&b.sends_db).enumerate() {
                if x != y {
                    mix(MixParam::Send(i as u16), *y);
                }
            }
            for (di, (x, y)) in o.devices.iter().zip(&t.devices).enumerate() {
                if x.bypass != y.bypass {
                    send(Command::SetDeviceBypass { track: tix, device: di as u16, bypass: y.bypass });
                }
                for pi in 0..y.kind.params().len() {
                    if x.param(pi) != y.param(pi) {
                        send(Command::SetDeviceParam { track: tix, device: di as u16, param: pi as u16, value: y.param(pi) });
                    }
                }
            }
        }
        self.synced = Some(p.clone());
        Ok(())
    }

    fn send_transport(&self, old: Option<&Project>, p: &Project, send: &mut impl FnMut(Command)) {
        let (n, o) = (&p.transport, old.map(|o| &o.transport));
        if o.is_none_or(|o| o.bpm != n.bpm) {
            send(Command::SetBpm(n.bpm));
        }
        if o.is_none_or(|o| o.time_signature != n.time_signature) {
            send(Command::SetTimeSignature(n.time_signature));
        }
        if o.is_none_or(|o| o.global_quantize != n.global_quantize) {
            send(Command::SetGlobalQuantize(n.global_quantize));
        }
        if o.is_none_or(|o| (o.loop_enabled, o.loop_start, o.loop_length) != (n.loop_enabled, n.loop_start, n.loop_length)) {
            send(Command::SetLoop { enabled: n.loop_enabled, start: n.loop_start, length: n.loop_length });
        }
        if o.is_none_or(|o| o.metronome != n.metronome) {
            send(Command::SetMetronome(n.metronome));
        }
        if o.is_none_or(|o| o.count_in_bars != n.count_in_bars) {
            send(Command::SetCountIn(n.count_in_bars));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use xdaw_dsp::DeviceKind;
    use xdaw_project::TrackKind;

    fn names(cmds: &[Command]) -> Vec<&'static str> {
        cmds.iter()
            .map(|c| match c {
                Command::SetGraph(_) => "graph",
                Command::SetClip { .. } => "clip",
                Command::SetArrangement { .. } => "arrangement",
                Command::SetAutomation { .. } => "automation",
                Command::SetMixer { .. } => "mixer",
                Command::SetDeviceParam { .. } => "param",
                Command::SetBpm(_) => "bpm",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn edits_become_fine_grained_and_structure_rebuilds() {
        let media: HashMap<String, std::sync::Arc<xdaw_dsp::SampleData>> = HashMap::new();
        let mut p = Project::new("x");
        p.add_scene("A");
        p.add_track(TrackKind::Midi, "M");
        p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
        let mut rec = Reconciler::new(48_000.0);
        let mut cmds = vec![];
        rec.sync(&p, &media, 0, |c| cmds.push(c)).unwrap();
        assert_eq!(names(&cmds)[0], "graph");

        cmds.clear();
        p.tracks[0].mixer.volume_db = -3.0;
        p.tracks[0].devices[0].params[5] = 500.0;
        p.transport.bpm = 100.0;
        rec.sync(&p, &media, 0, |c| cmds.push(c)).unwrap();
        let n = names(&cmds);
        assert!(n.contains(&"mixer") && n.contains(&"param") && n.contains(&"bpm") && !n.contains(&"graph"), "{n:?}");

        cmds.clear();
        p.add_device(TrackRef::Track(0), DeviceKind::Delay);
        rec.sync(&p, &media, 0, |c| cmds.push(c)).unwrap();
        assert_eq!(names(&cmds)[0], "graph");
        assert!(!rec.needs_sync(&p, 0));
        assert!(rec.needs_sync(&p, 1), "new media triggers a rebuild");
    }
}
