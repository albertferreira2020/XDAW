//! MIDI effects (spec: docs/specs/devices.md D4). They transform the note
//! events of a block; every note-on they emit gets a matching note-off.

use crate::device::{push_all, Processor};
use crate::params::{Ctx, NoteEvent, NoteKind, ParamDesc};
use crate::util::Rng;

#[inline]
fn push(out: &mut Vec<NoteEvent>, offset: u32, kind: NoteKind) {
    if out.len() < out.capacity() {
        out.push(NoteEvent { offset, kind });
    }
}

/// Most output notes one input note can produce (Chord Stack: itself + 6).
const FAN: usize = 7;

/// Which output pitches each input pitch is holding, with reference counts so
/// overlapping outputs are merged: an output starts on its first reference and
/// stops when the last one is released. Parameter changes between note-on and
/// note-off therefore never leave notes hanging.
pub struct NoteMap {
    outputs: [[u8; FAN]; 128],
    len: [u8; 128],
    refs: [u8; 128],
}

impl Default for NoteMap {
    fn default() -> Self {
        Self { outputs: [[0; FAN]; 128], len: [0; 128], refs: [0; 128] }
    }
}

impl NoteMap {
    pub fn on(&mut self, out: &mut Vec<NoteEvent>, offset: u32, input: u8, pitches: &[u8], velocities: &[u8]) {
        self.off(out, offset, input);
        let i = input as usize & 127;
        let mut n = 0;
        for (k, &p) in pitches.iter().enumerate().take(FAN) {
            let p = p & 127;
            if self.outputs[i][..n].contains(&p) {
                continue;
            }
            self.outputs[i][n] = p;
            n += 1;
            if self.refs[p as usize] == 0 {
                push(out, offset, NoteKind::On { pitch: p, velocity: velocities[k.min(velocities.len() - 1)].max(1) });
            }
            self.refs[p as usize] = self.refs[p as usize].saturating_add(1);
        }
        self.len[i] = n as u8;
    }

    pub fn off(&mut self, out: &mut Vec<NoteEvent>, offset: u32, input: u8) {
        let i = input as usize & 127;
        for k in 0..self.len[i] as usize {
            let p = self.outputs[i][k] as usize;
            self.refs[p] = self.refs[p].saturating_sub(1);
            if self.refs[p] == 0 {
                push(out, offset, NoteKind::Off { pitch: p as u8 });
            }
        }
        self.len[i] = 0;
    }

    pub fn clear(&mut self) {
        self.len = [0; 128];
        self.refs = [0; 128];
    }

    pub fn sounding(&self) -> usize {
        self.refs.iter().filter(|&&r| r > 0).count()
    }
}

/// Runs a per-note mapping over `input`: `map(pitch, velocity, outs)` fills the
/// output pitches/velocities and returns how many there are.
fn run_mapped(map_state: &mut NoteMap, input: &[NoteEvent], out: &mut Vec<NoteEvent>, mut map: impl FnMut(u8, u8, &mut [u8; FAN], &mut [u8; FAN]) -> usize) {
    for e in input {
        match e.kind {
            NoteKind::On { pitch, velocity } if velocity > 0 => {
                let (mut ps, mut vs) = ([0u8; FAN], [0u8; FAN]);
                let n = map(pitch, velocity, &mut ps, &mut vs);
                map_state.on(out, e.offset, pitch, &ps[..n], &vs[..n.max(1)]);
            }
            NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => map_state.off(out, e.offset, pitch),
            NoteKind::AllOff => {
                map_state.clear();
                push(out, e.offset, NoteKind::AllOff);
            }
            NoteKind::Sustain(_) => push(out, e.offset, e.kind),
        }
    }
}

macro_rules! params_impl {
    ($ty:ident, $table:ident, $n:literal) => {
        impl $ty {
            pub fn param_values(&self) -> &[f32] {
                &self.params
            }
            pub fn set_param(&mut self, i: usize, v: f32) {
                if let Some(d) = $table.get(i) {
                    self.params[i] = d.clamp(v);
                }
            }
            fn defaults() -> [f32; $n] {
                std::array::from_fn(|i| $table[i].default)
            }
        }
    };
}

macro_rules! midi_processor {
    ($ty:ident) => {
        impl Processor for $ty {
            fn set_param(&mut self, i: usize, v: f32) {
                $ty::set_param(self, i, v)
            }
            fn param_values(&self) -> &[f32] {
                &self.params
            }
            fn process_midi(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, n: usize, ctx: &Ctx) {
                self.run(input, out, n, ctx)
            }
        }
    };
}

// ---------------------------------------------------------------- Chord Stack

pub const CHORD_PARAMS: &[ParamDesc] = &[
    ParamDesc::int("shift1", "Shift 1", -24.0, 24.0, 4.0, "st"),
    ParamDesc::int("shift2", "Shift 2", -24.0, 24.0, 7.0, "st"),
    ParamDesc::int("shift3", "Shift 3", -24.0, 24.0, 0.0, "st"),
    ParamDesc::int("shift4", "Shift 4", -24.0, 24.0, 0.0, "st"),
    ParamDesc::int("shift5", "Shift 5", -24.0, 24.0, 0.0, "st"),
    ParamDesc::int("shift6", "Shift 6", -24.0, 24.0, 0.0, "st"),
    ParamDesc::lin("vel1", "Velocity 1", 0.0, 200.0, 100.0, "%"),
    ParamDesc::lin("vel2", "Velocity 2", 0.0, 200.0, 100.0, "%"),
    ParamDesc::lin("vel3", "Velocity 3", 0.0, 200.0, 100.0, "%"),
    ParamDesc::lin("vel4", "Velocity 4", 0.0, 200.0, 100.0, "%"),
    ParamDesc::lin("vel5", "Velocity 5", 0.0, 200.0, 100.0, "%"),
    ParamDesc::lin("vel6", "Velocity 6", 0.0, 200.0, 100.0, "%"),
];

pub struct ChordStack {
    params: [f32; 12],
    map: NoteMap,
}

params_impl!(ChordStack, CHORD_PARAMS, 12);

impl ChordStack {
    pub fn new() -> Self {
        Self { params: Self::defaults(), map: NoteMap::default() }
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, _n: usize, _ctx: &Ctx) {
        let p = self.params;
        run_mapped(&mut self.map, input, out, |pitch, vel, ps, vs| {
            ps[0] = pitch;
            vs[0] = vel;
            let mut n = 1;
            for k in 0..6 {
                let shift = p[k] as i32;
                let q = pitch as i32 + shift;
                if shift != 0 && (0..128).contains(&q) {
                    ps[n] = q as u8;
                    vs[n] = (vel as f32 * p[6 + k] / 100.0).round().clamp(1.0, 127.0) as u8;
                    n += 1;
                }
            }
            n
        });
    }
}

midi_processor!(ChordStack);

// ---------------------------------------------------------------- Scale Lock

pub const ROOTS: &[&str] = &["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
pub const SCALES: &[&str] = &["Major", "Minor", "Dorian", "Phrygian", "Lydian", "Mixolydian", "Locrian", "Harmonic Minor", "Pentatonic Major", "Pentatonic Minor", "Blues", "Chromatic"];
/// Semitone sets of `SCALES` as 12-bit masks (bit k = degree k above the root).
const SCALE_MASKS: [u16; 12] = [
    mask(&[0, 2, 4, 5, 7, 9, 11]),
    mask(&[0, 2, 3, 5, 7, 8, 10]),
    mask(&[0, 2, 3, 5, 7, 9, 10]),
    mask(&[0, 1, 3, 5, 7, 8, 10]),
    mask(&[0, 2, 4, 6, 7, 9, 11]),
    mask(&[0, 2, 4, 5, 7, 9, 10]),
    mask(&[0, 1, 3, 5, 6, 8, 10]),
    mask(&[0, 2, 3, 5, 7, 8, 11]),
    mask(&[0, 2, 4, 7, 9]),
    mask(&[0, 3, 5, 7, 10]),
    mask(&[0, 3, 5, 6, 7, 10]),
    0xfff,
];

const fn mask(degrees: &[u8]) -> u16 {
    let mut m = 0;
    let mut i = 0;
    while i < degrees.len() {
        m |= 1 << degrees[i];
        i += 1;
    }
    m
}

pub const DIRECTIONS: &[&str] = &["Nearest", "Up", "Down"];

pub const SCALE_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("root", "Root", ROOTS, 0.0),
    ParamDesc::choice("scale", "Scale", SCALES, 0.0),
    ParamDesc::choice("direction", "Direction", DIRECTIONS, 0.0),
];

/// Moves `pitch` into the scale (`direction`: 0 nearest with ties up, 1 up, 2 down).
pub fn snap_to_scale(pitch: u8, root: u8, scale: usize, direction: usize) -> u8 {
    let m = SCALE_MASKS[scale.min(11)];
    let in_scale = |p: i32| m & (1 << (p - root as i32).rem_euclid(12)) != 0;
    let p = pitch as i32;
    if in_scale(p) {
        return pitch;
    }
    for d in 1..12 {
        let up = (direction != 2).then_some(p + d).filter(|&q| q < 128 && in_scale(q));
        let down = (direction != 1).then_some(p - d).filter(|&q| q >= 0 && in_scale(q));
        if let Some(q) = up.or(down) {
            return q as u8;
        }
    }
    pitch
}

pub struct ScaleLock {
    params: [f32; 3],
    map: NoteMap,
}

params_impl!(ScaleLock, SCALE_PARAMS, 3);

impl ScaleLock {
    pub fn new() -> Self {
        Self { params: Self::defaults(), map: NoteMap::default() }
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, _n: usize, _ctx: &Ctx) {
        let (root, scale, dir) = (self.params[0] as u8, self.params[1] as usize, self.params[2] as usize);
        run_mapped(&mut self.map, input, out, |pitch, vel, ps, vs| {
            ps[0] = snap_to_scale(pitch, root, scale, dir);
            vs[0] = vel;
            1
        });
    }
}

midi_processor!(ScaleLock);

// ---------------------------------------------------------------- Pitch Randomizer

pub const SIGNS: &[&str] = &["Bipolar", "Up", "Down"];

pub const RANDOM_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("chance", "Chance", 0.0, 100.0, 50.0, "%"),
    ParamDesc::int("range", "Range", 0.0, 24.0, 12.0, "st"),
    ParamDesc::int("choices", "Choices", 1.0, 24.0, 1.0, ""),
    ParamDesc::choice("sign", "Sign", SIGNS, 0.0),
];

pub struct PitchRandom {
    params: [f32; 4],
    map: NoteMap,
    rng: Rng,
}

params_impl!(PitchRandom, RANDOM_PARAMS, 4);

impl PitchRandom {
    pub fn new() -> Self {
        Self { params: Self::defaults(), map: NoteMap::default(), rng: Rng::new(0x7a1d) }
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, _n: usize, _ctx: &Ctx) {
        let p = self.params;
        let rng = &mut self.rng;
        run_mapped(&mut self.map, input, out, |pitch, vel, ps, vs| {
            vs[0] = vel;
            ps[0] = pitch;
            if rng.next_unit() * 100.0 < p[0] as f64 && p[1] > 0.0 {
                let choices = p[2].max(1.0) as u32;
                let k = 1 + rng.next_u32() % choices;
                let step = p[1] / choices as f32;
                let up = match p[3] as usize {
                    1 => true,
                    2 => false,
                    _ => rng.next_u32() & 1 == 0,
                };
                let shift = (k as f32 * step).round() as i32 * if up { 1 } else { -1 };
                ps[0] = (pitch as i32 + shift).clamp(0, 127) as u8;
            }
            1
        });
    }
}

midi_processor!(PitchRandom);

// ---------------------------------------------------------------- Velocity Shaper

pub const VELOCITY_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("drive", "Drive", -1.0, 1.0, 0.0, ""),
    ParamDesc::int("out_low", "Out Low", 1.0, 127.0, 1.0, ""),
    ParamDesc::int("out_high", "Out High", 1.0, 127.0, 127.0, ""),
    ParamDesc::int("random", "Random", 0.0, 64.0, 0.0, ""),
];

/// Velocity curve without randomness (spec D4, Velocity Shaper).
pub fn shape_velocity(v: u8, drive: f32, low: f32, high: f32) -> u8 {
    let n = (v as f32 / 127.0).clamp(0.0, 1.0);
    let curved = n.powf(2f32.powf(-2.0 * drive));
    (low + curved * (high - low)).round().clamp(1.0, 127.0) as u8
}

pub struct VelocityShaper {
    params: [f32; 4],
    rng: Rng,
}

params_impl!(VelocityShaper, VELOCITY_PARAMS, 4);

impl VelocityShaper {
    pub fn new() -> Self {
        Self { params: Self::defaults(), rng: Rng::new(0xbe1) }
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, _n: usize, _ctx: &Ctx) {
        let p = self.params;
        for e in input {
            let kind = match e.kind {
                NoteKind::On { pitch, velocity } if velocity > 0 => {
                    let mut v = shape_velocity(velocity, p[0], p[1], p[2]) as f32;
                    if p[3] > 0.0 {
                        v += (self.rng.next_bipolar() * p[3]).round();
                    }
                    NoteKind::On { pitch, velocity: v.clamp(1.0, 127.0) as u8 }
                }
                k => k,
            };
            push(out, e.offset, kind);
        }
    }
}

midi_processor!(VelocityShaper);

// ---------------------------------------------------------------- Note Duration

pub const DURATION_MODES: &[&str] = &["Time", "Synced"];
pub const DURATION_SYNC: &[&str] = &["1/32", "1/16", "1/8", "1/4", "1/2", "1/1"];
const DURATION_BEATS: [f64; 6] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

pub const DURATION_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("mode", "Mode", DURATION_MODES, 0.0),
    ParamDesc::log("time", "Length", 1.0, 4_000.0, 100.0, "ms"),
    ParamDesc::choice("synced", "Length Sync", DURATION_SYNC, 1.0),
    ParamDesc::toggle("release_trigger", "Release Trigger", false),
];

pub struct NoteDuration {
    params: [f32; 4],
    /// Samples until each pitch's scheduled note-off (`u32::MAX` = none).
    remaining: [u32; 128],
    /// Velocity of notes held down while Release Trigger waits for their note-off.
    waiting: [u8; 128],
    sr: f32,
}

params_impl!(NoteDuration, DURATION_PARAMS, 4);

impl NoteDuration {
    pub fn new(sr: f32) -> Self {
        Self { params: Self::defaults(), remaining: [u32::MAX; 128], waiting: [0; 128], sr }
    }

    fn length_samples(&self, ctx: &Ctx) -> u32 {
        let s = if self.params[0] as usize == 1 {
            DURATION_BEATS[self.params[2] as usize] / ctx.beats_per_sample.max(1e-12)
        } else {
            self.params[1] as f64 * 0.001 * self.sr as f64
        };
        s.clamp(1.0, u32::MAX as f64 / 2.0) as u32
    }

    fn start(&mut self, out: &mut Vec<NoteEvent>, offset: u32, pitch: u8, velocity: u8, len: u32) {
        let p = pitch as usize & 127;
        if self.remaining[p] != u32::MAX {
            push(out, offset, NoteKind::Off { pitch });
        }
        push(out, offset, NoteKind::On { pitch, velocity });
        self.remaining[p] = offset + len;
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, n: usize, ctx: &Ctx) {
        let len = self.length_samples(ctx);
        let on_release = self.params[3] >= 0.5;
        for e in input {
            match e.kind {
                NoteKind::On { pitch, velocity } if velocity > 0 => {
                    if on_release {
                        self.waiting[pitch as usize & 127] = velocity;
                    } else {
                        self.start(out, e.offset, pitch, velocity, len);
                    }
                }
                NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
                    let v = std::mem::take(&mut self.waiting[pitch as usize & 127]);
                    if v > 0 {
                        self.start(out, e.offset, pitch, v, len);
                    }
                }
                NoteKind::AllOff => {
                    self.remaining = [u32::MAX; 128];
                    self.waiting = [0; 128];
                    push(out, e.offset, NoteKind::AllOff);
                }
                NoteKind::Sustain(_) => push(out, e.offset, e.kind),
            }
        }
        for (p, r) in self.remaining.iter_mut().enumerate() {
            if *r == u32::MAX {
                continue;
            }
            if (*r as usize) < n {
                push(out, *r, NoteKind::Off { pitch: p as u8 });
                *r = u32::MAX;
            } else {
                *r -= n as u32;
            }
        }
    }
}

midi_processor!(NoteDuration);

// ---------------------------------------------------------------- Arpeggiator

pub const ARP_PATTERNS: &[&str] = &["Up", "Down", "Up-Down", "Down-Up", "As Played", "Random"];
pub const ARP_RATES: &[&str] = &["1/4", "1/8", "1/8T", "1/16", "1/16T", "1/32"];
const ARP_STEP_BEATS: [f64; 6] = [1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];
pub const ARP_VELOCITY: &[&str] = &["Played", "Fixed"];

pub const ARP_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("pattern", "Pattern", ARP_PATTERNS, 0.0),
    ParamDesc::choice("rate", "Rate", ARP_RATES, 3.0),
    ParamDesc::lin("gate", "Gate", 5.0, 100.0, 60.0, "%"),
    ParamDesc::int("octaves", "Octaves", 1.0, 4.0, 1.0, ""),
    ParamDesc::choice("velocity", "Velocity", ARP_VELOCITY, 0.0),
    ParamDesc::int("fixed_velocity", "Fixed Vel", 1.0, 127.0, 100.0, ""),
    ParamDesc::toggle("hold", "Hold", false),
];

const MAX_HELD: usize = 32;

pub struct Arpeggiator {
    params: [f32; 7],
    /// Notes the arpeggio plays, in the order they were pressed.
    held: [(u8, u8); MAX_HELD],
    held_len: usize,
    /// Keys physically down (for Hold: a new chord replaces the latched one).
    down: [bool; 128],
    step: u64,
    /// Sounding arpeggio note and samples until its note-off.
    sounding: Option<(u8, u32)>,
    /// Free-running clock (samples since the arpeggio started) while the transport is stopped.
    free_pos: f64,
    rng: Rng,
}

params_impl!(Arpeggiator, ARP_PARAMS, 7);

impl Arpeggiator {
    pub fn new(_sr: f32) -> Self {
        Self { params: Self::defaults(), held: [(0, 0); MAX_HELD], held_len: 0, down: [false; 128], step: 0, sounding: None, free_pos: 0.0, rng: Rng::new(0xa4e9) }
    }

    fn add(&mut self, pitch: u8, vel: u8) {
        if self.held[..self.held_len].iter().any(|h| h.0 == pitch) || self.held_len == MAX_HELD {
            return;
        }
        self.held[self.held_len] = (pitch, vel);
        self.held_len += 1;
    }

    fn remove(&mut self, pitch: u8) {
        if let Some(i) = self.held[..self.held_len].iter().position(|h| h.0 == pitch) {
            self.held.copy_within(i + 1..self.held_len, i);
            self.held_len -= 1;
        }
    }

    /// The note (pitch, velocity) for step `k` of the current pattern.
    pub fn note_for_step(&mut self, k: u64) -> Option<(u8, u8)> {
        let n = self.held_len;
        if n == 0 {
            return None;
        }
        let mut sorted = [(0u8, 0u8); MAX_HELD];
        sorted[..n].copy_from_slice(&self.held[..n]);
        let pattern = self.params[0] as usize;
        if pattern != 4 {
            sorted[..n].sort_unstable_by_key(|h| h.0);
        }
        let oct = self.params[3].max(1.0) as usize;
        let total = n * oct;
        let idx = match pattern {
            1 => total - 1 - (k as usize % total),
            2 | 3 => {
                let cycle = (2 * total).saturating_sub(2).max(1);
                let i = k as usize % cycle;
                let up = if i < total { i } else { cycle - i };
                if pattern == 2 { up } else { total - 1 - up }
            }
            5 => self.rng.next_u32() as usize % total,
            _ => k as usize % total,
        };
        let (pitch, vel) = sorted[idx % n];
        let pitch = (pitch as usize + 12 * (idx / n)).min(127) as u8;
        let vel = if self.params[4] as usize == 1 { self.params[5] as u8 } else { vel };
        Some((pitch, vel))
    }

    fn stop_sounding(&mut self, out: &mut Vec<NoteEvent>, offset: u32) {
        if let Some((p, _)) = self.sounding.take() {
            push(out, offset, NoteKind::Off { pitch: p });
        }
    }

    fn play_step(&mut self, out: &mut Vec<NoteEvent>, offset: u32, step_samples: f64) {
        self.stop_sounding(out, offset);
        if let Some((pitch, vel)) = self.note_for_step(self.step) {
            push(out, offset, NoteKind::On { pitch, velocity: vel.max(1) });
            let len = (step_samples * self.params[2] as f64 / 100.0).max(1.0) as u32;
            self.sounding = Some((pitch, offset + len));
        }
        self.step += 1;
    }

    fn run(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, n: usize, ctx: &Ctx) {
        let hold = self.params[6] >= 0.5;
        let step_beats = ARP_STEP_BEATS[self.params[1] as usize];
        let step_samples = step_beats / ctx.beats_per_sample.max(1e-12);
        let was_empty = self.held_len == 0;
        let mut started_at = None;
        for e in input {
            match e.kind {
                NoteKind::On { pitch, velocity } if velocity > 0 => {
                    if hold && !self.down.iter().any(|&d| d) {
                        self.held_len = 0; // a new chord replaces the latched one
                    }
                    self.down[pitch as usize & 127] = true;
                    if self.held_len == 0 {
                        started_at.get_or_insert(e.offset);
                    }
                    self.add(pitch, velocity);
                }
                NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
                    self.down[pitch as usize & 127] = false;
                    if !hold {
                        self.remove(pitch);
                    }
                }
                NoteKind::AllOff => {
                    self.held_len = 0;
                    self.down = [false; 128];
                    self.sounding = None;
                    self.step = 0;
                    push(out, e.offset, NoteKind::AllOff);
                }
                NoteKind::Sustain(_) => push(out, e.offset, e.kind),
            }
        }
        if self.held_len == 0 {
            // Released: finish the current note and restart the pattern next time.
            if let Some((p, end)) = self.sounding {
                if (end as usize) < n {
                    push(out, end, NoteKind::Off { pitch: p });
                    self.sounding = None;
                } else {
                    self.sounding = Some((p, end - n as u32));
                }
            }
            self.step = 0;
            self.free_pos = 0.0;
            return;
        }
        // Step boundaries inside this block.
        let mut boundaries = [0u32; 64];
        let mut count = 0;
        if ctx.playing {
            let first = (ctx.beat / step_beats - 1e-9).ceil();
            let mut k = first;
            loop {
                let o = ((k * step_beats - ctx.beat) / ctx.beats_per_sample).round();
                if o >= n as f64 || count == boundaries.len() {
                    break;
                }
                boundaries[count] = o.max(0.0) as u32;
                count += 1;
                k += 1.0;
            }
        } else {
            if was_empty {
                // Free clock: the first step plays on the first note-on.
                self.free_pos = -(started_at.unwrap_or(0) as f64);
            }
            let mut k = (self.free_pos / step_samples - 1e-9).ceil();
            loop {
                let o = (k * step_samples - self.free_pos).round();
                if o >= n as f64 || count == boundaries.len() {
                    break;
                }
                boundaries[count] = o.max(0.0) as u32;
                count += 1;
                k += 1.0;
            }
            self.free_pos += n as f64;
        }
        let mut b = 0;
        loop {
            let next_step = if b < count { boundaries[b] } else { u32::MAX };
            // The current note ends before the next step.
            if let Some((p, end)) = self.sounding {
                if end < next_step && (end as usize) < n {
                    push(out, end, NoteKind::Off { pitch: p });
                    self.sounding = None;
                }
            }
            if b == count {
                break;
            }
            self.play_step(out, next_step, step_samples);
            b += 1;
        }
        if let Some((p, end)) = self.sounding {
            self.sounding = Some((p, end.saturating_sub(n as u32)));
        }
    }
}

midi_processor!(Arpeggiator);

/// Passes events through unchanged (bypassed MIDI effect).
pub fn passthrough(input: &[NoteEvent], out: &mut Vec<NoteEvent>) {
    push_all(out, input);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(offset: u32, pitch: u8, velocity: u8) -> NoteEvent {
        NoteEvent { offset, kind: NoteKind::On { pitch, velocity } }
    }

    fn off(offset: u32, pitch: u8) -> NoteEvent {
        NoteEvent { offset, kind: NoteKind::Off { pitch } }
    }

    fn run(p: &mut dyn Processor, input: &[NoteEvent], n: usize, ctx: &Ctx) -> Vec<NoteEvent> {
        let mut out = Vec::with_capacity(256);
        p.process_midi(input, &mut out, n, ctx);
        out.sort_by_key(|e| (e.offset, matches!(e.kind, NoteKind::On { .. })));
        out
    }

    /// Every pitch ends silent: on/off counts balance.
    fn balanced(events: &[NoteEvent]) -> bool {
        let mut depth = [0i32; 128];
        for e in events {
            match e.kind {
                NoteKind::On { pitch, .. } => depth[pitch as usize] += 1,
                NoteKind::Off { pitch } => depth[pitch as usize] -= 1,
                _ => {}
            }
        }
        depth.iter().all(|&d| d == 0)
    }

    const CTX: Ctx = Ctx::stopped(48_000.0, 120.0);

    #[test]
    fn chord_stack_adds_shifts_and_merges_duplicates() {
        let mut c = ChordStack::new();
        let out = run(&mut c, &[on(0, 60, 100)], 64, &CTX);
        let pitches: Vec<u8> = out.iter().filter_map(|e| if let NoteKind::On { pitch, .. } = e.kind { Some(pitch) } else { None }).collect();
        assert_eq!(pitches, vec![60, 64, 67]);
        // A second input landing on 64 shares that output note.
        let out2 = run(&mut c, &[on(0, 64, 90)], 64, &CTX);
        let new: Vec<u8> = out2.iter().filter_map(|e| if let NoteKind::On { pitch, .. } = e.kind { Some(pitch) } else { None }).collect();
        assert_eq!(new, vec![68, 71]);
        // Releasing 60 must not stop 64 (still held by the second note).
        let out3 = run(&mut c, &[off(0, 60)], 64, &CTX);
        assert!(out3.iter().all(|e| e.kind != NoteKind::Off { pitch: 64 }));
        let out4 = run(&mut c, &[off(0, 64)], 64, &CTX);
        let mut all = out.clone();
        all.extend(out2);
        all.extend(out3);
        all.extend(out4);
        assert!(balanced(&all));
    }

    #[test]
    fn changing_parameters_while_notes_hold_never_strands_notes() {
        let mut c = ChordStack::new();
        let mut all = run(&mut c, &[on(0, 60, 100)], 64, &CTX);
        c.set_param(0, 3.0);
        c.set_param(1, 10.0);
        all.extend(run(&mut c, &[off(5, 60)], 64, &CTX));
        assert!(balanced(&all));
        let mut s = ScaleLock::new();
        let mut all = run(&mut s, &[on(0, 61, 100)], 64, &CTX);
        s.set_param(0, 1.0); // root C#: 61 is now in scale
        all.extend(run(&mut s, &[off(0, 61)], 64, &CTX));
        assert!(balanced(&all));
    }

    #[test]
    fn scale_lock_snaps_to_the_scale() {
        // C major: C# (61) → D (62) with ties up; Down direction → C (60).
        assert_eq!(snap_to_scale(61, 0, 0, 0), 62);
        assert_eq!(snap_to_scale(61, 0, 0, 2), 60);
        assert_eq!(snap_to_scale(64, 0, 0, 0), 64);
        // A minor pentatonic (root A=9): B (71) is between A (69) and C (72) → nearest C.
        assert_eq!(snap_to_scale(71, 9, 9, 0), 72);
        // Chromatic leaves everything alone.
        assert!((0..128).all(|p| snap_to_scale(p, 3, 11, 0) == p));
    }

    #[test]
    fn pitch_randomizer_is_seeded_and_bounded() {
        let mut a = PitchRandom::new();
        let mut b = PitchRandom::new();
        a.set_param(0, 100.0);
        b.set_param(0, 100.0);
        a.set_param(2, 4.0);
        b.set_param(2, 4.0);
        let mut seq = vec![];
        for k in 0..50 {
            let x = run(&mut a, &[on(0, 60, 100), off(10, 60)], 64, &CTX);
            let y = run(&mut b, &[on(0, 60, 100), off(10, 60)], 64, &CTX);
            assert_eq!(x, y, "same seed, same output (step {k})");
            assert!(balanced(&x));
            if let NoteKind::On { pitch, .. } = x[0].kind {
                assert!([48, 51, 54, 57, 63, 66, 69, 72].contains(&pitch), "pitch {pitch}");
                seq.push(pitch);
            }
        }
        assert!(seq.iter().any(|&p| p > 60) && seq.iter().any(|&p| p < 60));
    }

    #[test]
    fn velocity_shaper_maps_range_and_curve() {
        assert_eq!(shape_velocity(127, 0.0, 1.0, 127.0), 127);
        assert_eq!(shape_velocity(64, 0.0, 1.0, 127.0), 64);
        assert_eq!(shape_velocity(127, 0.0, 40.0, 80.0), 80);
        assert_eq!(shape_velocity(1, 0.0, 40.0, 80.0), 40);
        // Positive drive raises soft notes.
        assert!(shape_velocity(32, 1.0, 1.0, 127.0) > 32);
        assert!(shape_velocity(32, -1.0, 1.0, 127.0) < 32);
    }

    #[test]
    fn note_duration_replaces_lengths_and_supports_release_trigger() {
        let mut d = NoteDuration::new(48_000.0);
        d.set_param(1, 10.0); // 10 ms = 480 samples
        let mut all = run(&mut d, &[on(100, 60, 90), off(110, 60)], 512, &CTX);
        assert_eq!(all, vec![on(100, 60, 90)]);
        let next = run(&mut d, &[], 512, &CTX);
        assert_eq!(next, vec![off(100 + 480 - 512, 60)]);
        all.extend(next);
        assert!(balanced(&all));
        // Release trigger: the note starts on the incoming note-off.
        d.set_param(3, 1.0);
        let out = run(&mut d, &[on(0, 62, 70), off(50, 62)], 256, &CTX);
        assert_eq!(out, vec![on(50, 62, 70)]);
    }

    #[test]
    fn arpeggiator_plays_up_pattern_on_the_song_grid() {
        let mut a = Arpeggiator::new(48_000.0);
        // 1/16 at 120 BPM = 6000 samples per step. Transport at beat 0.
        let mut ctx = CTX;
        ctx.playing = true;
        let mut events = vec![];
        let n = 512;
        let input = [on(0, 64, 100), on(0, 60, 90), on(0, 67, 80)];
        for blk in 0..60 {
            ctx.beat = blk as f64 * n as f64 * ctx.beats_per_sample;
            let inp: &[NoteEvent] = if blk == 0 { &input } else { &[] };
            for mut e in run(&mut a, inp, n, &ctx) {
                e.offset += (blk * n) as u32;
                events.push(e);
            }
        }
        let ons: Vec<(u32, u8)> = events.iter().filter_map(|e| if let NoteKind::On { pitch, .. } = e.kind { Some((e.offset, pitch)) } else { None }).collect();
        assert_eq!(ons[..5], [(0, 60), (6000, 64), (12_000, 67), (18_000, 60), (24_000, 64)]);
        // Gate 60 %: each note lasts 3600 samples.
        let first_off = events.iter().find(|e| e.kind == NoteKind::Off { pitch: 60 }).unwrap();
        assert_eq!(first_off.offset, 3600);
    }

    #[test]
    fn arpeggiator_patterns_and_octaves() {
        let mut a = Arpeggiator::new(48_000.0);
        a.add(60, 100);
        a.add(64, 100);
        a.add(67, 100);
        let seq = |a: &mut Arpeggiator, k: u64| (0..k).map(|i| a.note_for_step(i).unwrap().0).collect::<Vec<_>>();
        a.set_param(0, 2.0); // Up-Down
        assert_eq!(seq(&mut a, 6), vec![60, 64, 67, 64, 60, 64]);
        a.set_param(0, 1.0); // Down
        assert_eq!(seq(&mut a, 3), vec![67, 64, 60]);
        a.set_param(0, 0.0);
        a.set_param(3, 2.0); // two octaves
        assert_eq!(seq(&mut a, 6), vec![60, 64, 67, 72, 76, 79]);
    }

    #[test]
    fn arpeggiator_releases_everything_and_hold_latches() {
        let mut a = Arpeggiator::new(48_000.0);
        let mut all = vec![];
        all.extend(run(&mut a, &[on(0, 60, 100), on(0, 64, 100)], 512, &CTX));
        for _ in 0..40 {
            all.extend(run(&mut a, &[], 512, &CTX));
        }
        all.extend(run(&mut a, &[off(0, 60), off(0, 64)], 512, &CTX));
        for _ in 0..20 {
            all.extend(run(&mut a, &[], 512, &CTX));
        }
        assert!(balanced(&all), "stuck notes after release");
        let ons_after_release = all.len();
        assert!(run(&mut a, &[], 512, &CTX).is_empty());
        let _ = ons_after_release;
        // Hold: keeps playing after keys go up; a new chord replaces it.
        a.set_param(6, 1.0);
        run(&mut a, &[on(0, 60, 100), off(10, 60)], 512, &CTX);
        assert_eq!(a.held_len, 1);
        run(&mut a, &[on(0, 72, 100)], 512, &CTX);
        assert_eq!(&a.held[..a.held_len], &[(72, 100)]);
        let all_off = run(&mut a, &[NoteEvent { offset: 0, kind: NoteKind::AllOff }], 512, &CTX);
        assert!(all_off.iter().any(|e| e.kind == NoteKind::AllOff));
        assert_eq!(a.held_len, 0);
    }

    #[test]
    fn outputs_never_exceed_buffer_capacity() {
        let mut c = ChordStack::new();
        for k in 0..6 {
            c.set_param(k, (k as f32 + 1.0) * 2.0);
        }
        let input: Vec<NoteEvent> = (0..64).map(|p| on(0, 30 + p as u8, 100)).collect();
        let mut out = Vec::with_capacity(64);
        c.process_midi(&input, &mut out, 64, &CTX);
        assert_eq!(out.len(), 64);
        assert_eq!(out.capacity(), 64);
    }
}
