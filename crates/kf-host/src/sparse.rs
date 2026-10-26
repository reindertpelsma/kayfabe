//! Selective sparse reservation geometry. Receipts describe only our own RM objects.

use crate::{RmError, channel::GuestVaRange};
use std::collections::BTreeMap;

/// Normal reservation grain.
pub const NORMAL_GRAIN: u64 = 2 << 20;
/// Sparse reservation grain.
pub const SPARSE_GRAIN: u64 = 64 << 10;
/// Hard per-space bound on reservation objects.
pub const BLOCK_LIMIT: usize = 8192;
const PAGE: u64 = 4096;

/// Only an explicit `1` enables the selective sparse path.
#[must_use]
pub fn enabled_from(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Default OFF; sampled when the host session and walker are created.
#[must_use]
pub fn enabled() -> bool {
    enabled_from(std::env::var("KF3_SPARSE_TWIN").ok().as_deref())
}

/// Reservation verbs, with a host implementation and an adversarial model.
pub(crate) trait Ops {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError>;
    fn release(&self, handle: u32) -> Result<(), RmError>;
}

#[derive(Clone, Debug)]
struct Block {
    lo: u64,
    hi: u64,
    handle: u32,
    sparse: bool,
    live: [u64; 8],
    wanted: [u64; 8],
}

/// Bounded, saturating counters; no guest-controlled log stream.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Reservations created after initial space creation.
    pub made: u64,
    /// Empty reservations released.
    pub freed: u64,
    /// RM verb refusals.
    pub refused: u64,
    /// Object-budget refusals.
    pub overflow: u64,
    /// Sparse bytes that could not be honoured in this call.
    pub unhonoured_bytes: u64,
    /// Sparse-to-invalid bytes retained in occupied sparse blocks.
    pub retained_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct Reservations {
    blocks: BTreeMap<u64, Block>,
    limit: usize,
    pub(crate) stats: Stats,
    pub(crate) poisoned: bool,
}

impl Reservations {
    pub(crate) fn new(guest: &[GuestVaRange], limit: usize) -> Self {
        Self {
            blocks: guest.iter().filter(|g| g.handle != 0).map(|g| {
                (g.lo, Block { lo: g.lo, hi: g.hi, handle: g.handle,
                    sparse: false, live: [0; 8], wanted: [0; 8] })
            }).collect(),
            limit,
            stats: Stats::default(),
            poisoned: false,
        }
    }

    pub(crate) fn normal(&mut self, _: &dyn Ops, _: u64, _: u64) -> Result<(), RmError> {
        Ok(())
    }
    pub(crate) fn admit(&mut self, _: &dyn Ops, _: u64, _: u64) -> Result<bool, RmError> {
        Ok(false)
    }
    pub(crate) fn retire(
        &mut self, _: &dyn Ops, _: u64, _: u64, _: bool,
    ) -> Result<u64, RmError> {
        Ok(0)
    }
    pub(crate) fn mark(&mut self, _: u64, _: u64, _: bool) {}
    pub(crate) fn retry(&mut self, _: &dyn Ops) -> Result<(), RmError> { Ok(()) }
    pub(crate) fn pieces(
        &self, range: u32, va: u64, len: u64,
    ) -> Result<Vec<(u32, u64, u64)>, RmError> {
        Ok(vec![(range, va, len)])
    }
}

#[cfg(test)]
mod tests;
