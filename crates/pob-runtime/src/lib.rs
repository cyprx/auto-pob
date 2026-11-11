//! Subscript runtime and bundled Lua modules shared by the Rusty PoB runtime and the egui app.

pub mod modules;
pub mod subscript;
pub mod util;

/// Log target used for frame statistics.
pub const STATS_TARGET: &str = "rpob::stats";
