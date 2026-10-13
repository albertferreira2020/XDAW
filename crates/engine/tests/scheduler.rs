//! Deterministic scheduler and render tests (offline, no audio device).
//!
//! Contract: docs/research/launch-quantization.md. Tolerances: event onsets must
//! land on the first integer sample at or after the exact musical position
//! (deviation in [0, 1) sample).

use std::collections::HashMap;
use std::sync::Arc;
use xdaw_dsp::{DeviceKind, SampleData};
use xdaw_engine::protocol::*;
use xdaw_engine::{build_graph, create, EngineHandle, Renderer};
use xdaw_project::*;
use xdaw_sequencer::{ClipQuantize, ClipRegion};

type Media = HashMap<String, Arc<SampleData>>;

fn dc_media(seconds: f64, sr: f64) -> Arc<SampleData> {
    Arc::new(SampleData { channels: vec![vec![0.5; (seconds * sr) as usize]], sample_rate: sr })
}

fn audio_clip(p: &mut Project, key: &str, beats: f64, bpm: f64) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: key.into(),
        color: [0, 0, 0],
        content: ClipContent::Audio(AudioContent { media: MediaRef { path: key.into() }, warp: WarpMode::RePitch, source_bpm: bpm, markers: vec![] }),
        region: ClipRegion::looped(beats),
        launch_mode: Default::default(),
        quantize: ClipQuantize::Global,
        legato: false,
        gain_db: 0.0,
        transpose: 0,
    }
}

fn midi_clip(p: &mut Project, notes: Vec<Note>, beats: f64) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: "midi".into(),
        color: [0, 0, 0],
        content: ClipContent::Midi { notes },
        region: ClipRegion::looped(beats),
        launch_mode: Default::default(),
        quantize: ClipQuantize::Global,
        legato: false,
        gain_db: 0.0,
        transpose: 0,
    }
}

struct Rig {
    handle: EngineHandle,
    renderer: Renderer,
}

impl Rig {
    fn new(p: &Project, media: &Media, sr: f64) -> Self {
        let g = build_graph(p, media, sr).unwrap();
        let (handle, renderer, _) = create(g, p.transport.bpm, false);
        let mut rig = Rig { handle, renderer };
        rig.renderer.include_metronome = false;
        rig.handle.send(Command::SetGlobalQuantize(p.transport.global_quantize));
        rig
    }

    fn render(&mut self, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        let mut pos = 0;
        while pos < frames {
            let n = block.min(frames - pos);
            self.renderer.process_planar(&mut l[pos..pos + n], &mut r[pos..pos + n]);
            pos += n;
        }
        self.handle.collect_garbage();
        (l, r)
    }

    fn events(&mut self) -> Vec<EngineEvent> {
        let mut v = vec![];
        self.handle.poll_events(|e| v.push(e));
        v
    }
}

fn first_nonzero(x: &[f32]) -> Option<usize> {
    x.iter().position(|&v| v.abs() > 1e-9)
}

fn one_audio_track(sr: f64, bpm: f64) -> (Project, Media) {
    let mut p = Project::new("t");
    p.transport.bpm = bpm;
    for i in 0..2 {
        p.add_scene(&format!("S{i}"));
    }
    p.add_track(TrackKind::Audio, "A");
    let mut media = Media::new();
    media.insert("dc".into(), dc_media(30.0, sr));
    let c = audio_clip(&mut p, "dc", 4.0, bpm);
    p.tracks[0].slots[0].clip = Some(c);
    (p, media)
}

#[test]
fn quantized_launch_starts_on_next_bar_sample() {
    for (sr, bpm) in [(48_000.0, 120.0), (44_100.0, 128.0), (48_000.0, 97.3)] {
        let (p, media) = one_audio_track(sr, bpm);
        let mut rig = Rig::new(&p, &media, sr);
        // Transport running, launch requested mid-bar at beat ~1.3.
        rig.handle.send(Command::Play);
        let spb = 60.0 * sr / bpm;
        let request_at = (1.3 * spb) as usize;
        let (a, _) = rig.render(request_at, 256);
        assert!(first_nonzero(&a).is_none());
        rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
        let (b, _) = rig.render(8 * spb as usize, 256);
        let onset = request_at + first_nonzero(&b).expect("clip must start");
        let exact = 4.0 * spb; // bar 2
        let dev = onset as f64 - exact;
        assert!((0.0..1.0).contains(&dev), "sr {sr} bpm {bpm}: onset {onset} exact {exact}");
        let ev = rig.events();
        assert!(ev.iter().any(|e| matches!(e, EngineEvent::ClipStarted { slot: 0, at, .. } if (*at - 4.0).abs() < 1e-9)), "{ev:?}");
    }
}

#[test]
fn launch_while_stopped_starts_transport_immediately() {
    let (p, media) = one_audio_track(48_000.0, 120.0);
    let mut rig = Rig::new(&p, &media, 48_000.0);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let (l, _) = rig.render(1000, 128);
    assert_eq!(first_nonzero(&l), Some(0));
}

#[test]
fn replacing_clip_switches_on_same_sample_without_gap() {
    let sr = 48_000.0;
    let (mut p, mut media) = one_audio_track(sr, 120.0);
    media.insert("neg".into(), Arc::new(SampleData { channels: vec![vec![-0.25; 48_000 * 10]], sample_rate: sr }));
    let c = audio_clip(&mut p, "neg", 4.0, 120.0);
    p.tracks[0].slots[1].clip = Some(c);
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    rig.render(30_000, 256);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 1, down: true });
    let (l, _) = rig.render(80_000, 256);
    // Bar 2 begins at sample 96000 (= 66000 into this render): the old clip is
    // untouched before it, the 2 ms crossfade starts exactly on it, and only the
    // new clip remains afterwards.
    let b = 66_000;
    assert!((l[b - 1] - 0.5).abs() < 1e-6, "old clip intact before boundary");
    assert!(l[b] < 0.5 - 1e-3, "transition starts on the boundary sample");
    assert!((l[b + 96] + 0.25).abs() < 1e-6, "only the new clip after the crossfade: {}", l[b + 96]);
    // No silent sample across the switch beyond the zero crossing of the fade.
    let silent = l[b - 10..b + 200].iter().filter(|x| x.abs() < 1e-6).count();
    assert!(silent <= 1, "gap at switch");
}

#[test]
fn looping_audio_has_no_drift_after_many_loops() {
    // 44.1 kHz, 128 BPM: one bar = 82687.5 samples (non-integer).
    let sr = 44_100.0;
    let bpm = 128.0;
    let mut p = Project::new("t");
    p.transport.bpm = bpm;
    p.add_scene("S");
    p.add_track(TrackKind::Audio, "A");
    let bar_frames = (4.0 * 60.0 / bpm * sr) as usize + 1;
    let mut data = vec![0.0f32; bar_frames];
    data[0] = 1.0; // impulse at loop start
    let mut media = Media::new();
    media.insert("imp".into(), Arc::new(SampleData { channels: vec![data], sample_rate: sr }));
    let c = audio_clip(&mut p, "imp", 4.0, bpm);
    p.tracks[0].slots[0].clip = Some(c);
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let loops = 200;
    let spb = 60.0 * sr / bpm;
    let (l, _) = rig.render((loops as f64 * 4.0 * spb) as usize + 10, 256);
    for k in [1usize, 10, 100, 199] {
        let exact = k as f64 * 4.0 * spb;
        let lo = exact as usize - 4;
        let window = &l[lo..lo + 8];
        let peak = window.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0 + lo;
        assert!((peak as f64 - exact).abs() <= 1.0, "loop {k}: peak {peak} exact {exact}");
    }
}

#[test]
fn tempo_change_reschedules_boundary_musically() {
    let sr = 48_000.0;
    let (p, media) = one_audio_track(sr, 120.0);
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::Play);
    rig.render(24_000, 256); // beat 1
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    rig.render(24_000, 256); // beat 2
    rig.handle.send(Command::SetBpm(60.0));
    // Remaining 2 beats at 60 BPM = 96000 samples.
    let (l, _) = rig.render(200_000, 256);
    let onset = first_nonzero(&l).unwrap();
    assert_eq!(onset, 96_000);
}

#[test]
fn scene_launch_respects_stop_buttons() {
    let sr = 48_000.0;
    let mut p = Project::new("t");
    p.add_scene("A");
    p.add_scene("B");
    p.add_track(TrackKind::Audio, "1");
    p.add_track(TrackKind::Audio, "2");
    p.add_track(TrackKind::Audio, "3");
    let mut media = Media::new();
    media.insert("dc".into(), dc_media(30.0, sr));
    for t in 0..3 {
        let c = audio_clip(&mut p, "dc", 4.0, 120.0);
        p.tracks[t].slots[0].clip = Some(c);
    }
    // Scene B: track 1 has a clip, track 2 empty with stop button, track 3 empty without.
    let c = audio_clip(&mut p, "dc", 4.0, 120.0);
    p.tracks[0].slots[1].clip = Some(c);
    p.tracks[2].slots[1].has_stop_button = false;
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::LaunchScene { slot: 0 });
    rig.render(10_000, 256);
    rig.handle.send(Command::LaunchScene { slot: 1 });
    rig.render(100_000, 256);
    let st = |t: u16| rig.handle.status.track(TrackIx::Track(t)).playing_slot.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(st(0), 1);
    assert_eq!(st(1), NO_SLOT);
    assert_eq!(st(2), 0);
}

#[test]
fn session_launch_overrides_arrangement_until_back_to_arrangement() {
    let sr = 48_000.0;
    let (mut p, media) = one_audio_track(sr, 120.0);
    let c = audio_clip(&mut p, "dc", 4.0, 120.0);
    p.tracks[0].arrangement.push(ArrangementClip { id: 99, start: 0.0, length: 64.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: c });
    // Make the session clip distinguishable: half gain.
    if let Some(c) = p.tracks[0].slots[0].clip.as_mut() {
        c.gain_db = -6.0206;
    }
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::Play);
    let (l, _) = rig.render(10_000, 256);
    assert!((l[5000] - 0.5).abs() < 1e-4, "arrangement plays: {}", l[5000]);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let (l, _) = rig.render(100_000, 256);
    assert!((l[95_000] - 0.25).abs() < 1e-3, "session replaced arrangement: {}", l[95_000]);
    assert!(!rig.handle.status.track(TrackIx::Track(0)).arrangement_active.load(std::sync::atomic::Ordering::Relaxed));
    // Never both at once: level is the session level, not the sum.
    assert!(l[86_000..].iter().all(|x| x.abs() < 0.26));
    rig.handle.send(Command::BackToArrangement);
    let (l, _) = rig.render(5_000, 256);
    assert!((l[4_000] - 0.5).abs() < 1e-4, "back to arrangement: {}", l[4_000]);
}

#[test]
fn clip_stop_button_silences_arrangement_track() {
    let sr = 48_000.0;
    let (mut p, media) = one_audio_track(sr, 120.0);
    let c = audio_clip(&mut p, "dc", 4.0, 120.0);
    p.tracks[0].arrangement.push(ArrangementClip { id: 99, start: 0.0, length: 64.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: c });
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::Play);
    rig.render(1000, 256);
    rig.handle.send(Command::StopTrack { track: 0 });
    let (l, _) = rig.render(120_000, 256);
    assert!(l[90_000].abs() > 0.1);
    assert!(l[100_000..].iter().all(|x| x.abs() < 1e-6), "silence after quantized stop");
    let ev = rig.events();
    assert!(ev.iter().any(|e| matches!(e, EngineEvent::TrackReleased { track: 0, .. })));
}

fn midi_project(notes: Vec<Note>, clip_beats: f64) -> Project {
    let mut p = Project::new("m");
    p.add_scene("S");
    let t = p.add_track(TrackKind::Midi, "Synth");
    p.add_device(TrackRef::Track(t), DeviceKind::PolySynth);
    let c = midi_clip(&mut p, notes, clip_beats);
    p.tracks[t].slots[0].clip = Some(c);
    p
}

#[test]
fn midi_note_onsets_are_sample_accurate_across_loops() {
    let sr = 48_000.0;
    // A note at beat 0.5 inside a 1-beat loop: onsets at 0.5, 1.5, 2.5 ...
    let p = midi_project(vec![Note { pitch: 60, start: 0.5, length: 0.1, velocity: 127 }], 1.0);
    let mut rig = Rig::new(&p, &Media::new(), sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let (l, _) = rig.render(24_000 * 4, 100); // odd block size crosses boundaries
    for k in 0..4 {
        let exact = (k as f64 + 0.5) * 24_000.0;
        let seg = &l[exact as usize - 50..exact as usize + 50];
        let onset = first_nonzero(seg).unwrap() + exact as usize - 50;
        assert_eq!(onset, exact as usize, "loop {k}");
    }
}

#[test]
fn stopping_transport_leaves_no_stuck_notes() {
    let sr = 48_000.0;
    // Long sustained notes.
    let p = midi_project(vec![Note { pitch: 60, start: 0.0, length: 64.0, velocity: 100 }, Note { pitch: 67, start: 0.0, length: 64.0, velocity: 100 }], 64.0);
    let mut rig = Rig::new(&p, &Media::new(), sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let (l, _) = rig.render(20_000, 256);
    assert!(l.iter().any(|x| x.abs() > 0.01));
    rig.handle.send(Command::Stop);
    let (l, _) = rig.render(48_000, 256);
    assert!(l[24_000..].iter().all(|x| x.abs() < 1e-4), "release must end");
}

#[test]
fn clip_stop_and_relaunch_send_note_offs() {
    let sr = 48_000.0;
    let p = midi_project(vec![Note { pitch: 60, start: 0.0, length: 64.0, velocity: 100 }], 64.0);
    let mut rig = Rig::new(&p, &Media::new(), sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    rig.render(10_000, 256);
    rig.handle.send(Command::StopTrack { track: 0 });
    let (l, _) = rig.render(48_000 * 3, 256);
    // Stop at bar 2 (96000 total → 86000 here), release 250 ms default.
    assert!(l[86_000 - 100].abs() > 0.0);
    assert!(l[86_000 + 24_000..].iter().all(|x| x.abs() < 1e-4));
}

#[test]
fn render_is_independent_of_block_size_within_tolerance() {
    let sr = 48_000.0;
    let mut p = midi_project(
        vec![
            Note { pitch: 60, start: 0.0, length: 0.5, velocity: 100 },
            Note { pitch: 64, start: 0.75, length: 0.5, velocity: 90 },
            Note { pitch: 67, start: 1.5, length: 1.0, velocity: 80 },
        ],
        4.0,
    );
    p.add_device(TrackRef::Track(0), DeviceKind::Delay);
    let render = |block: usize| {
        let mut rig = Rig::new(&p, &Media::new(), sr);
        rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
        rig.render(48_000 * 3, block).0
    };
    let a = render(64);
    let b = render(256);
    let c = render(331);
    let max_diff = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
    // Differences come only from per-block coefficient update phase in filters.
    let peak = a.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(max_diff(&a, &b) < peak * 0.05, "64 vs 256: {}", max_diff(&a, &b));
    assert!(max_diff(&b, &c) < peak * 0.05, "256 vs 331: {}", max_diff(&b, &c));
}

#[test]
fn arrangement_loop_wraps_on_sample() {
    let sr = 48_000.0;
    let (mut p, mut media) = one_audio_track(sr, 120.0);
    let len = 48_000 * 4;
    let ramp: Vec<f32> = (0..len).map(|i| i as f32 / len as f32).collect();
    media.insert("ramp".into(), Arc::new(SampleData { channels: vec![ramp], sample_rate: sr }));
    let c = audio_clip(&mut p, "ramp", 8.0, 120.0);
    p.tracks[0].arrangement.push(ArrangementClip { id: 7, start: 0.0, length: 8.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: c });
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::SetLoop { enabled: true, start: 0.0, length: 4.0 });
    rig.handle.send(Command::Play);
    let (l, _) = rig.render(48_000 * 3, 300);
    // Loop end = beat 4 = sample 96000; the ramp must drop there.
    let drop = (1..l.len()).find(|&i| l[i] < l[i - 1] - 0.1).unwrap();
    assert_eq!(drop, 96_000);
}

#[test]
fn automation_drives_volume_and_manual_change_overrides() {
    let sr = 48_000.0;
    let (mut p, media) = one_audio_track(sr, 120.0);
    let c = audio_clip(&mut p, "dc", 4.0, 120.0);
    p.tracks[0].arrangement.push(ArrangementClip { id: 9, start: 0.0, length: 64.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip: c });
    let lane = p.tracks[0].lane_mut(ParamTarget::Volume);
    lane.insert(AutomationPoint { beat: 0.0, value: -6.0206 });
    lane.insert(AutomationPoint { beat: 64.0, value: -6.0206 });
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::Play);
    let (l, _) = rig.render(10_000, 256);
    assert!((l[9_000] - 0.25).abs() < 1e-3, "{}", l[9_000]);
    rig.handle.send(Command::SetMixer { track: TrackIx::Track(0), param: MixParam::Volume, value: 0.0 });
    let (l, _) = rig.render(10_000, 256);
    assert!((l[9_000] - 0.5).abs() < 1e-3, "manual override wins: {}", l[9_000]);
    assert!(rig.events().iter().any(|e| matches!(e, EngineEvent::AutomationOverridden { .. })));
    rig.handle.send(Command::BackToArrangement);
    let (l, _) = rig.render(10_000, 256);
    assert!((l[9_000] - 0.25).abs() < 1e-3, "automation re-enabled: {}", l[9_000]);
}

#[test]
fn sends_reach_returns_and_mute_solo_gate() {
    let sr = 48_000.0;
    let (mut p, media) = one_audio_track(sr, 120.0);
    p.add_return("R");
    p.tracks[0].mixer.sends_db[0] = 0.0;
    p.tracks[0].mixer.volume_db = -70.0; // track fader down: send is post-fader → silent
    let mut rig = Rig::new(&p, &media, sr);
    rig.handle.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let (l, _) = rig.render(5_000, 256);
    assert!(l[4_000].abs() < 1e-3);
    rig.handle.send(Command::SetMixer { track: TrackIx::Track(0), param: MixParam::Volume, value: 0.0 });
    let (l, _) = rig.render(5_000, 256);
    // Direct 0.5 plus send 0.5 through the return.
    assert!((l[4_000] - 1.0).abs() < 1e-3, "{}", l[4_000]);
    rig.handle.send(Command::SetMixer { track: TrackIx::Track(0), param: MixParam::Mute, value: 1.0 });
    let (l, _) = rig.render(5_000, 256);
    assert!(l[4_000].abs() < 1e-4, "mute silences track and its sends: {}", l[4_000]);
}
