//! Recording paths and export/playback equivalence (offline, no devices).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use xdaw_dsp::{DeviceKind, NoteKind, SampleData};
use xdaw_engine::audio_io::InputTap;
use xdaw_engine::export::{render_offline, ExportSettings, SampleFormat};
use xdaw_engine::protocol::*;
use xdaw_engine::recorder::Recorder;
use xdaw_engine::{build_graph, create};
use xdaw_project::*;
use xdaw_sequencer::ClipRegion;

type Media = HashMap<String, Arc<SampleData>>;

fn render(r: &mut xdaw_engine::Renderer, frames: usize) -> Vec<f32> {
    let (mut l, mut rr) = (vec![0.0; frames], vec![0.0; frames]);
    r.process_planar(&mut l, &mut rr);
    l
}

#[test]
fn midi_session_record_captures_notes_with_beats() {
    let mut p = Project::new("r");
    p.add_scene("S");
    p.add_track(TrackKind::Midi, "M");
    p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
    p.tracks[0].mixer.arm = true;
    let g = build_graph(&p, &Media::new(), 48_000.0).unwrap();
    let (mut h, mut r, _) = create(g, 120.0, false);
    h.send(Command::SessionRecord { track: 0, slot: 0, on: true }); // stopped → starts immediately
    render(&mut r, 24_000); // beat 1
    h.send(Command::Note { track: 0, kind: NoteKind::On { pitch: 64, velocity: 90 } });
    render(&mut r, 12_000); // +0.5 beat
    h.send(Command::Note { track: 0, kind: NoteKind::Off { pitch: 64 } });
    render(&mut r, 256);
    h.send(Command::SessionRecord { track: 0, slot: 0, on: false });
    render(&mut r, 256);
    let mut ev = vec![];
    h.poll_events(|e| ev.push(e));
    assert!(ev.iter().any(|e| matches!(e, EngineEvent::RecordingStarted { track: 0, slot: 0, at } if *at == 0.0)), "{ev:?}");
    let note = ev.iter().find_map(|e| match e {
        EngineEvent::NoteRecorded { pitch, velocity, start, end, .. } => Some((*pitch, *velocity, *start, *end)),
        _ => None,
    });
    let (pitch, vel, start, end) = note.expect("note recorded");
    assert_eq!((pitch, vel), (64, 90));
    // Live input is timestamped at block start: error bounded by one block (256 samples ≈ 0.0107 beats).
    assert!((start - 1.0).abs() < 0.011, "start {start}");
    assert!((end - 1.5).abs() < 0.011, "end {end}");
    assert!(ev.iter().any(|e| matches!(e, EngineEvent::RecordingStopped { .. })));
}

#[test]
fn audio_session_record_streams_input_to_wav() {
    let mut p = Project::new("a");
    p.add_scene("S");
    p.add_track(TrackKind::Audio, "A");
    p.tracks[0].mixer.arm = true;
    let g = build_graph(&p, &Media::new(), 48_000.0).unwrap();
    let (mut h, mut r, _) = create(g, 120.0, false);
    let dir = std::env::temp_dir().join(format!("xdaw-rec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (recorder, rec_tx) = Recorder::spawn(48_000, dir.clone());
    r.attach_recorder(rec_tx);
    let (mut in_tx, in_rx) = rtrb::RingBuffer::new(48_000 * 4);
    r.attach_input(InputTap { consumer: in_rx, channels: 1 });
    h.send(Command::SessionRecord { track: 0, slot: 0, on: true });
    // Feed one second of a 0.25 constant input, rendering as it arrives.
    let mut rendered = vec![];
    for _ in 0..(48_000 / 256) {
        for _ in 0..256 {
            in_tx.push(0.25).unwrap();
        }
        rendered.extend(render(&mut r, 256));
    }
    h.send(Command::SessionRecord { track: 0, slot: 0, on: false });
    render(&mut r, 256);
    let take = recorder.finished.recv_timeout(Duration::from_secs(5)).expect("take finished");
    assert!(take.error.is_none(), "{:?}", take.error);
    assert_eq!(take.frames, (48_000 / 256) * 256);
    let reader = hound::WavReader::open(&take.path).unwrap();
    assert_eq!(reader.spec().sample_rate, 48_000);
    let samples: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();
    assert!(samples.iter().all(|&x| (x - 0.25).abs() < 1e-6));
    // Auto monitoring on an armed track passes the input through.
    assert!((rendered[1000] - 0.25).abs() < 1e-3, "monitor {}", rendered[1000]);
}

#[test]
fn export_matches_realtime_render_of_the_same_state() {
    let sr = 48_000.0;
    let mut p = Project::new("e");
    p.transport.bpm = 120.0;
    p.add_scene("S");
    p.add_track(TrackKind::Midi, "M");
    p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
    p.add_device(TrackRef::Track(0), DeviceKind::Delay);
    let id = p.alloc_id();
    let clip = Clip {
        id,
        name: "c".into(),
        color: [0; 3],
        content: ClipContent::Midi { notes: vec![Note { pitch: 60, start: 0.0, length: 0.5, velocity: 100, chance: 100 }, Note { pitch: 67, start: 1.0, length: 1.0, velocity: 80, chance: 100 }] },
        region: ClipRegion::looped(4.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    };
    p.tracks[0].arrangement.push(ArrangementClip { id: 99, start: 0.0, length: 8.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
    let settings = ExportSettings { start: 0.0, end: 8.0, tail_seconds: 0.0, sample_rate: 48_000, format: SampleFormat::Float32, dither: false };
    let (el, _, _) = render_offline(&p, &Media::new(), &settings).unwrap();
    // "Realtime" path: device-sized blocks through process_interleaved.
    let g = build_graph(&p, &Media::new(), sr).unwrap();
    let (mut h, mut r, _) = create(g, 120.0, false);
    r.include_metronome = false;
    h.send(Command::Play);
    let mut out = vec![0.0f32; el.len() * 2];
    for chunk in out.chunks_mut(512 * 2) {
        r.process_interleaved(chunk, 2);
    }
    let rl: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    let peak = el.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let max_diff = el.iter().zip(&rl).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.01);
    // Tolerance: filter coefficient updates are block-phase dependent (see DSP notes).
    assert!(max_diff <= peak * 0.05, "export vs playback max diff {max_diff} (peak {peak})");
}

#[test]
fn launch_transition_has_no_click() {
    // Two DC clips at different levels: the switch must not jump more than the
    // 2 ms crossfade allows (one step of 0.75/96 per sample, plus small slack).
    let sr = 48_000.0;
    let mut p = Project::new("c");
    p.add_scene("A");
    p.add_scene("B");
    p.add_track(TrackKind::Audio, "A");
    let mut media = Media::new();
    media.insert("hi".into(), Arc::new(SampleData { channels: vec![vec![0.8; 480_000]], sample_rate: sr }));
    media.insert("lo".into(), Arc::new(SampleData { channels: vec![vec![0.05; 480_000]], sample_rate: sr }));
    for (s, key) in [(0, "hi"), (1, "lo")] {
        let id = p.alloc_id();
        p.tracks[0].slots[s].clip = Some(Clip {
            id,
            name: key.into(),
            color: [0; 3],
            content: ClipContent::Audio(AudioContent { media: MediaRef { path: key.into() }, warp: WarpMode::RePitch, source_bpm: 120.0, markers: vec![], envelopes: vec![] }),
            region: ClipRegion::looped(8.0),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
            follow: Default::default(),
        });
    }
    let g = build_graph(&p, &media, sr).unwrap();
    let (mut h, mut r, _) = create(g, 120.0, false);
    h.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    render(&mut r, 50_000);
    h.send(Command::LaunchSlot { track: 0, slot: 1, down: true });
    let l = render(&mut r, 60_000);
    let max_step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.02, "max sample step {max_step}");
}

#[test]
fn pcm16_export_round_trips_with_dither_bound() {
    let dir = std::env::temp_dir().join(format!("xdaw-x16-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.wav");
    let l: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
    let rep = xdaw_engine::export::write_wav(&path, &l, &l, 48_000, SampleFormat::Pcm16, true).unwrap();
    assert_eq!(rep.clipped_samples, 0);
    let mut reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().bits_per_sample, 16);
    let back: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    for (i, x) in l.iter().enumerate() {
        let y = back[i * 2] as f32 / i16::MAX as f32;
        // TPDF dither of ±1 LSB plus rounding: error ≤ 1.5 LSB.
        assert!((x - y).abs() <= 1.5 / 32_768.0 + 1e-6, "sample {i}: {x} vs {y}");
    }
    let clip = xdaw_engine::export::write_wav(&path, &[1.5], &[0.0], 48_000, SampleFormat::Pcm24, false).unwrap();
    assert_eq!(clip.clipped_samples, 1, "over-range samples are counted");
}

#[test]
fn decodes_wav_aiff_and_flac_identically() {
    // Fixtures: kick.wav generated by XDAW; .aif (24-bit BE) and .flac made with macOS afconvert.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let wav = xdaw_engine::media::decode_file(&dir.join("kick.wav")).unwrap();
    for f in ["kick.aif", "kick.flac"] {
        let other = xdaw_engine::media::decode_file(&dir.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(other.sample_rate, wav.sample_rate, "{f}");
        assert_eq!(other.frames(), wav.frames(), "{f}");
        let max = wav.channels[0].iter().zip(&other.channels[0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(max < 1e-6, "{f}: max diff {max}");
    }
    assert!(xdaw_engine::media::decode_file(&dir.join("missing.wav")).is_err());
    let bad = std::env::temp_dir().join(format!("xdaw-bad-{}.wav", std::process::id()));
    std::fs::write(&bad, b"RIFF....WAVEjunk").unwrap();
    assert!(xdaw_engine::media::decode_file(&bad).is_err(), "invalid file is rejected");
}

#[test]
fn stretch_clip_uses_pitch_preserving_rendering_when_available() {
    use xdaw_project::stretch_key;
    let sr = 48_000.0;
    let mut p = Project::new("s");
    p.transport.bpm = 100.0;
    p.add_scene("S");
    p.add_track(TrackKind::Audio, "A");
    let src = Arc::new(SampleData { channels: vec![(0..96_000).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.5).collect()], sample_rate: sr });
    let mut media = Media::new();
    media.insert("tone".into(), src.clone());
    let id = p.alloc_id();
    p.tracks[0].slots[0].clip = Some(Clip {
        id,
        name: "tone".into(),
        color: [0; 3],
        // Recorded at 120 BPM: 2 s = 4 beats. At 100 BPM, 4 beats last 2.4 s.
        content: ClipContent::Audio(AudioContent { media: MediaRef { path: "tone".into() }, warp: WarpMode::Stretch, source_bpm: 120.0, markers: vec![], envelopes: vec![] }),
        region: ClipRegion::looped(4.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    });
    let crossings = |x: &[f32]| x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
    let play = |media: &Media| {
        let g = build_graph(&p, media, sr).unwrap();
        let (mut h, mut r, _) = create(g, 100.0, false);
        h.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
        render(&mut r, 96_000)
    };
    // Without the rendering: Re-Pitch fallback, pitch drops by 100/120.
    let fallback = play(&media);
    let f0 = crossings(&fallback[4_800..91_200]) / 1.8;
    assert!((f0 - 440.0 * 100.0 / 120.0).abs() < 5.0, "fallback {f0} Hz");
    // With the WSOLA rendering: pitch preserved.
    media.insert(stretch_key("tone", 120.0, 100.0), Arc::new(xdaw_dsp::stretch::wsola(&src, 1.2)));
    let stretched = play(&media);
    let f1 = crossings(&stretched[4_800..91_200]) / 1.8;
    assert!((f1 - 440.0).abs() < 5.0, "stretched {f1} Hz");
}

#[test]
fn warp_markers_remap_source_time_in_repitch_and_stretch() {
    let sr = 48_000.0;
    let mut p = Project::new("w");
    p.transport.bpm = 120.0;
    p.add_scene("S");
    p.add_track(TrackKind::Audio, "A");
    // Ramp over 4 s so the playback position is readable from the value.
    let len = 192_000;
    let src = Arc::new(SampleData { channels: vec![(0..len).map(|i| i as f32 / len as f32).collect()], sample_rate: sr });
    let mut media = Media::new();
    media.insert("ramp".into(), src.clone());
    let id = p.alloc_id();
    p.tracks[0].slots[0].clip = Some(Clip {
        id,
        name: "ramp".into(),
        color: [0; 3],
        content: ClipContent::Audio(AudioContent {
            media: MediaRef { path: "ramp".into() },
            warp: WarpMode::RePitch,
            source_bpm: 120.0,
            // Beat 4 pinned to 1.0 s of source (twice the source tempo in the first bar).
            markers: vec![WarpMarker { beat: 0.0, source_seconds: 0.0 }, WarpMarker { beat: 4.0, source_seconds: 1.0 }],
            envelopes: vec![],
        }),
        region: ClipRegion::looped(8.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    });
    let g = build_graph(&p, &media, sr).unwrap();
    let (mut h, mut r, _) = create(g, 120.0, false);
    h.send(Command::LaunchSlot { track: 0, slot: 0, down: true });
    let l = render(&mut r, 96_000 + 24_000);
    // Beat 2 (sample 48000) reads source 0.5 s; beat 5 (sample 120000 - 1) reads 1.0 + 0.5 = 1.5 s.
    assert!((l[48_000] - 0.5 * sr as f32 / len as f32).abs() < 1e-3, "{}", l[48_000]);
    assert!((l[119_999] - 1.5 * sr as f32 / len as f32).abs() < 1e-3, "{}", l[119_999]);

    // Stretch rendering of the same markers keeps the segment lengths at project tempo.
    let segs = xdaw_sequencer::warp::segments(&[(0.0, 0.0), (4.0, 1.0)], 120.0, 8.0);
    let out = xdaw_engine::media::render_warped(&src, &segs, 120.0);
    assert_eq!(out.frames(), 192_000, "8 beats at 120 BPM");
}

/// A hot master chain: the Limiter preset keeps the exported peak at its ceiling,
/// the same chain without it clips past 0 dBFS.
#[test]
fn master_limiter_preset_holds_exported_peak_at_ceiling() {
    use xdaw_dsp::presets::{factory_presets, preset_values};
    let build = |limit: bool| {
        let mut p = Project::new("lim");
        p.transport.bpm = 120.0;
        p.add_scene("S");
        p.add_track(TrackKind::Midi, "M");
        p.add_device(TrackRef::Track(0), DeviceKind::PolySynth);
        let g = p.add_device(TrackRef::Track(0), DeviceKind::Gain);
        let gi = p.tracks[0].devices.iter().position(|d| d.id == g).unwrap();
        p.tracks[0].devices[gi].params = vec![24.0, 0.0, 1.0];
        if limit {
            let lid = p.add_device(TrackRef::Master, DeviceKind::Limiter);
            let preset = &factory_presets(DeviceKind::Limiter)[2]; // Safety Ceiling, -1 dB
            let li = p.master.devices.iter().position(|d| d.id == lid).unwrap();
            p.master.devices[li].params = preset_values(DeviceKind::Limiter, preset);
        }
        let id = p.alloc_id();
        let notes = (0..4).map(|k| Note { pitch: 48 + 4 * k, start: k as f64, length: 1.0, velocity: 127, chance: 100 }).collect();
        let clip = Clip {
            id,
            name: "c".into(),
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
        p
    };
    let settings = ExportSettings { start: 0.0, end: 4.0, tail_seconds: 0.2, sample_rate: 48_000, format: SampleFormat::Float32, dither: false };
    let peak = |p: &Project| {
        let (l, r, _) = render_offline(p, &Media::new(), &settings).unwrap();
        l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()))
    };
    let (dry, limited) = (peak(&build(false)), peak(&build(true)));
    let ceiling = 10f32.powf(-1.0 / 20.0);
    assert!(dry > 1.0, "chain should clip without the limiter, peak {dry}");
    assert!(limited <= ceiling + 1e-4 && limited > ceiling * 0.9, "limited peak {limited} vs ceiling {ceiling}");
}

fn arr_rig(dir: &str) -> (xdaw_engine::EngineHandle, xdaw_engine::Renderer, Recorder, rtrb::Producer<f32>) {
    let mut p = Project::new("a");
    p.add_scene("S");
    p.add_track(TrackKind::Audio, "A");
    p.tracks[0].mixer.arm = true;
    let g = build_graph(&p, &Media::new(), 48_000.0).unwrap();
    let (h, mut r, _) = create(g, 120.0, false);
    r.include_metronome = false;
    let dir = std::env::temp_dir().join(format!("xdaw-{dir}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (recorder, rec_tx) = Recorder::spawn(48_000, dir);
    r.attach_recorder(rec_tx);
    let (in_tx, in_rx) = rtrb::RingBuffer::new(48_000 * 20);
    r.attach_input(InputTap { consumer: in_rx, channels: 1 });
    (h, r, recorder, in_tx)
}

/// Feeds `frames` of a counter-valued input and renders it (256-frame blocks).
fn feed(r: &mut xdaw_engine::Renderer, tx: &mut rtrb::Producer<f32>, frames: usize) {
    for _ in 0..frames / 256 {
        for _ in 0..256 {
            tx.push(0.25).unwrap();
        }
        render(r, 256);
    }
}

#[test]
fn arrangement_record_honours_the_punch_window() {
    let (mut h, mut r, rec, mut tx) = arr_rig("punch");
    // 120 BPM: beat 2 = 1 s. Punch 2..4 beats = frames 48000..96000.
    h.send(Command::ArrRecord { track: 0, on: true, from: 2.0, to: 4.0 });
    h.send(Command::Play);
    feed(&mut r, &mut tx, 48_000 * 3);
    let take = rec.finished.recv_timeout(Duration::from_secs(5)).expect("take closed at punch-out");
    assert!(take.error.is_none() && take.slot == ARR_SLOT);
    assert!((take.frames as i64 - 48_000).abs() <= 256, "{} frames", take.frames);
    let mut events = vec![];
    h.poll_events(|e| events.push(e));
    let start = events.iter().find_map(|e| match e {
        EngineEvent::RecordingStarted { slot, at, .. } if *slot == ARR_SLOT => Some(*at),
        _ => None,
    });
    assert!(start.is_some_and(|b| (b - 2.0).abs() < 0.01), "start {start:?}");
    assert!(events.iter().any(|e| matches!(e, EngineEvent::RecordingStopped { slot, at, .. } if *slot == ARR_SLOT && (*at - 4.0).abs() < 0.01)));
}

#[test]
fn arrangement_record_makes_one_take_per_loop_pass_and_stops_with_transport() {
    let (mut h, mut r, rec, mut tx) = arr_rig("loop");
    h.send(Command::SetLoop { enabled: true, start: 0.0, length: 2.0 }); // 1 s loop
    h.send(Command::ArrRecord { track: 0, on: true, from: f64::NEG_INFINITY, to: f64::INFINITY });
    h.send(Command::Play);
    feed(&mut r, &mut tx, 48_000 * 2 + 24_000); // two full passes + half a pass
    h.send(Command::Stop);
    render(&mut r, 256);
    let takes: Vec<_> = (0..3).filter_map(|_| rec.finished.recv_timeout(Duration::from_secs(2)).ok()).collect();
    assert_eq!(takes.len(), 3, "two full passes and the partial one");
    assert!((takes[0].frames as i64 - 48_000).abs() <= 256 && (takes[1].frames as i64 - 48_000).abs() <= 256);
    assert!(takes[2].frames > 20_000 && takes[2].frames < 26_000, "{}", takes[2].frames);
}
