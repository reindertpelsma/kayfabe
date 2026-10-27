//! Selective sparse reservation geometry. Receipts describe only our own RM objects.

use crate::{RmError, channel::GuestVaRange};
use std::collections::{BTreeMap, BTreeSet};

#[path = "sparse/initial.rs"]
pub(crate) mod initial;
#[path = "sparse/state.rs"]
mod state;
use state::{Cache, Live};
#[path = "sparse/recycle.rs"]
mod recycle;
#[path = "sparse/slice.rs"]
pub(crate) mod slice;

/// Normal reservation grain.
pub const NORMAL_GRAIN: u64 = 2 << 20;
/// Sparse reservation grain.
pub const SPARSE_GRAIN: u64 = 64 << 10;
/// Legacy per-space policy floor; live limits come from [`policy::Budget`].
pub const BLOCK_LIMIT: usize = 8192;
const PAGE: u64 = 4096;
/// Refused sparse windows remembered per space.
const WINDOWS: usize = 64;
/// Retry ticks a refused window stays remembered.
const WINDOW_TTL: u64 = 256;
/// Empty adjacent pairs one recycle merges at most (3 RM verbs each).
const MAX_PAIRS: usize = 256;
/// Blocks kept free for explicit sparse admission: speculative plain-map refinement (which
/// only prepares a later carve) stops this far below the limit, and costs no RM verb there.
pub(crate) const fn speculative_reserve(limit: usize) -> usize {
    limit / 32
}
/// Free blocks one recycle makes (as far as empty adjacent pairs and the slice allow), so
/// a full tree is not recycled again by the very next admission.
pub(crate) const fn recycle_headroom(limit: usize) -> usize {
    if limit / 256 == 0 { 1 } else { limit / 256 }
}
/// Refresh ticks the first RM refusal silences optional layout work for; doubles per refusal.
pub(crate) const BACKOFF_FIRST: u64 = 2;
/// Longest refusal back-off, in refresh ticks.
pub(crate) const BACKOFF_MAX: u64 = 64;
/// Entries of the session-wide counter array ([`Stats::deltas`]).
pub(crate) const SPARSE_TOTALS: usize = 23;

#[path = "sparse/deficit.rs"]
mod deficit;
pub(crate) use deficit::adjust_gauge as adjust_deficit_gauge;
#[path = "sparse/policy.rs"]
pub mod policy;

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

/// Whether a tree whose layout repair failed refuses this call. Sparse bookkeeping stops;
/// routing a plain map or unmap never does (it only names handles that still exist).
#[must_use]
pub(crate) fn poison_refuses(poisoned: bool, routing: bool) -> bool {
    poisoned && !routing
}

/// Reservation verbs, with a host implementation and an adversarial model.
pub(crate) trait Ops {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError>;
    fn release(&self, handle: u32) -> Result<(), RmError>;
    /// Whether this refresh may START more layout work of this kind (the time slice,
    /// [`slice::Slice`]; `demand` is an explicit sparse admission, else optional work).
    /// Restoring a half-done replacement never asks: it must finish.
    fn budget_left(&self, _demand: bool) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
struct Block {
    lo: u64,
    hi: u64,
    handle: u32,
    sparse: bool,
    live: Live,
    wanted: [u64; 8],
}

/// Bounded event/byte counters and block gauges; each field states its scope.
/// No guest-controlled log stream or distinct-object inference from cumulative events.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Reservation objects created after initial space creation (cumulative).
    pub made: u64,
    /// Geometry releases (cumulative). Initial roots and final teardown make session
    /// made-minus-freed insufficient for live usage; use the live_blocks gauge.
    pub freed: u64,
    /// Host RM verb refusals (a reserve or release that RM, or the session cap, refused).
    pub refused: u64,
    /// SPARSE admissions declined by occupancy or object budget (closed fallback): one per
    /// declined admission. Plain maps never count here.
    pub overflow: u64,
    /// Sparse bytes left invalid at admission, summed per admission (a run retried later is
    /// counted again).
    pub unhonoured_bytes: u64,
    /// Sparse-to-invalid bytes retained in occupied sparse blocks.
    pub retained_bytes: u64,
    /// Plain maps placed in a coarse block because refinement was declined (object budget,
    /// a populated coarse block, or an RM refusal): one per map call. They are NOT refused.
    pub coarse_maps: u64,
    /// Default-on admission calls, including cached closed fallback.
    pub admit_calls: u64,
    /// Original blocks copied by local undo (never unrelated tree nodes).
    pub admit_blocks: u64,
    /// Genuine hard admission errors (validation/RM/repair), not occupied/budget fallback.
    pub admit_hard_err: u64,
    /// Closed admission results.
    pub admit_decline: u64,
    /// Total and peak admission wall time (not hardware time).
    pub admit_ns_total: u64,
    /// Maximum single admission wall time.
    pub admit_ns_max: u64,
    /// Negative-cache hits and bounded-entry evictions.
    pub negative_hits: u64,
    /// Negative-cache eviction events.
    pub negative_evicted: u64,
    /// Occupancy interval cap: the block stays conservatively occupied.
    pub occupancy_capped: u64,
    /// Live object high-water in this space, including initial roots.
    pub peak_blocks: u64,
    /// Current tree blocks, including initial roots.
    pub live_blocks: u64,
    /// Host RM `NoMemory` refusals turned into the counted closed fallback (cumulative).
    pub rm_declined: u64,
    /// Layout RM work skipped because a refusal back-off is running (cumulative).
    pub backoff_skips: u64,
    /// Layout work deferred to a later refresh because the time slice was spent.
    pub slice_deferred: u64,
    /// Speculative plain-map refinements skipped above the high-water mark (no RM verb).
    pub speculative_skipped: u64,
    /// Admissions answered by the remembered refused-window cache without an RM verb.
    pub window_hits: u64,
    /// Current guest-sparse bytes lacking native sparse coverage.
    pub current_unhonoured_bytes: u64,
    /// Conservative upper bound after missing-range tracking fills.
    pub missing_capped: bool,
    /// Distinct permanent withheld windows encountered.
    pub withheld_windows: u64,
}

impl Stats {
    pub(crate) fn deltas(self, before: Self) -> [u64; SPARSE_TOTALS] {
        let after = self;
        [
            after.made.saturating_sub(before.made),
            after.freed.saturating_sub(before.freed),
            after.refused.saturating_sub(before.refused),
            after.overflow.saturating_sub(before.overflow),
            after
                .unhonoured_bytes
                .saturating_sub(before.unhonoured_bytes),
            after.retained_bytes.saturating_sub(before.retained_bytes),
            after.coarse_maps.saturating_sub(before.coarse_maps),
            after.admit_calls.saturating_sub(before.admit_calls),
            after.admit_hard_err.saturating_sub(before.admit_hard_err),
            after.admit_decline.saturating_sub(before.admit_decline),
            after.admit_ns_total.saturating_sub(before.admit_ns_total),
            0,
            after.negative_hits.saturating_sub(before.negative_hits),
            after
                .negative_evicted
                .saturating_sub(before.negative_evicted),
            after
                .occupancy_capped
                .saturating_sub(before.occupancy_capped),
            0,
            after.admit_blocks.saturating_sub(before.admit_blocks),
            after.rm_declined.saturating_sub(before.rm_declined),
            after.backoff_skips.saturating_sub(before.backoff_skips),
            after.slice_deferred.saturating_sub(before.slice_deferred),
            after
                .speculative_skipped
                .saturating_sub(before.speculative_skipped),
            after.window_hits.saturating_sub(before.window_hits),
            after
                .withheld_windows
                .saturating_sub(before.withheld_windows),
        ]
    }
}

/// Bounded, nonblocking heartbeat census: native space id, current blocks, lifetime peak.
#[derive(Debug)]
pub struct Census {
    /// At most128 busiest spaces.
    pub rows: Vec<(u32, u64, u64)>,
    /// Active tree count before output truncation.
    pub total: usize,
    /// Contended rows (or a contended registry), omitted rather than waited on.
    pub skipped: usize,
}

#[derive(Debug)]
pub(crate) struct Reservations {
    blocks: BTreeMap<u64, Block>,
    limit: usize,
    pub(crate) stats: Stats,
    pub(crate) poisoned: bool,
    /// Set by occupancy or object budget; the caller decides what a decline means.
    declined: bool,
    cache: Cache,
    retry_dirty: BTreeSet<u64>,
    mergeable: BTreeSet<u64>,
    ceiling: u64,
    /// Retry calls so far: the clock of the refusal back-off and of the refused windows.
    tick: u64,
    backoff_until: u64,
    backoff_step: u64,
    /// Sparse tiles host RM refused, `lo -> (hi, tick)`: not asked again (bounded).
    refused: BTreeMap<u64, (u64, u64)>,
    /// Sparse blocks with neither intent nor live pages, waiting to become normal again.
    cleanup: BTreeSet<u64>,
    /// Set when the time slice or a back-off (not the object budget) stopped this call.
    deferred: bool,
    /// Set when host RM refused a reservation during this call.
    rm_refused: bool,
    allowed: Option<Vec<(u64, u64)>>,
    withheld_seen: u8,
    deficit: deficit::Deficit,
}

impl Reservations {
    pub(crate) fn new_guest(guest: &[GuestVaRange], limit: usize) -> Self {
        let mut tree = Self::new(guest, limit);
        let mut allowed: Vec<_> = guest
            .iter()
            .filter(|g| g.handle != 0)
            .map(|g| (g.lo, g.hi))
            .collect();
        allowed.sort_unstable();
        tree.allowed = Some(allowed);
        tree
    }
    pub(crate) fn missing_warning(&mut self, now_ns: u64) -> Option<String> {
        self.deficit.warning(now_ns, self.ceiling)
    }
    // Only initial guest reservations are sparse-able. Their complement has at most4
    // windows, independently of how many guest runs name them or how the tree is split.
    fn withheld(&mut self, va: u64, end: u64) -> bool {
        let Some(allowed) = &self.allowed else {
            return false;
        };
        let mut start = super::channel::TWIN_VA_FLOOR;
        let mut blocked = false;
        for (i, &(lo, hi)) in allowed
            .iter()
            .chain(std::iter::once(&(self.ceiling, self.ceiling)))
            .enumerate()
        {
            if start < lo && va < lo && start < end {
                blocked = true;
                let bit = 1u8 << i;
                if self.withheld_seen & bit == 0 {
                    self.withheld_seen |= bit;
                    self.stats.withheld_windows = self.stats.withheld_windows.saturating_add(1);
                }
            }
            start = hi;
        }
        blocked
    }
    fn reconcile_deficit(&mut self, va: u64, end: u64) {
        self.deficit.mark(va, end, false, self.ceiling);
        let mut cur = va;
        let lo = self.key(va).unwrap_or(va);
        for b in self.blocks.range(lo..end).map(|(_, b)| b) {
            let a = va.max(b.lo);
            let stop = end.min(b.hi);
            if cur < a {
                self.deficit.mark(cur, a, true, self.ceiling);
            }
            if !b.sparse {
                self.deficit.mark(a, stop, true, self.ceiling);
            }
            cur = stop;
        }
        if cur < end {
            self.deficit.mark(cur, end, true, self.ceiling);
        }
    }
    pub(crate) fn counters(&self) -> Stats {
        Stats {
            current_unhonoured_bytes: self.deficit.bytes,
            missing_capped: self.deficit.capped,
            live_blocks: self.blocks.len() as u64,
            peak_blocks: self.stats.peak_blocks.max(self.blocks.len() as u64),
            ..self.stats
        }
    }
    pub(crate) fn new(guest: &[GuestVaRange], limit: usize) -> Self {
        let mut tree = Self {
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
                            live: Live::default(),
                            wanted: [0; 8],
                        },
                    )
                })
                .collect(),
            limit,
            stats: Stats {
                peak_blocks: guest.iter().filter(|g| g.handle != 0).count() as u64,
                ..Stats::default()
            },
            poisoned: false,
            declined: false,
            cache: Cache::default(),
            retry_dirty: BTreeSet::new(),
            mergeable: BTreeSet::new(),
            ceiling: guest.iter().map(|g| g.hi).max().unwrap_or(0),
            tick: 0,
            backoff_until: 0,
            backoff_step: 0,
            refused: BTreeMap::new(),
            cleanup: BTreeSet::new(),
            deferred: false,
            rm_refused: false,
            allowed: None,
            withheld_seen: 0,
            deficit: deficit::Deficit::default(),
        };
        tree.reindex(0, u64::MAX);
        tree
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

    /// The block budget. A decline is only recorded here; the caller decides what it means:
    /// a plain map goes on in its coarse block (`coarse_maps`), a sparse admission is left
    /// invalid (`overflow`).
    fn budget(&mut self, count: usize) -> Result<(), RmError> {
        if count > self.limit {
            self.declined = true;
            return Err(RmError::NoMemory);
        }
        Ok(())
    }

    /// Whether layout RM work of this kind may start now. A running refusal back-off or a
    /// spent time slice defers it (`deferred`): nothing is lost, the caller keeps what it has
    /// and a later refresh continues.
    fn layout_ok(&mut self, ops: &dyn Ops, demand: bool) -> bool {
        if self.tick < self.backoff_until {
            self.stats.backoff_skips = self.stats.backoff_skips.saturating_add(1);
            self.deferred = true;
            return false;
        }
        if !ops.budget_left(demand) {
            self.stats.slice_deferred = self.stats.slice_deferred.saturating_add(1);
            self.deferred = true;
            return false;
        }
        true
    }

    /// Host RM said `NoMemory` to a reservation: a counted closed fallback, and layout RM work
    /// is silenced for a doubling number of ticks so a refusing RM is not asked per walk.
    fn note_refusal(&mut self) {
        self.stats.rm_declined = self.stats.rm_declined.saturating_add(1);
        self.rm_refused = true;
        self.backoff_step = if self.backoff_step == 0 {
            BACKOFF_FIRST
        } else {
            self.backoff_step.saturating_mul(2).min(BACKOFF_MAX)
        };
        self.backoff_until = self.tick.saturating_add(self.backoff_step);
    }

    fn note_success(&mut self) {
        if self.tick >= self.backoff_until {
            self.backoff_step = 0;
        }
    }

    fn window_hit(&self, tile: u64) -> bool {
        self.refused
            .range(..=tile)
            .next_back()
            .is_some_and(|(_, &(hi, at))| tile < hi && self.tick.saturating_sub(at) < WINDOW_TTL)
    }

    fn remember_refused(&mut self, lo: u64, hi: u64) {
        let (mut lo, mut hi) = (lo, hi);
        if let Some((&plo, &(phi, _))) = self.refused.range(..=lo).next_back()
            && phi >= lo
        {
            self.refused.remove(&plo);
            lo = plo;
            hi = hi.max(phi);
        }
        self.refused.insert(lo, (hi, self.tick));
        while self.refused.len() > WINDOWS {
            self.refused.pop_first();
        }
    }

    /// Whether `normal` has any coarse reservation to split in `[va, end)`.
    fn needs_split(&self, va: u64, end: u64) -> bool {
        let mut cur = va;
        while cur < end {
            if let Some(key) = self.key(cur) {
                let Some(b) = self.blocks.get(&key) else {
                    return false;
                };
                if b.hi - b.lo > NORMAL_GRAIN && !b.sparse {
                    return true;
                }
                cur = b.hi.min(end);
            } else {
                cur = self
                    .blocks
                    .range(cur..)
                    .next()
                    .map_or(end, |(&lo, _)| lo.min(end));
            }
        }
        false
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
        b.live.any()
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
        // A block lost to a failed repair has no reservation to free or to split.
        if old.handle == 0 {
            return Err(RmError::NoMemory);
        }
        if Self::occupied(&old) {
            self.declined = true;
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
        self.cache.touch(old.lo, old.hi);
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
                        live: Live::default(),
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
                    if e == RmError::NoMemory {
                        self.note_refusal();
                        self.declined = true;
                    }
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
        self.note_success();
        self.blocks.remove(&key);
        self.retry_dirty.remove(&key);
        self.mergeable.remove(&key);
        for child in made {
            if !child.sparse && child.wanted.iter().any(|&v| v != 0) {
                self.retry_dirty.insert(child.lo);
            }
            self.blocks.insert(child.lo, child);
        }
        self.stats.peak_blocks = self.stats.peak_blocks.max(self.blocks.len() as u64);
        self.reindex(old.lo, old.hi);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn normal(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<(), RmError> {
        self.normal_with(ops, va, len, true)
    }

    fn normal_with(
        &mut self,
        ops: &dyn Ops,
        va: u64,
        len: u64,
        demand: bool,
    ) -> Result<(), RmError> {
        let end = Self::end(va, len)?;
        // Nothing to split: no recycling, no budget check, no RM verb.
        if !self.needs_split(va, end) {
            return Ok(());
        }
        self.recycle(ops, 2, demand)?;
        self.budget(((end - 1) / NORMAL_GRAIN - va / NORMAL_GRAIN + 1) as usize)?;
        let mut cur = va;
        while cur < end {
            if let Some(key) = self.key(cur) {
                let b = self.blocks.get(&key).ok_or(RmError::NoMemory)?;
                let stop = b.hi.min(end);
                if b.hi - b.lo > NORMAL_GRAIN && !b.sparse {
                    let lo = (cur / NORMAL_GRAIN * NORMAL_GRAIN).max(b.lo);
                    let hi = lo.saturating_add(NORMAL_GRAIN).min(b.hi);
                    if !self.layout_ok(ops, demand) {
                        return Err(RmError::NoMemory);
                    }
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
        if Self::occupied(b) {
            self.declined = true;
            return Ok(false);
        }
        if !Self::wanted(b, va, va + SPARSE_GRAIN) {
            return Ok(false);
        }
        if !self.layout_ok(ops, true) {
            return Ok(false);
        }
        self.replace(ops, key, va, va + SPARSE_GRAIN, true)?;
        Ok(true)
    }

    pub(crate) fn admit(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<bool, RmError> {
        let start = std::time::Instant::now();
        self.stats.admit_calls = self.stats.admit_calls.saturating_add(1);
        let result = self.admit_local(ops, va, len);
        if result.is_ok() {
            self.reconcile_deficit(va, va + len);
        }
        let ns = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        crate::diag::admission(ns);
        self.stats.admit_ns_total = self.stats.admit_ns_total.saturating_add(ns);
        self.stats.admit_ns_max = self.stats.admit_ns_max.max(ns);
        self.stats.peak_blocks = self.stats.peak_blocks.max(self.blocks.len() as u64);
        if result.is_err() {
            self.stats.admit_hard_err = self.stats.admit_hard_err.saturating_add(1);
        } else if result == Ok(false) {
            self.stats.admit_decline = self.stats.admit_decline.saturating_add(1);
        }
        result
    }

    fn declined(&mut self, len: u64, overflow: bool) -> Result<bool, RmError> {
        if overflow {
            self.stats.overflow = self.stats.overflow.saturating_add(1);
        }
        self.stats.unhonoured_bytes = self.stats.unhonoured_bytes.saturating_add(len);
        Ok(false)
    }

    fn admit_local(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<bool, RmError> {
        let end = Self::end(va, len)?;
        if end > self.ceiling {
            return Err(RmError::NoMemory);
        }
        if self.withheld(va, end) {
            return self.declined(len, false);
        }
        let accepted_before = self.cache.entries.get(&(va, len)).is_some_and(|n| n.retry);
        if let Some(n) = self.cache.get((va, len), self.blocks.len(), self.tick) {
            self.stats.negative_hits = self.stats.negative_hits.saturating_add(1);
            self.cache.accept((va, len));
            return self.declined(len, n.overflow);
        }
        self.declined = false;
        self.deferred = false;
        self.rm_refused = false;
        let lo = self.key(va).unwrap_or(va);
        let hi = self
            .key(end - 1)
            .and_then(|k| self.blocks.get(&k))
            .map_or(end, |b| b.hi);
        // A request larger than the fixed tile budget needs no snapshot or RM work.
        if ((end - 1) / SPARSE_GRAIN - va / SPARSE_GRAIN + 1) as usize > self.limit {
            self.remember(va, len, lo, hi, true, true);
            return self.declined(len, true);
        }
        self.recycle(ops, 4, true)?;
        let lo = self.key(va).unwrap_or(va);
        let hi = self
            .key(end - 1)
            .and_then(|k| self.blocks.get(&k))
            .map_or(end, |b| b.hi);
        // Undo only the intersecting original blocks. All replacement children are within
        // their parents, so error cleanup cannot scan unrelated existing blocks.
        let snapshot: BTreeMap<_, _> = {
            let mut clone = crate::diag::Scope::start(crate::diag::Phase::Clone, 0);
            let saved: BTreeMap<_, _> = self
                .blocks
                .range(lo..end)
                .map(|(&k, b)| (k, b.clone()))
                .collect();
            clone.add_work(saved.len() as u64);
            saved
        };
        self.stats.admit_blocks = self
            .stats
            .admit_blocks
            .saturating_add(snapshot.len() as u64);
        let result = self.admit_inner(ops, va, len);
        if result.is_err() {
            let fresh: Vec<_> = self
                .blocks
                .range(lo..hi)
                .map(|(_, b)| b)
                .filter(|b| {
                    b.sparse
                        && !snapshot
                            .range(..=b.lo)
                            .next_back()
                            .is_some_and(|(_, old)| old.sparse && b.hi <= old.hi)
                })
                .map(|b| (b.lo, b.hi))
                .collect();
            for (a, b) in fresh {
                if self.replace(ops, a, a, b, false).is_err() {
                    self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(b - a);
                }
            }
            let mut scan = crate::diag::Scope::start(crate::diag::Phase::Restore, 0);
            for b in self.blocks.range_mut(lo..hi).map(|(_, b)| b) {
                b.wanted = [0; 8];
                let Some((_, old)) = snapshot.range(..=b.lo).next_back() else {
                    continue;
                };
                if old.wanted.iter().all(|&v| v == 0) {
                    continue;
                }
                for p in (b.lo..b.hi.min(b.lo.saturating_add(NORMAL_GRAIN))).step_by(PAGE as usize)
                {
                    scan.add_work(1);
                    if p < old.hi && Self::wanted(old, p, p + PAGE) {
                        Self::bits(&mut b.wanted, b.lo, p, p + PAGE, true);
                    }
                }
            }
        }
        if result != Ok(true) {
            let overflow = self.declined;
            self.remember(
                va,
                len,
                lo,
                hi,
                overflow,
                accepted_before || result.is_ok() || overflow,
            );
            if self.deferred {
                // The slice or a back-off stopped it, not the geometry: a later refresh resumes.
                self.cache.expire((va, len));
            }
            if result.is_ok() || overflow {
                return self.declined(len, overflow);
            }
        } else {
            self.cache.entries.remove(&(va, len));
        }
        result
    }

    fn remember(&mut self, va: u64, len: u64, lo: u64, hi: u64, overflow: bool, retry: bool) {
        let n = state::Negative {
            lo,
            hi,
            epoch: 0,
            blocks: self.blocks.len(),
            overflow,
            retry,
            due: if self.rm_refused {
                self.backoff_until
            } else {
                0
            },
        };
        if self.cache.insert((va, len), n) {
            self.stats.negative_evicted = self.stats.negative_evicted.saturating_add(1);
        }
    }

    fn admit_inner(&mut self, ops: &dyn Ops, va: u64, len: u64) -> Result<bool, RmError> {
        let end = Self::end(va, len)?;
        self.budget(((end - 1) / SPARSE_GRAIN - va / SPARSE_GRAIN + 1) as usize)?;
        // Empty normal geometry is prepared before any intent is recorded. A failure
        // therefore leaves no partly accepted intent for the GPU diff to forget.
        if let Err(e) = self.normal_with(ops, va, len, true) {
            return if self.deferred { Ok(false) } else { Err(e) };
        }
        let mut all = true;
        let mut cur = va;
        while cur < end {
            let tile = cur / SPARSE_GRAIN * SPARSE_GRAIN;
            let tile_end = tile.checked_add(SPARSE_GRAIN).ok_or(RmError::NoMemory)?;
            let stop = tile_end.min(end);
            if let Some(key) = self.key(cur) {
                let b = self.blocks.get_mut(&key).ok_or(RmError::NoMemory)?;
                let before = b.wanted;
                Self::bits(&mut b.wanted, b.lo, cur, stop, true);
                if b.wanted != before {
                    self.cache.touch(b.lo, b.hi);
                    if !b.sparse {
                        self.retry_dirty.insert(b.lo);
                    }
                }
                self.reindex(cur, stop);
                all &= self.sparsify(ops, tile)?;
                if self.deferred {
                    return Ok(false);
                }
            } else if cur == tile && stop == tile_end {
                if self.window_hit(tile) {
                    self.stats.window_hits = self.stats.window_hits.saturating_add(1);
                    all = false;
                    cur = stop;
                    continue;
                }
                if !self.layout_ok(ops, true) {
                    return Ok(false);
                }
                self.budget(self.blocks.len().saturating_add(1))?;
                let handle = match ops.reserve(tile, SPARSE_GRAIN, true) {
                    Ok(h) => h,
                    Err(e) => {
                        self.stats.refused = self.stats.refused.saturating_add(1);
                        if e == RmError::NoMemory {
                            self.note_refusal();
                            self.remember_refused(tile, tile_end);
                            self.declined = true;
                        }
                        return Err(e);
                    }
                };
                self.note_success();
                self.stats.made = self.stats.made.saturating_add(1);
                self.blocks.insert(
                    tile,
                    Block {
                        lo: tile,
                        hi: tile_end,
                        handle,
                        sparse: true,
                        live: Live::default(),
                        wanted: [u64::MAX >> 48, 0, 0, 0, 0, 0, 0, 0],
                    },
                );
            } else {
                all = false;
            }
            cur = stop;
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
        self.deficit.mark(va, end, false, self.ceiling);
        self.cache.retire(va, end);
        let start_key = self.key(va).unwrap_or(va);
        let keys: Vec<_> = self
            .blocks
            .range(start_key..end)
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
                // Becoming normal again is optional work: queued, done within the slice.
                self.cleanup.insert(key);
            } else {
                retained = retained.saturating_add(b.hi.min(end).saturating_sub(b.lo.max(va)));
            }
        }
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(retained);
        self.convert_cleanup(ops)?;
        self.reindex(va, end);
        Ok(retained)
    }

    pub(crate) fn mark(&mut self, va: u64, len: u64, live: bool) {
        let end = va.saturating_add(len);
        let start_key = self.key(va).unwrap_or(va);
        for b in self
            .blocks
            .range_mut(start_key..end)
            .map(|(_, b)| b)
            .filter(|b| va < b.hi)
        {
            let occupied_before = b.live.any();
            if b.live.mark(b.lo, b.hi, va, end, live) {
                self.stats.occupancy_capped = self.stats.occupancy_capped.saturating_add(1);
            }
            self.cache.touch(b.lo, b.hi);
            if occupied_before && !b.live.any() {
                self.cache.capacity_changed();
            }
            if !b.sparse && !b.live.any() && b.wanted.iter().any(|&v| v != 0) {
                self.retry_dirty.insert(b.lo);
            }
        }
        self.reindex(va, end);
    }

    /// Turn queued intent-free, unoccupied sparse blocks back into normal reservations, as far
    /// as the optional slice allows; the rest waits for the next refresh.
    fn convert_cleanup(&mut self, ops: &dyn Ops) -> Result<(), RmError> {
        while let Some(&key) = self.cleanup.first() {
            let Some(b) = self.blocks.get(&key) else {
                self.cleanup.remove(&key);
                continue;
            };
            if !b.sparse || Self::occupied(b) || b.wanted.iter().any(|&v| v != 0) {
                self.cleanup.remove(&key);
                continue;
            }
            let (lo, hi) = (b.lo, b.hi);
            if !self.layout_ok(ops, false) {
                break;
            }
            self.cleanup.remove(&key);
            match self.replace(ops, key, lo, hi, false) {
                Ok(()) => {}
                Err(RmError::NoMemory) => {
                    self.cleanup.insert(key);
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub(crate) fn retry(&mut self, ops: &dyn Ops) -> Result<(), RmError> {
        self.tick = self.tick.saturating_add(1);
        self.deferred = false;
        self.convert_cleanup(ops)?;
        let changed: Vec<_> = self
            .cache
            .entries
            .iter()
            .filter(|(_, n)| n.retry && !self.cache.valid(**n, self.blocks.len(), self.tick))
            .map(|(&key, _)| key)
            .collect();
        for (va, len) in changed {
            if !ops.budget_left(true) {
                self.stats.slice_deferred = self.stats.slice_deferred.saturating_add(1);
                return Ok(());
            }
            self.admit(ops, va, len)?;
        }
        if !ops.budget_left(true) {
            return Ok(());
        }
        let dirty = std::mem::take(&mut self.retry_dirty);
        let tiles: Vec<_> = dirty
            .into_iter()
            .filter_map(|key| self.blocks.get(&key))
            .filter(|b| !b.sparse && !Self::occupied(b) && b.hi - b.lo <= NORMAL_GRAIN)
            .flat_map(|b| {
                let lo = b.lo.div_ceil(SPARSE_GRAIN) * SPARSE_GRAIN;
                (lo..b.hi)
                    .step_by(SPARSE_GRAIN as usize)
                    .filter(|&p| p + SPARSE_GRAIN <= b.hi && Self::wanted(b, p, p + SPARSE_GRAIN))
            })
            .collect();
        let mut tiles = tiles.into_iter();
        while let Some(tile) = tiles.next() {
            if !ops.budget_left(true) {
                self.stats.slice_deferred = self.stats.slice_deferred.saturating_add(1);
                for t in std::iter::once(tile).chain(tiles) {
                    if let Some(k) = self.key(t) {
                        self.retry_dirty.insert(k);
                    }
                }
                break;
            }
            let cached = self.cache.entries.iter().any(|(&(va, len), n)| {
                va <= tile
                    && tile + SPARSE_GRAIN <= va.saturating_add(len)
                    && self.cache.valid(*n, self.blocks.len(), self.tick)
            });
            if !cached {
                self.admit(ops, tile, SPARSE_GRAIN)?;
            }
        }
        Ok(())
    }

    pub(crate) fn map_undo(&self, va: u64, len: u64) -> MapUndo {
        let end = va.saturating_add(len);
        let lo = self.key(va).unwrap_or(va);
        MapUndo(
            self.blocks
                .range(lo..end)
                .map(|(&k, b)| (k, b.live.clone()))
                .collect(),
        )
    }

    pub(crate) fn restore_map(&mut self, undo: MapUndo) {
        for (key, live) in undo.0 {
            if let Some(b) = self.blocks.get_mut(&key) {
                b.live = live;
            }
            self.cache
                .touch(key, self.blocks.get(&key).map_or(key, |b| b.hi));
            self.reindex(key, key.saturating_add(1));
        }
    }

    /// Route a plain (backed) map: refine the geometry around it, then name the host
    /// reservation handle of every piece. Occupancy is recorded before the host call.
    #[cfg(test)]
    pub(crate) fn route_map(
        &mut self,
        ops: &dyn Ops,
        range: u32,
        va: u64,
        len: u64,
        refine: bool,
    ) -> Result<Vec<(u32, u64, u64)>, RmError> {
        let pieces = self.prepare_map(ops, range, va, len, refine)?;
        self.mark(va, len, true);
        Ok(pieces)
    }

    pub(crate) fn prepare_map(
        &mut self,
        ops: &dyn Ops,
        range: u32,
        va: u64,
        len: u64,
        refine: bool,
    ) -> Result<Vec<(u32, u64, u64)>, RmError> {
        // Arguments the twin-OFF path also refuses (below the floor, wrapping, unaligned).
        let end = Self::end(va, len)?;
        // Refinement only prepares a later sparse carve. A decline (budget, a populated coarse
        // block, an RM refusal) never refuses the map: it goes in the coarse block exactly as
        // with the twin OFF, and is counted. Above the high-water mark, with no empty pair to
        // recycle, the refinement is skipped without any RM verb (ENS6): what is left is for
        // explicit sparse admission. With pairs to recycle it recycles in a batch, under the
        // slice and the refusal back-off.
        self.declined = false;
        self.deferred = false;
        let coarse = if !refine || self.poisoned {
            true
        } else if !self.needs_split(va, end) {
            false
        } else if self.mergeable.is_empty()
            && self
                .blocks
                .len()
                .saturating_add(2)
                .saturating_add(speculative_reserve(self.limit))
                > self.limit
        {
            self.stats.speculative_skipped = self.stats.speculative_skipped.saturating_add(1);
            true
        } else {
            self.normal_with(ops, va, len, false).is_err()
        };
        if coarse {
            self.stats.coarse_maps = self.stats.coarse_maps.saturating_add(1);
        }
        self.pieces(range, va, len)
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
                // A block lost to a failed repair is placed through the range object, as an
                // unreserved range is with the twin OFF.
                (if b.handle == 0 { range } else { b.handle }, b.hi.min(end))
            } else {
                (
                    range,
                    self.blocks
                        .range(cur..)
                        .next()
                        .map_or(end, |(&lo, _)| lo.min(end)),
                )
            };
            if handle == 0
                || stop <= cur
                || pieces.len() > self.limit.saturating_mul(2).saturating_add(1)
            {
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
        self.deficit = deficit::Deficit::default();
        self.withheld_seen = 0;
        self.cache = Cache::default();
        self.retry_dirty.clear();
        self.refused.clear();
        self.cleanup.clear();
        self.backoff_until = 0;
        self.backoff_step = 0;
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

pub(crate) struct MapUndo(Vec<(u64, Live)>);

#[cfg(test)]
mod tests {
    include!("sparse/tests.rs");
}
