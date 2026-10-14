//! Device kinds and enum dispatch. Devices live in an enum (no boxed trait
//! objects); calls go through `&mut dyn Processor` borrowed from it.
//!
//! Every device is listed once in `devices!` below; the macro generates
//! `DeviceKind`, `Device` and their dispatch. Each device type implements
//! `Processor`. Chain rules: docs/specs/devices.md D1.

use crate::dynamics::*;
use crate::effects::*;
use crate::instruments::*;
use crate::midi_fx::*;
use crate::modulation::*;
use crate::params::{Ctx, NoteEvent, ParamDesc};
use crate::synths::*;
use crate::tone::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where a device sits in a chain (spec D1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    MidiEffect,
    Instrument,
    AudioEffect,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::MidiEffect => "MIDI Effects",
            Category::Instrument => "Instruments",
            Category::AudioEffect => "Audio Effects",
        }
    }
}

/// Audio buffers and input events of one processing call.
pub struct Io<'a> {
    pub events: &'a [NoteEvent],
    pub l: &'a mut [f32],
    pub r: &'a mut [f32],
    /// Detector input from another track (post-devices, pre-fader), if routed.
    pub sidechain: Option<(&'a [f32], &'a [f32])>,
}

/// Implemented by every device type. Instruments add to `io.l`/`io.r`, audio
/// effects transform them in place, MIDI effects implement `process_midi`.
pub trait Processor {
    fn set_param(&mut self, i: usize, v: f32);
    fn param_values(&self) -> &[f32];
    fn process(&mut self, _io: Io, _ctx: &Ctx) {}
    /// Transforms `input` (events of an `n`-sample block) into `out`; `out` is
    /// empty and has the same capacity as `input`'s buffer. Must never push past
    /// the capacity.
    fn process_midi(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, _n: usize, _ctx: &Ctx) {
        push_all(out, input);
    }
    fn gain_reduction_db(&self) -> f32 {
        0.0
    }
    /// Samples of latency this device adds with its current parameters.
    fn latency_samples(&self) -> usize {
        0
    }
}

/// Pushes as many events as capacity allows (never reallocates).
pub fn push_all(out: &mut Vec<NoteEvent>, events: &[NoteEvent]) {
    let room = out.capacity() - out.len();
    out.extend_from_slice(&events[..events.len().min(room)]);
}

macro_rules! devices {
    ($( $(#[$m:meta])* $kind:ident($ty:ty) = $name:literal, $params:ident, $cat:ident, |$sr:ident, $media:ident| $ctor:expr; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum DeviceKind {
            $( $(#[$m])* $kind, )*
        }

        impl DeviceKind {
            pub const ALL: &'static [DeviceKind] = &[$(DeviceKind::$kind),*];

            pub fn name(self) -> &'static str {
                match self { $(DeviceKind::$kind => $name,)* }
            }

            pub fn params(self) -> &'static [ParamDesc] {
                match self { $(DeviceKind::$kind => $params,)* }
            }

            pub fn category(self) -> Category {
                match self { $(DeviceKind::$kind => Category::$cat,)* }
            }
        }

        pub enum Device {
            $( $kind(Box<$ty>), )*
        }

        impl Device {
            /// Allocates; call off the audio thread.
            pub fn new(kind: DeviceKind, sr: f32, media: DeviceMedia) -> Self {
                #[allow(unused_variables)]
                match kind {
                    $(DeviceKind::$kind => { let ($sr, $media) = (sr, media); Device::$kind(Box::new($ctor)) })*
                }
            }

            pub fn kind(&self) -> DeviceKind {
                match self { $(Device::$kind(_) => DeviceKind::$kind,)* }
            }

            fn processor(&mut self) -> &mut dyn Processor {
                match self { $(Device::$kind(d) => &mut **d,)* }
            }

            fn processor_ref(&self) -> &dyn Processor {
                match self { $(Device::$kind(d) => &**d,)* }
            }
        }
    };
}

devices! {
    PolySynth(Synth) = "Poly", SYNTH_PARAMS, Instrument, |sr, m| Synth::new(sr);
    Sampler(Sampler) = "Sampler", SAMPLER_PARAMS, Instrument, |sr, m| Sampler::new(sr, m.sample);
    /// Projects written before 2026-10-13 store this kind under its former name (alias below).
    #[serde(alias = "DrumRack")]
    DrumKit(DrumKit) = "Drum Kit", DRUM_PARAMS, Instrument, |sr, m| DrumKit::new(sr, m.pads);
    FmSynth(FmSynth) = "FM Synth", FM_PARAMS, Instrument, |sr, m| FmSynth::new(sr);
    MorphSynth(MorphSynth) = "Morph Synth", MORPH_PARAMS, Instrument, |sr, m| MorphSynth::new(sr);
    Gain(GainFx) = "Utility", GAIN_PARAMS, AudioEffect, |sr, m| GainFx::new(sr);
    Eq3(Eq3) = "EQ Three-Band", EQ_PARAMS, AudioEffect, |sr, m| Eq3::new(sr);
    ParamEq(ParamEq) = "Parametric EQ", PEQ_PARAMS, AudioEffect, |sr, m| ParamEq::new(sr);
    Filter(FilterFx) = "Filter", FILTER_PARAMS, AudioEffect, |sr, m| FilterFx::new(sr);
    MotionFilter(MotionFilter) = "Motion Filter", MOTION_PARAMS, AudioEffect, |sr, m| MotionFilter::new(sr);
    Compressor(Compressor) = "Compressor", COMP_PARAMS, AudioEffect, |sr, m| Compressor::new(sr);
    BusComp(BusComp) = "Bus Compressor", BUS_PARAMS, AudioEffect, |sr, m| BusComp::new(sr);
    Multiband(Multiband) = "Multiband Compressor", MB_PARAMS, AudioEffect, |sr, m| Multiband::new(sr);
    Gate(Gate) = "Gate", GATE_PARAMS, AudioEffect, |sr, m| Gate::new(sr);
    Limiter(Limiter) = "Limiter", LIMITER_PARAMS, AudioEffect, |sr, m| Limiter::new(sr);
    Saturator(Saturator) = "Saturator", SAT_PARAMS, AudioEffect, |sr, m| Saturator::new(sr);
    Chorus(Chorus) = "Chorus", CHORUS_PARAMS, AudioEffect, |sr, m| Chorus::new(sr);
    Phaser(Phaser) = "Phaser", PHASER_PARAMS, AudioEffect, |sr, m| Phaser::new(sr);
    Delay(Delay) = "Echo Delay", DELAY_PARAMS, AudioEffect, |sr, m| Delay::new(sr);
    Reverb(Reverb) = "Room Reverb", REVERB_PARAMS, AudioEffect, |sr, m| Reverb::new(sr);
    Arpeggiator(Arpeggiator) = "Arpeggiator", ARP_PARAMS, MidiEffect, |sr, m| Arpeggiator::new(sr);
    ChordStack(ChordStack) = "Chord Stack", CHORD_PARAMS, MidiEffect, |sr, m| ChordStack::new();
    ScaleLock(ScaleLock) = "Scale Lock", SCALE_PARAMS, MidiEffect, |sr, m| ScaleLock::new();
    NoteDuration(NoteDuration) = "Note Duration", DURATION_PARAMS, MidiEffect, |sr, m| NoteDuration::new(sr);
    PitchRandom(PitchRandom) = "Pitch Randomizer", RANDOM_PARAMS, MidiEffect, |sr, m| PitchRandom::new();
    VelocityShaper(VelocityShaper) = "Velocity Shaper", VELOCITY_PARAMS, MidiEffect, |sr, m| VelocityShaper::new();
}

impl DeviceKind {
    pub fn is_instrument(self) -> bool {
        self.category() == Category::Instrument
    }

    pub fn is_midi_effect(self) -> bool {
        self.category() == Category::MidiEffect
    }

    /// MIDI effects and instruments only make sense on MIDI tracks.
    pub fn needs_midi_track(self) -> bool {
        self.category() != Category::AudioEffect
    }

    /// Kinds whose detector can listen to another track (spec D3.1).
    pub fn supports_sidechain(self) -> bool {
        matches!(self, DeviceKind::Compressor | DeviceKind::BusComp | DeviceKind::Gate)
    }
}

/// Media a device needs at construction (sampler and drum kit).
#[derive(Default, Clone)]
pub struct DeviceMedia {
    pub sample: Option<Arc<SampleData>>,
    pub pads: [Option<Arc<SampleData>>; PADS],
}

impl Device {
    pub fn set_param(&mut self, i: usize, v: f32) {
        self.processor().set_param(i, v);
    }

    pub fn param_values(&self) -> &[f32] {
        self.processor_ref().param_values()
    }

    /// Applies every parameter of `other` (same kind) to self; no allocation.
    pub fn copy_params_from(&mut self, other: &Device) {
        if self.kind() != other.kind() {
            return;
        }
        let n = other.param_values().len();
        for i in 0..n {
            let v = other.param_values()[i];
            self.set_param(i, v);
        }
    }

    /// Instruments add their output to `io.l`/`io.r`; audio effects transform them in place.
    pub fn process(&mut self, io: Io, ctx: &Ctx) {
        self.processor().process(io, ctx);
    }

    pub fn process_midi(&mut self, input: &[NoteEvent], out: &mut Vec<NoteEvent>, n: usize, ctx: &Ctx) {
        self.processor().process_midi(input, out, n, ctx);
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.processor_ref().gain_reduction_db()
    }

    pub fn latency_samples(&self) -> usize {
        self.processor_ref().latency_samples()
    }

    /// Moves media from `other` into self (used when a rebuilt graph keeps a
    /// running device): self keeps its voice state, `other` receives the old media
    /// so it is dropped with the retired graph off the audio thread.
    pub fn exchange_media(&mut self, other: &mut Device) {
        match (self, other) {
            (Device::Sampler(a), Device::Sampler(b)) => {
                let new = b.swap_sample(None);
                let old = a.swap_sample(new);
                b.swap_sample(old);
            }
            (Device::DrumKit(a), Device::DrumKit(b)) => {
                let mut tmp: [Option<Arc<SampleData>>; PADS] = Default::default();
                b.swap_samples(&mut tmp); // tmp = new media, b empty
                a.swap_samples(&mut tmp); // a = new media, tmp = old
                b.swap_samples(&mut tmp); // b holds old media
            }
            _ => {}
        }
    }
}

/// Implements `Processor` for a device type with inherent `set_param`,
/// `param_values` and a processing method adapted by `$body`.
macro_rules! processor {
    ($ty:ty, |$s:ident, $io:ident, $ctx:ident| $body:expr $(, gr = $gr:ident)?) => {
        impl Processor for $ty {
            fn set_param(&mut self, i: usize, v: f32) {
                <$ty>::set_param(self, i, v)
            }
            fn param_values(&self) -> &[f32] {
                <$ty>::param_values(self)
            }
            #[allow(unused_variables)]
            fn process(&mut self, $io: Io, $ctx: &Ctx) {
                let $s = self;
                $body
            }
            $(fn gain_reduction_db(&self) -> f32 {
                self.$gr
            })?
        }
    };
}
pub(crate) use processor;

processor!(Synth, |s, io, ctx| s.render(io.events, io.l, io.r, ctx));
processor!(Sampler, |s, io, ctx| s.render(io.events, io.l, io.r, ctx));
processor!(DrumKit, |s, io, ctx| s.render(io.events, io.l, io.r, ctx));
processor!(GainFx, |s, io, ctx| s.process(io.l, io.r));
processor!(Eq3, |s, io, ctx| s.process(io.l, io.r));
processor!(FilterFx, |s, io, ctx| s.process(io.l, io.r));
processor!(Delay, |s, io, ctx| s.process(io.l, io.r, ctx));
processor!(Reverb, |s, io, ctx| s.process(io.l, io.r, ctx));
