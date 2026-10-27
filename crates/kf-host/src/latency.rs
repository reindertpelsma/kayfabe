//! Opt-in bounded latency observations; host observations are not GPU timestamps.
mod core;
mod journal;
mod runtime;
mod splits;
pub use core::{Cause, Kind};
pub use runtime::*;
pub use splits::SplitScope;
#[cfg(test)]
mod tests;
