//! Selective sparse reservation geometry. Receipts describe only our own RM objects.

use crate::{RmError, channel::GuestVaRange};
use std::collections::BTreeMap;

pub(crate) mod initial;

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
            blocks: guest
                .iter()
                .filter(|g| g.handle != 0)
                .map(|g| {
                    (
                        g.lo,
                        Block {
                            lo: g.lo,
                            hi: g.hi,
                            handle: g.handle,
                            sparse: false,
                            live: [0; 8],
                            wanted: [0; 8],
                        },
                    )
                })
                .collect(),
            limit,
            stats: Stats::default(),
            poisoned: false,
        }
    }

    fn end(va: u64, len: u64) -> Result<u64, RmError> {
        let end = va
            .checked_add(len)
            .ok_or(RmError::Other(super::channel::VA_ROW_WRAPS))?;
        if va < super::channel::TWIN_VA_FLOOR
            || len == 0
            || !va.is_multiple_of(PAGE)
            || !len.is_multiple_of(PAGE)
        {
            return Err(RmError::NoMemory);
        }
        Ok(end)
    }

    fn budget(&mut self, count: usize) -> Result<(), RmError> {
        if count > self.limit {
            self.stats.overflow = self.stats.overflow.saturating_add(1);
            return Err(RmError::NoMemory);
        }
        Ok(())
    }

    fn key(&self, va: u64) -> Option<u64> {
        self.blocks
            .range(..=va)
            .next_back()
            .and_then(|(&lo, b)| (va < b.hi).then_some(lo))
    }

    fn bits(mask: &mut [u64; 8], lo: u64, start: u64, end: u64, set: bool) {
        for (i, word) in mask.iter_mut().enumerate() {
            for bit in 0..64 {
                let p = lo.saturating_add((i as u64 * 64 + bit) * PAGE);
                if start <= p && p < end {
                    if set {
                        *word |= 1 << bit;
                    } else {
                        *word &= !(1 << bit);
                    }
                }
            }
        }
    }

    fn wanted(b: &Block, start: u64, end: u64) -> bool {
        (start..end).step_by(PAGE as usize).all(|p| {
            let n = (p - b.lo) / PAGE;
            b.wanted
                .get((n / 64) as usize)
                .is_some_and(|v| v & (1 << (n % 64)) != 0)
        })
    }

    fn occupied(b: &Block) -> bool {
        b.live.iter().any(|&v| v != 0)
    }

    // Only EMPTY blocks can change geometry. Failed replacement restores the original
    // reservation before allowing any non-FIXED host placement to run again.
    fn replace(
        &mut self,
        ops: &dyn Ops,
        key: u64,
        lo: u64,
        hi: u64,
        sparse: bool,
    ) -> Result<(), RmError> {
        let old = self.blocks.get(&key).cloned().ok_or(RmError::NoMemory)?;
        if Self::occupied(&old) {
            return Err(RmError::NoMemory);
        }
        let parts: Vec<_> = [(old.lo, lo, false), (lo, hi, sparse), (hi, old.hi, false)]
            .into_iter()
            .filter(|&(a, b, _)| a < b)
            .collect();
        self.budget(
            self.blocks
                .len()
                .saturating_sub(1)
                .saturating_add(parts.len()),
        )?;
        if let Err(e) = ops.release(old.handle) {
            self.stats.refused = self.stats.refused.saturating_add(1);
            self.poisoned = true;
            return Err(e);
        }
        self.stats.freed = self.stats.freed.saturating_add(1);
        let mut made = Vec::new();
        for &(a, b, kind) in &parts {
            match ops.reserve(a, b - a, kind) {
                Ok(handle) => {
                    self.stats.made = self.stats.made.saturating_add(1);
                    let mut child = Block {
                        lo: a,
                        hi: b,
                        handle,
                        sparse: kind,
                        live: [0; 8],
                        wanted: [0; 8],
                    };
                    if old.wanted.iter().any(|&v| v != 0) {
                        for p in (a..b).step_by(PAGE as usize) {
                            if Self::wanted(&old, p, p + PAGE) {
                                Self::bits(&mut child.wanted, a, p, p + PAGE, true);
                            }
                        }
                    }
                    made.push(child);
                }
                Err(e) => {
                    self.stats.refused = self.stats.refused.saturating_add(1);
                    for child in &made {
                        if ops.release(child.handle).is_err() {
                            self.poisoned = true;
                        }
                        self.stats.freed = self.stats.freed.saturating_add(1);
                    }
                    match ops.reserve(old.lo, old.hi - old.lo, old.sparse) {
                        Ok(handle) => {
                            self.stats.made = self.stats.made.saturating_add(1);
                            self.blocks.insert(key, Block { handle, ..old });
                        }
                        Err(_) => {
                            self.poisoned = true;
                            self.blocks.insert(key, Block { handle: 0, ..old });
                        }
                    }
                    return Err(e);
                }
            }
        }
        self.blocks.remove(&key);
        for child in made {
            self.blocks.insert(child.lo, child);
        }
        Ok(())
    }

    pub(crate) fn normal(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<(), RmError> {
        let end = Self::end(va, len)?;
        self.budget(((end - 1) / NORMAL_GRAIN - va / NORMAL_GRAIN + 1) as usize)?;
        let mut cur = va;
        while cur < end {
            if let Some(key) = self.key(cur) {
                let b = self.blocks.get(&key).ok_or(RmError::NoMemory)?;
                let stop = b.hi.min(end);
                if b.hi - b.lo > NORMAL_GRAIN && !b.sparse {
                    let lo = (cur / NORMAL_GRAIN * NORMAL_GRAIN).max(b.lo);
                    let hi = lo.saturating_add(NORMAL_GRAIN).min(b.hi);
                    self.replace(ops, key, lo, hi, false)?;
                    cur = hi.min(end);
                } else {
                    cur = stop;
                }
            } else {
                cur = self
                    .blocks
                    .range(cur..)
                    .next()
                    .map_or(end, |(&lo, _)| lo.min(end));
            }
        }
        Ok(())
    }

    fn sparsify(&mut self, ops: &dyn Ops, va: u64) -> Result<bool, RmError> {
        let key = self.key(va).ok_or(RmError::NoMemory)?;
        let b = self.blocks.get(&key).ok_or(RmError::NoMemory)?;
        if b.sparse {
            return Ok(true);
        }
        if Self::occupied(b) || !Self::wanted(b, va, va + SPARSE_GRAIN) {
            return Ok(false);
        }
        self.replace(ops, key, va, va + SPARSE_GRAIN, true)?;
        Ok(true)
    }

    pub(crate) fn admit(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<bool, RmError> {
        let snapshot = self.blocks.clone();
        let result = self.admit_inner(ops, va, len);
        if result.is_err() {
            let fresh: Vec<_> = self
                .blocks
                .values()
                .filter(|b| b.sparse)
                .filter(|b| {
                    !snapshot
                        .range(..=b.lo)
                        .next_back()
                        .is_some_and(|(_, old)| old.sparse && b.hi <= old.hi)
                })
                .map(|b| (b.lo, b.hi))
                .collect();
            for (lo, hi) in fresh {
                if self.replace(ops, lo, lo, hi, false).is_err() {
                    self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(hi - lo);
                }
            }
            for b in self.blocks.values_mut() {
                b.wanted = [0; 8];
                // An intent-bearing block is at most NORMAL_GRAIN; coarse siblings
                // have no intent. This loop is bounded independently of guest VA.
                for p in (b.lo..b.hi.min(b.lo.saturating_add(NORMAL_GRAIN))).step_by(PAGE as usize)
                {
                    if let Some((_, old)) = snapshot.range(..=p).next_back()
                        && p < old.hi
                        && Self::wanted(old, p, p + PAGE)
                    {
                        Self::bits(&mut b.wanted, b.lo, p, p + PAGE, true);
                    }
                }
            }
        }
        result
    }

    fn admit_inner(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<bool, RmError> {
        let end = Self::end(va, len)?;
        self.budget(((end - 1) / SPARSE_GRAIN - va / SPARSE_GRAIN + 1) as usize)?;
        // Empty normal geometry is prepared before any intent is recorded. A failure
        // therefore leaves no partly accepted intent for the GPU diff to forget.
        self.normal(ops, va, len)?;
        let mut all = true;
        let mut cur = va;
        while cur < end {
            let tile = cur / SPARSE_GRAIN * SPARSE_GRAIN;
            let tile_end = tile.checked_add(SPARSE_GRAIN).ok_or(RmError::NoMemory)?;
            let stop = tile_end.min(end);
            if let Some(key) = self.key(cur) {
                let b = self.blocks.get_mut(&key).ok_or(RmError::NoMemory)?;
                Self::bits(&mut b.wanted, b.lo, cur, stop, true);
                all &= self.sparsify(ops, tile)?;
            } else if cur == tile && stop == tile_end {
                self.budget(self.blocks.len().saturating_add(1))?;
                let handle = match ops.reserve(tile, SPARSE_GRAIN, true) {
                    Ok(h) => h,
                    Err(e) => {
                        self.stats.refused = self.stats.refused.saturating_add(1);
                        return Err(e);
                    }
                };
                self.stats.made = self.stats.made.saturating_add(1);
                self.blocks.insert(
                    tile,
                    Block {
                        lo: tile,
                        hi: tile_end,
                        handle,
                        sparse: true,
                        live: [0; 8],
                        wanted: [u64::MAX >> 48, 0, 0, 0, 0, 0, 0, 0],
                    },
                );
            } else {
                all = false;
            }
            cur = stop;
        }
        if !all {
            self.stats.unhonoured_bytes = self.stats.unhonoured_bytes.saturating_add(len);
        }
        Ok(all)
    }

    pub(crate) fn retire(
        &mut self,
        ops: &dyn Ops,
        va: u64,
        len: u64,
        preserve: bool,
    ) -> Result<u64, RmError> {
        let end = Self::end(va, len)?;
        let keys: Vec<_> = self
            .blocks
            .range(..end)
            .filter(|(_, b)| va < b.hi)
            .map(|(&k, _)| k)
            .collect();
        let mut retained = 0u64;
        for key in keys {
            let b = self.blocks.get_mut(&key).ok_or(RmError::NoMemory)?;
            Self::bits(&mut b.wanted, b.lo, va, end, false);
            if !b.sparse || preserve {
                continue;
            }
            if !Self::occupied(b) && b.wanted.iter().all(|&v| v == 0) {
                let (lo, hi) = (b.lo, b.hi);
                self.replace(ops, key, lo, hi, false)?;
            } else {
                retained = retained.saturating_add(b.hi.min(end).saturating_sub(b.lo.max(va)));
            }
        }
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(retained);
        Ok(retained)
    }

    pub(crate) fn mark(&mut self, va: u64, len: u64, live: bool) {
        let end = va.saturating_add(len);
        for b in self
            .blocks
            .range_mut(..end)
            .map(|(_, b)| b)
            .filter(|b| va < b.hi)
        {
            Self::bits(&mut b.live, b.lo, va, end, live);
        }
    }

    pub(crate) fn retry(&mut self, ops: &dyn Ops) -> Result<(), RmError> {
        let tiles: Vec<_> = self
            .blocks
            .values()
            .filter(|b| !b.sparse && !Self::occupied(b) && b.hi - b.lo <= NORMAL_GRAIN)
            .flat_map(|b| {
                let lo = b.lo.div_ceil(SPARSE_GRAIN) * SPARSE_GRAIN;
                (lo..b.hi)
                    .step_by(SPARSE_GRAIN as usize)
                    .filter(|&p| p + SPARSE_GRAIN <= b.hi && Self::wanted(b, p, p + SPARSE_GRAIN))
            })
            .collect();
        for tile in tiles {
            self.sparsify(ops, tile)?;
        }
        Ok(())
    }

    /// Route a plain (backed) map: refine the geometry around it, then name the host
    /// reservation handle of every piece. Occupancy is recorded before the host call.
    pub(crate) fn route_map(
        &mut self,
        ops: &dyn Ops,
        range: u32,
        va: u64,
        len: u64,
    ) -> Result<Vec<(u32, u64, u64)>, RmError> {
        self.normal(ops, va, len)?;
        let pieces = self.pieces(range, va, len)?;
        self.mark(va, len, true);
        Ok(pieces)
    }

    pub(crate) fn pieces(
        &self,
        range: u32,
        va: u64,
        len: u64,
    ) -> Result<Vec<(u32, u64, u64)>, RmError> {
        let end = Self::end(va, len)?;
        let mut pieces = Vec::new();
        let mut cur = va;
        while cur < end {
            let (handle, stop) = if let Some(key) = self.key(cur) {
                let b = self.blocks.get(&key).ok_or(RmError::NoMemory)?;
                (b.handle, b.hi.min(end))
            } else {
                (
                    range,
                    self.blocks
                        .range(cur..)
                        .next()
                        .map_or(end, |(&lo, _)| lo.min(end)),
                )
            };
            if handle == 0 || stop <= cur || pieces.len() > self.limit.saturating_mul(2) {
                return Err(RmError::NoMemory);
            }
            pieces.push((handle, cur, stop - cur));
            cur = stop;
        }
        Ok(pieces)
    }

    pub(crate) fn clear(&mut self, ops: &dyn Ops) -> Result<(), RmError> {
        // Reuse belongs to a new guest VA space. No leaf intent or sparse absorb
        // from the previous space may survive it, including unhonoured intent.
        if self.blocks.values().any(Self::occupied) {
            self.stats.refused = self.stats.refused.saturating_add(1);
            return Err(RmError::NoMemory);
        }
        for block in self.blocks.values_mut() {
            block.wanted = [0; 8];
        }
        let sparse: Vec<_> = self
            .blocks
            .values()
            .filter(|b| b.sparse)
            .map(|b| (b.lo, b.hi))
            .collect();
        for (lo, hi) in sparse {
            self.replace(ops, lo, lo, hi, false)?;
        }
        Ok(())
    }

    pub(crate) fn handles(&self) -> Vec<u32> {
        self.blocks
            .values()
            .map(|b| b.handle)
            .filter(|&h| h != 0)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    include!("sparse/tests.rs");
}
