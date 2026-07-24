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
//! - [`assert_send_sync!`] / [`assert_send!`] — the compile-time concurrency
//!   assertions every logic crate applies to its public types (decision #17).

pub mod interval_map;
pub mod time;

pub use interval_map::{IntervalError, IntervalMap};
pub use time::Instant;
/// Re-export of the pure, OS-free duration type (from `core::time`).
pub use core::time::Duration;

/// Compile-time assertion that each listed type is [`Send`]`+`[`Sync`] — **the
/// concurrency contract, enforced by the build** (design decision #17).
///
/// Every logic crate invokes this over its public types at the bottom of the
/// defining module, so the assertion is checked on every `cargo check`/`build`:
/// if a future edit sneaks an `Rc`, `Cell`, `RefCell`, or an un-bounded trait
/// object into a core type, **compilation fails** — a data-race hazard can never
/// reach a test run, let alone production. Accepts unsized types, so trait-object
/// seams are assertable directly (`assert_send_sync!(dyn Arch)`).
#[macro_export]
macro_rules! assert_send_sync {
    ($($t:ty),+ $(,)?) => {
        const _: () = {
            const fn assert_send_sync<T: Send + Sync + ?Sized>() {}
            $(assert_send_sync::<$t>();)+
        };
    };
}

/// Compile-time assertion that each listed type is [`Send`] (see
/// [`assert_send_sync!`]). For the rare seam that is deliberately only `Send`
/// (e.g. `dyn RmBackend`, reachable exclusively through `&mut`): the exception is
/// explicit in the source, per the "thread-safe by default, exceptions documented"
/// rule (decision #17).
#[macro_export]
macro_rules! assert_send {
    ($($t:ty),+ $(,)?) => {
        const _: () = {
            const fn assert_send<T: Send + ?Sized>() {}
            $(assert_send::<$t>();)+
        };
    };
}

// The contract applies to this crate's own types first.
crate::assert_send_sync!(Instant, IntervalMap<()>, IntervalError);
