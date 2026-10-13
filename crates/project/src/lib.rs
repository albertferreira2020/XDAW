//! Project model, persistence and undo/redo.

pub mod history;
pub mod io;
pub mod model;

pub use history::{Edit, History};
pub use model::*;

#[cfg(test)]
mod tests;
