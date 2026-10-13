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

fn synth_project() -> Project {
    use xdaw_dsp::DeviceKind;
    let mut p = Project::new("freeze");
    p.transport.bpm = 120.0;
    p.add_scene("S");
    p.add_track(TrackKind::Midi, "M");
    p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
    p.add_device(TrackRef::Track(0), DeviceKind::Delay);
    let id = p.alloc_id();
    let notes = vec![Note { pitch: 60, start: 0.0, length: 1.0, velocity: 100, chance: 100 }, Note { pitch: 67, start: 2.0, length: 1.0, velocity: 90, chance: 100 }];
    let clip = Clip {
        id,
        name: "m".into(),
        color: [0; 3],
        content: ClipContent::Midi { notes },
        region: ClipRegion::looped(4.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    };
    p.tracks[0].arrangement.push(ArrangementClip { id: 99, start: 0.0, length: 4.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
    p.tracks[0].mixer.volume_db = -6.0;
    p
}

#[test]
fn frozen_track_plays_back_like_the_live_track() {
    use xdaw_engine::export::render_track_offline;
    let p = synth_project();
    let s = ExportSettings { start: 0.0, end: 8.0, tail_seconds: 0.0, sample_rate: SR, format: SampleFormat::Float32, dither: false };
    let (live_l, _, _) = render_offline(&p, &Media::new(), &s).unwrap();
    // Freeze: render the track through its devices (mixer left out), 8 beats = 4 s.
    let (fl, fr) = render_track_offline(&p, &Media::new(), 0, &s, true).unwrap();
    assert!(fl.iter().any(|x| x.abs() > 0.01), "freeze rendered something");
    let mut m = Media::new();
    m.insert("frozen.wav".into(), Arc::new(SampleData { channels: vec![fl, fr], sample_rate: SR as f64 }));
    let mut q = p.clone();
    q.tracks[0].frozen = Some(Frozen { media: MediaRef { path: "frozen.wav".into() }, bpm: 120.0, length: 8.0 });
    let (frozen_l, _, _) = render_offline(&q, &m, &s).unwrap();
    let err = live_l.iter().zip(&frozen_l).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    let peak = live_l.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(err < peak * 0.001, "max error {err} vs peak {peak}");
}

#[test]
fn consolidated_audio_is_unity_gain_and_ignores_the_mixer() {
    use xdaw_engine::export::render_track_offline;
    let mut p = Project::new("x");
    let c = clip(&mut p, vec![]);
    let mut p2 = project(vec![(0.0, 4.0, 0.0, c)]);
    p2.tracks[0].mixer.volume_db = -12.0;
    p2.tracks[0].mixer.pan = 0.8;
    let s = ExportSettings { start: 0.0, end: 4.0, tail_seconds: 0.0, sample_rate: SR, format: SampleFormat::Float32, dither: false };
    let (l, r) = render_track_offline(&p2, &media(), 0, &s, false).unwrap();
    assert!((l[at(2.0)] - 0.5).abs() < 1e-4 && (r[at(2.0)] - 0.5).abs() < 1e-4, "{} {}", l[at(2.0)], r[at(2.0)]);
}

#[test]
fn nested_groups_apply_each_level_in_order() {
    let mut p = Project::new("x");
    let c = clip(&mut p, vec![]);
    let mut p = project(vec![(0.0, 4.0, 0.0, c)]); // track 0: audio source
    // The inner group has a LOWER index than the outer one in one case and a higher one in the other.
    for outer_first in [false, true] {
        let mut q = p.clone();
        let (a, b) = (q.add_track(TrackKind::Group, "inner"), q.add_track(TrackKind::Group, "outer"));
        let (inner, outer) = if outer_first { (b, a) } else { (a, b) };
        q.tracks[inner].mixer.volume_db = -6.0;
        q.tracks[outer].mixer.volume_db = -6.0;
        let (iid, oid) = (q.tracks[inner].id, q.tracks[outer].id);
        q.tracks[0].group = Some(iid);
        q.tracks[inner].group = Some(oid);
        assert!(q.validate().is_empty(), "{:?}", q.validate());
        let (l, _) = render(&q, 4.0);
        let expected = 0.5 * 10f32.powf(-12.0 / 20.0);
        assert!((l[at(2.0)] - expected).abs() / expected < 0.02, "outer_first={outer_first}: {} vs {expected}", l[at(2.0)]);
    }
    // A loop is rejected.
    let a = p.add_track(TrackKind::Group, "a");
    let b = p.add_track(TrackKind::Group, "b");
    let (ia, ib) = (p.tracks[a].id, p.tracks[b].id);
    p.tracks[a].group = Some(ib);
    p.tracks[b].group = Some(ia);
    assert!(p.validate().iter().any(|i| i.contains("loop")));
}

#[test]
fn note_chance_thins_out_passes_but_full_chance_always_plays() {
    use xdaw_dsp::DeviceKind;
    let passes = 200usize;
    let build = |chance: u8| {
        let mut p = Project::new("chance");
        p.transport.bpm = 480.0; // 1 beat = 125 ms
        p.add_scene("S");
        p.add_track(TrackKind::Midi, "M");
        let d = p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
        // Short release so one pass does not ring into the next.
        let di = p.tracks[0].devices.iter().position(|x| x.id == d).unwrap();
        let rel = DeviceKind::PolySynth.params().iter().position(|x| x.id == "amp_release").unwrap();
        p.tracks[0].devices[di].params[rel] = 5.0;
        let id = p.alloc_id();
        let clip = Clip {
            id,
            name: "m".into(),
            color: [0; 3],
            content: ClipContent::Midi { notes: vec![Note { pitch: 72, start: 0.0, length: 0.5, velocity: 120, chance }] },
            region: ClipRegion::looped(1.0),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        };
        p.tracks[0].arrangement.push(ArrangementClip { id: 99, start: 0.0, length: passes as f64, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
        p
    };
    let s = ExportSettings { start: 0.0, end: passes as f64, tail_seconds: 0.0, sample_rate: SR, format: SampleFormat::Float32, dither: false };
    let count = |p: &Project| {
        let (l, _, _) = render_offline(p, &Media::new(), &s).unwrap();
        let per = (SR as f64 * 0.125) as usize;
        (0..passes).filter(|k| l[k * per..(k + 1) * per].iter().any(|x| x.abs() > 0.005)).count()
    };
    assert_eq!(count(&build(100)), passes);
    let half = count(&build(50));
    assert!((60..=140).contains(&half), "50% chance played {half} of {passes}");
    let low = count(&build(10));
    assert!(low < half && low > 0, "10% chance played {low}");
}
