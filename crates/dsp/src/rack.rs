//! Racks: parallel chains of devices (spec: docs/specs/devices.md D6).
//!
//! A rack's parameters form one flat index space, laid out by `RackLayout`:
//! for each chain, its `CHAIN_PARAMS`, then the parameters of each of its
//! devices in order. The engine, the graph compiler and the project sync all
//! address rack parameters through this layout.

use crate::device::{run_chain, ChainSlot, Device, DeviceKind, Io, Processor};
use crate::params::{Ctx, NoteEvent, NoteKind, ParamDesc};
use crate::util::{db_to_gain, pan_gains, Smoothed};

pub const CHAIN_PARAMS: &[ParamDesc] = &[
    ParamDesc::lin("volume", "Volume", -70.0, 6.0, 0.0, "dB"),
    ParamDesc::lin("pan", "Pan", -1.0, 1.0, 0.0, ""),
    ParamDesc::toggle("mute", "Mute", false),
    ParamDesc::toggle("solo", "Solo", false),
    ParamDesc::int("key_lo", "Key Low", 0.0, 127.0, 0.0, ""),
    ParamDesc::int("key_hi", "Key High", 0.0, 127.0, 127.0, ""),
    ParamDesc::int("vel_lo", "Vel Low", 1.0, 127.0, 1.0, ""),
    ParamDesc::int("vel_hi", "Vel High", 1.0, 127.0, 127.0, ""),
];
const VOLUME: usize = 0;
const PAN: usize = 1;
const MUTE: usize = 2;
const SOLO: usize = 3;
const KEY_LO: usize = 4;
const KEY_HI: usize = 5;
const VEL_LO: usize = 6;
const VEL_HI: usize = 7;

/// Where each chain's and device's parameters start in a rack's flat index space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RackLayout {
    /// `chains[c] = (chain base, [device bases])`.
    chains: Vec<(usize, Vec<usize>)>,
    total: usize,
}

/// What a flat rack parameter index refers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RackParam {
    Chain { chain: usize, param: usize },
    Device { chain: usize, device: usize, param: usize },
}

impl RackLayout {
    pub fn new<'a>(chains: impl IntoIterator<Item = &'a [DeviceKind]>) -> Self {
        let mut total = 0;
        let chains = chains
            .into_iter()
            .map(|kinds| {
                let base = total;
                total += CHAIN_PARAMS.len();
                let devices = kinds
                    .iter()
                    .map(|k| {
                        let b = total;
                        total += k.params().len();
                        b
                    })
                    .collect();
                (base, devices)
            })
            .collect();
        Self { chains, total }
    }

    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn chain_param(&self, chain: usize, param: usize) -> usize {
        self.chains[chain].0 + param
    }

    pub fn device_param(&self, chain: usize, device: usize, param: usize) -> usize {
        self.chains[chain].1[device] + param
    }

    pub fn locate(&self, flat: usize) -> Option<RackParam> {
        let c = self.chains.iter().rposition(|(b, _)| *b <= flat)?;
        let (base, devices) = &self.chains[c];
        if flat < base + CHAIN_PARAMS.len() {
            return Some(RackParam::Chain { chain: c, param: flat - base });
        }
        let d = devices.iter().rposition(|b| *b <= flat)?;
        Some(RackParam::Device { chain: c, device: d, param: flat - devices[d] })
    }
}

/// One device inside a chain.
pub struct RackSlot {
    pub device: Device,
    pub bypass: bool,
}

impl ChainSlot for RackSlot {
    fn device(&mut self) -> &mut Device {
        &mut self.device
    }
    fn bypassed(&self) -> bool {
        self.bypass
    }
}

pub struct RackChain {
    slots: Vec<RackSlot>,
    params: [f32; 8],
    gain_l: Smoothed,
    gain_r: Smoothed,
    l: Vec<f32>,
    r: Vec<f32>,
    events: Vec<NoteEvent>,
    events_b: Vec<NoteEvent>,
    /// Pitches whose note-on this chain received (note-offs follow them).
    sounding: u128,
}

impl RackChain {
    fn audible(&self, any_solo: bool) -> bool {
        self.params[MUTE] < 0.5 && (!any_solo || self.params[SOLO] >= 0.5)
    }

    fn update_gain(&mut self) {
        let (pl, pr) = pan_gains(self.params[PAN]);
        let g = if self.params[VOLUME] <= CHAIN_PARAMS[VOLUME].min { 0.0 } else { db_to_gain(self.params[VOLUME]) * std::f32::consts::SQRT_2 };
        self.gain_l.set_target(g * pl);
        self.gain_r.set_target(g * pr);
    }

    /// Does a note-on with this pitch and velocity reach the chain (spec D6.3)?
    pub fn accepts(&self, pitch: u8, velocity: u8) -> bool {
        let p = &self.params;
        (p[KEY_LO]..=p[KEY_HI]).contains(&(pitch as f32)) && (p[VEL_LO]..=p[VEL_HI]).contains(&(velocity as f32))
    }
}

pub struct Rack {
    instrument: bool,
    chains: Vec<RackChain>,
    layout: RackLayout,
    flat: Vec<f32>,
    sr: f32,
    block: usize,
    max_events: usize,
}

impl Rack {
    /// An empty rack; chains are added with `push_chain` (off the audio thread).
    pub fn new(sr: f32, instrument: bool) -> Self {
        Self { instrument, chains: vec![], layout: RackLayout::default(), flat: vec![], sr, block: 256, max_events: 1024 }
    }

    /// Sets the largest block and event count the rack will be given (buffer sizes).
    pub fn set_capacity(&mut self, block: usize, events: usize) {
        self.block = block;
        self.max_events = events;
        for c in self.chains.iter_mut() {
            c.l.resize(block, 0.0);
            c.r.resize(block, 0.0);
            c.events.reserve(events.saturating_sub(c.events.capacity()));
            c.events_b.reserve(events.saturating_sub(c.events_b.capacity()));
        }
    }

    /// Adds a chain (allocates; call off the audio thread).
    pub fn push_chain(&mut self, slots: Vec<RackSlot>, params: &[f32]) {
        let mut c = RackChain {
            slots,
            params: std::array::from_fn(|i| CHAIN_PARAMS[i].default),
            gain_l: Smoothed::new(1.0, self.sr, 20.0),
            gain_r: Smoothed::new(1.0, self.sr, 20.0),
            l: vec![0.0; self.block],
            r: vec![0.0; self.block],
            events: Vec::with_capacity(self.max_events),
            events_b: Vec::with_capacity(self.max_events),
            sounding: 0,
        };
        for (i, d) in CHAIN_PARAMS.iter().enumerate() {
            c.params[i] = d.clamp(params.get(i).copied().unwrap_or(d.default));
        }
        c.update_gain();
        c.gain_l.reset(c.gain_l.target());
        c.gain_r.reset(c.gain_r.target());
        self.chains.push(c);
        let kinds: Vec<Vec<DeviceKind>> = self.chains.iter().map(|c| c.slots.iter().map(|s| s.device.kind()).collect()).collect();
        self.layout = RackLayout::new(kinds.iter().map(Vec::as_slice));
        self.flat = vec![0.0; self.layout.len()];
        for (ci, c) in self.chains.iter().enumerate() {
            for (k, v) in c.params.iter().enumerate() {
                self.flat[self.layout.chain_param(ci, k)] = *v;
            }
            for (di, s) in c.slots.iter().enumerate() {
                for (k, v) in s.device.param_values().iter().enumerate() {
                    self.flat[self.layout.device_param(ci, di, k)] = *v;
                }
            }
        }
    }

    pub fn layout(&self) -> &RackLayout {
        &self.layout
    }

    /// True when `other` has the same chains and device kinds (state can carry over).
    pub fn same_shape(&self, other: &Rack) -> bool {
        self.layout == other.layout && self.chains.iter().zip(&other.chains).all(|(a, b)| a.slots.iter().map(|s| s.device.kind()).eq(b.slots.iter().map(|s| s.device.kind())))
    }

    /// Pairs of devices at the same position in both racks (for media exchange).
    pub fn device_pairs<'a>(&'a mut self, other: &'a mut Rack) -> impl Iterator<Item = (&'a mut Device, &'a mut Device)> {
        self.chains.iter_mut().zip(other.chains.iter_mut()).flat_map(|(a, b)| a.slots.iter_mut().zip(b.slots.iter_mut()).map(|(x, y)| (&mut x.device, &mut y.device)))
    }

    pub fn set_param(&mut self, i: usize, v: f32) {
        match self.layout.locate(i) {
            Some(RackParam::Chain { chain, param }) => {
                let c = &mut self.chains[chain];
                c.params[param] = CHAIN_PARAMS[param].clamp(v);
                c.update_gain();
                self.flat[i] = c.params[param];
            }
            Some(RackParam::Device { chain, device, param }) => {
                let d = &mut self.chains[chain].slots[device].device;
                d.set_param(param, v);
                self.flat[i] = d.param_values().get(param).copied().unwrap_or(v);
            }
            None => {}
        }
    }

    pub fn param_values(&self) -> &[f32] {
        &self.flat
    }

    pub fn latency_samples(&self) -> usize {
        self.chains.iter().map(|c| c.slots.iter().filter(|s| !s.bypass).map(|s| s.device.latency_samples()).sum::<usize>()).max().unwrap_or(0)
    }

    pub fn process(&mut self, events: &[NoteEvent], l: &mut [f32], r: &mut [f32], ctx: &Ctx) {
        let n = l.len();
        if self.chains.is_empty() {
            // An empty effect rack passes its input; an empty instrument rack adds nothing.
            return;
        }
        let any_solo = self.chains.iter().any(|c| c.params[SOLO] >= 0.5);
        for c in self.chains.iter_mut() {
            // Route events by zone; note-offs follow the note-ons this chain took.
            c.events.clear();
            for e in events {
                let take = match e.kind {
                    NoteKind::On { pitch, velocity } if velocity > 0 => {
                        let ok = c.accepts(pitch, velocity);
                        if ok {
                            c.sounding |= 1u128 << (pitch & 127);
                        }
                        ok
                    }
                    NoteKind::On { pitch, .. } | NoteKind::Off { pitch } => {
                        let bit = 1u128 << (pitch & 127);
                        let had = c.sounding & bit != 0;
                        c.sounding &= !bit;
                        had
                    }
                    NoteKind::AllOff => {
                        c.sounding = 0;
                        true
                    }
                    NoteKind::Sustain(_) => true,
                };
                if take && c.events.len() < c.events.capacity() {
                    c.events.push(*e);
                }
            }
            let (cl, cr) = (&mut c.l[..n], &mut c.r[..n]);
            if self.instrument {
                cl.fill(0.0);
                cr.fill(0.0);
            } else {
                cl.copy_from_slice(l);
                cr.copy_from_slice(r);
            }
            run_chain(&mut c.slots, &mut c.events, &mut c.events_b, cl, cr, ctx, &|_| None);
        }
        if !self.instrument {
            l.fill(0.0);
            r.fill(0.0);
        }
        for c in self.chains.iter_mut() {
            let on = c.audible(any_solo);
            for i in 0..n {
                let (gl, gr) = (c.gain_l.next(), c.gain_r.next());
                if on {
                    l[i] += c.l[i] * gl;
                    r[i] += c.r[i] * gr;
                }
            }
        }
    }
}

impl Processor for Rack {
    fn set_param(&mut self, i: usize, v: f32) {
        Rack::set_param(self, i, v)
    }
    fn param_values(&self) -> &[f32] {
        &self.flat
    }
    fn process(&mut self, io: Io, ctx: &Ctx) {
        Rack::process(self, io.events, io.l, io.r, ctx)
    }
    fn latency_samples(&self) -> usize {
        Rack::latency_samples(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceMedia;

    const SR: f32 = 48_000.0;

    fn slot(kind: DeviceKind) -> RackSlot {
        RackSlot { device: Device::new(kind, SR, DeviceMedia::default()), bypass: false }
    }

    #[test]
    fn layout_addresses_chains_then_their_devices() {
        let l = RackLayout::new([&[DeviceKind::Gain, DeviceKind::Saturator][..], &[][..], &[DeviceKind::Gain][..]]);
        let (g, s, c) = (DeviceKind::Gain.params().len(), DeviceKind::Saturator.params().len(), CHAIN_PARAMS.len());
        assert_eq!(l.len(), 3 * c + 2 * g + s);
        assert_eq!(l.chain_param(1, 0), c + g + s);
        assert_eq!(l.device_param(2, 0, 1), 2 * c + g + s + c + 1);
        for flat in 0..l.len() {
            let back = match l.locate(flat).unwrap() {
                RackParam::Chain { chain, param } => l.chain_param(chain, param),
                RackParam::Device { chain, device, param } => l.device_param(chain, device, param),
            };
            assert_eq!(back, flat);
        }
    }

    #[test]
    fn effect_rack_sums_parallel_chains_and_passes_through_when_empty() {
        let mut rack = Rack::new(SR, false);
        let input = vec![0.25f32; 256];
        let (mut l, mut r) = (input.clone(), input.clone());
        rack.process(&[], &mut l, &mut r, &Ctx::stopped(SR, 120.0));
        assert_eq!(l, input, "empty rack passes through");
        // Two chains: unity Utility and a -6 dB Utility → 1 + 0.501 of the input.
        rack.push_chain(vec![slot(DeviceKind::Gain)], &[]);
        let mut quiet = slot(DeviceKind::Gain);
        quiet.device.set_param(0, -6.0);
        rack.push_chain(vec![quiet], &[]);
        let (mut l, mut r) = (input.clone(), input.clone());
        for _ in 0..4 {
            l.copy_from_slice(&input);
            r.copy_from_slice(&input);
            rack.process(&[], &mut l, &mut r, &Ctx::stopped(SR, 120.0));
        }
        let expected = 0.25 * (1.0 + db_to_gain(-6.0));
        assert!((l[200] - expected).abs() < 1e-4, "{} vs {expected}", l[200]);
        // Solo the second chain: only it is heard.
        let solo = rack.layout().chain_param(1, SOLO);
        rack.set_param(solo, 1.0);
        for _ in 0..4 {
            l.copy_from_slice(&input);
            r.copy_from_slice(&input);
            rack.process(&[], &mut l, &mut r, &Ctx::stopped(SR, 120.0));
        }
        assert!((l[200] - 0.25 * db_to_gain(-6.0)).abs() < 1e-4);
    }

    #[test]
    fn instrument_rack_routes_notes_by_key_zone_and_offs_follow_their_ons() {
        let mut rack = Rack::new(SR, true);
        rack.push_chain(vec![slot(DeviceKind::PolySynth)], &[0.0, 0.0, 0.0, 0.0, 0.0, 59.0, 1.0, 127.0]);
        rack.push_chain(vec![slot(DeviceKind::FmSynth)], &[0.0, 0.0, 0.0, 0.0, 60.0, 127.0, 1.0, 127.0]);
        let ctx = Ctx::stopped(SR, 120.0);
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        let on = |pitch| NoteEvent { offset: 0, kind: NoteKind::On { pitch, velocity: 100 } };
        rack.process(&[on(48)], &mut l, &mut r, &ctx);
        assert_eq!(rack.chains[0].sounding, 1u128 << 48);
        assert_eq!(rack.chains[1].sounding, 0);
        // Moving the zone while the note holds: its note-off still reaches chain 0.
        let hi = rack.layout().chain_param(0, KEY_HI);
        rack.set_param(hi, 10.0);
        rack.process(&[NoteEvent { offset: 0, kind: NoteKind::Off { pitch: 48 } }], &mut l, &mut r, &ctx);
        assert_eq!(rack.chains[0].sounding, 0);
        rack.process(&[on(72)], &mut l, &mut r, &ctx);
        assert_eq!(rack.chains[1].sounding, 1u128 << 72);
        // Velocity zone.
        let vlo = rack.layout().chain_param(1, VEL_LO);
        rack.set_param(vlo, 120.0);
        rack.process(&[on(74)], &mut l, &mut r, &ctx);
        assert_eq!(rack.chains[1].sounding & (1u128 << 74), 0);
    }

    #[test]
    fn rack_parameters_reach_inner_devices_and_mirror_their_clamped_values() {
        let mut rack = Rack::new(SR, false);
        rack.push_chain(vec![slot(DeviceKind::Gain)], &[]);
        let gain = rack.layout().device_param(0, 0, 0);
        rack.set_param(gain, 99.0);
        assert_eq!(rack.param_values()[gain], 36.0, "clamped to the Utility's range");
        assert_eq!(rack.chains[0].slots[0].device.param_values()[0], 36.0);
    }
}
