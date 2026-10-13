//! Original demo content: every sample is synthesized here with XDAW's own DSP,
//! so the demo session ships no third-party audio.

use crate::export::{write_wav, SampleFormat};
use std::path::Path;
use xdaw_dsp::instruments::Synth;
use xdaw_dsp::util::Rng;
use xdaw_dsp::{Ctx, DeviceKind, NoteEvent, NoteKind};
use xdaw_project::*;
use xdaw_sequencer::{ClipQuantize, ClipRegion, LaunchMode, Quantize};

pub const BPM: f64 = 122.0;
pub const SR: u32 = 48_000;

fn secs_per_beat() -> f64 {
    60.0 / BPM
}

fn frames_for_beats(beats: f64) -> usize {
    (beats * secs_per_beat() * SR as f64).round() as usize
}

fn decay(t: f32, tau: f32) -> f32 {
    (-t / tau).exp()
}

/// One-shot drum voices.
pub fn kick() -> Vec<f32> {
    let n = (0.5 * SR as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 45.0 + 110.0 * decay(t, 0.035);
            phase += f / SR as f32;
            let body = (phase * std::f32::consts::TAU).sin() * decay(t, 0.28);
            let click = if i < 48 { (1.0 - i as f32 / 48.0) * 0.4 } else { 0.0 };
            (body + click).tanh() * 0.9
        })
        .collect()
}

fn noise_hit(seed: u32, len_s: f32, tau: f32, hp: f32, tone: Option<(f32, f32)>) -> Vec<f32> {
    let n = (len_s * SR as f32) as usize;
    let mut rng = Rng::new(seed);
    let mut prev_in = 0.0f32;
    let mut prev_out = 0.0f32;
    let a = 1.0 / (1.0 + std::f32::consts::TAU * hp / SR as f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let x = rng.next_bipolar();
            // One-pole high-pass.
            let y = a * (prev_out + x - prev_in);
            prev_in = x;
            prev_out = y;
            let mut s = y * decay(t, tau);
            if let Some((f, tt)) = tone {
                s += (t * f * std::f32::consts::TAU).sin() * decay(t, tt) * 0.6;
            }
            s * 0.7
        })
        .collect()
}

pub fn snare() -> Vec<f32> {
    noise_hit(11, 0.35, 0.09, 1200.0, Some((185.0, 0.06)))
}

pub fn clap() -> Vec<f32> {
    let mut v = noise_hit(23, 0.4, 0.12, 900.0, None);
    // Three quick bursts before the tail.
    for (i, x) in v.iter_mut().enumerate().take((0.03 * SR as f32) as usize) {
        let ms = i as f32 / SR as f32 * 1000.0;
        *x *= if (ms % 10.0) < 4.0 { 1.4 } else { 0.3 };
    }
    v
}

pub fn hat(open: bool) -> Vec<f32> {
    noise_hit(if open { 37 } else { 41 }, if open { 0.45 } else { 0.08 }, if open { 0.16 } else { 0.022 }, 7000.0, None)
}

pub fn tom() -> Vec<f32> {
    let n = (0.4 * SR as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            phase += (110.0 + 90.0 * decay(t, 0.05)) / SR as f32;
            (phase * std::f32::consts::TAU).sin() * decay(t, 0.16) * 0.8
        })
        .collect()
}

pub fn rim() -> Vec<f32> {
    let n = (0.08 * SR as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            ((t * 1750.0 * std::f32::consts::TAU).sin() + 0.5 * (t * 520.0 * std::f32::consts::TAU).sin()) * decay(t, 0.012) * 0.7
        })
        .collect()
}

pub fn perc() -> Vec<f32> {
    let n = (0.25 * SR as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let sq = |f: f32| if (t * f).fract() < 0.5 { 1.0 } else { -1.0 };
            (sq(587.0) + sq(845.0)) * 0.25 * decay(t, 0.05)
        })
        .collect()
}

/// Inharmonic bell at C3 (261.63 Hz) for the sampler.
pub fn bell() -> Vec<f32> {
    let n = (2.5 * SR as f32) as usize;
    let f0 = 261.63f32;
    let partials = [(1.0, 1.0, 1.6), (2.76, 0.5, 0.9), (5.4, 0.25, 0.5), (8.93, 0.12, 0.3)];
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let attack = (t / 0.002).min(1.0);
            partials.iter().map(|&(r, a, tau)| (t * f0 * r * std::f32::consts::TAU).sin() * a * decay(t, tau)).sum::<f32>() * 0.35 * attack
        })
        .collect()
}

/// Mixes one-shots into a loop buffer at beat positions.
fn place(buf: &mut [f32], hit: &[f32], beat: f64, gain: f32) {
    let start = frames_for_beats(beat);
    for (i, &x) in hit.iter().enumerate() {
        if let Some(y) = buf.get_mut(start + i) {
            *y += x * gain;
        }
    }
}

pub fn drum_loop(variation: bool) -> Vec<f32> {
    let beats = 8.0;
    let mut buf = vec![0.0f32; frames_for_beats(beats)];
    let (k, s, h, o, c) = (kick(), snare(), hat(false), hat(true), clap());
    for b in 0..8 {
        let b = b as f64;
        if !variation || b as i32 % 4 != 3 {
            place(&mut buf, &k, b, 0.9);
        }
        place(&mut buf, &h, b + 0.5, 0.45);
        if b as i32 % 2 == 1 {
            place(&mut buf, if variation { &s } else { &c }, b, 0.7);
        }
    }
    if variation {
        for b in [2.75, 3.5, 6.75, 7.25] {
            place(&mut buf, &k, b, 0.7);
        }
        for b in [1.75, 5.75] {
            place(&mut buf, &o, b, 0.3);
        }
    } else {
        place(&mut buf, &o, 7.5, 0.35);
    }
    buf
}

/// Renders notes (beats) through an XDAW Poly synth configured by `setup`.
fn render_synth(notes: &[(f64, f64, u8, u8)], beats: f64, setup: impl Fn(&mut Synth)) -> Vec<f32> {
    let total = frames_for_beats(beats);
    let tail = (0.5 * SR as f64) as usize;
    let mut synth = Synth::new(SR as f32);
    setup(&mut synth);
    let mut timed: Vec<(usize, NoteKind)> = vec![];
    for &(start, len, pitch, vel) in notes {
        timed.push((frames_for_beats(start), NoteKind::On { pitch, velocity: vel }));
        timed.push((frames_for_beats(start + len), NoteKind::Off { pitch }));
    }
    timed.sort_by_key(|(f, k)| (*f, matches!(k, NoteKind::On { .. })));
    let ctx = Ctx { sample_rate: SR as f32, bpm: BPM as f32 };
    let mut l = vec![0.0f32; total + tail];
    let mut r = vec![0.0f32; total + tail];
    let block = 256;
    let mut ev = 0;
    let mut pos = 0;
    while pos < total + tail {
        let n = block.min(total + tail - pos);
        let mut events = vec![];
        while ev < timed.len() && timed[ev].0 < pos + n {
            events.push(NoteEvent { offset: (timed[ev].0 - pos) as u32, kind: timed[ev].1 });
            ev += 1;
        }
        synth.render(&events, &mut l[pos..pos + n], &mut r[pos..pos + n], &ctx);
        pos += n;
    }
    // Fold the release tail back onto the loop start so the loop is seamless.
    let mut out: Vec<f32> = l[..total].to_vec();
    for i in 0..tail {
        out[i % total] += l[total + i];
    }
    out
}

const PROGRESSION: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]]; // Am F C G

pub fn bass_loop() -> Vec<f32> {
    let roots = [33u8, 29, 36, 31];
    let mut notes = vec![];
    for (bar, &root) in roots.iter().enumerate() {
        for step in 0..8 {
            let start = bar as f64 * 4.0 + step as f64 * 0.5;
            let pitch = if step == 3 || step == 7 { root + 12 } else { root };
            notes.push((start, 0.4, pitch, if step % 2 == 0 { 120 } else { 90 }));
        }
    }
    render_synth(&notes, 16.0, |s| {
        s.set_param(0, 0.0);
        s.set_param(1, 1.0);
        s.set_param(2, -12.0);
        s.set_param(5, 380.0);
        s.set_param(6, 0.35);
        s.set_param(7, 2.5);
        s.set_param(13, 120.0);
        s.set_param(14, 0.0);
        s.set_param(11, 60.0);
        s.set_param(17, -4.0);
    })
}

pub fn chords_loop() -> Vec<f32> {
    let mut notes = vec![];
    for (bar, chord) in PROGRESSION.iter().enumerate() {
        for hit in [0.0, 1.5, 2.5, 3.5] {
            for &p in chord {
                notes.push((bar as f64 * 4.0 + hit, 0.35, p + 12, 100));
            }
        }
    }
    render_synth(&notes, 16.0, |s| {
        s.set_param(3, 9.0);
        s.set_param(5, 1800.0);
        s.set_param(7, 3.0);
        s.set_param(13, 180.0);
        s.set_param(14, 0.1);
        s.set_param(11, 200.0);
        s.set_param(17, -9.0);
    })
}

pub fn texture_loop() -> Vec<f32> {
    let total = frames_for_beats(16.0);
    let mut rng = Rng::new(99);
    let mut lp = 0.0f32;
    (0..total)
        .map(|i| {
            let t = i as f32 / total as f32;
            let swell = (t * std::f32::consts::PI).sin().powi(2);
            let cutoff = 0.01 + 0.08 * swell;
            lp += cutoff * (rng.next_bipolar() - lp);
            let secs = i as f32 / SR as f32;
            let drone = (secs * 110.0 * std::f32::consts::TAU).sin() * 0.15 + (secs * 164.8 * std::f32::consts::TAU).sin() * 0.1;
            (lp * 2.5 + drone) * (0.3 + 0.7 * swell) * 0.6
        })
        .collect()
}

fn write_mono(path: &Path, data: &[f32]) -> Result<(), String> {
    write_wav(path, data, data, SR, SampleFormat::Pcm24, true).map(|_| ()).map_err(|e| e.to_string())
}

fn clip(p: &mut Project, name: &str, color: [u8; 3], content: ClipContent, beats: f64) -> Clip {
    Clip {
        id: p.alloc_id(),
        name: name.into(),
        color,
        content,
        region: ClipRegion::looped(beats),
        launch_mode: LaunchMode::Trigger,
        quantize: ClipQuantize::Global,
        legato: false,
        gain_db: 0.0,
        transpose: 0,
        follow: Default::default(),
    }
}

fn audio(path: &str) -> ClipContent {
    ClipContent::Audio(AudioContent { media: MediaRef { path: path.into() }, warp: WarpMode::RePitch, source_bpm: BPM, markers: vec![], envelopes: vec![] })
}

fn notes(v: &[(f64, f64, u8, u8)]) -> ClipContent {
    ClipContent::Midi { notes: v.iter().map(|&(start, length, pitch, velocity)| Note { pitch, start, length, velocity, chance: 100 }).collect() }
}

fn beat_pattern(fill: bool) -> Vec<(f64, f64, u8, u8)> {
    // Pads: 36 kick, 37 snare, 38 clap, 39 closed hat, 40 open hat, 41 tom, 42 rim, 43 perc.
    let mut v = vec![];
    for b in 0..4 {
        let b = b as f64;
        v.push((b, 0.25, 36, 120));
        v.push((b + 0.5, 0.25, 39, 80));
        v.push((b + 0.75, 0.25, 39, 50));
    }
    v.push((1.0, 0.25, 37, 110));
    v.push((3.0, 0.25, 37, 110));
    v.push((2.25, 0.25, 42, 70));
    if fill {
        for (i, b) in [3.0, 3.25, 3.5, 3.75].iter().enumerate() {
            v.push((*b, 0.25, 41, 90 + i as u8 * 8));
        }
        v.push((3.5, 0.5, 40, 70));
    }
    v
}

/// Writes the demo samples into `<dir>/Samples` and returns the demo project.
pub fn build_demo(dir: &Path) -> Result<Project, String> {
    let samples = dir.join("Samples");
    std::fs::create_dir_all(&samples).map_err(|e| e.to_string())?;
    let files: Vec<(&str, Vec<f32>)> = vec![
        ("drums_a.wav", drum_loop(false)),
        ("drums_b.wav", drum_loop(true)),
        ("bass.wav", bass_loop()),
        ("chords.wav", chords_loop()),
        ("texture.wav", texture_loop()),
        ("kick.wav", kick()),
        ("snare.wav", snare()),
        ("clap.wav", clap()),
        ("hat_closed.wav", hat(false)),
        ("hat_open.wav", hat(true)),
        ("tom.wav", tom()),
        ("rim.wav", rim()),
        ("perc.wav", perc()),
        ("bell.wav", bell()),
    ];
    for (name, data) in &files {
        write_mono(&samples.join(name), data)?;
    }

    let mut p = Project::new("XDAW Demo");
    p.transport.bpm = BPM;
    p.transport.global_quantize = Quantize::Bar1;
    for name in ["Intro", "Groove", "Lift", "Drop", "Outro"] {
        p.add_scene(name);
    }
    let names = ["Drums", "Bass", "Chords", "Texture", "Beat", "Lead", "Bells", "Pad"];
    for (i, n) in names.iter().enumerate() {
        p.add_track(if i < 4 { TrackKind::Audio } else { TrackKind::Midi }, n);
    }
    p.add_return("A Reverb");
    p.add_return("B Delay");
    p.add_device(TrackRef::Return(0), DeviceKind::Reverb);
    p.add_device(TrackRef::Return(1), DeviceKind::Delay);
    // Returns are fully wet.
    p.returns[0].devices[0].params[4] = 1.0;
    p.returns[1].devices[0].params[6] = 1.0;
    p.add_device(TrackRef::Master, DeviceKind::Compressor);
    p.master.devices[0].params = vec![-10.0, 2.0, 15.0, 150.0, 6.0, 1.0];

    // Instruments.
    let drum = p.add_device(TrackRef::Track(4), DeviceKind::DrumRack);
    let pads = ["kick.wav", "snare.wav", "clap.wav", "hat_closed.wav", "hat_open.wav", "tom.wav", "rim.wav", "perc.wav"];
    let rack = p.tracks[4].devices.iter_mut().find(|d| d.id == drum).unwrap();
    for (i, f) in pads.iter().enumerate() {
        rack.pads[i] = Some(MediaRef { path: format!("Samples/{f}") });
    }
    rack.preset_name = Some("Synthetic Kit".into());
    p.add_device(TrackRef::Track(5), DeviceKind::PolySynth);
    p.tracks[5].devices[0].preset_name = Some("Bright Lead".into());
    p.tracks[5].devices[0].params[0] = 1.0; // square
    p.tracks[5].devices[0].params[5] = 2500.0;
    p.tracks[5].devices[0].params[17] = -14.0;
    p.add_device(TrackRef::Track(5), DeviceKind::Filter);
    let samp = p.add_device(TrackRef::Track(6), DeviceKind::Sampler);
    let s = p.tracks[6].devices.iter_mut().find(|d| d.id == samp).unwrap();
    s.sample = Some(MediaRef { path: "Samples/bell.wav".into() });
    s.params[0] = 60.0;
    s.params[6] = 800.0;
    s.preset_name = Some("Glass Bell".into());
    p.add_device(TrackRef::Track(7), DeviceKind::PolySynth);
    let pad = &mut p.tracks[7].devices[0];
    pad.preset_name = Some("Slow Pad".into());
    pad.params[0] = 0.0;
    pad.params[1] = 0.0;
    pad.params[3] = 12.0;
    pad.params[5] = 900.0;
    pad.params[7] = 1.0;
    pad.params[8] = 900.0;
    pad.params[11] = 1500.0;
    pad.params[17] = -16.0;
    p.add_device(TrackRef::Track(7), DeviceKind::Eq3);

    // Mixer.
    let vols = [-8.0, -9.0, -12.0, -14.0, -9.0, -12.0, -12.0, -10.0];
    p.master.mixer.volume_db = -2.0;
    for (t, v) in vols.iter().enumerate() {
        p.tracks[t].mixer.volume_db = *v;
    }
    p.tracks[2].mixer.sends_db[0] = -12.0;
    p.tracks[5].mixer.sends_db[1] = -10.0;
    p.tracks[6].mixer.sends_db[0] = -8.0;
    p.tracks[6].mixer.sends_db[1] = -14.0;
    p.tracks[7].mixer.sends_db[0] = -6.0;
    p.tracks[1].mixer.pan = -0.1;
    p.tracks[6].mixer.pan = 0.3;

    // Session clips per scene.
    let c = |p: &mut Project, name: &str, track: usize, content: ClipContent, beats: f64| {
        let color = p.tracks[track].color;
        clip(p, name, color, content, beats)
    };
    let drums_a = c(&mut p, "Drums A", 0, audio("Samples/drums_a.wav"), 8.0);
    let drums_b = c(&mut p, "Drums B", 0, audio("Samples/drums_b.wav"), 8.0);
    let bass = c(&mut p, "Bass", 1, audio("Samples/bass.wav"), 16.0);
    let chords = c(&mut p, "Chords", 2, audio("Samples/chords.wav"), 16.0);
    let texture = c(&mut p, "Texture", 3, audio("Samples/texture.wav"), 16.0);
    let beat = c(&mut p, "Beat", 4, notes(&beat_pattern(false)), 4.0);
    let beat_fill = c(&mut p, "Beat Fill", 4, notes(&beat_pattern(true)), 4.0);
    let lead_notes: Vec<_> = [(0.0, 69), (0.75, 72), (1.5, 76), (2.5, 74), (4.0, 72), (4.75, 69), (5.5, 67), (6.5, 69)]
        .iter()
        .map(|&(s, p)| (s, 0.6, p as u8, 100))
        .collect();
    let lead = c(&mut p, "Lead", 5, notes(&lead_notes), 8.0);
    let bells = c(&mut p, "Bells", 6, notes(&[(0.0, 1.0, 76, 90), (1.5, 1.0, 79, 80), (3.0, 1.0, 72, 85), (6.0, 1.5, 74, 80)]), 8.0);
    let pad_notes: Vec<_> = PROGRESSION
        .iter()
        .enumerate()
        .flat_map(|(bar, ch)| ch.iter().map(move |&pitch| (bar as f64 * 4.0, 3.9, pitch, 80)))
        .collect();
    let pad_clip = c(&mut p, "Pad", 7, notes(&pad_notes), 16.0);

    let grid: [[Option<&Clip>; 8]; 5] = [
        [None, None, None, Some(&texture), None, None, None, Some(&pad_clip)],
        [Some(&drums_a), Some(&bass), None, None, Some(&beat), None, None, Some(&pad_clip)],
        [Some(&drums_a), Some(&bass), Some(&chords), None, Some(&beat), Some(&lead), Some(&bells), None],
        [Some(&drums_b), Some(&bass), Some(&chords), Some(&texture), Some(&beat_fill), Some(&lead), Some(&bells), Some(&pad_clip)],
        [None, None, Some(&chords), Some(&texture), None, None, Some(&bells), Some(&pad_clip)],
    ];
    let mut assignments = vec![];
    for (scene, row) in grid.iter().enumerate() {
        for (track, cell) in row.iter().enumerate() {
            if let Some(clip) = cell {
                assignments.push((scene, track, (*clip).clone()));
            }
        }
    }
    for (scene, track, mut clip) in assignments {
        clip.id = p.alloc_id();
        p.tracks[track].slots[scene].clip = Some(clip);
    }
    for (i, color) in [[90, 110, 150], [120, 160, 90], [190, 150, 70], [200, 90, 80], [110, 110, 110]].iter().enumerate() {
        p.scenes[i].color = *color;
    }

    // A short Arrangement (16 bars) with automation, so Arrangement playback,
    // export and automation are demonstrable immediately.
    let arr = |p: &mut Project, track: usize, slot: usize, start: f64, length: f64| {
        let clip = p.tracks[track].slots[slot].clip.clone().unwrap();
        let id = p.alloc_id();
        p.tracks[track].arrangement.push(ArrangementClip { id, start, length, offset: 0.0, fade_in: 0.0, fade_out: 0.0, clip });
    };
    arr(&mut p, 7, 0, 0.0, 64.0);
    arr(&mut p, 3, 0, 0.0, 16.0);
    arr(&mut p, 0, 1, 16.0, 32.0);
    arr(&mut p, 1, 1, 16.0, 48.0);
    arr(&mut p, 4, 1, 16.0, 32.0);
    arr(&mut p, 2, 2, 32.0, 32.0);
    arr(&mut p, 5, 2, 32.0, 32.0);
    arr(&mut p, 6, 2, 48.0, 16.0);
    arr(&mut p, 0, 3, 48.0, 16.0);
    arr(&mut p, 4, 3, 48.0, 16.0);
    if let Some(last) = p.tracks[7].arrangement.last_mut() {
        last.fade_out = 4.0;
    }
    let pad_vol = p.tracks[7].lane_mut(ParamTarget::Volume);
    pad_vol.insert(AutomationPoint { beat: 0.0, value: -30.0 });
    pad_vol.insert(AutomationPoint { beat: 16.0, value: -6.0 });
    let filter_id = p.tracks[5].devices[1].id;
    let cutoff = p.tracks[5].lane_mut(ParamTarget::Device { device: filter_id, param: 1 });
    cutoff.insert(AutomationPoint { beat: 32.0, value: 400.0 });
    cutoff.insert(AutomationPoint { beat: 48.0, value: 12_000.0 });
    p.transport.loop_start = 16.0;
    p.transport.loop_length = 16.0;
    Ok(p)
}
