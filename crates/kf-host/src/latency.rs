//! Opt-in bounded latency observations; host observations are not GPU timestamps.
mod core;
mod journal;
mod runtime;
pub use core::{Cause, Kind};
pub use runtime::*;
#[cfg(test)]
mod tests;
