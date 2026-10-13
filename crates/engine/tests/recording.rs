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
        content: ClipContent::Midi { notes: vec![Note { pitch: 60, start: 0.0, length: 0.5, velocity: 100 }, Note { pitch: 67, start: 1.0, length: 1.0, velocity: 80 }] },
        region: ClipRegion::looped(4.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
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
            content: ClipContent::Audio(AudioContent { media: MediaRef { path: key.into() }, warp: WarpMode::RePitch, source_bpm: 120.0, markers: vec![] }),
            region: ClipRegion::looped(8.0),
            launch_mode: Default::default(),
            quantize: Default::default(),
            legato: false,
            gain_db: 0.0,
            transpose: 0,
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
