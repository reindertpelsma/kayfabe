//! The L1 ranked-lock adapter — **re-exported from [`kayfabe_util::lock`]**.
//!
//! ⊘ `[w236, 2026-08-11]` The definitions moved DOWN to `kayfabe-util` and the move was
//! forced, not cosmetic: `kayfabe_device::RegPlane::state` is now [`LockRank::Plane`], and
//! `kayfabe-rt` depends on `kayfabe-device`, so the rank vocabulary cannot live here without
//! a dependency cycle. ★ Same argument this crate already made for the held-mask counter.
//!
//! Every name is re-exported, so existing call sites are unchanged.

pub use kayfabe_util::lock::*;
