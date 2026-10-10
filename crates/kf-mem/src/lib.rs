//! `kf-mem` — the one-object model of `THE_TRANSLATED_PLANE.md` §18.
//!
//! Two ground truths: guest RAM (the hypervisor's memfd) and guest VRAM (GPGA, one RM object).
//! Everything else is a map onto one of them. ⊘ This crate holds no join, no table of FB ranges,
//! and no copy of anything — see §18.2's test: bytes that are neither guest RAM nor GPGA, with
//! something to copy/sync between them, is a shadow and forbidden.

// ★ Review addendum (2026-10-10, `V3_BATCHED_MAP.md` §8.8.9): THE GUEST IS UNTRUSTED, and every
// module below ingests values the guest controls (VAs, lengths, leaf-size codes, backing addresses
// from its page tables). A panic on the VA thread is a denial of service, so no site may panic: a
// guest-derived value is added/subtracted with `checked_*`/`saturating_*` (a refusal by name or a
// saturated range the host refuses), never `+`/`-`. These lints make a NEW panic site fail CI
// (`scripts/ci/clippy.sh`); the test code is exempt. The only `allow`s are the ones written next to
// the code with the invariant that makes them safe.
#![cfg_attr(
    not(test),
    deny(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

pub mod addr;
pub mod store;

pub use addr::{Fb, Gpa, Gva, HostToken, PAGE, StoreOffset};
pub use store::{Store, StoreRefusal};
pub mod apply;
pub mod batch;
pub mod cpuwin;
pub mod ledger;
pub mod maplog;
#[cfg(test)]
pub(crate) mod sim;
pub mod vasmgr;
