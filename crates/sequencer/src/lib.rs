//! Musical time, launch quantization and Session clip scheduling.
//!
//! Pure, allocation-free logic shared by realtime playback and offline export.

pub mod clip;
pub mod launch;
pub mod quantize;
pub mod time;

pub use clip::ClipRegion;
pub use launch::{LaunchEvent, LaunchMode, Pending, Playing, TrackLauncher};
pub use quantize::{ClipQuantize, Quantize};
pub use time::{Beats, Clock, TimeSignature};
