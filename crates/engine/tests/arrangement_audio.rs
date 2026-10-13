//! Arrangement audio: clip envelopes and crossfades (offline render).

use std::collections::HashMap;
use std::sync::Arc;
use xdaw_dsp::SampleData;
use xdaw_engine::export::{render_offline, ExportSettings, SampleFormat};
use xdaw_project::*;
use xdaw_sequencer::ClipRegion;

type Media = HashMap<String, Arc<SampleData>>;
const SR: u32 = 48_000;

fn media() -> Media {
    let mut m = Media::new();
    m.insert("dc".into(), Arc::new(SampleData { channels: vec![vec![0.5; SR as usize * 30]], sample_rate: SR as f64 }));
    m
}

fn clip(p: &mut Project, envelopes: Vec<ClipEnvelope>) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: "dc".into(),
        color: [0; 3],
        content: ClipContent::Audio(AudioContent { media: MediaRef { path: "dc".into() }, warp: WarpMode::RePitch, source_bpm: 120.0, markers: vec![], envelopes }),
        region: ClipRegion::looped(32.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    }
}

fn project(clips: Vec<(f64, f64, f64, Clip)>) -> Project {
    let mut p = Project::new("env");
    p.transport.bpm = 120.0;
    p.add_scene("S");
    let t = p.add_track(TrackKind::Audio, "A");
    for (start, length, offset, clip) in clips {
        let id = p.alloc_id();
        p.tracks[t].arrangement.push(ArrangementClip { id, start, length, offset, fade_in: 0.0, fade_out: 0.0, clip });
    }
    p
}

fn render(p: &Project, end: f64) -> (Vec<f32>, Vec<f32>) {
    let s = ExportSettings { start: 0.0, end, tail_seconds: 0.0, sample_rate: SR, format: SampleFormat::Float32, dither: false };
    let (l, r, _) = render_offline(p, &media(), &s).unwrap();
    (l, r)
}

/// Frame of a song beat at 120 BPM.
fn at(beat: f64) -> usize {
    (beat * 0.5 * SR as f64) as usize
}

#[test]
fn gain_envelope_follows_its_ramp() {
    let mut p = Project::new("x");
    let env = ClipEnvelope { target: ClipEnvTarget::Gain, points: vec![AutomationPoint { beat: 0.0, value: 0.0 }, AutomationPoint { beat: 4.0, value: -20.0 }] };
    let c = clip(&mut p, vec![env]);
    let p = project(vec![(0.0, 4.0, 0.0, c)]);
    let (l, _) = render(&p, 4.0);
    let ratio = l[at(3.5)] / l[at(0.5)];
    let expected = 10f32.powf(-15.0 / 20.0);
    assert!((ratio - expected).abs() / expected < 0.02, "ratio {ratio} vs {expected}");
}

#[test]
fn pan_envelope_moves_the_clip_to_one_side() {
    let mut p = Project::new("x");
    let env = ClipEnvelope { target: ClipEnvTarget::Pan, points: vec![AutomationPoint { beat: 0.0, value: 1.0 }] };
    let c = clip(&mut p, vec![env]);
    let p = project(vec![(0.0, 4.0, 0.0, c)]);
    let (l, r) = render(&p, 4.0);
    assert!(l[at(2.0)].abs() < 1e-6, "left must be silent, got {}", l[at(2.0)]);
    assert!(r[at(2.0)] > 0.05);
}

#[test]
fn crossfade_keeps_a_constant_level_through_the_overlap() {
    let mut p = Project::new("x");
    let (a, b) = (clip(&mut p, vec![]), clip(&mut p, vec![]));
    let mut p = project(vec![(0.0, 4.0, 0.0, a), (4.0, 4.0, 4.0, b)]);
    let steady = render(&p, 8.0).0[at(2.0)];
    p.tracks[0].crossfade(0, 2.0).unwrap(); // overlap beats 3..5
    let (l, _) = render(&p, 8.0);
    for beat in [1.0, 3.0, 3.5, 4.0, 4.5, 5.0, 6.0] {
        let v = l[at(beat) + 10];
        assert!((v - steady).abs() / steady < 0.02, "beat {beat}: {v} vs {steady}");
    }
}

#[test]
fn butt_joined_clips_without_crossfade_still_declick() {
    let mut p = Project::new("x");
    let (a, b) = (clip(&mut p, vec![]), clip(&mut p, vec![]));
    let p = project(vec![(0.0, 4.0, 0.0, a), (4.0, 4.0, 4.0, b)]);
    let (l, _) = render(&p, 8.0);
    assert!(l[at(4.0)].abs() < l[at(2.0)], "edge declick dips at the join");
}
