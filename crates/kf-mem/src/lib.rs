//! `kf-mem` — the one-object model of `THE_TRANSLATED_PLANE.md` §18.
//!
//! Two ground truths: guest RAM (the hypervisor's memfd) and guest VRAM (GPGA, one RM object).
//! Everything else is a map onto one of them. ⊘ This crate holds no join, no table of FB ranges,
//! and no copy of anything — see §18.2's test: bytes that are neither guest RAM nor GPGA, with
//! something to copy/sync between them, is a shadow and forbidden.

pub mod addr;
pub mod store;

pub use addr::{Fb, Gpa, Gva, HostToken, StoreOffset, PAGE};
pub use store::{Store, StoreRefusal};
pub mod apply;
pub mod batch;
pub mod cpuwin;
pub mod ledger;
pub mod vasmgr;
