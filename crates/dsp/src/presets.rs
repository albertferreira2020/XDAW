//! Factory presets for the built-in effects. Parameters are named by id so a
//! preset survives parameter reordering and unknown ids are simply ignored.

use crate::device::DeviceKind;

pub struct FactoryPreset {
    pub name: &'static str,
    pub params: &'static [(&'static str, f32)],
}

const EQ: &[FactoryPreset] = &[
    FactoryPreset { name: "Flat", params: &[] },
    FactoryPreset { name: "Vocal Presence", params: &[("low_gain", -4.0), ("low_freq", 100.0), ("mid_gain", 3.0), ("mid_freq", 3_000.0), ("mid_q", 1.0), ("high_gain", 2.0), ("high_freq", 8_000.0)] },
    FactoryPreset { name: "Bass Boost", params: &[("low_gain", 6.0), ("low_freq", 90.0), ("mid_gain", -1.5), ("mid_freq", 400.0), ("mid_q", 0.8)] },
    FactoryPreset { name: "Mud Cut", params: &[("mid_gain", -4.0), ("mid_freq", 300.0), ("mid_q", 1.2)] },
    FactoryPreset { name: "Air", params: &[("high_gain", 5.0), ("high_freq", 10_000.0)] },
    FactoryPreset { name: "Low Cut Rumble", params: &[("low_gain", -18.0), ("low_freq", 60.0)] },
    FactoryPreset { name: "Telephone", params: &[("low_gain", -18.0), ("low_freq", 500.0), ("mid_gain", 6.0), ("mid_freq", 1_500.0), ("mid_q", 1.5), ("high_gain", -18.0), ("high_freq", 3_000.0)] },
];

const GAIN: &[FactoryPreset] = &[
    FactoryPreset { name: "Unity", params: &[] },
    FactoryPreset { name: "-6 dB", params: &[("gain", -6.0)] },
    FactoryPreset { name: "-12 dB", params: &[("gain", -12.0)] },
    FactoryPreset { name: "+6 dB", params: &[("gain", 6.0)] },
    FactoryPreset { name: "Mono", params: &[("width", 0.0)] },
    FactoryPreset { name: "Wide", params: &[("width", 1.5)] },
];

const LIMITER: &[FactoryPreset] = &[
    FactoryPreset { name: "Transparent Master", params: &[("gain", 0.0), ("ceiling", -0.3), ("release", 150.0)] },
    FactoryPreset { name: "Loud Master", params: &[("gain", 6.0), ("ceiling", -0.1), ("release", 60.0)] },
    FactoryPreset { name: "Safety Ceiling", params: &[("gain", 0.0), ("ceiling", -1.0), ("release", 250.0)] },
    FactoryPreset { name: "Drum Bus", params: &[("gain", 4.0), ("ceiling", -1.0), ("release", 30.0)] },
];

/// Presets shipped for `kind` (empty for devices without any).
pub fn factory_presets(kind: DeviceKind) -> &'static [FactoryPreset] {
    match kind {
        DeviceKind::Eq3 => EQ,
        DeviceKind::Gain => GAIN,
        DeviceKind::Limiter => LIMITER,
        _ => &[],
    }
}

/// Full parameter vector for a preset: defaults, then the preset's overrides.
pub fn preset_values(kind: DeviceKind, preset: &FactoryPreset) -> Vec<f32> {
    let mut v: Vec<f32> = kind.params().iter().map(|p| p.default).collect();
    for (id, value) in preset.params {
        if let Some(i) = kind.params().iter().position(|p| p.id == *id) {
            v[i] = kind.params()[i].clamp(*value);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_uses_known_in_range_params() {
        for kind in DeviceKind::ALL {
            for p in factory_presets(kind) {
                for (id, v) in p.params {
                    let d = kind.params().iter().find(|d| d.id == *id).unwrap_or_else(|| panic!("{:?}/{}: unknown param {id}", kind, p.name));
                    assert!(*v >= d.min && *v <= d.max, "{:?}/{}: {id}={v} outside {}..{}", kind, p.name, d.min, d.max);
                }
            }
        }
    }

    #[test]
    fn names_are_unique_per_kind() {
        for kind in DeviceKind::ALL {
            let mut n: Vec<_> = factory_presets(kind).iter().map(|p| p.name).collect();
            n.sort();
            n.dedup();
            assert_eq!(n.len(), factory_presets(kind).len());
        }
    }
}
