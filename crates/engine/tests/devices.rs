//! Device chain in the engine: MIDI effects, sidechain routing and
//! plugin-delay compensation (spec: docs/specs/devices.md D1, D3, D4).

use std::collections::HashMap;
use std::sync::Arc;
use xdaw_dsp::{DeviceKind, SampleData};
use xdaw_engine::build_graph;
use xdaw_engine::export::{render_offline, ExportSettings, SampleFormat};
use xdaw_project::*;
use xdaw_sequencer::ClipRegion;

type Media = HashMap<String, Arc<SampleData>>;
const SR: u32 = 48_000;

/// "dc": constant 0.5. "pulse": 0.9 for the first 100 ms of every beat (120 BPM).
/// "click": a single 0.4 sample 10 ms in (past the clip-edge declick).
fn media() -> Media {
    let n = SR as usize * 10;
    let beat = SR as usize / 2;
    let mut m = Media::new();
    m.insert("dc".into(), Arc::new(SampleData { channels: vec![vec![0.5; n]], sample_rate: SR as f64 }));
    let pulse = (0..n).map(|i| if i % beat < SR as usize / 10 { 0.9 } else { 0.0 }).collect();
    m.insert("pulse".into(), Arc::new(SampleData { channels: vec![pulse], sample_rate: SR as f64 }));
    let mut click = vec![0.0; n];
    click[480] = 0.4;
    m.insert("click".into(), Arc::new(SampleData { channels: vec![click], sample_rate: SR as f64 }));
    m
}

fn clip(p: &mut Project, content: ClipContent, len: f64) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: "c".into(),
        color: [0; 3],
        content,
        region: ClipRegion::looped(len),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    }
}

fn audio(path: &str) -> ClipContent {
    ClipContent::Audio(AudioContent { media: MediaRef { path: path.into() }, warp: WarpMode::RePitch, source_bpm: 120.0, markers: vec![], envelopes: vec![] })
}

fn place(p: &mut Project, t: usize, start: f64, len: f64, content: ClipContent) {
    let c = clip(p, content, len);
    let id = p.alloc_id();
    p.tracks[t].arrangement.push(ArrangementClip { id, start, length: len, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: c });
}

fn set(p: &mut Project, t: usize, d: usize, param: &str, v: f32) {
    let dev = &mut p.tracks[t].devices[d];
    let i = dev.kind.params().iter().position(|x| x.id == param).unwrap_or_else(|| panic!("no param {param}"));
    dev.params[i] = v;
}

fn render(p: &Project, end: f64) -> (Vec<f32>, Vec<f32>) {
    let s = ExportSettings { start: 0.0, end, tail_seconds: 0.0, sample_rate: SR, format: SampleFormat::Float32, dither: false };
    let (l, r, nonfinite) = render_offline(p, &media(), &s).unwrap();
    assert_eq!(nonfinite, 0);
    (l, r)
}

fn at(beat: f64) -> usize {
    (beat * 0.5 * SR as f64) as usize
}

fn new_project() -> Project {
    let mut p = Project::new("devices");
    p.transport.bpm = 120.0;
    p.add_scene("S");
    p
}

#[test]
fn arpeggiator_in_front_of_an_instrument_plays_sixteenths_on_the_grid() {
    let mut p = new_project();
    let t = p.add_track(TrackKind::Midi, "Arp");
    p.add_device(TrackRef::Track(t), DeviceKind::PolySynth);
    // Added after the instrument but placed before it (MIDI effects come first).
    p.add_device(TrackRef::Track(t), DeviceKind::Arpeggiator);
    assert_eq!(p.tracks[t].devices[0].kind, DeviceKind::Arpeggiator);
    set(&mut p, t, 1, "amp_attack", 0.5);
    set(&mut p, t, 1, "amp_decay", 30.0);
    set(&mut p, t, 1, "amp_sustain", 0.0);
    set(&mut p, t, 1, "amp_release", 5.0);
    let chord = [60, 64, 67].map(|pitch| Note { pitch, start: 0.0, length: 4.0, velocity: 110, chance: 100 }).to_vec();
    place(&mut p, t, 0.0, 4.0, ClipContent::Midi { notes: chord });
    let (l, _) = render(&p, 4.0);
    let data = SampleData { channels: vec![l], sample_rate: SR as f64 };
    let onsets = xdaw_dsp::slicing::detect_onsets(&data, 0.0, 2.0, 0.5);
    assert_eq!(onsets.len(), 16, "onsets {onsets:?}");
    for (k, o) in onsets.iter().enumerate() {
        assert!((o - k as f64 * 0.125).abs() < 0.006, "step {k} at {o}");
    }
}

/// Track "Pad" (dc) with a Compressor keyed by a muted "Kick" (pulse) track.
fn sidechain_project(listener_first: bool) -> Project {
    let mut p = new_project();
    let (pad, kick) = if listener_first {
        (p.add_track(TrackKind::Audio, "Pad"), p.add_track(TrackKind::Audio, "Kick"))
    } else {
        let k = p.add_track(TrackKind::Audio, "Kick");
        (p.add_track(TrackKind::Audio, "Pad"), k)
    };
    place(&mut p, pad, 0.0, 4.0, audio("dc"));
    place(&mut p, kick, 0.0, 4.0, audio("pulse"));
    p.tracks[kick].mixer.mute = true;
    p.add_device(TrackRef::Track(pad), DeviceKind::Compressor);
    set(&mut p, pad, 0, "threshold", -30.0);
    set(&mut p, pad, 0, "ratio", 20.0);
    set(&mut p, pad, 0, "attack", 0.1);
    set(&mut p, pad, 0, "release", 5.0);
    p.tracks[pad].devices[0].sidechain = Some(p.tracks[kick].id);
    p
}

#[test]
fn sidechain_ducks_the_listener_in_the_same_block_whatever_the_track_order() {
    let (a, _) = render(&sidechain_project(true), 4.0);
    let (b, _) = render(&sidechain_project(false), 4.0);
    let during = a[at(1.0) + 2400];
    let between = a[at(1.0) + 16_800];
    assert!(during < 0.1 * between, "pulse must duck the pad: {during} vs {between}");
    // The source is rendered before its listener in both orders: identical output.
    let worst = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "track order changed the result by {worst}");
}

#[test]
fn sidechain_cycles_fall_back_to_self_instead_of_failing() {
    let mut p = sidechain_project(true);
    p.add_device(TrackRef::Track(1), DeviceKind::Gate);
    p.tracks[1].devices[0].sidechain = Some(p.tracks[0].id);
    p.tracks[1].mixer.mute = false;
    let (l, _) = render(&p, 2.0);
    assert!(l.iter().all(|v| v.is_finite()));
}

#[test]
fn lookahead_latency_is_compensated_across_tracks_and_in_the_export() {
    let mut p = new_project();
    let a = p.add_track(TrackKind::Audio, "A");
    let b = p.add_track(TrackKind::Audio, "B");
    place(&mut p, a, 1.0, 1.0, audio("click"));
    place(&mut p, b, 1.0, 1.0, audio("click"));
    p.add_device(TrackRef::Track(a), DeviceKind::Limiter);
    set(&mut p, a, 0, "lookahead", 3.0); // 6 ms = 288 samples
    let graph = build_graph(&p, &media(), SR as f64).unwrap();
    assert_eq!(graph.tracks[a].device_latency(), 288);
    let (l, _) = render(&p, 2.0);
    let expected = at(1.0) + 480;
    // Both clicks land together, on their timeline position.
    assert!((l[expected] - 0.8).abs() < 1e-4, "aligned sum {}", l[expected]);
    let stray = l.iter().enumerate().filter(|&(i, v)| i != expected && v.abs() > 1e-4).count();
    assert_eq!(stray, 0, "click energy outside the expected sample");
}

#[test]
fn group_latency_counts_for_its_members() {
    let mut p = new_project();
    let g = p.add_track(TrackKind::Group, "G");
    let a = p.add_track(TrackKind::Audio, "A");
    let b = p.add_track(TrackKind::Audio, "B");
    p.tracks[a].group = Some(p.tracks[g].id);
    place(&mut p, a, 1.0, 1.0, audio("click"));
    place(&mut p, b, 1.0, 1.0, audio("click"));
    p.add_device(TrackRef::Track(g), DeviceKind::Limiter);
    set(&mut p, g, 0, "lookahead", 1.0); // 72 samples on the group
    let (l, _) = render(&p, 2.0);
    let expected = at(1.0) + 480;
    assert!((l[expected] - 0.8).abs() < 1e-4, "aligned sum {}", l[expected]);
}

#[test]
fn midi_effects_and_instruments_are_rejected_on_audio_tracks() {
    let mut p = new_project();
    let t = p.add_track(TrackKind::Audio, "A");
    p.tracks[t].devices.push(DeviceState::new(999, DeviceKind::ChordStack));
    assert!(build_graph(&p, &media(), SR as f64).is_err());
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn instrument_rack_splits_notes_between_chains_by_key_zone() {
    let render_note = |pitch: u8| {
        let mut p = new_project();
        let t = p.add_track(TrackKind::Midi, "Rack");
        let r = TrackRef::Track(t);
        let rack = p.add_device(r, DeviceKind::InstrumentRack);
        let low = p.add_chain(r, rack).unwrap();
        let high = p.add_chain(r, rack).unwrap();
        p.add_device_to_chain(r, low, DeviceKind::PolySynth).unwrap();
        p.add_device_to_chain(r, high, DeviceKind::FmSynth).unwrap();
        p.tracks[t].set_param(low, 5, 59.0); // Key High
        p.tracks[t].set_param(high, 4, 60.0); // Key Low
        p.tracks[t].set_param(high, 2, 1.0); // mute the high chain
        place(&mut p, t, 0.0, 2.0, ClipContent::Midi { notes: vec![Note { pitch, start: 0.0, length: 1.0, velocity: 110, chance: 100 }] });
        let (l, _) = render(&p, 2.0);
        rms(&l[..at(1.0)])
    };
    assert!(render_note(48) > 0.01, "the low chain plays C2");
    assert!(render_note(72) < 1e-6, "C4 goes only to the muted high chain");
}

#[test]
fn automation_reaches_a_device_inside_an_effect_rack() {
    let mut p = new_project();
    let t = p.add_track(TrackKind::Audio, "A");
    let r = TrackRef::Track(t);
    place(&mut p, t, 0.0, 4.0, audio("dc"));
    let rack = p.add_device(r, DeviceKind::EffectRack);
    let chain = p.add_chain(r, rack).unwrap();
    let util = p.add_device_to_chain(r, chain, DeviceKind::Gain).unwrap();
    let static_level = render(&p, 2.0).0[at(1.0)];
    assert!((static_level - 0.5).abs() < 1e-3, "unity rack: {static_level}");
    p.tracks[t].automation.push(AutomationLane {
        target: ParamTarget::Device { device: util, param: 0 },
        points: vec![AutomationPoint { beat: 0.0, value: -12.0 }, AutomationPoint { beat: 4.0, value: -12.0 }],
    });
    let automated = render(&p, 2.0).0[at(1.0)];
    let expected = 0.5 * 10f32.powf(-12.0 / 20.0);
    assert!((automated - expected).abs() < 1e-3, "automated {automated} vs {expected}");
}
