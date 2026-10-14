//! FM Synth and Morph Synth (spec: docs/specs/devices.md D5).

use crate::device::{Io, Processor};
use crate::env::Adsr;
use crate::filter::{Svf, SvfMode};
use crate::instruments::{event_steps, Step, MAX_VOICES};
use crate::params::{midi_to_hz, Ctx, NoteKind, ParamDesc};
use crate::util::{db_to_gain, Smoothed};
use std::f32::consts::TAU;
use std::sync::OnceLock;

/// Voice bookkeeping shared by both synths (allocation, sustain pedal, stealing).
#[derive(Clone, Copy, Default)]
struct Slot {
    note: u8,
    gain: f32,
    age: u64,
    held: bool,
    sustained: bool,
}

trait Voice: Copy {
    fn slot(&mut self) -> &mut Slot;
    fn age(&self) -> u64;
    fn amp(&mut self) -> &mut Adsr;
    fn active(&self) -> bool;
    /// Starts the voice; `fresh` = it was idle (reset phases and filters).
    fn start(&mut self, fresh: bool);
}

struct Voices<V: Voice> {
    v: [V; MAX_VOICES],
    sustain: bool,
    counter: u64,
}

impl<V: Voice> Voices<V> {
    fn new(v: V) -> Self {
        Self { v: [v; MAX_VOICES], sustain: false, counter: 0 }
    }

    /// Returns the voice index to start for `note`.
    fn note_on(&mut self, note: u8, velocity: u8, vel_sens: f32) -> usize {
        let idx = self.v.iter().position(|v| !v.active()).unwrap_or_else(|| {
            let mut best = 0;
            for i in 1..MAX_VOICES {
                if self.v[i].age() < self.v[best].age() {
                    best = i;
                }
            }
            best
        });
        self.counter += 1;
        let counter = self.counter;
        let voice = &mut self.v[idx];
        let fresh = !voice.active();
        let vel = velocity as f32 / 127.0;
        *voice.slot() = Slot { note, gain: 1.0 - vel_sens + vel_sens * vel * vel, age: counter, held: true, sustained: false };
        voice.start(fresh);
        idx
    }

    fn apply(&mut self, kind: &NoteKind, vel_sens: f32, sr: f32) -> Option<usize> {
        match *kind {
            NoteKind::On { pitch, velocity } if velocity > 0 => return Some(self.note_on(pitch, velocity, vel_sens)),
            NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
                let sustain = self.sustain;
                for v in self.v.iter_mut() {
                    let s = v.slot();
                    if s.held && s.note == pitch {
                        s.held = false;
                        if sustain {
                            s.sustained = true;
                        } else {
                            v.amp().gate_off();
                        }
                    }
                }
            }
            NoteKind::Sustain(on) => {
                self.sustain = on;
                if !on {
                    for v in self.v.iter_mut() {
                        if v.slot().sustained {
                            v.slot().sustained = false;
                            v.amp().gate_off();
                        }
                    }
                }
            }
            NoteKind::AllOff => {
                self.sustain = false;
                for v in self.v.iter_mut() {
                    v.slot().held = false;
                    v.slot().sustained = false;
                    v.amp().fast_release(sr);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------- FM Synth

pub const FM_ALGORITHMS: &[&str] = &["1: 4>3>2>1", "2: (4+3)>2>1", "3: 4>3>1, 2>1", "4: 4>3, 2>1", "5: 4>(1,2,3)", "6: 4>3, 1, 2", "7: (4+3+2)>1", "8: 1+2+3+4"];

/// For each algorithm: `MODS[a][k]` = operators (bit i = operator i+1) that
/// modulate operator k+1; `CARRIERS[a]` = operators heard at the output.
const MODS: [[u8; 4]; 8] = [
    [0b0010, 0b0100, 0b1000, 0],
    [0b0010, 0b1100, 0, 0],
    [0b0110, 0, 0b1000, 0],
    [0b0010, 0, 0b1000, 0],
    [0b1000, 0b1000, 0b1000, 0],
    [0, 0, 0b1000, 0],
    [0b1110, 0, 0, 0],
    [0, 0, 0, 0],
];
const CARRIERS: [u8; 8] = [0b0001, 0b0001, 0b0001, 0b0101, 0b0111, 0b0111, 0b0001, 0b1111];

macro_rules! fm_op {
    ($n:literal, $ratio:expr, $level:expr) => {
        [
            ParamDesc::log(concat!("op", $n, "_ratio"), concat!("Op", $n, " Ratio"), 0.25, 16.0, $ratio, ""),
            ParamDesc::lin(concat!("op", $n, "_level"), concat!("Op", $n, " Level"), 0.0, 1.0, $level, ""),
            ParamDesc::log(concat!("op", $n, "_attack"), concat!("Op", $n, " Attack"), 0.5, 5_000.0, 2.0, "ms"),
            ParamDesc::log(concat!("op", $n, "_decay"), concat!("Op", $n, " Decay"), 1.0, 10_000.0, 600.0, "ms"),
            ParamDesc::lin(concat!("op", $n, "_sustain"), concat!("Op", $n, " Sustain"), 0.0, 1.0, 0.5, ""),
            ParamDesc::log(concat!("op", $n, "_release"), concat!("Op", $n, " Release"), 1.0, 10_000.0, 300.0, "ms"),
        ]
    };
}

const OP_STRIDE: usize = 6;
const FM_N: usize = 4 * OP_STRIDE + 6;

const fn fm_table() -> [ParamDesc; FM_N] {
    let ops = [fm_op!(1, 1.0, 1.0), fm_op!(2, 2.0, 0.5), fm_op!(3, 1.0, 0.0), fm_op!(4, 1.0, 0.0)];
    let tail = [
        ParamDesc::choice("algorithm", "Algorithm", FM_ALGORITHMS, 0.0),
        ParamDesc::lin("feedback", "Feedback", 0.0, 1.0, 0.0, ""),
        ParamDesc::log("cutoff", "Cutoff", 30.0, 18_000.0, 18_000.0, "Hz"),
        ParamDesc::lin("resonance", "Resonance", 0.0, 0.95, 0.0, ""),
        ParamDesc::lin("vel_sens", "Velocity", 0.0, 1.0, 0.6, ""),
        ParamDesc::lin("volume", "Volume", -36.0, 6.0, -10.0, "dB"),
    ];
    let mut out = [tail[0]; FM_N];
    let mut i = 0;
    while i < 4 * OP_STRIDE {
        out[i] = ops[i / OP_STRIDE][i % OP_STRIDE];
        i += 1;
    }
    while i < FM_N {
        out[i] = tail[i - 4 * OP_STRIDE];
        i += 1;
    }
    out
}

const FM_TABLE: [ParamDesc; FM_N] = fm_table();
pub const FM_PARAMS: &[ParamDesc] = &FM_TABLE;
const ALG: usize = 24;

/// Phase-modulation depth (radians) of a modulator at full level.
const MOD_INDEX: f32 = 4.0;

#[derive(Clone, Copy, Default)]
struct FmVoice {
    slot: Slot,
    phase: [f32; 4],
    env: [Adsr; 4],
    last4: [f32; 2],
    filter: Svf,
}

impl Voice for FmVoice {
    fn slot(&mut self) -> &mut Slot {
        &mut self.slot
    }
    fn age(&self) -> u64 {
        self.slot.age
    }
    /// The voice ends when every carrier envelope has ended; operator 1's
    /// envelope drives gate handling for all four (they share gate timing).
    fn amp(&mut self) -> &mut Adsr {
        &mut self.env[0]
    }
    fn active(&self) -> bool {
        self.env.iter().any(Adsr::is_active)
    }
    fn start(&mut self, fresh: bool) {
        if fresh {
            self.phase = [0.0; 4];
            self.last4 = [0.0; 2];
            self.filter.reset();
        }
    }
}

pub struct FmSynth {
    params: [f32; FM_N],
    voices: Voices<FmVoice>,
    volume: Smoothed,
    sr: f32,
}

impl FmSynth {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    pub fn new(sr: f32) -> Self {
        let params: [f32; FM_N] = std::array::from_fn(|i| FM_PARAMS[i].default);
        Self { volume: Smoothed::new(db_to_gain(params[FM_N - 1]), sr, 20.0), params, voices: Voices::new(FmVoice::default()), sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = FM_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            if i == FM_N - 1 {
                self.volume.set_target(db_to_gain(self.params[i]));
            }
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.v.iter().filter(|v| v.active()).count()
    }

    fn apply(&mut self, k: &NoteKind) {
        let p = self.params;
        if let Some(i) = self.voices.apply(k, p[ALG + 4], self.sr) {
            let v = &mut self.voices.v[i];
            for op in 0..4 {
                let o = op * OP_STRIDE;
                v.env[op].set(self.sr, p[o + 2], p[o + 3], p[o + 4], p[o + 5]);
                v.env[op].gate_on();
            }
        } else if matches!(k, NoteKind::On { .. } | NoteKind::Off { .. } | NoteKind::Sustain(false) | NoteKind::AllOff) {
            // Operator 1 got the gate change; mirror it on the other operators.
            for v in self.voices.v.iter_mut() {
                let stage = v.env[0].stage;
                for op in 1..4 {
                    if stage == crate::env::Stage::Release && v.env[op].stage != crate::env::Stage::Release {
                        if matches!(k, NoteKind::AllOff) {
                            v.env[op].fast_release(self.sr);
                        } else {
                            v.env[op].gate_off();
                        }
                    }
                }
            }
        }
    }

    fn render_range(&mut self, l: &mut [f32], r: &mut [f32]) {
        let p = self.params;
        let alg = p[ALG] as usize;
        let (mods, carriers) = (MODS[alg], CARRIERS[alg]);
        let n_car = carriers.count_ones() as f32;
        let fb = p[ALG + 1] * std::f32::consts::PI;
        let sr = self.sr;
        let inc_scale = 1.0 / sr;
        let ratios = [p[0], p[OP_STRIDE], p[2 * OP_STRIDE], p[3 * OP_STRIDE]];
        let levels = [p[1], p[OP_STRIDE + 1], p[2 * OP_STRIDE + 1], p[3 * OP_STRIDE + 1]];
        for i in 0..l.len() {
            let vol = self.volume.next();
            let mut acc = 0.0;
            for (vi, v) in self.voices.v.iter_mut().enumerate() {
                if !v.active() {
                    continue;
                }
                let f = midi_to_hz(v.slot.note as f32);
                let mut out = [0.0f32; 4];
                // Operator 4 is computed first: it only modulates (or is a carrier).
                for op in (0..4).rev() {
                    let mut m = 0.0;
                    for src in 0..4 {
                        if mods[op] & (1 << src) != 0 {
                            m += out[src];
                        }
                    }
                    if op == 3 {
                        m += fb * (v.last4[0] + v.last4[1]) * 0.5;
                    }
                    let e = v.env[op].next();
                    out[op] = (v.phase[op] * TAU + m * MOD_INDEX).sin() * levels[op] * e;
                    v.phase[op] = (v.phase[op] + f * ratios[op] * inc_scale).fract();
                }
                v.last4 = [out[3], v.last4[0]];
                let mut s = 0.0;
                for op in 0..4 {
                    if carriers & (1 << op) != 0 {
                        s += out[op];
                    }
                }
                if (i + vi) & 7 == 0 {
                    v.filter.set(sr, p[ALG + 2], p[ALG + 3]);
                }
                acc += v.filter.process(s / n_car, SvfMode::LowPass) * v.slot.gain;
            }
            let out = acc * vol * 0.5;
            l[i] += out;
            r[i] += out;
        }
    }
}

impl Processor for FmSynth {
    fn set_param(&mut self, i: usize, v: f32) {
        FmSynth::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.params
    }
    fn process(&mut self, io: Io, ctx: &Ctx) {
        self.sr = ctx.sample_rate;
        let (l, r) = (io.l, io.r);
        event_steps(io.events, l.len(), |step| match step {
            Step::Event(k) => self.apply(k),
            Step::Segment(a, b) => self.render_range(&mut l[a..b], &mut r[a..b]),
        });
    }
}

// ---------------------------------------------------------------- Morph Synth

pub const MORPH_TABLES: &[&str] = &["Basic", "Vocal", "Digital", "Organ"];
const FRAMES: usize = 8;
const TABLE_LEN: usize = 1024;
/// Harmonics kept per mip level: level 0 is used for the lowest notes.
const LEVEL_HARMONICS: [usize; 9] = [256, 128, 64, 32, 16, 8, 4, 2, 1];

/// `tables[table][frame][level]` single cycles, generated by additive synthesis.
pub struct MorphTables {
    data: Vec<f32>,
}

impl MorphTables {
    fn idx(table: usize, frame: usize, level: usize) -> usize {
        ((table * FRAMES + frame) * LEVEL_HARMONICS.len() + level) * TABLE_LEN
    }

    pub fn cycle(&self, table: usize, frame: usize, level: usize) -> &[f32] {
        let i = Self::idx(table, frame, level);
        &self.data[i..i + TABLE_LEN]
    }
}

/// Amplitude of harmonic `k` (1-based) for `table` at morph position `t` in [0, 1].
pub fn harmonic(table: usize, k: usize, t: f32) -> f32 {
    let kf = k as f32;
    let odd = k % 2 == 1;
    match table {
        0 => {
            // Sine → triangle → square → saw.
            let shapes = [
                if k == 1 { 1.0 } else { 0.0 },
                if odd { 8.0 / (std::f32::consts::PI.powi(2) * kf * kf) * if (k / 2) % 2 == 0 { 1.0 } else { -1.0 } } else { 0.0 },
                if odd { 4.0 / (std::f32::consts::PI * kf) } else { 0.0 },
                2.0 / (std::f32::consts::PI * kf) * if odd { 1.0 } else { -1.0 },
            ];
            let x = t * 3.0;
            let i = (x.floor() as usize).min(2);
            let f = x - i as f32;
            shapes[i] * (1.0 - f) + shapes[i + 1] * f
        }
        1 => {
            // Two formant bumps that glide upwards with t, on a 1/k tilt.
            let f1 = 3.0 + 6.0 * t;
            let f2 = 9.0 + 14.0 * t;
            let bump = |c: f32, w: f32| (-((kf - c) / w).powi(2)).exp();
            (bump(f1, 1.5) + 0.6 * bump(f2, 3.0) + 0.15) / kf.sqrt()
        }
        2 => {
            // Pulse wave narrowing from 50 % to 5 % duty.
            let w = 0.5 - 0.45 * t;
            2.0 / (std::f32::consts::PI * kf) * (std::f32::consts::PI * kf * w).sin()
        }
        _ => {
            // Drawbar-like registers: harmonics 1, 2, 3, 4, 6, 8 fading in with t.
            let set = [(1, 1.0, 0.0), (2, 0.8, 0.0), (3, 0.6, 0.2), (4, 0.5, 0.4), (6, 0.4, 0.6), (8, 0.35, 0.8)];
            set.iter().filter(|s| s.0 == k).map(|&(_, a, on)| a * ((t - on) * 5.0 + 1.0).clamp(0.0, 1.0)).sum()
        }
    }
}

pub fn morph_tables() -> &'static MorphTables {
    static T: OnceLock<MorphTables> = OnceLock::new();
    T.get_or_init(|| {
        let levels = LEVEL_HARMONICS.len();
        let mut data = vec![0.0f32; MORPH_TABLES.len() * FRAMES * levels * TABLE_LEN];
        let sine: Vec<f32> = (0..TABLE_LEN).map(|i| (i as f32 / TABLE_LEN as f32 * TAU).sin()).collect();
        for table in 0..MORPH_TABLES.len() {
            for frame in 0..FRAMES {
                let t = frame as f32 / (FRAMES - 1) as f32;
                let mut scale = 1.0;
                for level in 0..levels {
                    let i0 = MorphTables::idx(table, frame, level);
                    let out = &mut data[i0..i0 + TABLE_LEN];
                    for k in 1..=LEVEL_HARMONICS[level] {
                        let a = harmonic(table, k, t);
                        if a == 0.0 {
                            continue;
                        }
                        for (i, o) in out.iter_mut().enumerate() {
                            *o += a * sine[(i * k) % TABLE_LEN];
                        }
                    }
                    if level == 0 {
                        // All levels share level 0's normalisation (peak 1).
                        scale = 1.0 / out.iter().fold(1e-6f32, |m, v| m.max(v.abs()));
                    }
                    for o in out.iter_mut() {
                        *o *= scale;
                    }
                }
            }
        }
        MorphTables { data }
    })
}

/// Mip level whose harmonics stay below Nyquist for `freq`.
pub fn mip_level(freq: f32, sr: f32) -> usize {
    let allowed = (sr * 0.5 / freq.max(1.0)) as usize;
    LEVEL_HARMONICS.iter().position(|&h| h <= allowed).unwrap_or(LEVEL_HARMONICS.len() - 1)
}

pub const MORPH_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("table", "Table", MORPH_TABLES, 0.0),
    ParamDesc::lin("position", "Position", 0.0, 1.0, 0.3, ""),
    ParamDesc::lin("pos_env", "Pos Env", -1.0, 1.0, 0.0, ""),
    ParamDesc::lin("detune", "Detune", 0.0, 50.0, 10.0, "ct"),
    ParamDesc::int("unison", "Unison", 1.0, 4.0, 1.0, ""),
    ParamDesc::log("cutoff", "Cutoff", 30.0, 18_000.0, 6_000.0, "Hz"),
    ParamDesc::lin("resonance", "Resonance", 0.0, 0.95, 0.1, ""),
    ParamDesc::log("attack", "Attack", 0.5, 5_000.0, 5.0, "ms"),
    ParamDesc::log("decay", "Decay", 1.0, 10_000.0, 400.0, "ms"),
    ParamDesc::lin("sustain", "Sustain", 0.0, 1.0, 0.7, ""),
    ParamDesc::log("release", "Release", 1.0, 10_000.0, 300.0, "ms"),
    ParamDesc::lin("vel_sens", "Velocity", 0.0, 1.0, 0.6, ""),
    ParamDesc::lin("volume", "Volume", -36.0, 6.0, -10.0, "dB"),
];

#[derive(Clone, Copy, Default)]
struct MorphVoice {
    slot: Slot,
    phase: [f32; 4],
    amp: Adsr,
    filter: [Svf; 2],
}

impl Voice for MorphVoice {
    fn slot(&mut self) -> &mut Slot {
        &mut self.slot
    }
    fn age(&self) -> u64 {
        self.slot.age
    }
    fn amp(&mut self) -> &mut Adsr {
        &mut self.amp
    }
    fn active(&self) -> bool {
        self.amp.is_active()
    }
    fn start(&mut self, fresh: bool) {
        if fresh {
            // Spread unison phases so the voices do not start in unison.
            self.phase = [0.0, 0.31, 0.57, 0.83];
            self.filter = [Svf::default(); 2];
        }
    }
}

pub struct MorphSynth {
    params: [f32; 13],
    voices: Voices<MorphVoice>,
    volume: Smoothed,
    tables: &'static MorphTables,
    sr: f32,
}

impl MorphSynth {
    pub fn param_values(&self) -> &[f32] {
        &self.params
    }

    /// Generates the shared tables on first use (call off the audio thread).
    pub fn new(sr: f32) -> Self {
        let params: [f32; 13] = std::array::from_fn(|i| MORPH_PARAMS[i].default);
        Self { volume: Smoothed::new(db_to_gain(params[12]), sr, 20.0), params, voices: Voices::new(MorphVoice::default()), tables: morph_tables(), sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = MORPH_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            if i == 12 {
                self.volume.set_target(db_to_gain(self.params[12]));
            }
        }
    }

    fn apply(&mut self, k: &NoteKind) {
        let p = self.params;
        if let Some(i) = self.voices.apply(k, p[11], self.sr) {
            let v = &mut self.voices.v[i];
            v.amp.set(self.sr, p[7], p[8], p[9], p[10]);
            v.amp.gate_on();
        }
    }

    #[inline]
    fn read(cycle: &[f32], phase: f32) -> f32 {
        let x = phase * TABLE_LEN as f32;
        let i = x as usize % TABLE_LEN;
        let f = x - x.floor();
        cycle[i] + (cycle[(i + 1) % TABLE_LEN] - cycle[i]) * f
    }

    fn render_range(&mut self, l: &mut [f32], r: &mut [f32]) {
        let p = self.params;
        let table = p[0] as usize;
        let unison = p[4].clamp(1.0, 4.0) as usize;
        let sr = self.sr;
        let tables = self.tables;
        for i in 0..l.len() {
            let vol = self.volume.next();
            let (mut al, mut ar) = (0.0, 0.0);
            for (vi, v) in self.voices.v.iter_mut().enumerate() {
                if !v.amp.is_active() {
                    continue;
                }
                let env = v.amp.next();
                let pos = (p[1] + p[2] * env).clamp(0.0, 1.0) * (FRAMES - 1) as f32;
                let f0 = (pos as usize).min(FRAMES - 2);
                let fr = pos - f0 as f32;
                let base = midi_to_hz(v.slot.note as f32);
                let level = mip_level(base * 2f32.powf(p[3] / 1200.0), sr);
                let (ca, cb) = (tables.cycle(table, f0, level), tables.cycle(table, f0 + 1, level));
                let (mut sl, mut sr_) = (0.0, 0.0);
                for u in 0..unison {
                    // Unison voices spread symmetrically over ±Detune and alternate sides.
                    let spread = if unison == 1 { 0.0 } else { (u as f32 / (unison - 1) as f32) * 2.0 - 1.0 };
                    let f = base * 2f32.powf(spread * p[3] / 1200.0);
                    let ph = v.phase[u];
                    let s = Self::read(ca, ph) * (1.0 - fr) + Self::read(cb, ph) * fr;
                    v.phase[u] = (ph + f / sr).fract();
                    if unison == 1 || u % 2 == 0 {
                        sl += s;
                    }
                    if unison == 1 || u % 2 == 1 {
                        sr_ += s;
                    }
                }
                let norm = 1.0 / (unison as f32 / 2.0).max(1.0);
                if (i + vi) & 7 == 0 {
                    for flt in v.filter.iter_mut() {
                        flt.set(sr, p[5], p[6]);
                    }
                }
                let g = env * v.slot.gain * norm;
                al += v.filter[0].process(sl, SvfMode::LowPass) * g;
                ar += v.filter[1].process(sr_, SvfMode::LowPass) * g;
            }
            l[i] += al * vol * 0.5;
            r[i] += ar * vol * 0.5;
        }
    }
}

impl Processor for MorphSynth {
    fn set_param(&mut self, i: usize, v: f32) {
        MorphSynth::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.params
    }
    fn process(&mut self, io: Io, ctx: &Ctx) {
        self.sr = ctx.sample_rate;
        let (l, r) = (io.l, io.r);
        event_steps(io.events, l.len(), |step| match step {
            Step::Event(k) => self.apply(k),
            Step::Segment(a, b) => self.render_range(&mut l[a..b], &mut r[a..b]),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::NoteEvent;

    const SR: f32 = 48_000.0;

    fn play(p: &mut dyn Processor, note: u8, frames: usize, release_after: usize) -> (Vec<f32>, Vec<f32>) {
        let ctx = Ctx::stopped(SR, 120.0);
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let mut pos = 0;
        while pos < frames {
            let n = 256.min(frames - pos);
            let mut ev = vec![];
            if pos == 0 {
                ev.push(NoteEvent { offset: 0, kind: NoteKind::On { pitch: note, velocity: 100 } });
            }
            if (pos..pos + n).contains(&release_after) {
                ev.push(NoteEvent { offset: (release_after - pos) as u32, kind: NoteKind::Off { pitch: note } });
            }
            p.process(Io { events: &ev, l: &mut l[pos..pos + n], r: &mut r[pos..pos + n], sidechain: None }, &ctx);
            pos += n;
        }
        (l, r)
    }

    /// Dominant frequency by zero crossings (good enough for near-sine outputs).
    fn zero_cross_hz(x: &[f32]) -> f32 {
        let c = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        c as f32 * SR / x.len() as f32
    }

    #[test]
    fn fm_with_one_carrier_and_no_modulation_is_a_sine_at_the_note() {
        let mut fm = FmSynth::new(SR);
        fm.set_param(OP_STRIDE + 1, 0.0); // op2 level 0 → no modulation
        let (l, _) = play(&mut fm, 69, 48_000, 40_000);
        let hz = zero_cross_hz(&l[4_800..38_400]);
        assert!((hz - 440.0).abs() < 2.0, "{hz} Hz");
    }

    #[test]
    fn fm_modulation_adds_harmonics_and_voices_end_after_release() {
        let mut plain = FmSynth::new(SR);
        plain.set_param(OP_STRIDE + 1, 0.0);
        let mut bright = FmSynth::new(SR);
        bright.set_param(OP_STRIDE + 1, 1.0);
        // Brightness: more sign changes of the first difference.
        let wiggles = |x: &[f32]| x.windows(3).filter(|w| (w[1] - w[0]) * (w[2] - w[1]) < 0.0).count();
        let (a, _) = play(&mut plain, 57, 24_000, 100_000);
        let (b, _) = play(&mut bright, 57, 24_000, 100_000);
        assert!(wiggles(&b[2400..]) > 2 * wiggles(&a[2400..]));
        let mut fm = FmSynth::new(SR);
        let (l, _) = play(&mut fm, 60, 96_000, 9_600);
        assert!(l.iter().all(|v| v.is_finite()));
        assert_eq!(fm.active_voices(), 0, "all operators must release");
    }

    #[test]
    fn every_fm_algorithm_is_finite_and_audible() {
        for alg in 0..FM_ALGORITHMS.len() {
            let mut fm = FmSynth::new(SR);
            fm.set_param(ALG, alg as f32);
            fm.set_param(ALG + 1, 1.0);
            for op in 0..4 {
                fm.set_param(op * OP_STRIDE + 1, 1.0);
            }
            let (l, _) = play(&mut fm, 48, 9_600, 100_000);
            assert!(l.iter().all(|v| v.is_finite() && v.abs() < 4.0), "alg {alg}");
            assert!(l.iter().any(|v| v.abs() > 0.01), "alg {alg} silent");
        }
    }

    #[test]
    fn morph_tables_are_normalised_and_band_limited() {
        let t = morph_tables();
        for table in 0..MORPH_TABLES.len() {
            for frame in 0..FRAMES {
                let c = t.cycle(table, frame, 0);
                let peak = c.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!((peak - 1.0).abs() < 1e-3, "table {table} frame {frame} peak {peak}");
            }
        }
        // The top mip level of the saw end of Basic holds only the fundamental.
        let top = t.cycle(0, FRAMES - 1, LEVEL_HARMONICS.len() - 1);
        let expected: f32 = top.iter().map(|v| v.abs()).fold(0.0, f32::max);
        for (i, v) in top.iter().enumerate() {
            let s = (i as f32 / TABLE_LEN as f32 * TAU).sin() * expected;
            assert!((v - s).abs() < 1e-3);
        }
        assert_eq!(mip_level(20.0, 48_000.0), 0);
        assert_eq!(mip_level(10_000.0, 48_000.0), 7);
        assert_eq!(mip_level(23_000.0, 48_000.0), LEVEL_HARMONICS.len() - 1);
    }

    #[test]
    fn morph_synth_plays_the_note_pitch() {
        let mut m = MorphSynth::new(SR);
        m.set_param(1, 0.0); // Basic, sine frame
        m.set_param(3, 0.0);
        m.set_param(5, 18_000.0);
        let (l, r) = play(&mut m, 69, 48_000, 40_000);
        let hz = zero_cross_hz(&l[4_800..38_400]);
        assert!((hz - 440.0).abs() < 2.0, "{hz} Hz");
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-6), "unison 1 is centred");
        m.set_param(4, 4.0);
        let (l, r) = play(&mut m, 60, 9_600, 100_000);
        assert!(l.iter().zip(&r).any(|(a, b)| (a - b).abs() > 1e-3), "unison spreads left/right");
    }
}
