use crate::io::{self, ProjectError};
use crate::model::*;
use crate::History;
use ost_dsp::DeviceKind;
use ost_sequencer::ClipRegion;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ost-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn midi_clip(p: &mut Project) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: "Chords".into(),
        color: [1, 2, 3],
        content: ClipContent::Midi { notes: vec![Note { pitch: 60, start: 0.0, length: 1.0, velocity: 100 }] },
        region: ClipRegion::looped(4.0),
        launch_mode: Default::default(),
        quantize: Default::default(),
        legato: false,
        gain_db: 0.0,
        transpose: 0,
    }
}

fn sample_project() -> Project {
    let mut p = Project::new("Test");
    p.add_scene("A");
    p.add_scene("B");
    let t = p.add_track(TrackKind::Midi, "Keys");
    p.add_track(TrackKind::Audio, "Drums");
    p.add_return("Reverb");
    p.add_device(TrackRef::Track(t), DeviceKind::PolySynth);
    p.add_device(TrackRef::Track(t), DeviceKind::Delay);
    let c = midi_clip(&mut p);
    p.tracks[t].slots[1].clip = Some(c);
    p.tracks[t].lane_mut(ParamTarget::Volume).insert(AutomationPoint { beat: 0.0, value: -6.0 });
    p.tracks[t].lane_mut(ParamTarget::Volume).insert(AutomationPoint { beat: 4.0, value: 0.0 });
    p
}

#[test]
fn round_trip_preserves_everything() {
    let p = sample_project();
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    let dir = temp_dir("roundtrip");
    let path = dir.join("song.ostp");
    io::save(&p, &path).unwrap();
    let back = io::load(&path).unwrap();
    assert_eq!(p, back);
    assert!(!dir.read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains(".tmp-")));
}

#[test]
fn invalid_and_future_files_are_rejected_with_reasons() {
    let dir = temp_dir("invalid");
    let path = dir.join("bad.ostp");
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(matches!(io::load(&path), Err(ProjectError::Parse { .. })));
    let mut v = serde_json::to_value(sample_project()).unwrap();
    v["version"] = 99.into();
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(matches!(io::load(&path), Err(ProjectError::Unsupported { version: 99, .. })));
    assert!(matches!(io::load(&dir.join("absent.ostp")), Err(ProjectError::Read { .. })));
}

#[test]
fn undo_redo_track_and_scene_edits() {
    let mut p = sample_project();
    let original = p.clone();
    let mut h = History::default();
    h.edit_track(&mut p, TrackRef::Track(0), "Mute", |t| t.mixer.mute = true);
    let before = crate::history::scenes_snapshot(&p);
    p.insert_scene(1, "New");
    let after = crate::history::scenes_snapshot(&p);
    h.record("Insert Scene", crate::Edit::Scenes { before, after });
    assert_eq!(p.scenes.len(), 3);
    assert_eq!(h.undo(&mut p).as_deref(), Some("Insert Scene"));
    assert_eq!(p.scenes.len(), 2);
    assert_eq!(p.tracks[0].slots.len(), 2);
    h.undo(&mut p);
    assert_eq!(p.tracks, original.tracks);
    h.redo(&mut p);
    assert!(p.tracks[0].mixer.mute);
    h.redo(&mut p);
    assert_eq!(p.scenes.len(), 3);
    assert!(p.validate().is_empty());
}

#[test]
fn automation_interpolates_and_holds() {
    let p = sample_project();
    let lane = p.tracks[0].lane(ParamTarget::Volume).unwrap();
    assert_eq!(lane.value_at(-1.0), Some(-6.0));
    assert_eq!(lane.value_at(2.0), Some(-3.0));
    assert_eq!(lane.value_at(10.0), Some(0.0));
}

#[test]
fn clearing_a_range_splits_straddling_clips() {
    let mut p = sample_project();
    let clip = midi_clip(&mut p);
    let t = &mut p.tracks[0];
    t.arrangement.push(ArrangementClip { id: 100, start: 0.0, length: 8.0, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
    let mut next = 200;
    t.clear_arrangement_range(2.0, 4.0, || {
        next += 1;
        next
    });
    let spans: Vec<_> = t.arrangement.iter().map(|c| (c.start, c.length, c.offset)).collect();
    assert_eq!(spans, vec![(0.0, 2.0, 0.0), (4.0, 4.0, 4.0)]);
}

#[test]
fn missing_media_reported_and_collect_copies_into_project() {
    let dir = temp_dir("collect");
    let outside = temp_dir("collect-src");
    let wav = outside.join("kick.wav");
    std::fs::write(&wav, b"RIFF").unwrap();
    let mut p = sample_project();
    let id = p.add_device(TrackRef::Track(0), DeviceKind::Sampler);
    let _ = id;
    p.tracks[0].devices[0].sample = Some(MediaRef { path: wav.to_string_lossy().into() });
    p.tracks[1].devices.clear();
    let mut missing_clip = midi_clip(&mut p);
    missing_clip.content = ClipContent::Audio(AudioContent {
        media: MediaRef { path: "Samples/missing.wav".into() },
        warp: WarpMode::RePitch,
        source_bpm: 120.0,
        markers: vec![],
    });
    p.tracks[1].slots[0].clip = Some(missing_clip);
    let missing = io::missing_media(&p, Some(&dir));
    assert_eq!(missing, vec![MediaRef { path: "Samples/missing.wav".into() }]);
    assert_eq!(io::collect_media(&mut p, &dir).unwrap(), 1);
    assert_eq!(p.tracks[0].devices[0].sample.as_ref().unwrap().path, "Samples/kick.wav");
    assert!(dir.join("Samples/kick.wav").is_file());
}

#[test]
fn autosave_newer_than_project_is_offered_for_recovery() {
    let dir = temp_dir("autosave");
    let path = dir.join("song.ostp");
    let p = sample_project();
    io::save(&p, &path).unwrap();
    assert_eq!(io::recovery_candidate(Some(&path)), None);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut edited = p.clone();
    edited.transport.bpm = 99.0;
    let auto = io::write_autosave(&edited, Some(&path)).unwrap();
    assert_eq!(io::recovery_candidate(Some(&path)), Some(auto.clone()));
    assert_eq!(io::load(&auto).unwrap().transport.bpm, 99.0);
    io::discard_autosave(Some(&path));
    assert_eq!(io::recovery_candidate(Some(&path)), None);
}
