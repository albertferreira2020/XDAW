//! Original instruments: polyphonic subtractive synth ("Poly"), a pitched
//! sampler ("Sampler") and a 16-pad drum rack ("Drum Rack").

use crate::env::{Adsr, Osc, Wave};
use crate::filter::{Svf, SvfMode};
use crate::params::{Ctx, NoteEvent, NoteKind, ParamDesc, midi_to_hz};
use crate::util::{db_to_gain, pan_gains, Smoothed};
use std::sync::Arc;

/// Decoded audio kept in memory (planar f32).
#[derive(Debug)]
pub struct SampleData {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: f64,
}

impl SampleData {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate
    }

    /// 4-point Hermite interpolation; positions outside the data read silence.
    #[inline]
    pub fn read(&self, pos: f64) -> (f32, f32) {
        let n = self.frames();
        if n == 0 || pos < 0.0 || pos >= n as f64 {
            return (0.0, 0.0);
        }
        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        let at = |ch: &Vec<f32>, k: isize| -> f32 { if k < 0 || k as usize >= n { 0.0 } else { ch[k as usize] } };
        let interp = |ch: &Vec<f32>| -> f32 {
            let (xm1, x0, x1, x2) = (at(ch, i - 1), at(ch, i), at(ch, i + 1), at(ch, i + 2));
            let c1 = 0.5 * (x1 - xm1);
            let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
            let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
            ((c3 * t + c2) * t + c1) * t + x0
        };
        let l = interp(&self.channels[0]);
        let r = self.channels.get(1).map_or(l, interp);
        (l, r)
    }
}

/// Visits a block split at event offsets: `Segment(from, to)` for audio, `Event` between.
pub enum Step<'a> {
    Segment(usize, usize),
    Event(&'a NoteKind),
}

pub fn event_steps(events: &[NoteEvent], len: usize, mut f: impl FnMut(Step<'_>)) {
    let mut pos = 0;
    for e in events {
        let off = (e.offset as usize).min(len);
        if off > pos {
            f(Step::Segment(pos, off));
            pos = off;
        }
        f(Step::Event(&e.kind));
    }
    if pos < len {
        f(Step::Segment(pos, len));
    }
}

pub const MAX_VOICES: usize = 16;

// ---------------------------------------------------------------- Poly synth

pub const SYNTH_PARAMS: &[ParamDesc] = &[
    ParamDesc::choice("osc1_wave", "Osc 1", Wave::NAMES, 0.0),
    ParamDesc::choice("osc2_wave", "Osc 2", Wave::NAMES, 1.0),
    ParamDesc::lin("osc2_semi", "Osc 2 Semi", -24.0, 24.0, 0.0, "st"),
    ParamDesc::lin("osc2_detune", "Osc 2 Detune", -50.0, 50.0, 7.0, "ct"),
    ParamDesc::lin("osc_mix", "Osc Mix", 0.0, 1.0, 0.5, ""),
    ParamDesc::log("cutoff", "Cutoff", 30.0, 18_000.0, 2_000.0, "Hz"),
    ParamDesc::lin("resonance", "Resonance", 0.0, 0.95, 0.2, ""),
    ParamDesc::lin("filter_env", "Filter Env", -4.0, 6.0, 2.0, "oct"),
    ParamDesc::log("amp_attack", "Amp Attack", 0.5, 5_000.0, 5.0, "ms"),
    ParamDesc::log("amp_decay", "Amp Decay", 1.0, 10_000.0, 300.0, "ms"),
    ParamDesc::lin("amp_sustain", "Amp Sustain", 0.0, 1.0, 0.7, ""),
    ParamDesc::log("amp_release", "Amp Release", 1.0, 10_000.0, 250.0, "ms"),
    ParamDesc::log("flt_attack", "Flt Attack", 0.5, 5_000.0, 2.0, "ms"),
    ParamDesc::log("flt_decay", "Flt Decay", 1.0, 10_000.0, 400.0, "ms"),
    ParamDesc::lin("flt_sustain", "Flt Sustain", 0.0, 1.0, 0.2, ""),
    ParamDesc::log("flt_release", "Flt Release", 1.0, 10_000.0, 300.0, "ms"),
    ParamDesc::lin("vel_sens", "Velocity", 0.0, 1.0, 0.6, ""),
    ParamDesc::lin("volume", "Volume", -36.0, 6.0, -10.0, "dB"),
];

#[derive(Clone, Copy, Default)]
struct SynthVoice {
    note: u8,
    gain: f32,
    age: u64,
    held: bool,
    sustained: bool,
    osc1: Osc,
    osc2: Osc,
    filter: Svf,
    amp: Adsr,
    fenv: Adsr,
}

pub struct Synth {
    params: [f32; SYNTH_PARAMS.len()],
    voices: [SynthVoice; MAX_VOICES],
    sustain: bool,
    counter: u64,
    volume: Smoothed,
    sr: f32,
}

impl Synth {
    pub fn new(sr: f32) -> Self {
        let mut params = [0.0; SYNTH_PARAMS.len()];
        for (p, d) in params.iter_mut().zip(SYNTH_PARAMS) {
            *p = d.default;
        }
        let volume = Smoothed::new(db_to_gain(params[17]), sr, 20.0);
        Self { params, voices: [SynthVoice::default(); MAX_VOICES], sustain: false, counter: 0, volume, sr }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = SYNTH_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            if i == 17 {
                self.volume.set_target(db_to_gain(self.params[17]));
            }
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.amp.is_active()).count()
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        let p = self.params;
        let idx = self
            .voices
            .iter()
            .position(|v| !v.amp.is_active())
            .unwrap_or_else(|| (0..MAX_VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        self.counter += 1;
        let v = &mut self.voices[idx];
        let vel = velocity as f32 / 127.0;
        v.gain = 1.0 - p[16] + p[16] * vel * vel;
        v.note = note;
        v.age = self.counter;
        v.held = true;
        v.sustained = false;
        if !v.amp.is_active() {
            v.osc1.reset(0.0);
            v.osc2.reset(0.25);
            v.filter.reset();
        }
        v.amp.set(self.sr, p[8], p[9], p[10], p[11]);
        v.fenv.set(self.sr, p[12], p[13], p[14], p[15]);
        v.amp.gate_on();
        v.fenv.gate_on();
    }

    fn note_off(&mut self, note: u8) {
        let sustain = self.sustain;
        for v in self.voices.iter_mut().filter(|v| v.held && v.note == note) {
            v.held = false;
            if sustain {
                v.sustained = true;
            } else {
                v.amp.gate_off();
                v.fenv.gate_off();
            }
        }
    }

    fn apply(&mut self, kind: &NoteKind) {
        match *kind {
            NoteKind::On { pitch, velocity } if velocity > 0 => self.note_on(pitch, velocity),
            NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => self.note_off(pitch),
            NoteKind::Sustain(on) => {
                self.sustain = on;
                if !on {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.amp.gate_off();
                        v.fenv.gate_off();
                    }
                }
            }
            NoteKind::AllOff => {
                self.sustain = false;
                for v in self.voices.iter_mut() {
                    v.held = false;
                    v.sustained = false;
                    v.amp.fast_release(self.sr);
                    v.fenv.gate_off();
                }
            }
        }
    }

    pub fn render(&mut self, events: &[NoteEvent], l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        self.sr = ctx.sample_rate;
        event_steps(events, l.len(), |step| match step {
            Step::Event(k) => self.apply(k),
            Step::Segment(a, b) => self.render_range(&mut l[a..b], &mut r[a..b]),
        });
    }

    fn render_range(&mut self, l: &mut [f32], r: &mut [f32]) {
        let p = self.params;
        let sr = self.sr;
        let (w1, w2) = (Wave::from_index(p[0]), Wave::from_index(p[1]));
        let osc2_ratio = 2f32.powf((p[2] + p[3] / 100.0) / 12.0);
        let mix = p[4];
        for i in 0..l.len() {
            let vol = self.volume.next();
            let mut acc = 0.0;
            for (vi, v) in self.voices.iter_mut().enumerate() {
                if !v.amp.is_active() {
                    continue;
                }
                let f = midi_to_hz(v.note as f32);
                let s = v.osc1.next(w1, f, sr) * (1.0 - mix) + v.osc2.next(w2, f * osc2_ratio, sr) * mix;
                let fe = v.fenv.next();
                // Update filter coefficients every 8 samples per voice (staggered).
                if (i + vi) & 7 == 0 {
                    let cutoff = p[5] * 2f32.powf(p[7] * fe);
                    v.filter.set(sr, cutoff, p[6]);
                }
                acc += v.filter.process(s, SvfMode::LowPass) * v.amp.next() * v.gain;
            }
            let out = acc * vol * 0.5;
            l[i] += out;
            r[i] += out;
        }
    }
}

// ---------------------------------------------------------------- Sampler

pub const SAMPLER_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("root", "Root Key", 0.0, 127.0, 60.0, ""),
    ParamDesc::lin("transpose", "Transpose", -48.0, 48.0, 0.0, "st"),
    ParamDesc::lin("fine", "Detune", -50.0, 50.0, 0.0, "ct"),
    ParamDesc::log("attack", "Attack", 0.5, 5_000.0, 1.0, "ms"),
    ParamDesc::log("decay", "Decay", 1.0, 20_000.0, 1_000.0, "ms"),
    ParamDesc::lin("sustain", "Sustain", 0.0, 1.0, 1.0, ""),
    ParamDesc::log("release", "Release", 1.0, 10_000.0, 50.0, "ms"),
    ParamDesc::lin("vel_sens", "Velocity", 0.0, 1.0, 0.5, ""),
    ParamDesc::lin("volume", "Volume", -36.0, 6.0, -6.0, "dB"),
];

#[derive(Clone, Copy, Default)]
struct SampleVoice {
    note: u8,
    held: bool,
    sustained: bool,
    pos: f64,
    rate: f64,
    gain: f32,
    pan: (f32, f32),
    age: u64,
    amp: Adsr,
}

impl SampleVoice {
    #[inline]
    fn tick(&mut self, sample: &SampleData) -> (f32, f32) {
        if !self.amp.is_active() {
            return (0.0, 0.0);
        }
        let (a, b) = sample.read(self.pos);
        self.pos += self.rate;
        if self.pos >= sample.frames() as f64 {
            self.amp = Adsr::default();
            return (0.0, 0.0);
        }
        let g = self.amp.next() * self.gain;
        (a * g * self.pan.0, b * g * self.pan.1)
    }
}

pub struct Sampler {
    params: [f32; SAMPLER_PARAMS.len()],
    sample: Option<Arc<SampleData>>,
    voices: [SampleVoice; MAX_VOICES],
    sustain: bool,
    counter: u64,
    volume: Smoothed,
    sr: f32,
}

impl Sampler {
    pub fn new(sr: f32, sample: Option<Arc<SampleData>>) -> Self {
        let mut params = [0.0; SAMPLER_PARAMS.len()];
        for (p, d) in params.iter_mut().zip(SAMPLER_PARAMS) {
            *p = d.default;
        }
        let volume = Smoothed::new(db_to_gain(params[8]), sr, 20.0);
        Self { params, sample, voices: [SampleVoice::default(); MAX_VOICES], sustain: false, counter: 0, volume, sr }
    }

    /// Replaces the sample; returns the previous one so the caller can drop it off the audio thread.
    pub fn swap_sample(&mut self, sample: Option<Arc<SampleData>>) -> Option<Arc<SampleData>> {
        for v in self.voices.iter_mut() {
            v.amp = Adsr::default();
        }
        std::mem::replace(&mut self.sample, sample)
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = SAMPLER_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
            if i == 8 {
                self.volume.set_target(db_to_gain(self.params[8]));
            }
        }
    }

    fn apply(&mut self, kind: &NoteKind) {
        let Some(sample) = self.sample.as_ref() else { return };
        let p = self.params;
        match *kind {
            NoteKind::On { pitch, velocity } if velocity > 0 => {
                let idx = self
                    .voices
                    .iter()
                    .position(|v| !v.amp.is_active())
                    .unwrap_or_else(|| (0..MAX_VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
                self.counter += 1;
                let semis = pitch as f32 - p[0] + p[1] + p[2] / 100.0;
                let vel = velocity as f32 / 127.0;
                let v = &mut self.voices[idx];
                *v = SampleVoice {
                    note: pitch,
                    held: true,
                    sustained: false,
                    pos: 0.0,
                    rate: 2f64.powf(semis as f64 / 12.0) * sample.sample_rate / self.sr as f64,
                    gain: 1.0 - p[7] + p[7] * vel,
                    pan: (1.0, 1.0),
                    age: self.counter,
                    amp: Adsr::default(),
                };
                v.amp.set(self.sr, p[3], p[4], p[5], p[6]);
                v.amp.gate_on();
            }
            NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
                let sustain = self.sustain;
                for v in self.voices.iter_mut().filter(|v| v.held && v.note == pitch) {
                    v.held = false;
                    if sustain { v.sustained = true } else { v.amp.gate_off() }
                }
            }
            NoteKind::Sustain(on) => {
                self.sustain = on;
                if !on {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.amp.gate_off();
                    }
                }
            }
            NoteKind::AllOff => {
                self.sustain = false;
                for v in self.voices.iter_mut() {
                    v.held = false;
                    v.sustained = false;
                    v.amp.fast_release(self.sr);
                }
            }
        }
    }

    pub fn render(&mut self, events: &[NoteEvent], l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        self.sr = ctx.sample_rate;
        event_steps(events, l.len(), |step| match step {
            Step::Event(k) => self.apply(k),
            Step::Segment(a, b) => {
                let Some(sample) = self.sample.as_deref() else { return };
                for i in a..b {
                    let vol = self.volume.next();
                    for v in self.voices.iter_mut() {
                        let (x, y) = v.tick(sample);
                        l[i] += x * vol;
                        r[i] += y * vol;
                    }
                }
            }
        });
    }
}

// ---------------------------------------------------------------- Drum rack

pub const PADS: usize = 16;
/// First pad note (C1 in the C3 = 60 convention).
pub const FIRST_PAD_NOTE: u8 = 36;
pub const PAD_PARAMS_EACH: usize = 4;

const fn pad_params() -> [ParamDesc; PADS * PAD_PARAMS_EACH] {
    const NAMES: [[&str; 4]; PADS] = {
        let mut out = [["", "", "", ""]; PADS];
        let ids: [[&str; 4]; PADS] = [
            ["p1_gain", "p1_pan", "p1_tune", "p1_decay"],
            ["p2_gain", "p2_pan", "p2_tune", "p2_decay"],
            ["p3_gain", "p3_pan", "p3_tune", "p3_decay"],
            ["p4_gain", "p4_pan", "p4_tune", "p4_decay"],
            ["p5_gain", "p5_pan", "p5_tune", "p5_decay"],
            ["p6_gain", "p6_pan", "p6_tune", "p6_decay"],
            ["p7_gain", "p7_pan", "p7_tune", "p7_decay"],
            ["p8_gain", "p8_pan", "p8_tune", "p8_decay"],
            ["p9_gain", "p9_pan", "p9_tune", "p9_decay"],
            ["p10_gain", "p10_pan", "p10_tune", "p10_decay"],
            ["p11_gain", "p11_pan", "p11_tune", "p11_decay"],
            ["p12_gain", "p12_pan", "p12_tune", "p12_decay"],
            ["p13_gain", "p13_pan", "p13_tune", "p13_decay"],
            ["p14_gain", "p14_pan", "p14_tune", "p14_decay"],
            ["p15_gain", "p15_pan", "p15_tune", "p15_decay"],
            ["p16_gain", "p16_pan", "p16_tune", "p16_decay"],
        ];
        let mut i = 0;
        while i < PADS {
            out[i] = ids[i];
            i += 1;
        }
        out
    };
    let mut out = [ParamDesc::lin("", "", 0.0, 1.0, 0.0, ""); PADS * PAD_PARAMS_EACH];
    let mut i = 0;
    while i < PADS {
        out[i * 4] = ParamDesc::lin(NAMES[i][0], "Gain", -36.0, 6.0, 0.0, "dB");
        out[i * 4 + 1] = ParamDesc::lin(NAMES[i][1], "Pan", -1.0, 1.0, 0.0, "");
        out[i * 4 + 2] = ParamDesc::lin(NAMES[i][2], "Tune", -24.0, 24.0, 0.0, "st");
        out[i * 4 + 3] = ParamDesc::log(NAMES[i][3], "Decay", 10.0, 10_000.0, 10_000.0, "ms");
        i += 1;
    }
    out
}

pub const DRUM_PARAMS: &[ParamDesc] = &pad_params();
const VOICES_PER_PAD: usize = 4;

pub struct DrumRack {
    params: [f32; PADS * PAD_PARAMS_EACH],
    samples: [Option<Arc<SampleData>>; PADS],
    voices: [[SampleVoice; VOICES_PER_PAD]; PADS],
    counter: u64,
    sr: f32,
}

impl DrumRack {
    pub fn new(sr: f32, samples: [Option<Arc<SampleData>>; PADS]) -> Self {
        let mut params = [0.0; PADS * PAD_PARAMS_EACH];
        for (p, d) in params.iter_mut().zip(DRUM_PARAMS) {
            *p = d.default;
        }
        Self { params, samples, voices: [[SampleVoice::default(); VOICES_PER_PAD]; PADS], counter: 0, sr }
    }

    pub fn swap_samples(&mut self, samples: &mut [Option<Arc<SampleData>>; PADS]) {
        std::mem::swap(&mut self.samples, samples);
        for pad in self.voices.iter_mut() {
            for v in pad.iter_mut() {
                v.amp = Adsr::default();
            }
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        if let Some(d) = DRUM_PARAMS.get(i) {
            self.params[i] = d.clamp(v);
        }
    }

    fn apply(&mut self, kind: &NoteKind) {
        match *kind {
            NoteKind::On { pitch, velocity } if velocity > 0 => {
                let Some(pad) = (pitch as usize).checked_sub(FIRST_PAD_NOTE as usize).filter(|&p| p < PADS) else {
                    return;
                };
                let Some(sample) = self.samples[pad].as_ref() else { return };
                let base = pad * PAD_PARAMS_EACH;
                let (gain_db, pan, tune, decay) =
                    (self.params[base], self.params[base + 1], self.params[base + 2], self.params[base + 3]);
                let voices = &mut self.voices[pad];
                let idx = voices
                    .iter()
                    .position(|v| !v.amp.is_active())
                    .unwrap_or_else(|| (0..VOICES_PER_PAD).min_by_key(|&i| voices[i].age).unwrap_or(0));
                // Retriggering a pad fades its older voices quickly to avoid smearing.
                for v in voices.iter_mut() {
                    if v.amp.is_active() {
                        v.amp.fast_release(self.sr);
                    }
                }
                self.counter += 1;
                let vel = velocity as f32 / 127.0;
                let v = &mut voices[idx];
                *v = SampleVoice {
                    note: pitch,
                    held: false,
                    sustained: false,
                    pos: 0.0,
                    rate: 2f64.powf(tune as f64 / 12.0) * sample.sample_rate / self.sr as f64,
                    gain: db_to_gain(gain_db) * (0.3 + 0.7 * vel),
                    pan: pan_gains(pan),
                    age: self.counter,
                    amp: Adsr::default(),
                };
                v.amp.set(self.sr, 0.3, decay, 0.0, decay.min(200.0));
                if decay >= 9_999.0 {
                    v.amp.set(self.sr, 0.3, 1.0, 1.0, 30.0);
                }
                v.amp.gate_on();
            }
            NoteKind::AllOff => {
                for pad in self.voices.iter_mut() {
                    for v in pad.iter_mut() {
                        v.amp.fast_release(self.sr);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn render(&mut self, events: &[NoteEvent], l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        self.sr = ctx.sample_rate;
        event_steps(events, l.len(), |step| match step {
            Step::Event(k) => self.apply(k),
            Step::Segment(a, b) => {
                for (pad, voices) in self.voices.iter_mut().enumerate() {
                    let Some(sample) = self.samples[pad].as_deref() else { continue };
                    for v in voices.iter_mut().filter(|v| v.amp.is_active()) {
                        for i in a..b {
                            let (x, y) = v.tick(sample);
                            l[i] += x;
                            r[i] += y;
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: Ctx = Ctx { sample_rate: 48_000.0, bpm: 120.0 };

    fn on(offset: u32, pitch: u8) -> NoteEvent {
        NoteEvent { offset, kind: NoteKind::On { pitch, velocity: 100 } }
    }

    fn off(offset: u32, pitch: u8) -> NoteEvent {
        NoteEvent { offset, kind: NoteKind::Off { pitch } }
    }

    #[test]
    fn synth_note_starts_at_event_offset_and_releases() {
        let mut s = Synth::new(48_000.0);
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        s.render(&[on(100, 60)], &mut l, &mut r, &CTX);
        assert!(l[..100].iter().all(|&x| x == 0.0));
        assert!(l[100..].iter().any(|&x| x != 0.0));
        s.render(&[off(0, 60)], &mut vec![0.0; 512], &mut vec![0.0; 512], &CTX);
        for _ in 0..200 {
            s.render(&[], &mut vec![0.0; 512], &mut vec![0.0; 512], &CTX);
        }
        assert_eq!(s.active_voices(), 0, "release must finish: no stuck notes");
    }

    #[test]
    fn sustain_pedal_holds_until_released_and_all_off_clears() {
        let mut s = Synth::new(48_000.0);
        let buf = || (vec![0.0; 256], vec![0.0; 256]);
        let (mut l, mut r) = buf();
        s.render(
            &[NoteEvent { offset: 0, kind: NoteKind::Sustain(true) }, on(0, 60), off(10, 60)],
            &mut l,
            &mut r,
            &CTX,
        );
        for _ in 0..400 {
            let (mut l, mut r) = buf();
            s.render(&[], &mut l, &mut r, &CTX);
        }
        assert_eq!(s.active_voices(), 1, "sustained voice keeps sounding");
        let (mut l, mut r) = buf();
        s.render(&[NoteEvent { offset: 0, kind: NoteKind::AllOff }], &mut l, &mut r, &CTX);
        for _ in 0..10 {
            let (mut l, mut r) = buf();
            s.render(&[], &mut l, &mut r, &CTX);
        }
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn voice_stealing_keeps_output_finite_and_bounded() {
        let mut s = Synth::new(48_000.0);
        let events: Vec<_> = (0..40).map(|i| on(i * 10, 40 + i as u8)).collect();
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        s.render(&events, &mut l, &mut r, &CTX);
        assert!(l.iter().all(|x| x.is_finite() && x.abs() < 8.0));
        assert_eq!(s.active_voices(), MAX_VOICES);
    }

    #[test]
    fn sampler_plays_at_root_rate_and_octave_up() {
        let data = Arc::new(SampleData { channels: vec![(0..1000).map(|i| i as f32 / 1000.0).collect()], sample_rate: 48_000.0 });
        let mut s = Sampler::new(48_000.0, Some(data.clone()));
        s.set_param(3, 0.5); // fast attack
        s.set_param(8, 0.0);
        let (mut l, mut r) = (vec![0.0; 2000], vec![0.0; 2000]);
        s.render(&[on(0, 72)], &mut l, &mut r, &CTX);
        // Octave up consumes 1000 frames in ~500 output samples.
        assert!(l[490] > 0.5 && l[520] == 0.0, "{} {}", l[490], l[520]);
    }

    #[test]
    fn drum_pad_maps_note_to_pad() {
        let click = Arc::new(SampleData { channels: vec![vec![1.0; 64]], sample_rate: 48_000.0 });
        let mut samples: [Option<Arc<SampleData>>; PADS] = Default::default();
        samples[1] = Some(click);
        let mut d = DrumRack::new(48_000.0, samples);
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        d.render(&[on(0, 36)], &mut l, &mut r, &CTX);
        assert!(l.iter().all(|&x| x == 0.0), "pad 1 is empty");
        d.render(&[on(10, 37)], &mut l, &mut r, &CTX);
        assert_eq!(l[9], 0.0);
        assert!(l[12] > 0.0);
    }
}
