//! ★ 2026-11-18 — **Option C2: exact block tiling of a guest VA range** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md`
//! §8). STAGE 1: the placement engine and its GPU-free proof; NOT wired into `apply` yet (stage 2 wires it
//! when the walker emits SPARSE runs).
//!
//! Every byte of the managed range `[lo, hi)` lies in exactly one host `NV50_MEMORY_VIRTUAL` block, and
//! every block has a **base**: [`Base::Invalid`] (a plain reservation: an unmapped VA faults) or
//! [`Base::Sparse`] (`_SPARSE`: an unmapped VA reads zero, and an unmap inside it returns to sparse,
//! `ogkm-580 gpu_vaspace.c:1577-1600`, `2279-2289`). The engine keeps the host's bases equal to the
//! guest's wish ([`Tiling::set_base`], one interval map) at 64 KiB tile granularity:
//! - **S1**: a tile whose unmapped pages are all guest-SPARSE is SPARSE on the host;
//! - **S2**: a tile with any guest-INVALID page is INVALID on the host (INVALID wins a mixed tile, so a
//!   real fault is never turned into a silent zero).
//!
//! A block that holds a mapping (a **row block**) is never freed: freeing it would unmap a guest row
//! (the guard: "an unchanged VA stays mapped at all times"). Only EMPTY blocks are freed and re-laid out.
//! So a mapping is placed through a block that covers just its own tiles ([`Tiling::prepare_row`] carves
//! one out of an empty block first), and the rest of the range stays free to re-tile.
//!
//! Host facts the layout obeys (`[measured 2026-11-18, kf-va-tiling-probe M5, RTX 3090, 580.159.04]`):
//! - a default-attr block of 2 MiB or more is rounded OUTWARD to 2 MiB at both ends
//!   (`gpu_vaspace.c:1835-1930`), so such a block must be 2 MiB-aligned, or be a fence with
//!   `_ATTR_PAGE_SIZE_BIG` + `_ALIGNMENT_FORCE` ([`Attr::BigForced`], exact at 64 KiB);
//! - a `_SPARSE` block smaller than 64 KiB is refused: the tile is 64 KiB ([`TILE`]).
//!
//! Hostile-guest bounds: every guest value is clipped to the range with checked arithmetic; the block
//! count is capped ([`TilingCfg::max_blocks`]); the wish map is capped ([`TilingCfg::max_wish_runs`]);
//! optional work (making a tile SPARSE, carving a row block, merging) is cut by a per-batch host-call
//! budget ([`Tiling::begin_batch`]), and what is cut stays visible ([`Tiling::pending`]). Restoring
//! INVALID (S2) is never cut by the budget. A refused host call rolls the step back to the old layout;
//! a refused restore leaves an unreserved gap (which reads INVALID, so S2 still holds) that every later
//! call heals first.

use std::collections::BTreeMap;

/// The tile: the host's 64 KiB big page, the smallest `_SPARSE` block host RM accepts.
pub const TILE: u64 = 0x1_0000;
/// The huge page: a default-attr block this long or longer is rounded to it.
pub const HUGE: u64 = 0x20_0000;

/// What an unmapped VA of a host block reads as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// A plain reservation: an access faults.
    Invalid,
    /// `NVOS32_ALLOC_FLAGS_SPARSE`: a read returns zero, a write is dropped.
    Sparse,
}

/// How a block is reserved (its `NVOS32` attr and alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attr {
    /// Default attr and alignment: exact for a block under [`HUGE`], or one 2 MiB-aligned at both ends.
    Default,
    /// `_ATTR_PAGE_SIZE_BIG` + `_ALLOC_FLAGS_ALIGNMENT_FORCE`, alignment 64 KiB: exact at any 64 KiB
    /// edges. Used for EMPTY blocks only (a row block keeps the default page sizes).
    BigForced,
}

/// One host block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// End (exclusive); the start is the map key.
    pub hi: u64,
    /// What its unmapped VAs read as.
    pub base: Base,
    /// How it was reserved.
    pub attr: Attr,
    /// The host handle (the `hDma` a mapping inside it names).
    pub handle: u32,
}

/// The two host verbs the engine needs. Production: `HostRm` reservations under the space's tiling
/// lock; tests: the `sim` host model.
pub trait TileVerbs {
    /// Reserve exactly `[lo, hi)` with `base` and `attr`.
    ///
    /// # Errors
    /// The host's refusal.
    fn reserve(&self, lo: u64, hi: u64, base: Base, attr: Attr) -> Result<u32, String>;
    /// Free a block the engine reserved (it holds no mapping).
    ///
    /// # Errors
    /// The host's refusal (the block is then still there).
    fn free(&self, handle: u32) -> Result<(), String>;
    /// ★ §8.3/§8.9: a re-tile is about to free a block — take the PLACEMENT LOCK
    /// ([`crate::placement`]) so no non-FIXED placement can land in the freed VA until
    /// [`TileVerbs::retile_end`]. Production refuses on a vCPU or guest-trap thread (re-tiles run
    /// on the VA-manager thread); the refusal stops the re-tile before any host call.
    ///
    /// # Errors
    /// Why the re-tile may not run here.
    fn retile_begin(&self) -> Result<(), String> {
        Ok(())
    }
    /// The re-tile is done (its last reserve, or its rollback): release the placement lock.
    fn retile_end(&self) {}
}

/// Limits.
#[derive(Debug, Clone, Copy)]
pub struct TilingCfg {
    /// Blocks per managed range. Optional growth stops here; mandatory S2 work may exceed it by the
    /// pieces of one layout (at most 3) per step.
    pub max_blocks: usize,
    /// Runs in the wish map; a SPARSE wish beyond it is refused (counted; the tile stays INVALID).
    pub max_wish_runs: usize,
    /// Host calls per batch for optional work ([`Tiling::begin_batch`]).
    pub batch_calls: u64,
    /// Whether [`Attr::BigForced`] may be used for empty blocks (`[measured 2026-11-18, kf-va-tiling-probe M1 and M5, traces/sparse_c2_tiling_probe_20261118]`: yes on 580).
    pub big_forced: bool,
}

impl Default for TilingCfg {
    fn default() -> Self {
        Self {
            max_blocks: 4096,
            max_wish_runs: 32_768,
            batch_calls: 4096,
            big_forced: true,
        }
    }
}

/// Why a guest request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TileError {
    /// Empty, wrapping or outside the managed range.
    Outside,
    /// A row that overlaps a live row, or a release of a row that is not live.
    NotARow,
    /// Part of the row has no host block (an unhealed gap after a refused restore).
    Unplaceable,
    /// The host refused a call; the step was rolled back.
    Refused(String),
}

/// What the engine did, for the budget and the log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TilingStats {
    /// Host calls (reserve + free).
    pub calls: u64,
    /// Steps rolled back after a refusal.
    pub rollbacks: u64,
    /// Restores that were themselves refused (a gap was left; healed later).
    pub gaps_left: u64,
    /// Optional steps cut by the batch budget or the block cap.
    pub cut: u64,
    /// Rows mapped through an existing empty block without a carve (cap or budget): the block is
    /// pinned until the row goes.
    pub pinned: u64,
    /// SPARSE wishes refused because the wish map is full.
    pub wish_refused: u64,
}

/// The exact tiling of one guest range.
#[derive(Debug)]
pub struct Tiling {
    lo: u64,
    hi: u64,
    cfg: TilingCfg,
    blocks: BTreeMap<u64, Block>,
    /// The guest's wish for unmapped VAs, `lo -> (hi, base)`, coalesced, covering `[lo, hi)`.
    wish: BTreeMap<u64, (u64, Base)>,
    /// Live rows, `va -> end`, non-overlapping.
    rows: BTreeMap<u64, u64>,
    batch_used: u64,
    /// Counters.
    pub stats: TilingStats,
}

/// `v` rounded down to `a` (a non-zero power of two: [`TILE`] or [`HUGE`]).
const fn down(v: u64, a: u64) -> u64 {
    v & !a.wrapping_sub(1)
}

/// `v` rounded up to `a` (as [`down`]); `None` if it wraps.
fn up(v: u64, a: u64) -> Option<u64> {
    v.checked_add(a.wrapping_sub(1)).map(|x| down(x, a))
}

/// `hi - lo`, or 0 for a reversed pair.
const fn span(lo: u64, hi: u64) -> u64 {
    hi.saturating_sub(lo)
}

fn bump(c: &mut u64) {
    *c = c.saturating_add(1);
}

/// The blocks that reserve `[lo, hi)` (tile-aligned) exactly: one block when the host will not round it,
/// else an aligned 2 MiB core plus edges under 2 MiB.
#[must_use]
pub fn layout(lo: u64, hi: u64, row: bool, big_forced: bool) -> Vec<(u64, u64, Attr)> {
    if hi <= lo {
        return Vec::new();
    }
    if span(lo, hi) < HUGE || (lo.is_multiple_of(HUGE) && hi.is_multiple_of(HUGE)) {
        return vec![(lo, hi, Attr::Default)];
    }
    if !row && big_forced {
        return vec![(lo, hi, Attr::BigForced)];
    }
    let a = up(lo, HUGE).unwrap_or(hi).min(hi);
    let b = down(hi, HUGE).max(a);
    [(lo, a), (a, b), (b, hi)]
        .into_iter()
        .filter(|&(x, y)| x < y)
        .map(|(x, y)| (x, y, Attr::Default))
        .collect()
}

/// A planned block: `(lo, hi, base, attr)`.
type Spec = (u64, u64, Base, Attr);

/// Adjacent runs with the same base become one.
fn coalesce(runs: Vec<(u64, u64, Base)>) -> Vec<(u64, u64, Base)> {
    let mut out: Vec<(u64, u64, Base)> = Vec::new();
    for r in runs {
        match out.last_mut() {
            Some(last) if last.1 == r.0 && last.2 == r.2 => last.1 = r.1,
            _ => out.push(r),
        }
    }
    out
}

impl Tiling {
    /// Tile `[lo, hi)` as one INVALID fence (its layout).
    ///
    /// # Errors
    /// [`TileError::Outside`] for an unaligned or empty range; the host's refusal (nothing is left).
    pub fn new(v: &dyn TileVerbs, lo: u64, hi: u64, cfg: TilingCfg) -> Result<Self, TileError> {
        if lo >= hi || !lo.is_multiple_of(TILE) || !hi.is_multiple_of(TILE) {
            return Err(TileError::Outside);
        }
        let mut t = Self {
            lo,
            hi,
            cfg,
            blocks: BTreeMap::new(),
            wish: BTreeMap::from([(lo, (hi, Base::Invalid))]),
            rows: BTreeMap::new(),
            batch_used: 0,
            stats: TilingStats::default(),
        };
        let new: Vec<Spec> = layout(lo, hi, false, cfg.big_forced)
            .into_iter()
            .map(|(a, b, at)| (a, b, Base::Invalid, at))
            .collect();
        if let Err(e) = t.replace(v, &[], &new) {
            t.release_all(v);
            return Err(e);
        }
        Ok(t)
    }

    /// Start a batch: the optional-work budget is [`TilingCfg::batch_calls`] again, and work cut in
    /// earlier batches is retried first (bounded by the same budget).
    pub fn begin_batch(&mut self, v: &dyn TileVerbs) {
        self.batch_used = 0;
        let pending = self.pending();
        // S2 first and never cut by the budget; then S1 while the budget lasts.
        for (a, b, _) in pending.iter().filter(|p| p.2 == Base::Invalid) {
            self.reconcile(v, *a, *b);
        }
        for (a, b, _) in pending.iter().filter(|p| p.2 == Base::Sparse) {
            if self.batch_used >= self.cfg.batch_calls {
                break;
            }
            self.reconcile(v, *a, *b);
        }
    }

    /// The managed range.
    #[must_use]
    pub const fn range(&self) -> (u64, u64) {
        (self.lo, self.hi)
    }

    /// The host blocks, `lo -> block`.
    #[must_use]
    pub const fn blocks(&self) -> &BTreeMap<u64, Block> {
        &self.blocks
    }

    /// Host calls left in this batch's optional budget.
    #[must_use]
    pub const fn budget_left(&self) -> u64 {
        self.cfg.batch_calls.saturating_sub(self.batch_used)
    }

    fn clip(&self, va: u64, len: u64) -> Result<(u64, u64), TileError> {
        let end = va.checked_add(len).ok_or(TileError::Outside)?;
        if len == 0 || va < self.lo || end > self.hi {
            return Err(TileError::Outside);
        }
        Ok((va, end))
    }

    fn occupied(&self, lo: u64, hi: u64) -> bool {
        self.rows
            .range(..hi)
            .next_back()
            .is_some_and(|(_, &e)| e > lo)
    }

    /// The wish runs clipped to `[a, b)`, in order.
    fn wish_in(&self, a: u64, b: u64) -> Vec<(u64, u64, Base)> {
        let start = self.wish.range(..=a).next_back().map_or(a, |(&k, _)| k);
        self.wish
            .range(start..b)
            .filter(|&(_, &(e, _))| e > a)
            .map(|(&s, &(e, base))| (s.max(a), e.min(b), base))
            .collect()
    }

    /// The blocks intersecting `[a, b)`, by start.
    fn blocks_in(&self, a: u64, b: u64) -> Vec<(u64, Block)> {
        let start = self.blocks.range(..=a).next_back().map_or(a, |(&k, _)| k);
        self.blocks
            .range(start..b)
            .filter(|&(_, blk)| blk.hi > a)
            .map(|(&k, &blk)| (k, blk))
            .collect()
    }

    /// Spans (tile-aligned) where the host base differs from the wish, or where no block exists, with
    /// the wished base. Rows' own pages do not matter, but a row block whose base differs from the wish
    /// of its tiles is listed (a mixed tile: it is fixed when the row goes).
    #[must_use]
    pub fn pending(&self) -> Vec<(u64, u64, Base)> {
        let mut out: Vec<(u64, u64, Base)> = Vec::new();
        for (a, b, base) in self.wish_in(self.lo, self.hi) {
            let mut cur = a;
            for (k, blk) in self.blocks_in(a, b) {
                let (s, e) = (k.max(a), blk.hi.min(b));
                if cur < s {
                    out.push((cur, s, base)); // a gap
                }
                if blk.base != base {
                    out.push((s, e, base));
                }
                cur = e;
            }
            if cur < b {
                out.push((cur, b, base));
            }
        }
        out
    }

    /// Free `old` (empty blocks, by start) and reserve `new` in their place, all or nothing: on a
    /// refusal the step is rolled back to `old`'s layout; a refused restore leaves a gap.
    fn replace(&mut self, v: &dyn TileVerbs, old: &[u64], new: &[Spec]) -> Result<(), TileError> {
        let olds: Vec<(u64, Block)> = old
            .iter()
            .filter_map(|k| self.blocks.get(k).map(|b| (*k, *b)))
            .collect();
        let same = olds.len() == new.len()
            && olds.iter().zip(new).all(|((k, b), &(lo, hi, base, attr))| {
                *k == lo && b.hi == hi && b.base == base && b.attr == attr
            });
        if same {
            return Ok(());
        }
        // ★ §8.9: the whole step, from the first free to the last reserve or rollback, holds the
        // placement lock; a refusal to take it stops the step before any host call.
        v.retile_begin().map_err(TileError::Refused)?;
        let r = self.replace_locked(v, &olds, new);
        v.retile_end();
        r
    }

    fn replace_locked(
        &mut self,
        v: &dyn TileVerbs,
        olds: &[(u64, Block)],
        new: &[Spec],
    ) -> Result<(), TileError> {
        let cost = u64::try_from(olds.len().saturating_add(new.len())).unwrap_or(u64::MAX);
        self.batch_used = self.batch_used.saturating_add(cost);
        let mut freed: Vec<(u64, Block)> = Vec::new();
        let mut fail = None;
        for &(k, b) in olds {
            bump(&mut self.stats.calls);
            match v.free(b.handle) {
                Ok(()) => {
                    self.blocks.remove(&k);
                    freed.push((k, b));
                }
                Err(e) => {
                    fail = Some(e);
                    break;
                }
            }
        }
        let mut made: Vec<u64> = Vec::new();
        if fail.is_none() {
            for &(lo, hi, base, attr) in new {
                bump(&mut self.stats.calls);
                match v.reserve(lo, hi, base, attr) {
                    Ok(handle) => {
                        self.blocks.insert(
                            lo,
                            Block {
                                hi,
                                base,
                                attr,
                                handle,
                            },
                        );
                        made.push(lo);
                    }
                    Err(e) => {
                        fail = Some(e);
                        break;
                    }
                }
            }
        }
        let Some(e) = fail else { return Ok(()) };
        // Roll back: free what was made, restore what was freed.
        bump(&mut self.stats.rollbacks);
        for k in made {
            if let Some(b) = self.blocks.get(&k).copied() {
                bump(&mut self.stats.calls);
                if v.free(b.handle).is_ok() {
                    self.blocks.remove(&k);
                }
            }
        }
        for (k, b) in freed {
            if self.blocks_in(k, b.hi).is_empty() {
                bump(&mut self.stats.calls);
                match v.reserve(k, b.hi, b.base, b.attr) {
                    Ok(handle) => {
                        self.blocks.insert(k, Block { handle, ..b });
                    }
                    Err(_) => bump(&mut self.stats.gaps_left),
                }
            } else {
                bump(&mut self.stats.gaps_left);
            }
        }
        Err(TileError::Refused(e))
    }

    /// Bring `[a, b)` to the wish. The work is split into SEGMENTS: maximal stretches of empty blocks
    /// and gaps, separated by occupied blocks (which are never touched). Each segment is re-laid out
    /// in one [`Tiling::replace`]: the parts of its end blocks outside `[a, b)` keep their base, an
    /// empty neighbour with the same base is merged in, and blocks that already match at either end
    /// are left alone. Returns the first refusal; the layout stays consistent.
    fn reconcile(&mut self, v: &dyn TileVerbs, a: u64, b: u64) -> Option<TileError> {
        let mut segs: Vec<(u64, u64, Vec<u64>)> = Vec::new();
        let mut seg: Option<(u64, u64, Vec<u64>)> = None;
        let mut pos = a;
        for (k, blk) in self.blocks_in(a, b) {
            if k > pos {
                let s = seg.get_or_insert((pos, pos, Vec::new()));
                s.1 = k; // a gap
            }
            if self.occupied(k, blk.hi) {
                segs.extend(seg.take());
            } else {
                let s = seg.get_or_insert((k, k, Vec::new()));
                s.1 = blk.hi;
                s.2.push(k);
            }
            pos = blk.hi;
        }
        if pos < b {
            let s = seg.get_or_insert((pos, pos, Vec::new()));
            s.1 = b;
        }
        segs.extend(seg);
        let mut first = None;
        for (s, e, olds) in segs {
            if let Some(err) = self.fix_segment(v, s, e, olds, a, b) {
                first.get_or_insert(err);
            }
        }
        first
    }

    /// One segment of [`Tiling::reconcile`].
    fn fix_segment(
        &mut self,
        v: &dyn TileVerbs,
        s: u64,
        e: u64,
        mut olds: Vec<u64>,
        a: u64,
        b: u64,
    ) -> Option<TileError> {
        let base_of =
            |t: &Self, k: Option<&u64>| k.and_then(|k| t.blocks.get(k)).map(|blk| blk.base);
        let mut runs: Vec<(u64, u64, Base)> = Vec::new();
        if s < a
            && let Some(base) = base_of(self, olds.first())
        {
            runs.push((s, a, base));
        }
        runs.extend(self.wish_in(s.max(a), e.min(b)));
        if e > b
            && let Some(base) = base_of(self, olds.last())
        {
            runs.push((b, e, base));
        }
        // Merge an empty neighbour that already holds the base of the adjacent run.
        if let Some(&(_, _, base)) = runs.first()
            && let Some((k, blk)) = self
                .blocks
                .range(..s)
                .next_back()
                .map(|(&k, &blk)| (k, blk))
            && blk.hi == s
            && blk.base == base
            && !self.occupied(k, blk.hi)
            && self.mergeable(k, blk)
        {
            olds.insert(0, k);
            runs.insert(0, (k, s, base));
        }
        if let Some(&(_, _, base)) = runs.last()
            && let Some(blk) = self.blocks.get(&e).copied()
            && blk.base == base
            && !self.occupied(e, blk.hi)
            && self.mergeable(e, blk)
        {
            olds.push(e);
            runs.push((e, blk.hi, base));
        }
        let runs = coalesce(runs);
        let mut new = self.layouts(&runs);
        // Leave blocks that already match at either end alone.
        while let (Some(k), Some(&spec)) = (olds.first(), new.first()) {
            if self.is_spec(*k, spec) {
                olds.remove(0);
                new.remove(0);
            } else {
                break;
            }
        }
        while let (Some(k), Some(&spec)) = (olds.last(), new.last()) {
            if self.is_spec(*k, spec) {
                olds.pop();
                new.pop();
            } else {
                break;
            }
        }
        if new.is_empty() && olds.is_empty() {
            return None;
        }
        let mandatory = self.s2_needed(&olds, &new);
        let growth = new.len().saturating_sub(olds.len());
        let cost = u64::try_from(olds.len().saturating_add(new.len())).unwrap_or(u64::MAX);
        let optional_ok = self.blocks.len().saturating_add(growth) <= self.cfg.max_blocks
            && self.batch_used.saturating_add(cost) <= self.cfg.batch_calls;
        let plan = if optional_ok {
            new
        } else if mandatory {
            // S2 must hold: what is wished SPARSE in this segment stays INVALID for now (S1 cut,
            // loud, pending), so the layout never grows past one layout per run.
            bump(&mut self.stats.cut);
            let (lo, hi) = (
                new.first().map_or(s, |x| x.0),
                new.last().map_or(e, |x| x.1),
            );
            let lo = olds.first().map_or(lo, |k| lo.min(*k));
            let hi = olds
                .last()
                .and_then(|k| self.blocks.get(k))
                .map_or(hi, |blk| hi.max(blk.hi));
            self.layouts(&[(lo, hi, Base::Invalid)])
        } else {
            bump(&mut self.stats.cut);
            return None;
        };
        self.replace(v, &olds, &plan).err()
    }

    fn layouts(&self, runs: &[(u64, u64, Base)]) -> Vec<Spec> {
        runs.iter()
            .flat_map(|&(x, y, base)| {
                layout(x, y, false, self.cfg.big_forced)
                    .into_iter()
                    .map(move |(p, q, at)| (p, q, base, at))
            })
            .collect()
    }

    fn is_spec(&self, k: u64, (lo, hi, base, attr): Spec) -> bool {
        self.blocks
            .get(&k)
            .is_some_and(|b| k == lo && b.hi == hi && b.base == base && b.attr == attr)
    }

    /// Whether an empty block beyond a stretch may be merged into it: its base already matches its wish.
    fn mergeable(&self, k: u64, blk: Block) -> bool {
        self.wish_in(k, blk.hi)
            .iter()
            .all(|&(_, _, w)| w == blk.base)
    }

    /// Whether the planned change is needed for S2: it fills a gap, or it turns a SPARSE block that
    /// holds a wished-INVALID tile to INVALID.
    fn s2_needed(&self, olds: &[u64], new: &[Spec]) -> bool {
        let old_bytes: u64 = olds
            .iter()
            .filter_map(|k| self.blocks.get(k).map(|b| span(*k, b.hi)))
            .sum();
        let new_bytes: u64 = new.iter().map(|x| span(x.0, x.1)).sum();
        if old_bytes < new_bytes {
            return true;
        }
        olds.iter().any(|k| {
            self.blocks.get(k).is_some_and(|b| {
                b.base == Base::Sparse
                    && self
                        .wish_in(*k, b.hi)
                        .iter()
                        .any(|&(_, _, w)| w == Base::Invalid)
            })
        })
    }

    /// The guest set the unmapped pages of `[va, va+len)` to `base` (a SPARSE run, or a return to
    /// INVALID). SPARSE is rounded INWARD to tiles, INVALID OUTWARD (INVALID wins a mixed tile).
    ///
    /// # Errors
    /// [`TileError::Outside`]; the first host refusal (the layout stays consistent).
    pub fn set_base(
        &mut self,
        v: &dyn TileVerbs,
        va: u64,
        len: u64,
        base: Base,
    ) -> Result<(), TileError> {
        match self.wish_only(va, len, base)? {
            Some((a, b)) => self.reconcile(v, a, b).map_or(Ok(()), Err),
            None => Ok(()),
        }
    }

    /// ★ §8.9: a whole batch of guest wishes (one walk's SPARSE and UNSPARSE runs), assigned first
    /// and re-tiled ONCE over their spans — a bind inside a sparse range (UNSPARSE the old run,
    /// SPARSE its pieces) costs one re-tile, not two. A refused wish (outside, the wish cap) is
    /// skipped and counted; the others still apply.
    ///
    /// # Errors
    /// The first refusal (of a wish, or of the re-tile; the layout stays consistent).
    pub fn set_bases(
        &mut self,
        v: &dyn TileVerbs,
        wishes: &[(u64, u64, Base)],
    ) -> Result<(), TileError> {
        let mut first = None;
        let mut spans: Vec<(u64, u64)> = Vec::new();
        for &(va, len, base) in wishes {
            match self.wish_only(va, len, base) {
                Ok(Some(s)) => spans.push(s),
                Ok(None) => {}
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        spans.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for (a, b) in spans {
            match merged.last_mut() {
                Some(m) if a <= m.1 => m.1 = m.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        for (a, b) in merged {
            if let Some(e) = self.reconcile(v, a, b) {
                first.get_or_insert(e);
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// Assign one wish (rounded: SPARSE inward, INVALID outward); the tile-aligned span to re-tile.
    fn wish_only(
        &mut self,
        va: u64,
        len: u64,
        base: Base,
    ) -> Result<Option<(u64, u64)>, TileError> {
        let (a, b) = self.clip(va, len)?;
        let (a, b) = match base {
            Base::Sparse => (up(a, TILE).ok_or(TileError::Outside)?, down(b, TILE)),
            Base::Invalid => (down(a, TILE), up(b, TILE).ok_or(TileError::Outside)?),
        };
        if a >= b {
            return Ok(None);
        }
        if base == Base::Sparse && self.wish.len().saturating_add(2) > self.cfg.max_wish_runs {
            bump(&mut self.stats.wish_refused);
            return Ok(None);
        }
        self.assign_wish(a, b, base);
        Ok(Some((a, b)))
    }

    fn assign_wish(&mut self, a: u64, b: u64, base: Base) {
        let parts = self.wish_in(a, b);
        if let (Some(&(f, _, fb)), Some(&(_, l, lb))) = (parts.first(), parts.last()) {
            let first_key = self.wish.range(..=f).next_back().map(|(&k, _)| k);
            let last_end = self.wish.range(..l).next_back().map(|(_, &(e, _))| e);
            for (s, _, _) in &parts {
                let key = self.wish.range(..=*s).next_back().map(|(&k, _)| k);
                if let Some(k) = key {
                    self.wish.remove(&k);
                }
            }
            if let Some(k) = first_key
                && k < a
            {
                self.wish.insert(k, (a, fb));
            }
            if let Some(e) = last_end
                && e > b
            {
                self.wish.insert(b, (e, lb));
            }
        }
        self.wish.insert(a, (b, base));
        // Coalesce with equal neighbours.
        let mut s = a;
        let mut e = b;
        if let Some((&k, &(ke, kb))) = self.wish.range(..a).next_back()
            && ke == a
            && kb == base
        {
            self.wish.remove(&k);
            s = k;
        }
        if let Some(&(re, rb)) = self.wish.get(&b)
            && rb == base
        {
            self.wish.remove(&b);
            e = re;
        }
        self.wish.remove(&a);
        self.wish.insert(s, (e, base));
    }

    /// Make `[va, va+len)` mappable: carve its tiles out of the empty blocks that hold them, so the
    /// row's mapping pins only blocks of its own tiles. Returns the `(handle, va, len)` pieces to map
    /// through (one per block). The row is NOT live until [`Tiling::commit_row`].
    ///
    /// # Errors
    /// [`TileError::Outside`], [`TileError::NotARow`] (overlaps a live row), [`TileError::Unplaceable`]
    /// (a gap that could not be healed), or a refusal of the carve (rolled back).
    pub fn prepare_row(
        &mut self,
        v: &dyn TileVerbs,
        va: u64,
        len: u64,
    ) -> Result<Vec<(u32, u64, u64)>, TileError> {
        let (a, b) = self.clip(va, len)?;
        if self.occupied(a, b) {
            return Err(TileError::NotARow);
        }
        let (t0, t1) = (down(a, TILE), up(b, TILE).ok_or(TileError::Outside)?);
        // Heal a gap first (mandatory).
        let _ = self.reconcile(v, t0, t1);
        for (k, blk) in self.blocks_in(t0, t1) {
            if self.occupied(k, blk.hi) || (k >= t0 && blk.hi <= t1) {
                continue; // a row block of a neighbour in a shared tile, or already exactly ours
            }
            let (c0, c1) = (k.max(t0), blk.hi.min(t1));
            let keep = |p: u64, q: u64| -> Vec<Spec> {
                layout(p, q, false, self.cfg.big_forced)
                    .into_iter()
                    .map(|(x, y, at)| (x, y, blk.base, at))
                    .collect()
            };
            let mut plan = keep(k, c0);
            // A partial edge tile may be shared with a neighbouring row: it gets a block of its own,
            // so a row pins only tiles it uses (never a tile it does not touch, after its neighbour goes).
            let mut cuts = vec![c0];
            let first_edge = down(a, TILE).saturating_add(TILE);
            for cut in [first_edge, down(b, TILE)] {
                let edge = (cut == first_edge && !a.is_multiple_of(TILE))
                    || (cut == down(b, TILE) && !b.is_multiple_of(TILE));
                if edge && c0 < cut && cut < c1 && !cuts.contains(&cut) {
                    cuts.push(cut);
                }
            }
            cuts.push(c1);
            cuts.sort_unstable();
            for (&x, &y) in cuts.iter().zip(cuts.iter().skip(1)) {
                plan.extend(
                    layout(x, y, true, self.cfg.big_forced)
                        .into_iter()
                        .map(|(x, y, at)| (x, y, blk.base, at)),
                );
            }
            plan.extend(keep(c1, blk.hi));
            let growth = plan.len().saturating_sub(1);
            let cost = u64::try_from(plan.len().saturating_add(1)).unwrap_or(u64::MAX);
            if self.blocks.len().saturating_add(growth) > self.cfg.max_blocks
                || self.batch_used.saturating_add(cost) > self.cfg.batch_calls
            {
                bump(&mut self.stats.pinned);
                continue;
            }
            self.replace(v, &[k], &plan)?;
        }
        let mut out = Vec::new();
        let mut cur = a;
        for (k, blk) in self.blocks_in(a, b) {
            if k > cur {
                return Err(TileError::Unplaceable);
            }
            let e = blk.hi.min(b);
            out.push((blk.handle, cur, span(cur, e)));
            cur = e;
        }
        if cur < b {
            return Err(TileError::Unplaceable);
        }
        Ok(out)
    }

    /// The row `[va, va+len)` is mapped through the pieces [`Tiling::prepare_row`] returned.
    ///
    /// # Errors
    /// [`TileError::Outside`], [`TileError::NotARow`].
    pub fn commit_row(&mut self, va: u64, len: u64) -> Result<(), TileError> {
        let (a, b) = self.clip(va, len)?;
        if self.occupied(a, b) {
            return Err(TileError::NotARow);
        }
        self.rows.insert(a, b);
        Ok(())
    }

    /// The end of the live row (coverage) that starts exactly at `va`.
    #[must_use]
    pub fn row_end(&self, va: u64) -> Option<u64> {
        self.rows.get(&va).copied()
    }

    /// The pieces a live row is mapped through (to unmap it).
    #[must_use]
    pub fn row_pieces(&self, va: u64) -> Vec<(u32, u64, u64)> {
        let Some(&b) = self.rows.get(&va) else {
            return Vec::new();
        };
        self.blocks_in(va, b)
            .into_iter()
            .map(|(k, blk)| {
                let s = k.max(va);
                (blk.handle, s, span(s, blk.hi.min(b)))
            })
            .collect()
    }

    /// The row at `va` was unmapped (every piece): forget it, then bring its tiles to the wish (a
    /// mixed tile is fixed now) and merge the emptied blocks.
    ///
    /// # Errors
    /// [`TileError::NotARow`]; the first host refusal of the re-tile (the layout stays consistent).
    pub fn release_row(&mut self, v: &dyn TileVerbs, va: u64) -> Result<(), TileError> {
        let b = self.rows.remove(&va).ok_or(TileError::NotARow)?;
        let (t0, t1) = (down(va, TILE), up(b, TILE).unwrap_or(self.hi).min(self.hi));
        self.reconcile(v, t0, t1).map_or(Ok(()), Err)
    }

    /// ★ §8.9: every block intersecting `[va, va+len)`, clipped: `(handle, va, len)` — the pieces a
    /// mapping placed through [`Tiling::prepare_row`] lies in (to unmap it, by range or by start).
    #[must_use]
    pub fn pieces_in(&self, va: u64, len: u64) -> Vec<(u32, u64, u64)> {
        let b = va.saturating_add(len);
        self.blocks_in(va, b)
            .into_iter()
            .map(|(k, blk)| {
                let s = k.max(va);
                (blk.handle, s, span(s, blk.hi.min(b)))
            })
            .collect()
    }

    /// ★ §8.9: `[va, va+len)` was unmapped (by range: any part of any rows): forget that coverage —
    /// a row cut in the middle keeps its outside parts — then bring the tiles to the wish.
    ///
    /// # Errors
    /// [`TileError::Outside`]; the first host refusal of the re-tile (the layout stays consistent).
    pub fn release_range(&mut self, v: &dyn TileVerbs, va: u64, len: u64) -> Result<(), TileError> {
        let (a, b) = self.clip(va, len)?;
        let hit: Vec<(u64, u64)> = self
            .rows
            .range(..b)
            .filter(|&(_, &e)| e > a)
            .map(|(&s, &e)| (s, e))
            .collect();
        for (s, e) in hit {
            self.rows.remove(&s);
            if s < a {
                self.rows.insert(s, a);
            }
            if e > b {
                self.rows.insert(b, e);
            }
        }
        let (t0, t1) = (down(a, TILE), up(b, TILE).unwrap_or(self.hi).min(self.hi));
        self.reconcile(v, t0, t1).map_or(Ok(()), Err)
    }

    /// The base the host holds at `va` (`None`: no block, which reads INVALID).
    #[must_use]
    pub fn host_base(&self, va: u64) -> Option<Base> {
        self.blocks
            .range(..=va)
            .next_back()
            .filter(|(_, b)| va < b.hi)
            .map(|(_, b)| b.base)
    }

    /// The wished base at `va`.
    #[must_use]
    pub fn wished(&self, va: u64) -> Option<Base> {
        self.wish
            .range(..=va)
            .next_back()
            .filter(|(_, (e, _))| va < *e)
            .map(|(_, (_, b))| *b)
    }

    /// The engine's own invariants: blocks are tile-aligned, ordered, disjoint, inside the range,
    /// never rounded by the host ([`layout`]'s rule), and every live row lies wholly in blocks.
    ///
    /// # Errors
    /// The first broken invariant.
    pub fn check(&self) -> Result<(), String> {
        let mut prev = self.lo;
        for (&k, b) in &self.blocks {
            if k % TILE != 0 || b.hi % TILE != 0 || k >= b.hi || k < prev || b.hi > self.hi {
                return Err(format!(
                    "block {k:#x}..{:#x} misplaced (prev end {prev:#x})",
                    b.hi
                ));
            }
            let exact = match b.attr {
                Attr::Default => {
                    span(k, b.hi) < HUGE || (k.is_multiple_of(HUGE) && b.hi.is_multiple_of(HUGE))
                }
                Attr::BigForced => self.cfg.big_forced,
            };
            if !exact {
                return Err(format!(
                    "block {k:#x}..{:#x} {:?} would be rounded by the host",
                    b.hi, b.attr
                ));
            }
            prev = b.hi;
        }
        for (&a, &b) in &self.rows {
            let mut cur = a;
            for (k, blk) in self.blocks_in(a, b) {
                if k > cur {
                    return Err(format!("row {a:#x}..{b:#x} has a gap at {cur:#x}"));
                }
                cur = blk.hi;
            }
            if cur < b {
                return Err(format!("row {a:#x}..{b:#x} has a gap at {cur:#x}"));
            }
        }
        Ok(())
    }

    /// Free every block (teardown; the caller unmapped every row).
    pub fn release_all(&mut self, v: &dyn TileVerbs) {
        for (_, b) in std::mem::take(&mut self.blocks) {
            bump(&mut self.stats.calls);
            let _ = v.free(b.handle);
        }
        self.rows.clear();
    }
}

/// ★ §8.9: why a tiled map failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TiledMapError<E> {
    /// The engine refused (outside, overlap, a gap, a refused carve).
    Tile(TileError),
    /// A piece's map was refused; the pieces before it were unmapped again.
    Map(E),
}

/// ★ §8.9 — **map one FIXED row through the tiling, all or nothing**: carve its tiles
/// ([`Tiling::prepare_row`]), call `place(handle, va, len)` once per block piece, and commit it. On a
/// refused piece the pieces already placed are taken down again (`unplace(handle, va)`, newest
/// first). The SAME glue serves `HostRm`'s piece routing and the GPU-free model.
///
/// # Errors
/// [`TiledMapError`].
pub fn map_tiled<E>(
    t: &mut Tiling,
    v: &dyn TileVerbs,
    va: u64,
    len: u64,
    place: &mut dyn FnMut(u32, u64, u64) -> Result<(), E>,
    unplace: &mut dyn FnMut(u32, u64),
) -> Result<(), TiledMapError<E>> {
    let pieces = t.prepare_row(v, va, len).map_err(TiledMapError::Tile)?;
    for (i, &(h, pva, plen)) in pieces.iter().enumerate() {
        if let Err(e) = place(h, pva, plen) {
            for &(h0, va0, _) in pieces.iter().take(i).rev() {
                unplace(h0, va0);
            }
            return Err(TiledMapError::Map(e));
        }
    }
    t.commit_row(va, len).map_err(TiledMapError::Tile)
}

/// ★ §8.9 — **unmap through the tiling**: one `unplace(handle, va, size)` per block piece of
/// `[va, va+len)` (`size` 0 = the whole mapping keyed by the piece's start, else the piece's length),
/// then [`Tiling::release_range`]. Every piece is attempted; the first refusal is returned (the
/// coverage is released only when every piece went).
///
/// # Errors
/// The first refused piece, or the re-tile's refusal (as [`TiledMapError::Tile`]).
pub fn unmap_tiled<E>(
    t: &mut Tiling,
    v: &dyn TileVerbs,
    va: u64,
    len: u64,
    by_range: bool,
    unplace: &mut dyn FnMut(u32, u64, u64) -> Result<(), E>,
) -> Result<(), TiledMapError<E>> {
    let mut first = None;
    for (h, pva, plen) in t.pieces_in(va, len) {
        if let Err(e) = unplace(h, pva, if by_range { plen } else { 0 }) {
            first.get_or_insert(e);
        }
    }
    if let Some(e) = first {
        return Err(TiledMapError::Map(e));
    }
    t.release_range(v, va, len).map_err(TiledMapError::Tile)
}
