//! # nvkvm-util — purely generic utilities
//!
//! Design decision #2: *"Util lib = purely generic, no GPU concepts."* Nothing in this
//! crate may name a GPU, NVIDIA, VMM, or driver concept; it is the bottom of the
//! dependency graph and must compile for any target (no OS dependencies, no
//! `std::fs`/`std::net`/`std::time::Instant`).
//!
//! Contents:
//! - [`IntervalMap`] — the non-overlapping range map used by the per-VAS address table
//!   (`mode2_rust_rewrite_architecture.md` §4.3.1 `Vas::bindings`,
//!   `mode2_address_table.md`: one forward-populated table, MISS=FAULT).
//! - [`Instant`] / [`Duration`] re-export — a **virtual clock**. The core never reads
//!   real time; time is a value the VMM adapter (or a test's mock) advances explicitly
//!   (`mode2_rust_testing_strategy.md` §4: "the virtual clock is load-bearing").

pub mod interval_map;
pub mod time;

pub use interval_map::{IntervalMap, OverlapError};
pub use time::Instant;
/// Re-export of the pure, OS-free duration type (from `core::time`).
pub use core::time::Duration;
