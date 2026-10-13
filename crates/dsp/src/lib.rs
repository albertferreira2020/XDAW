//! Realtime-safe DSP building blocks, instruments and effects.
//!
//! Every `process`/`render` method is allocation-free and lock-free; buffers
//! are created in constructors, which must run off the audio thread.

pub mod device;
pub mod effects;
pub mod env;
pub mod filter;
pub mod instruments;
pub mod meter;
pub mod params;
pub mod util;

pub use device::{Device, DeviceKind, DeviceMedia};
pub use instruments::SampleData;
pub use params::{Ctx, NoteEvent, NoteKind, ParamDesc, ParamKind};
