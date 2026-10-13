//! Device kinds and enum dispatch (no trait objects on the audio thread).

use crate::effects::*;
use crate::instruments::*;
use crate::params::{Ctx, NoteEvent, ParamDesc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    PolySynth,
    Sampler,
    DrumRack,
    Gain,
    Eq3,
    Filter,
    Compressor,
    Delay,
    Reverb,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 9] = [
        DeviceKind::PolySynth,
        DeviceKind::Sampler,
        DeviceKind::DrumRack,
        DeviceKind::Gain,
        DeviceKind::Eq3,
        DeviceKind::Filter,
        DeviceKind::Compressor,
        DeviceKind::Delay,
        DeviceKind::Reverb,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::PolySynth => "Poly",
            DeviceKind::Sampler => "Sampler",
            DeviceKind::DrumRack => "Drum Rack",
            DeviceKind::Gain => "Utility",
            DeviceKind::Eq3 => "EQ Three-Band",
            DeviceKind::Filter => "Filter",
            DeviceKind::Compressor => "Compressor",
            DeviceKind::Delay => "Echo Delay",
            DeviceKind::Reverb => "Room Reverb",
        }
    }

    pub fn is_instrument(self) -> bool {
        matches!(self, DeviceKind::PolySynth | DeviceKind::Sampler | DeviceKind::DrumRack)
    }

    pub fn params(self) -> &'static [ParamDesc] {
        match self {
            DeviceKind::PolySynth => SYNTH_PARAMS,
            DeviceKind::Sampler => SAMPLER_PARAMS,
            DeviceKind::DrumRack => DRUM_PARAMS,
            DeviceKind::Gain => GAIN_PARAMS,
            DeviceKind::Eq3 => EQ_PARAMS,
            DeviceKind::Filter => FILTER_PARAMS,
            DeviceKind::Compressor => COMP_PARAMS,
            DeviceKind::Delay => DELAY_PARAMS,
            DeviceKind::Reverb => REVERB_PARAMS,
        }
    }

    /// Samples of latency introduced; all built-in devices are zero-latency.
    /// The mixer compensates non-zero values (plugin hosting) per track.
    pub fn latency_samples(self) -> usize {
        0
    }
}

pub enum Device {
    PolySynth(Box<Synth>),
    Sampler(Box<Sampler>),
    DrumRack(Box<DrumRack>),
    Gain(Box<GainFx>),
    Eq3(Box<Eq3>),
    Filter(Box<FilterFx>),
    Compressor(Box<Compressor>),
    Delay(Box<Delay>),
    Reverb(Box<Reverb>),
}

/// Media a device needs at construction (sampler and drum rack).
#[derive(Default, Clone)]
pub struct DeviceMedia {
    pub sample: Option<Arc<SampleData>>,
    pub pads: [Option<Arc<SampleData>>; PADS],
}

impl Device {
    /// Allocates; call off the audio thread.
    pub fn new(kind: DeviceKind, sr: f32, media: DeviceMedia) -> Self {
        match kind {
            DeviceKind::PolySynth => Device::PolySynth(Box::new(Synth::new(sr))),
            DeviceKind::Sampler => Device::Sampler(Box::new(Sampler::new(sr, media.sample))),
            DeviceKind::DrumRack => Device::DrumRack(Box::new(DrumRack::new(sr, media.pads))),
            DeviceKind::Gain => Device::Gain(Box::new(GainFx::new(sr))),
            DeviceKind::Eq3 => Device::Eq3(Box::new(Eq3::new(sr))),
            DeviceKind::Filter => Device::Filter(Box::new(FilterFx::new(sr))),
            DeviceKind::Compressor => Device::Compressor(Box::new(Compressor::new(sr))),
            DeviceKind::Delay => Device::Delay(Box::new(Delay::new(sr))),
            DeviceKind::Reverb => Device::Reverb(Box::new(Reverb::new(sr))),
        }
    }

    pub fn kind(&self) -> DeviceKind {
        match self {
            Device::PolySynth(_) => DeviceKind::PolySynth,
            Device::Sampler(_) => DeviceKind::Sampler,
            Device::DrumRack(_) => DeviceKind::DrumRack,
            Device::Gain(_) => DeviceKind::Gain,
            Device::Eq3(_) => DeviceKind::Eq3,
            Device::Filter(_) => DeviceKind::Filter,
            Device::Compressor(_) => DeviceKind::Compressor,
            Device::Delay(_) => DeviceKind::Delay,
            Device::Reverb(_) => DeviceKind::Reverb,
        }
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        match self {
            Device::PolySynth(d) => d.set_param(i, v),
            Device::Sampler(d) => d.set_param(i, v),
            Device::DrumRack(d) => d.set_param(i, v),
            Device::Gain(d) => d.set_param(i, v),
            Device::Eq3(d) => d.set_param(i, v),
            Device::Filter(d) => d.set_param(i, v),
            Device::Compressor(d) => d.set_param(i, v),
            Device::Delay(d) => d.set_param(i, v),
            Device::Reverb(d) => d.set_param(i, v),
        }
    }

    /// Instruments add their output to `l`/`r`; effects transform `l`/`r` in place.
    pub fn process(&mut self, events: &[NoteEvent], l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        match self {
            Device::PolySynth(d) => d.render(events, l, r, ctx),
            Device::Sampler(d) => d.render(events, l, r, ctx),
            Device::DrumRack(d) => d.render(events, l, r, ctx),
            Device::Gain(d) => d.process(l, r),
            Device::Eq3(d) => d.process(l, r),
            Device::Filter(d) => d.process(l, r),
            Device::Compressor(d) => d.process(l, r),
            Device::Delay(d) => d.process(l, r, ctx),
            Device::Reverb(d) => d.process(l, r, ctx),
        }
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
            (Device::DrumRack(a), Device::DrumRack(b)) => {
                let mut tmp: [Option<Arc<SampleData>>; PADS] = Default::default();
                b.swap_samples(&mut tmp); // tmp = new media, b empty
                a.swap_samples(&mut tmp); // a = new media, tmp = old
                b.swap_samples(&mut tmp); // b holds old media
            }
            _ => {}
        }
    }

    pub fn gain_reduction_db(&self) -> f32 {
        match self {
            Device::Compressor(c) => c.gain_reduction_db,
            _ => 0.0,
        }
    }
}
