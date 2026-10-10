//! ★★★ **The handles behind our batched placements** — `V3_BATCHED_MAP.md` §5.
//!
//! A batched map (`kf_host::HostRm::map_scattered`) places N VA-contiguous guest-RAM runs through
//! ONE host object of our own. The walker still commits — and later unmaps — those runs one by one
//! (or in ranges), so the object must outlive every piece of it and be freed exactly when its last
//! piece is gone: freeing it earlier would make RM unmap every piece still in use
//! (`ogkm-580 rs_client.c:1342-1395`), never freeing it pins guest pages for the VM's life.
//!
//! This is a ledger of OUR OWN host handles (the kind `V3_BUILD.md`'s 2026-09-25 amendment
//! sanctions), never a copy of the guest's tables: nothing resolves through it and nothing about
//! a translation is read from it. It answers one question — *"which of our objects are now wholly
//! unmapped?"* — and it answers it from what WE unmapped, page by page, so an unmap repeated or
//! overlapping two objects cannot double-count.
//!
//! ⊘ Pure data, no host calls: the caller frees what [`BatchBook::unmapped`] returns, outside any
//! lock (THE_CONSTRAINTS "no blocking under a lock others block on").

use crate::ledger::{Desired, HostVas, MapTarget, Mapped, SkedRow};
use std::collections::BTreeMap;

/// ★ 2026-10-09 — **the host verbs of ONE host VA space**, as [`BatchedVas`] uses them. Production
/// is [`HostVas`] (the in-process host RM); the GPU-free tests drive the same [`BatchedVas`] code
/// against a simulated host RM (`crate::sim`) that implements `ogkm-595.84 rs_server.c:2419-2520`
/// and `virt_mem_allocator_gm107.c:1531-1638` exactly, foreign mappings included.
pub trait SpaceVerbs {
    /// One FIXED map of a row ([`MapTarget::map`] semantics: `HeldByHost` for an occupied VA).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_row(&self, d: &Desired, defer: bool) -> Result<Mapped, String>;
    /// One FIXED message-kind map ([`MapTarget::map_sked`] semantics).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_sked_row(&self, s: &SkedRow, defer: bool) -> Result<Mapped, String>;
    /// One stitched object mapped at `rows[0].va` ([`HostVas::map_scattered`]); returns its handle.
    ///
    /// # Errors
    /// The host's refusal, by name; nothing of ours is then placed.
    fn map_scattered(
        &self,
        ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String>;
    /// The whole-mapping unmap keyed by the EXACT start `va` (`NVOS47` `size == 0`).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap_whole(&self, va: u64, defer: bool) -> Result<(), String>;
    /// The whole-mapping unmap of every piece of the row `[va, va+len)` ([`HostVas::unmap_row`]).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap_row(&self, va: u64, len: u64, defer: bool) -> Result<(), String>;
    /// ONE host range unmap (`NVOS47` `size != 0`): RM removes or SPLITS every mapping of this
    /// client's `hDma` that intersects the range, whoever placed it.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String>;
    /// Free one of our host objects (a batch object: RM unmaps every mapping of it first).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn free(&self, handle: u32) -> Result<(), String>;
    /// ★ Whether host RM can SPLIT a mapping lying in `[va, va+len)` (a range unmap that covers
    /// part of it) without damaging the part it keeps — see [`BatchedVas::unmap_range`].
    fn splits_safely(&self, va: u64, len: u64) -> bool;
    /// ★ 2026-10-09: reserve `[va, va+len)` with a lazy FIXED `NV50_MEMORY_VIRTUAL` (a "micro
    /// reservation"); returns its handle (an `hDma`). Freed with [`SpaceVerbs::free`].
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn reserve(&self, va: u64, len: u64) -> Result<u32, String>;
    /// [`SpaceVerbs::map_row`] THROUGH the reservation `h` (no piece routing).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_row_in(&self, h: u32, d: &Desired, defer: bool) -> Result<Mapped, String>;
    /// [`SpaceVerbs::map_sked_row`] THROUGH the reservation `h`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_sked_in(&self, h: u32, s: &SkedRow, defer: bool) -> Result<Mapped, String>;
    /// [`SpaceVerbs::map_scattered`] THROUGH the reservation `h`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_scattered_in(
        &self,
        h: u32,
        ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String>;
    /// ONE unmap THROUGH the reservation `h`: `size == 0` whole (exact start), else a range.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap_in(&self, h: u32, va: u64, size: u64, defer: bool) -> Result<(), String>;
}

impl SpaceVerbs for HostVas<'_> {
    fn map_row(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        MapTarget::map(self, d, defer)
    }
    fn map_sked_row(&self, s: &SkedRow, defer: bool) -> Result<Mapped, String> {
        MapTarget::map_sked(self, s, defer)
    }
    fn map_scattered(
        &self,
        ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String> {
        HostVas::map_scattered(self, ram_fd, rows, defer)
    }
    fn unmap_whole(&self, va: u64, defer: bool) -> Result<(), String> {
        MapTarget::unmap(self, va, defer)
    }
    fn unmap_row(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        HostVas::unmap_row(self, va, len, defer)
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        MapTarget::unmap_range(self, va, len, defer)
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        self.rm.free(handle).map_err(|e| format!("{e:?}"))
    }
    fn splits_safely(&self, va: u64, len: u64) -> bool {
        self.space.guest_reserved(va, len)
    }
    fn reserve(&self, va: u64, len: u64) -> Result<u32, String> {
        self.rm
            .reserve_va(self.space.space, va, len)
            .map_err(|e| format!("reserve {va:#x}+{len:#x}: {e:?}"))
    }
    fn map_row_in(&self, h: u32, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let obj = if d.ram {
            self.ram_obj
                .ok_or_else(|| format!("map {:#x}: guest-RAM row and no RAM object", d.va))?
        } else {
            self.store
        };
        held_or(
            self.rm.map_in(
                h,
                obj,
                kf_host::MapBacking::SharedSlice,
                d.off,
                d.len,
                d.va,
                defer,
                d.kind,
                d.perm,
            ),
            d.va,
            d.len,
        )
    }
    fn map_sked_in(&self, h: u32, s: &SkedRow, defer: bool) -> Result<Mapped, String> {
        held_or(
            self.rm.map_in(
                h,
                self.store,
                kf_host::MapBacking::SharedSlice,
                s.off,
                s.len,
                s.va,
                defer,
                kf_chip::sked::PTE_KIND_SMSKED_MESSAGE,
                s.perm,
            ),
            s.va,
            s.len,
        )
    }
    fn map_scattered_in(
        &self,
        h: u32,
        ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String> {
        HostVas::map_scattered_through(self, Some(h), ram_fd, rows, defer)
    }
    fn unmap_in(&self, h: u32, va: u64, size: u64, defer: bool) -> Result<(), String> {
        self.rm
            .unmap_in(h, va, size, defer)
            .map_err(|e| format!("unmap {va:#x}+{size:#x} in {h:#x}: {e:?}"))
    }
}

/// A FIXED map's answer as [`Mapped`]: an occupied VA is `HeldByHost` (not ours).
fn held_or(r: Result<u64, kf_host::RmError>, va: u64, len: u64) -> Result<Mapped, String> {
    match r {
        Ok(_) => Ok(Mapped::Placed),
        Err(kf_host::RmError::Other(kf_host::VA_ALREADY_MAPPED)) => Ok(Mapped::HeldByHost),
        Err(e) => Err(format!("map {va:#x}+{len:#x}: {e:?}")),
    }
}

/// The granule the book tracks liveness in — the smallest GMMU page of every family, and the unit
/// every guest-RAM row is whole multiples of (`crate::apply` refuses a sub-page row).
pub const BATCH_PAGE: u64 = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    handle: u32,
    len: u64,
    /// One bit per [`BATCH_PAGE`] still mapped.
    live: Vec<u64>,
    live_pages: u64,
}

/// Our batch objects in one host VA space, by `(VA their mapping starts at, handle)` — two can
/// start at one VA (a later batch placed over an earlier one's dead pages while other pages of the
/// earlier one are still live).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchBook {
    by_va: BTreeMap<(u64, u32), Entry>,
    /// The longest entry ever recorded — bounds the backward scan in [`BatchBook::unmapped`].
    max_len: u64,
}

/// Why [`BatchBook::insert`] refused to record a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookRefusal {
    /// Empty, not whole [`BATCH_PAGE`]s, or overflowing.
    Shape,
    /// That handle is already tracked at that VA.
    Occupied,
}

/// ★ D2 (2026-10-10): the most ledger entries — or bitmap words — one lock hold may touch. Every
/// critical section on the ledger (`book`, `own`, `micro`) is bounded by this, so a thread that
/// serves input and takes one of these locks (the channel plane's act thread,
/// [`BatchedVas::hand_to_host`]) never waits behind a hold proportional to a guest-sized row. Big
/// inserts and removals go in chunks of this size, the structure consistent between chunks.
pub const LEDGER_CHUNK: usize = 2048;

/// A batch ready to be recorded: its liveness bitmap is built OUTSIDE the book's lock (an
/// allocation proportional to the batch's pages — never under a lock).
#[derive(Debug)]
pub struct PreparedBatch {
    key: (u64, u32),
    entry: Entry,
}

/// Where a chunked [`BatchBook::unmapped_step`] resumes: the entry (by key) and the page inside it.
#[derive(Debug, Default, Clone, Copy)]
pub struct BookCursor {
    key: Option<(u64, u32)>,
    page: u64,
}

/// The whole contents of a book taken out in O(1) ([`BatchBook::take_all`]); dismantled outside the
/// lock.
#[derive(Debug, Default)]
pub struct TakenBook(BTreeMap<(u64, u32), Entry>);

impl TakenBook {
    /// `(handle, va, len)` of every batch, in VA order.
    #[must_use]
    pub fn extents(&self) -> Vec<(u32, u64, u64)> {
        self.0.iter().map(|(&(s, h), e)| (h, s, e.len)).collect()
    }
}

impl BatchBook {
    /// Build the entry for a batch object `handle` mapped at `[va, va+len)` (every page live).
    /// Allocates O(pages / 64): call it OUTSIDE any lock, then [`BatchBook::insert_prepared`].
    ///
    /// # Errors
    /// [`BookRefusal::Shape`].
    pub fn prepare(va: u64, len: u64, handle: u32) -> Result<PreparedBatch, BookRefusal> {
        if len == 0 || !(va | len).is_multiple_of(BATCH_PAGE) || va.checked_add(len).is_none() {
            return Err(BookRefusal::Shape);
        }
        let pages = len / BATCH_PAGE;
        let words = usize::try_from(pages.div_ceil(64)).map_err(|_| BookRefusal::Shape)?;
        let mut live = vec![u64::MAX; words];
        if !pages.is_multiple_of(64)
            && let Some(last) = live.last_mut()
        {
            *last = (1u64 << (pages % 64)) - 1;
        }
        Ok(PreparedBatch {
            key: (va, handle),
            entry: Entry {
                handle,
                len,
                live,
                live_pages: pages,
            },
        })
    }

    /// Record a [`BatchBook::prepare`]d batch: O(log n), no allocation proportional to its size.
    ///
    /// # Errors
    /// [`BookRefusal::Occupied`].
    pub fn insert_prepared(&mut self, p: PreparedBatch) -> Result<(), BookRefusal> {
        if self.by_va.contains_key(&p.key) {
            return Err(BookRefusal::Occupied);
        }
        self.max_len = self.max_len.max(p.entry.len);
        self.by_va.insert(p.key, p.entry);
        Ok(())
    }

    /// Record a batch object `handle` mapped at `[va, va+len)`, every page live.
    ///
    /// # Errors
    /// [`BookRefusal`] — the caller must then free `handle` itself (it cannot be tracked).
    pub fn insert(&mut self, va: u64, len: u64, handle: u32) -> Result<(), BookRefusal> {
        if self.by_va.contains_key(&(va, handle)) {
            return Err(BookRefusal::Occupied);
        }
        self.insert_prepared(Self::prepare(va, len, handle)?)
    }

    /// `[va, va+len)` is no longer mapped by us: clear those pages in every batch they touch, and
    /// return (and forget) the handles with no live page left — the caller frees them.
    #[must_use]
    pub fn unmapped(&mut self, va: u64, len: u64) -> Vec<u32> {
        self.unmapped_extents(va, len)
            .into_iter()
            .map(|(h, _, _)| h)
            .collect()
    }

    /// [`BatchBook::unmapped`], with each emptied object's mapped extent `(handle, va, len)`.
    /// Unbounded in one call (tests, diagnostics): production goes by [`BatchBook::unmapped_step`].
    #[must_use]
    pub fn unmapped_extents(&mut self, va: u64, len: u64) -> Vec<(u32, u64, u64)> {
        let end = va.saturating_add(len);
        let mut cur = BookCursor::default();
        let mut out = Vec::new();
        while !self
            .unmapped_step(va, end, &mut cur, usize::MAX, &mut out)
            .1
        {}
        out
    }

    /// ★ D2: ONE bounded step of [`BatchBook::unmapped_extents`] — it touches at most `budget`
    /// units (an entry visited, a bitmap word cleared), appends the objects it emptied (and
    /// forgot) to `out`, and returns `(units touched, finished)`. Not finished ⇒ call again with
    /// the same cursor, after releasing the lock; clearing a page twice is idempotent, so the book
    /// is consistent between steps.
    pub fn unmapped_step(
        &mut self,
        va: u64,
        end: u64,
        cur: &mut BookCursor,
        budget: usize,
        out: &mut Vec<(u32, u64, u64)>,
    ) -> (usize, bool) {
        let floor = va.saturating_sub(self.max_len);
        let from = cur.key.unwrap_or((floor, 0));
        let mut touched = 0usize;
        // At least one unit of real work per call, so a step always makes progress whatever the
        // budget; with the production budget the bound is `budget` (the first unit is within it).
        let mut worked = false;
        let mut emptied: Vec<(u64, u32)> = Vec::new();
        let mut resume: Option<((u64, u32), u64)> = None;
        'entries: for (&(start, h), e) in self.by_va.range_mut(from..(end, 0)) {
            if touched >= budget {
                resume = Some(((start, h), 0));
                break;
            }
            touched += 1;
            let e_end = start + e.len;
            if e_end <= va {
                continue;
            }
            let lo = (va.max(start) - start) / BATCH_PAGE;
            let hi = (end.min(e_end) - start).div_ceil(BATCH_PAGE);
            let mut p = if cur.key == Some((start, h)) {
                cur.page.max(lo)
            } else {
                lo
            };
            while p < hi {
                if touched >= budget && worked {
                    resume = Some(((start, h), p));
                    break 'entries;
                }
                let w = (p / 64) as usize;
                let wend = ((p / 64 + 1) * 64).min(hi);
                let (b0, b1) = (p % 64, wend - (p / 64) * 64);
                let mask = if b1 - b0 == 64 {
                    u64::MAX
                } else {
                    ((1u64 << (b1 - b0)) - 1) << b0
                };
                let was = e.live[w] & mask;
                e.live[w] &= !mask;
                e.live_pages -= u64::from(was.count_ones());
                touched += 1;
                worked = true;
                p = wend;
            }
            worked = true;
            if e.live_pages == 0 {
                emptied.push((start, h));
            }
        }
        for s in emptied {
            if let Some(e) = self.by_va.remove(&s) {
                out.push((e.handle, s.0, e.len));
            }
        }
        match resume {
            Some((key, page)) => {
                *cur = BookCursor {
                    key: Some(key),
                    page,
                };
                (touched, false)
            }
            None => (touched, true),
        }
    }

    /// Whether a batch of ours has a live page at `va` — the caller must then unmap by RANGE (a
    /// whole-mapping unmap keyed by `va` would take the rest of that batch with it).
    #[must_use]
    pub fn covers(&self, va: u64) -> bool {
        let floor = va.saturating_sub(self.max_len);
        self.by_va
            .range((floor, 0)..=(va, u32::MAX))
            .any(|(&(start, _), e)| {
                let p = (va - start) / BATCH_PAGE;
                va < start + e.len && e.live[(p / 64) as usize] & (1 << (p % 64)) != 0
            })
    }

    /// Forget every batch and return every handle (the space is being retired).
    #[must_use]
    pub fn drain(&mut self) -> Vec<u32> {
        self.take_all().extents().into_iter().map(|x| x.0).collect()
    }

    /// ★ D2: forget every batch in O(1); the caller dismantles the result outside the lock.
    #[must_use]
    pub fn take_all(&mut self) -> TakenBook {
        self.max_len = 0;
        TakenBook(core::mem::take(&mut self.by_va))
    }

    /// Batches tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_va.len()
    }

    /// Whether none is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_va.is_empty()
    }
}

/// ★ 2026-10-09 — one host mapping WE made in this space: a per-run row (or one `hDma` piece of
/// one), a SKED row, a batch object's mapping, or a remnant host RM kept when one of ours was split
/// by an exact (reservation) range unmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnMap {
    /// Bytes.
    pub len: u64,
    /// The batch object it maps (`None`: a per-run row of the RAM object or the store).
    pub batch: Option<u32>,
    /// The micro reservation it was mapped THROUGH (`None`: the space's own routing).
    pub via: Option<u32>,
}

/// ★★★ 2026-10-09 — **the ledger of OUR OWN host mappings in one space, at host-mapping
/// granularity** (`va → OwnMap`). It answers the questions a range unmap must never guess: *which
/// bytes of `[va, end)` are ours* (a range unmap goes over exactly those, never over a gap: host RM
/// removes whatever of this client's mappings lies in a range, foreign ones included,
/// `rs_server.c:2453-2514`), *through which `hDma`*, and *which of our mappings it would SPLIT*
/// (see [`BatchedVas::unmap_range`]). A ledger of our own handles and VAs (`V3_BUILD.md` 2026-09-25
/// amendment) — never a copy of guest tables.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnMaps {
    by_va: BTreeMap<u64, OwnMap>,
}

/// What a range would do to [`OwnMaps`]: the maximal owned spans inside it (one per `hDma`), and
/// the mappings it would split (straddling an edge).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnPlan {
    /// Maximal VA-contiguous owned spans clipped to the range, `(start, end, via)`, in VA order; a
    /// span never mixes two `via`s.
    pub spans: Vec<(u64, u64, Option<u32>)>,
    /// Our mappings that cross an edge of the range, `(va, len, via)`.
    pub split: Vec<(u64, u64, Option<u32>)>,
}

impl OwnMaps {
    /// Record a mapping of ours.
    pub fn insert(&mut self, va: u64, len: u64, batch: Option<u32>, via: Option<u32>) {
        if len > 0 {
            self.by_va.insert(va, OwnMap { len, batch, via });
        }
    }

    /// The mapping of ours that contains `va`, `(start, entry)`.
    #[must_use]
    pub fn containing(&self, va: u64) -> Option<(u64, OwnMap)> {
        self.by_va
            .range(..=va)
            .next_back()
            .filter(|&(&s, m)| va - s < m.len)
            .map(|(&s, &m)| (s, m))
    }

    /// Our mappings intersecting `[va, end)`, in VA order. ★ D2: UNBOUNDED (O(k)) — tests and
    /// diagnostics only; production goes by [`OwnMaps::within_limited`].
    #[must_use]
    pub fn within(&self, va: u64, end: u64) -> Vec<(u64, OwnMap)> {
        self.within_limited(va, end, usize::MAX)
    }

    /// ★ D2: at most `limit` of our mappings intersecting `[va, end)`, in VA order — O(log n +
    /// limit). The next call resumes at the end of the last one returned.
    #[must_use]
    pub fn within_limited(&self, va: u64, end: u64, limit: usize) -> Vec<(u64, OwnMap)> {
        let first = self.containing(va).map_or(va, |(s, _)| s);
        self.by_va
            .range(first..end)
            .map(|(&s, &m)| (s, m))
            .filter(|&(s, m)| s.saturating_add(m.len) > va)
            .take(limit)
            .collect()
    }

    /// The owned spans of `[va, end)` and the mappings the range would split.
    #[must_use]
    pub fn plan(&self, va: u64, end: u64) -> OwnPlan {
        let mut plan = OwnPlan::default();
        for (s, m) in self.within(va, end) {
            let e = s.saturating_add(m.len);
            if s < va || e > end {
                plan.split.push((s, m.len, m.via));
            }
            let (cs, ce) = (s.max(va), e.min(end));
            match plan.spans.last_mut() {
                Some((_, last_end, via)) if *last_end == cs && *via == m.via => *last_end = ce,
                _ => plan.spans.push((cs, ce, m.via)),
            }
        }
        plan
    }

    /// `[va, end)` is no longer mapped by us — exactly as host RM removes it: a mapping wholly
    /// inside goes, a straddler keeps its outside part(s) (same batch object, same `via`).
    /// UNBOUNDED (O(k)); production goes by [`OwnMaps::cut_chunk`].
    pub fn cut(&mut self, va: u64, end: u64) {
        for (s, m) in self.within(va, end) {
            self.by_va.remove(&s);
            let e = s.saturating_add(m.len);
            if s < va {
                self.insert(s, va - s, m.batch, m.via);
            }
            if e > end {
                self.insert(end, e - end, m.batch, m.via);
            }
        }
    }

    /// ★ D2: [`OwnMaps::cut`] limited to at most `limit` mappings; `true` when nothing of ours
    /// intersects `[va, end)` any more (call again, after releasing the lock, until it is). The
    /// remnants a straddler leaves lie outside the range, so every call makes progress.
    pub fn cut_chunk(&mut self, va: u64, end: u64, limit: usize) -> (usize, bool) {
        let hit = self.within_limited(va, end, limit);
        let n = hit.len();
        for (s, m) in hit {
            self.by_va.remove(&s);
            let e = s.saturating_add(m.len);
            if s < va {
                self.insert(s, va - s, m.batch, m.via);
            }
            if e > end {
                self.insert(end, e - end, m.batch, m.via);
            }
        }
        (n, n < limit)
    }

    /// Forget every mapping of batch object `h` (freeing it unmapped them). UNBOUNDED (O(n)):
    /// tests only; production goes by [`OwnMaps::forget_batch_step`].
    pub fn forget_batch(&mut self, h: u32) {
        self.by_va.retain(|_, m| m.batch != Some(h));
    }

    /// ★ D2: forget the mappings of batch object `h` that start in `[from, end)`, scanning at
    /// most `limit` mappings (every mapping of a batch object lies in its own extent, so the lock
    /// is held O(log n + limit), never O(n)). Returns `Some(next)` to resume at, or `None` when
    /// the range is done. Review fix 2026-10-10: the ledger is shared with the channel plane's act
    /// thread ([`BatchedVas::hand_to_host`]).
    pub fn forget_batch_step(
        &mut self,
        h: u32,
        from: u64,
        end: u64,
        limit: usize,
    ) -> (usize, Option<u64>) {
        let mut scanned = 0usize;
        let mut gone: Vec<u64> = Vec::new();
        let mut resume = None;
        for (&s, m) in self.by_va.range(from..end) {
            if scanned >= limit {
                resume = Some(s);
                break;
            }
            scanned += 1;
            if m.batch == Some(h) {
                gone.push(s);
            }
        }
        for s in gone {
            self.by_va.remove(&s);
        }
        (scanned, resume)
    }

    /// Whether any mapping of ours intersects `[va, end)` — O(log n).
    #[must_use]
    pub fn any_in(&self, va: u64, end: u64) -> bool {
        self.containing(va).is_some()
            || self
                .by_va
                .range(va..end)
                .next()
                .is_some_and(|(&s, m)| s.saturating_add(m.len) > va)
    }

    /// Mappings recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_va.len()
    }

    /// Whether none is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_va.is_empty()
    }
}

/// ★ 2026-10-09: batching outside the guest reservations is worth its fixed cost (a micro
/// reservation alloc + free, a descriptor alloc + free, the view's stitch/munmap) only from this
/// many runs up (`V3_BATCHED_MAP.md` §8.4: per-run ≈ 20 µs map + one unmap per run; the batch's
/// fixed part ≈ 4 RM calls + 2 mm syscalls) — below it the rows go per run.
pub const LOW_RANGE_MIN_RUNS: usize = 8;

/// ★ 2026-10-09: the most host mappings ONE row is split into outside a reservation — a bound on
/// host calls per row, never sized by an unchecked guest value.
///
/// ★ D1 (2026-10-10, `V3_BATCHED_MAP.md` §8.8): the unit counted is the 4 KiB GRAIN, the grain every
/// big-leaf row without a micro reservation is placed at (so any later partial change is exact).
/// ⊘ The text this corrects: *"per leaf up to this bound (4 GiB of 4 KiB leaves …)"* — the unit was
/// the guest LEAF (a 64 KiB / 2 MiB leaf was ONE host mapping, which the guest could split later;
/// host RM cannot remove part of such a mapping outside a VA-reserving `hDma`, so the apply
/// re-made it, transiently unmapping unchanged VAs).
///
/// ★ Review fix 2026-10-10 (finding 1): a row beyond the bound is NO LONGER placed as one `NV01`
/// mapping. ⊘ The text this corrects: *"A longer row stays one mapping (its later partial change
/// is then refused by name, [`SPLIT_OUTSIDE_RESERVATION`], never split)"* — that refusal acked the
/// UNMAP run FAILED while its fully-kept MAP runs were acked APPLIED, so the walker held
/// overlapping placements and the next refresh unmapped the old one whole, unchanged pages with it
/// (`[model]` `sim::adversarial::a_huge_nv01_row_…`, no host error needed). Now: 4 KiB grain up to
/// this bound (4 GiB — the whole Windows low range `[1 MiB, 4.5 GiB)` fits in one row's budget),
/// beyond it THROUGH a micro reservation ([`BatchedVas::low_reserve`], the default), else refused
/// by name ([`HUGE_ROW_OUTSIDE_RESERVATION`], counted in [`BatchedVas::huge_refused`]): absence,
/// never an unsplittable mapping.
pub const MAX_LEAF_PIECES: u64 = 1 << 20;

/// ★ Review fix 2026-10-10: why [`BatchedVas::map`] refused a row (see [`MAX_LEAF_PIECES`]).
pub const HUGE_ROW_OUTSIDE_RESERVATION: &str = "row has more host mappings than one row may be split into outside a VA-reserving hDma and no micro reservation can hold it — refused (a bigger mapping there could never be partially unmapped exactly)";

/// ★ D2 (2026-10-10, `V3_BATCHED_MAP.md` §8.8): **what the ledger locks cost the threads that take
/// them.** Every critical section on `book`, `own` and `micro` is bounded by [`LEDGER_CHUNK`]
/// touched units — recorded here for every hold, so a test (and the steer's log line) can show the
/// bound instead of claiming it. No ledger lock is ever held across a host call, a syscall, a log
/// line, or an allocation proportional to a large `n`.
#[derive(Debug, Default)]
pub struct HoldStats {
    /// Lock holds recorded.
    pub holds: std::sync::atomic::AtomicU64,
    /// The most ledger entries / bitmap words any one hold touched (the bound: [`LEDGER_CHUNK`]).
    pub max_touched: std::sync::atomic::AtomicU64,
    /// The longest any one hold lasted, nanoseconds.
    pub max_hold_ns: std::sync::atomic::AtomicU64,
}

impl HoldStats {
    fn note(&self, touched: usize, held: std::time::Duration) {
        use std::sync::atomic::Ordering::Relaxed;
        self.holds.fetch_add(1, Relaxed);
        self.max_touched
            .fetch_max(u64::try_from(touched).unwrap_or(u64::MAX), Relaxed);
        self.max_hold_ns
            .fetch_max(u64::try_from(held.as_nanos()).unwrap_or(u64::MAX), Relaxed);
    }

    /// `(max entries touched by one hold, longest hold in µs)`.
    #[must_use]
    pub fn max(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.max_touched.load(Relaxed),
            self.max_hold_ns.load(Relaxed).div_ceil(1000),
        )
    }
}

/// A ledger lock, poison-tolerant: a panic elsewhere must not turn the ledger into "nothing of
/// ours here" (the old `if let Ok(..)` did exactly that, silently).
fn lk<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One `(hDma, start, end)` piece of a row: `None` = the space's own routing.
type Seg = (Option<u32>, u64, u64);

/// ★★★ **A host VA space that places batches and keeps their objects' books** — the one
/// implementation production (`kf_qemu::mem::GpuMirror`) and the hardware gate share.
///
/// Every verb here is one WE author on OUR host space (§9); no lock is ever held across a host
/// call (plan under the lock, call, then record). ⊘ Corrected 2026-10-10, above the text it
/// corrects: the VA thread is no longer the only caller — the channel plane's falcon-context steer
/// calls [`BatchedVas::hand_to_host`] from the act thread; a micro reservation with a map in flight
/// through it is pinned ([`MicroResv::pins`]) so neither frees what the other is using.
/// (Was: "the VA thread is the only caller; plan under the lock, call, then record".)
///
/// ★★★ **STATUS (2026-10-09, amended 2026-10-10 by D1/D2): three rules, from host RM's own source,
/// enforced HERE so no caller can break them** (`V3_BATCHED_MAP.md` §8, §8.8):
/// 1. **A range unmap covers OUR mappings only** — one host call per maximal owned span and `hDma`
///    ([`OwnMaps::plan`]), never a gap: host RM removes every mapping of this client's `hDma` in a
///    range, whoever placed it.
/// 2. **No unmap ever SPLITS one of our mappings in the space's `NV01` range.** That `hDma` is an
///    `NV01_MEMORY_VIRTUAL` (`bReserveVaOnAlloc = NV_FALSE`, `virtual_mem.c:349`): a map through it
///    allocates its own VA block and an unmap frees "the block containing the unmapped part's
///    start" (`virt_mem_allocator_gm107.c:1635-1638` → `gpu_vaspace.c:1631-1640`), so a PARTIAL
///    unmap there frees the whole block and the remnants host RM still lists lose their PTEs.
///    `[measured, Windows runs 242/243, RTX 4070, 595.91.07]` host Xid 31 `FAULT_PTE` at
///    `0x4034000` and `NV_ASSERT(NULL != pMemBlock) @ gpu_vaspace.c:1639` at exit; neither with
///    `KF3_NO_BATCHED_MAP=1` (run 244). ★ D1: therefore EVERY mapping of ours outside a VA-reserving
///    `hDma` is exactly ONE 4 KiB page (the unit nothing can split), and a row whose guest leaf is
///    bigger goes THROUGH a micro reservation sized to it (exact partial unmaps), or — when host RM
///    refuses the reservation — at 4 KiB grain. A batch lives inside a VA-RESERVING `hDma`
///    (`NV50_MEMORY_VIRTUAL`, whose unmap invalidates exactly the unmapped PTEs,
///    `virt_mem_allocator_gm107.c:1578-1633`): a guest reservation, or a MICRO reservation made
///    over exactly the batch's VA ([`BatchedVas::low_reserve`], default ON).
/// 3. **No remap, ever.** A VA whose guest mapping did not change is never transiently unmapped
///    (owner, 2026-10-09): partial unmaps are made exact instead.
///
/// ★ D2 — **the locks.** `book`, `own` and `micro` are taken by the VA thread AND by the act thread
/// ([`BatchedVas::hand_to_host`]). The act thread taking them is accepted (a steer request answered
/// by the VA thread would make the act thread WAIT on it, strictly worse) ON CONDITION that no
/// hold is long: see [`HoldStats`] and [`LEDGER_CHUNK`].
#[derive(Debug)]
pub struct BatchedVas<'rm, V: SpaceVerbs = HostVas<'rm>> {
    /// The host space.
    pub vas: V,
    /// Our batch objects in it.
    pub book: std::sync::Mutex<BatchBook>,
    /// ★ Every host mapping of ours in it ([`OwnMaps`]).
    pub own: std::sync::Mutex<OwnMaps>,
    /// ★ Our micro reservations, by their low VA (rule 2).
    pub micro: std::sync::Mutex<BTreeMap<u64, MicroResv>>,
    /// ★ Micro reservations are used (rule 2): outside the guest reservations, a batch of at least
    /// [`LOW_RANGE_MIN_RUNS`] rows, a row bigger than [`MAX_LEAF_PIECES`] grains, and every row
    /// whose guest leaf is bigger than 4 KiB go through one. ★ D3 (2026-10-10): DEFAULT ON; off
    /// (`KF3_DIAG_NO_MICRO_RESERVE`, a diagnostic) rows go at 4 KiB grain and over-bound rows are
    /// refused by name. A reservation host RM refuses at run time is ALWAYS a clean fallback to the
    /// 4 KiB grain; one it accepts is always used.
    pub low_reserve: bool,
    /// Batch objects freed (a counter for the instruments).
    pub frees: std::sync::atomic::AtomicU64,
    /// Micro reservations made / freed / refused by the host (the batch then went per run).
    pub micro_made: std::sync::atomic::AtomicU64,
    /// See [`BatchedVas::micro_made`].
    pub micro_freed: std::sync::atomic::AtomicU64,
    /// See [`BatchedVas::micro_made`].
    pub micro_refused: std::sync::atomic::AtomicU64,
    /// ★ Range unmaps refused because they would split one of our mappings in the `NV01` range
    /// (the caller then unmaps run by run). ★ D1: unreachable for page-aligned ranges — every
    /// mapping of ours there is one 4 KiB page ([`BatchedVas::rigid_seen`]) — and counted anyway;
    /// expected 0, never silent.
    pub unsafe_splits: std::sync::atomic::AtomicU64,
    /// ★ Review fix 2026-10-10: the per-row bound ([`MAX_LEAF_PIECES`]; tests lower it).
    pub max_leaf_pieces: u64,
    /// ★ Review fix 2026-10-10: rows refused by [`HUGE_ROW_OUTSIDE_RESERVATION`].
    pub huge_refused: std::sync::atomic::AtomicU64,
    /// ★ Review fix 2026-10-10: mappings of ours found where a NEW row was about to be mapped (a
    /// mapping a refused rollback or take-down left behind) and removed first — counted.
    pub strays_removed: std::sync::atomic::AtomicU64,
    /// ★ Review fix 2026-10-10 (finding 5): batch objects whose free host RM refused — still OURS,
    /// still mapped somewhere: [`BatchedVas::leftovers`] reports them so the space is never recycled.
    pub stuck_objects: std::sync::Mutex<Vec<u32>>,
    /// ★ D1: big-leaf segments placed THROUGH a micro reservation sized to them (one reservation
    /// per segment, or per leaf when the whole segment was refused and was too big for 4 KiB grain).
    pub leaf_reserved: std::sync::atomic::AtomicU64,
    /// ★ D1: big-leaf segments placed at 4 KiB grain (no reservation: off, or refused by host RM).
    pub leaf_grained: std::sync::atomic::AtomicU64,
    /// ★ D1: mappings of ours outside every VA-reserving `hDma` and bigger than 4 KiB that
    /// [`BatchedVas::own_view`] reported (what the apply would have had to RE-MAKE). Must stay 0.
    pub rigid_seen: std::sync::atomic::AtomicU64,
    /// ★ D2: what the ledger locks cost ([`HoldStats`]).
    pub holds: HoldStats,
    _rm: std::marker::PhantomData<&'rm ()>,
}

/// ★ One micro reservation of ours: `[lo, hi)` (its key is `lo`), its handle, and the maps in
/// flight THROUGH it.
///
/// ★ Review fix 2026-10-10 (finding 6): `pins` > 0 while a map through it is between its host call
/// and its record in the ledger — a reservation is released only when nothing of ours is in it
/// AND nothing is being mapped through it, so a caller on another thread (the falcon-context
/// steer, [`BatchedVas::hand_to_host`]) can release it without racing the VA thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroResv {
    /// Its end.
    pub hi: u64,
    /// Its handle (an `hDma`).
    pub handle: u32,
    /// Maps in flight through it.
    pub pins: u32,
}

/// ★ Review fix 2026-10-10 (finding 6): what [`BatchedVas::hand_to_host`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOver {
    /// Nothing of ours is mapped or reserved there any more: host RM can place its own mapping.
    Free,
    /// Our mappings there are gone, but a micro reservation of ours still covers part of the range
    /// (other rows live in it): host RM cannot place there until it is released.
    StillReserved,
    /// A mapping of ours is there again (the VA thread mapped a new row meanwhile).
    StillOurs,
    /// Host RM refused (or the range would split one of ours in the `NV01` range); the ledger
    /// keeps whatever is still ours.
    Refused(String),
}

/// ★ Why [`BatchedVas::unmap_range`] refused before any host call. ★ D1: unreachable for
/// page-aligned ranges outside a VA-reserving `hDma` (every mapping of ours there is one page) and
/// inside one (splits are exact); kept as the refusal for an unaligned range, which no apply path
/// produces.
pub const SPLIT_OUTSIDE_RESERVATION: &str = "range would split one of our mappings outside a VA-reserving hDma (host RM would free its whole VA block) — refused, unmap run by run";

fn bump(c: &std::sync::atomic::AtomicU64) {
    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

impl<V: SpaceVerbs> BatchedVas<'_, V> {
    /// Batches over `vas`; ★ D3: micro reservations ON (see [`BatchedVas::low_reserve`]).
    #[must_use]
    pub fn new(vas: V) -> Self {
        Self::with_low_reserve(vas, true)
    }

    /// Batches over `vas`; `low_reserve` uses micro reservations outside the guest reservations
    /// (rule 2); off, every row there goes at the 4 KiB grain.
    #[must_use]
    pub fn with_low_reserve(vas: V, low_reserve: bool) -> Self {
        BatchedVas {
            vas,
            book: std::sync::Mutex::new(BatchBook::default()),
            own: std::sync::Mutex::new(OwnMaps::default()),
            micro: std::sync::Mutex::new(BTreeMap::new()),
            low_reserve,
            frees: std::sync::atomic::AtomicU64::new(0),
            micro_made: std::sync::atomic::AtomicU64::new(0),
            micro_freed: std::sync::atomic::AtomicU64::new(0),
            micro_refused: std::sync::atomic::AtomicU64::new(0),
            unsafe_splits: std::sync::atomic::AtomicU64::new(0),
            max_leaf_pieces: MAX_LEAF_PIECES,
            huge_refused: std::sync::atomic::AtomicU64::new(0),
            strays_removed: std::sync::atomic::AtomicU64::new(0),
            stuck_objects: std::sync::Mutex::new(Vec::new()),
            leaf_reserved: std::sync::atomic::AtomicU64::new(0),
            leaf_grained: std::sync::atomic::AtomicU64::new(0),
            rigid_seen: std::sync::atomic::AtomicU64::new(0),
            holds: HoldStats::default(),
            _rm: std::marker::PhantomData,
        }
    }

    /// ★ D2 — **the only way the ledger locks are taken.** Runs `f` under `m`; `f` returns its
    /// result and how many ledger entries / bitmap words it touched, recorded in
    /// [`BatchedVas::holds`]. `f` must stay within [`LEDGER_CHUNK`] units, and must not call the
    /// host, log, or allocate in proportion to a large `n`.
    fn hold<T, R>(&self, m: &std::sync::Mutex<T>, f: impl FnOnce(&mut T) -> (R, usize)) -> R {
        let mut g = lk(m);
        let t0 = std::time::Instant::now();
        let (r, touched) = f(&mut g);
        self.holds.note(touched, t0.elapsed());
        r
    }

    /// [`BatchedVas::hold`] over two ledger locks, always `micro` then `own`.
    fn hold2<A, B, R>(
        &self,
        a: &std::sync::Mutex<A>,
        b: &std::sync::Mutex<B>,
        f: impl FnOnce(&mut A, &mut B) -> (R, usize),
    ) -> R {
        let mut ga = lk(a);
        let mut gb = lk(b);
        let t0 = std::time::Instant::now();
        let (r, touched) = f(&mut ga, &mut gb);
        self.holds.note(touched, t0.elapsed());
        r
    }

    /// ★ D2: `(max entries touched by one lock hold, longest hold in µs)` so far.
    #[must_use]
    pub fn hold_stats(&self) -> (u64, u64) {
        self.holds.max()
    }

    /// `[va, end)` cut at our micro reservations' edges: `(via, start, end)` in VA order. ★ Every
    /// reservation it routes through is PINNED ([`MicroResv::pins`]) until the caller's
    /// [`BatchedVas::unpin`] — so no release frees it while a map through it is in flight.
    ///
    /// ★ D2: walked in chunks of [`LEDGER_CHUNK`] reservations (the lock is dropped between them).
    /// Only the calling (VA) thread ever ADDS a reservation, so none appears in a gap meanwhile;
    /// one the act thread releases before this chunk pins it is simply not seen.
    fn segments(&self, va: u64, end: u64) -> Vec<Seg> {
        let mut out = Vec::new();
        if end <= va {
            return out;
        }
        let (mut cur, mut from, mut first) = (va, va, true);
        loop {
            let (list, next) = self.hold(&self.micro, |m| {
                let mut list: Vec<(u64, u64, u32)> = Vec::new();
                if first
                    && let Some((&lo, r)) = m.range_mut(..va).next_back()
                    && r.hi > va
                {
                    r.pins += 1;
                    list.push((lo, r.hi, r.handle));
                }
                let mut n = 0usize;
                let mut last = 0u64;
                for (&lo, r) in m.range_mut(from..end).take(LEDGER_CHUNK) {
                    r.pins += 1;
                    list.push((lo, r.hi, r.handle));
                    n += 1;
                    last = lo;
                }
                let touched = list.len();
                let next = (n == LEDGER_CHUNK).then(|| last + 1);
                ((list, next), touched)
            });
            for (lo, hi, h) in list {
                if lo > cur {
                    out.push((None, cur, lo.min(end)));
                }
                let (s, e) = (lo.max(cur), hi.min(end));
                if s < e {
                    out.push((Some(h), s, e));
                    cur = e;
                }
            }
            first = false;
            match next {
                Some(n) if n < end => from = n,
                _ => break,
            }
        }
        if cur < end {
            out.push((None, cur, end));
        }
        out
    }

    /// Record a NEW micro reservation of ours, pinned once (the caller maps through it next).
    fn add_micro(&self, lo: u64, hi: u64, handle: u32) {
        bump(&self.micro_made);
        self.hold(&self.micro, |m| {
            m.insert(
                lo,
                MicroResv {
                    hi,
                    handle,
                    pins: 1,
                },
            );
            ((), 1)
        });
    }

    /// Undo [`BatchedVas::segments`]' pins (and [`BatchedVas::add_micro`]'s) on `segs`. ★ D2:
    /// chunked, and found by `lo` (a segment lies inside its reservation), never by a scan.
    fn unpin(&self, segs: &[Seg]) {
        for chunk in segs.chunks(LEDGER_CHUNK) {
            self.hold(&self.micro, |m| {
                for &(via, s, _) in chunk {
                    if let Some(h) = via
                        && let Some((_, r)) = m.range_mut(..=s).next_back()
                        && r.handle == h
                    {
                        r.pins = r.pins.saturating_sub(1);
                    }
                }
                ((), chunk.len())
            });
        }
    }

    /// A micro reservation host RM refused: counted, the first few named.
    fn reserve_refused(&self, va: u64, len: u64, why: &str) {
        if self
            .micro_refused
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            < 8
        {
            eprintln!(
                "kf-mem: micro reservation {va:#x}+{len:#x} refused ({why}) — the 4 KiB grain / per-run path instead"
            );
        }
    }

    /// ★★★ 2026-10-09 — **[`BatchedVas::segments`], then every unreserved part placed so that a
    /// later partial change is EXACT** (rule 2).
    ///
    /// ★ D1 (2026-10-10): outside every VA-reserving `hDma` (the `NV01` range) a mapping of ours is
    /// either ONE 4 KiB page or inside a micro reservation — never a bigger `NV01` mapping, which
    /// the guest could later split (a 64 KiB / 2 MiB leaf re-expressed as 4 KiB leaves with one
    /// page re-pointed) and host RM could not partly unmap. So per unreserved segment `[s, e)`:
    /// - **(a)** a segment of whole big leaves (`leaf` > 4 KiB, aligned), or one beyond
    ///   [`BatchedVas::max_leaf_pieces`] grains, goes THROUGH one micro reservation made over
    ///   exactly it ([`BatchedVas::low_reserve`]) — one alloc and one free per segment, the map is
    ///   one host mapping, later partial unmaps are exact;
    /// - **(b)** when there is no reservation (off, or host RM refused it at run time — nothing is
    ///   placed or lost by the refusal), 4 KiB grain: one host mapping per page, the mapping IS
    ///   the unit of unmap, so no RM partial-unmap semantics are relied on at all;
    /// - **(a2)** when (a) was refused AND (b) would exceed the bound, but the segment is whole
    ///   leaves, ONE reservation per leaf instead (still exact; a leaf whose reservation is also
    ///   refused goes at 4 KiB grain, within the same grain budget);
    /// - else refused by name ([`HUGE_ROW_OUTSIDE_RESERVATION`]): absence, never an unsplittable
    ///   mapping.
    ///
    /// ⊘ The text this corrects (2026-10-09/10): *"one segment per guest LEAF … `leaf` bytes when
    /// the segment is whole aligned leaves of it, else the 4 KiB grain"* — a big leaf was one
    /// `NV01` mapping, so a split of it forced the apply to re-make (transiently unmap) the
    /// unchanged part. A reservation made here is released by the caller's failure path
    /// ([`BatchedVas::release_micro`]) when nothing of ours is in it.
    ///
    /// # Errors
    /// [`HUGE_ROW_OUTSIDE_RESERVATION`], before any map.
    fn leaf_segments(&self, va: u64, end: u64, leaf: u64) -> Result<Vec<Seg>, String> {
        let segs = self.segments(va, end);
        let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
        let mut fresh: Vec<Seg> = Vec::new();
        for &(via, s, e) in &segs {
            if via.is_some() || self.vas.splits_safely(s, e - s) {
                out.push((via, s, e));
                continue;
            }
            let big = leaf.is_power_of_two() && leaf > BATCH_PAGE && (s | e).is_multiple_of(leaf);
            let grains = (e - s) / BATCH_PAGE;
            let over = grains > self.max_leaf_pieces;
            if (big || over) && self.low_reserve {
                match self.vas.reserve(s, e - s) {
                    Ok(h) => {
                        self.add_micro(s, e, h);
                        fresh.push((Some(h), s, e));
                        out.push((Some(h), s, e));
                        if big {
                            bump(&self.leaf_reserved);
                        }
                        continue;
                    }
                    Err(why) => self.reserve_refused(s, e - s, &why),
                }
            }
            if !over {
                if big {
                    bump(&self.leaf_grained);
                }
                push_grains(&mut out, s, e);
                continue;
            }
            // Over the grain bound and no reservation for the whole segment.
            let leaves = (e - s) / leaf;
            if big && self.low_reserve && leaves <= self.max_leaf_pieces {
                match self.per_leaf_reservations(s, e, leaf, &mut out, &mut fresh) {
                    Ok(()) => continue,
                    Err(msg) => {
                        self.fail_segments(&segs, &fresh);
                        return Err(msg);
                    }
                }
            }
            self.fail_segments(&segs, &fresh);
            return Err(format!(
                "map {s:#x}+{:#x} ({grains} grains of {BATCH_PAGE:#x}): {HUGE_ROW_OUTSIDE_RESERVATION}",
                e - s
            ));
        }
        Ok(out)
    }

    /// Every pin taken so far goes (the caller maps nothing); the fresh reservations stay for the
    /// caller's [`BatchedVas::release_micro`].
    fn fail_segments(&self, segs: &[Seg], fresh: &[Seg]) {
        bump(&self.huge_refused);
        self.unpin(segs);
        self.unpin(fresh);
    }

    /// (a2): one reservation per leaf of `[s, e)`; a leaf whose reservation is refused goes at 4 KiB
    /// grain, the total of such grains within [`BatchedVas::max_leaf_pieces`].
    fn per_leaf_reservations(
        &self,
        s: u64,
        e: u64,
        leaf: u64,
        out: &mut Vec<Seg>,
        fresh: &mut Vec<Seg>,
    ) -> Result<(), String> {
        let mut grains = 0u64;
        let mut cur = s;
        while cur < e {
            let next = cur + leaf;
            match self.vas.reserve(cur, leaf) {
                Ok(h) => {
                    self.add_micro(cur, next, h);
                    fresh.push((Some(h), cur, next));
                    out.push((Some(h), cur, next));
                    bump(&self.leaf_reserved);
                }
                Err(why) => {
                    self.reserve_refused(cur, leaf, &why);
                    grains += leaf / BATCH_PAGE;
                    if grains > self.max_leaf_pieces {
                        return Err(format!(
                            "map {s:#x}+{:#x} (leaves of {leaf:#x}): {HUGE_ROW_OUTSIDE_RESERVATION}",
                            e - s
                        ));
                    }
                    bump(&self.leaf_grained);
                    push_grains(out, cur, next);
                }
            }
            cur = next;
        }
        Ok(())
    }

    /// ★ Review fix 2026-10-10: before a NEW row is mapped at `[va, end)`, remove any mapping of
    /// ours still recorded there. The apply only maps a piece no committed placement keeps (a kept
    /// page is never re-mapped; a piece over a placement whose unmap failed is never attempted),
    /// so what the ledger still holds there is a STRAY — a rollback or a take-down host RM refused
    /// earlier — never an unchanged VA. Without this the map would find the VA occupied, answer
    /// HELD, and the stray would outlive the walker's record of it.
    fn clear_strays(&self, va: u64, end: u64, defer: bool) -> Result<(), String> {
        let any = self.hold(&self.own, |o| (o.any_in(va, end), 1));
        if !any {
            return Ok(());
        }
        bump(&self.strays_removed);
        self.unmap_range(va, end - va, defer)
            .map_err(|e| format!("map {va:#x}+{:#x}: a stray mapping of ours is there and could not be removed first: {e}", end - va))
    }

    /// Our mappings intersecting `[va, end)`, in VA order, collected in chunks (the lock is
    /// dropped between them; the `Vec` is built OUTSIDE it).
    fn collect_within(&self, va: u64, end: u64) -> Vec<(u64, OwnMap)> {
        let mut all = Vec::new();
        let mut cur = va;
        while cur < end {
            let chunk = self.hold(&self.own, |o| {
                let c = o.within_limited(cur, end, LEDGER_CHUNK);
                let n = c.len();
                (c, n)
            });
            let full = chunk.len() == LEDGER_CHUNK;
            if let Some(&(s, m)) = chunk.last() {
                cur = cur.max(s.saturating_add(m.len));
            }
            all.extend(chunk);
            if !full {
                break;
            }
        }
        all
    }

    /// ★ Review fix 2026-10-10 (findings 1 and 3): what the apply may KEEP inside `[va, end)` —
    /// the bytes a mapping of ours covers (`owned`, merged), and the mappings of ours there that
    /// host RM cannot split exactly (`rigid`: outside every VA-reserving `hDma`, rule 2), each
    /// `(start, end)` whole even where it crosses the range's edges.
    ///
    /// ★ D1: `rigid` is EMPTY by construction — outside a reservation every mapping of ours is one
    /// 4 KiB page, and a page cannot be split. A mapping bigger than that reported here is counted
    /// ([`BatchedVas::rigid_seen`], must stay 0) and left to the apply's last-resort re-make.
    /// ★ D2: read in chunks ([`LEDGER_CHUNK`] mappings per lock hold).
    #[must_use]
    pub fn own_view(&self, va: u64, end: u64) -> crate::ledger::OwnView {
        let mut v = crate::ledger::OwnView::default();
        let mut cur = va;
        while cur < end {
            let chunk = self.hold(&self.own, |o| {
                let c = o.within_limited(cur, end, LEDGER_CHUNK);
                let n = c.len();
                (c, n)
            });
            let full = chunk.len() == LEDGER_CHUNK;
            for &(s, m) in &chunk {
                let e = s.saturating_add(m.len);
                let (cs, ce) = (s.max(va), e.min(end));
                match v.owned.last_mut() {
                    Some(last) if last.1 == cs => last.1 = ce,
                    _ => v.owned.push((cs, ce)),
                }
                if m.via.is_none() && m.len > BATCH_PAGE && !self.vas.splits_safely(s, m.len) {
                    bump(&self.rigid_seen);
                    v.rigid.push((s, e));
                }
                cur = cur.max(e);
            }
            if !full {
                break;
            }
        }
        v
    }

    /// ★ Review fix 2026-10-10 (finding 6) — **hand `[va, va+len)` to host RM**: unmap every
    /// mapping of OURS there through the ledger (owned spans only, each through the `hDma` it was
    /// mapped through — a micro reservation included; never a split in the `NV01` range) and say
    /// whether host RM can now place its own mapping there. Safe OFF the VA-manager thread (the
    /// channel plane's falcon-context steer): the ledger locks are never held across a host call
    /// and every hold is bounded ([`LEDGER_CHUNK`], D2), and a micro reservation emptied here is
    /// released only while no map through it is in flight ([`MicroResv::pins`]). ⊘ It replaces a raw
    /// `HostRm::unmap_range` that was not limited to our mappings, left the ledger (and batch book)
    /// stale, and — for a row mapped through a micro reservation — named the wrong `hDma`, so it
    /// removed nothing while the log said "steered".
    #[must_use]
    pub fn hand_to_host(&self, va: u64, len: u64) -> HandOver {
        let end = va.saturating_add(len);
        if let Err(e) = self.unmap_owned(va, len, false) {
            return HandOver::Refused(e);
        }
        // Race-free with the VA thread: a reservation is released only when nothing of ours is in
        // it and no map through it is in flight ([`MicroResv::pins`]).
        self.release_micro(va, end);
        if self.hold(&self.own, |o| (o.any_in(va, end), 1)) {
            HandOver::StillOurs
        } else if self.micro_covers(va, end) {
            HandOver::StillReserved
        } else {
            HandOver::Free
        }
    }

    /// ★ Review fix 2026-10-10: whether one of OUR micro reservations still covers any byte of
    /// `[va, end)` (host RM cannot place its own mapping there while it lives). ★ D2: O(log n)
    /// (reservations never overlap: only the predecessor can reach into `va`).
    #[must_use]
    pub fn micro_covers(&self, va: u64, end: u64) -> bool {
        if end <= va {
            return false;
        }
        self.hold(&self.micro, |m| {
            (
                m.range(..va).next_back().is_some_and(|(_, r)| r.hi > va)
                    || m.range(va..end).next().is_some(),
                2,
            )
        })
    }

    /// ★ Review fix 2026-10-10 (finding 5): what this space still holds of ours that a retire
    /// could not release — mappings in the ledger, micro reservations and batch objects whose free
    /// host RM refused. Non-zero ⇒ the space must not be recycled (free it instead).
    #[must_use]
    pub fn leftovers(&self) -> usize {
        self.hold(&self.own, |o| (o.len(), 1))
            + self.hold(&self.micro, |m| (m.len(), 1))
            + self.hold(&self.book, |b| (b.len(), 1))
            + self.hold(&self.stuck_objects, |s| (s.len(), 1))
    }

    /// Record the pieces just placed, [`LEDGER_CHUNK`] per lock hold.
    fn record_all(&self, placed: &[Seg]) {
        for chunk in placed.chunks(LEDGER_CHUNK) {
            self.hold(&self.own, |o| {
                for &(via, s, e) in chunk {
                    o.insert(s, e - s, None, via);
                }
                ((), chunk.len())
            });
        }
    }

    fn record(&self, va: u64, len: u64, batch: Option<u32>, via: Option<u32>) {
        self.hold(&self.own, |o| {
            o.insert(va, len, batch, via);
            ((), 1)
        });
    }

    /// `[s, e)` is no longer ours on the host: cut the ledger, [`LEDGER_CHUNK`] mappings per hold.
    fn cut_own(&self, s: u64, e: u64) {
        loop {
            let done = self.hold(&self.own, |o| {
                let (n, d) = o.cut_chunk(s, e, LEDGER_CHUNK);
                (d, n)
            });
            if done {
                break;
            }
        }
    }

    /// Forget the ledger entries of batch object `h` (freed, so unmapped) inside its extent.
    fn forget_batch_range(&self, h: u32, s: u64, e: u64) {
        let mut from = s;
        while from < e {
            let resume = self.hold(&self.own, |o| {
                let (n, r) = o.forget_batch_step(h, from, e, LEDGER_CHUNK);
                (r, n)
            });
            match resume {
                Some(n) => from = n,
                None => break,
            }
        }
    }

    /// Whole-mapping unmap of one OWN entry, by its routing.
    fn unmap_entry(&self, s: u64, m: OwnMap, defer: bool) -> Result<(), String> {
        match m.via {
            Some(h) => self.vas.unmap_in(h, s, 0, defer),
            // By ROW: a row straddling a guest-reservation edge is one host mapping per `hDma`.
            None => self.vas.unmap_row(s, m.len, defer),
        }
    }

    /// Map `[va, end)` piece by piece (one host map per segment), all or nothing; records each
    /// placed piece. `one(via, start, len)` performs one piece.
    fn map_segments(
        &self,
        va: u64,
        end: u64,
        leaf: u64,
        defer: bool,
        one: &dyn Fn(Option<u32>, u64, u64) -> Result<Mapped, String>,
    ) -> Result<Mapped, String> {
        self.clear_strays(va, end, defer)?;
        let segs = match self.leaf_segments(va, end, leaf) {
            Ok(s) => s,
            Err(e) => {
                self.release_micro(va, end);
                return Err(e);
            }
        };
        let mut placed: Vec<Seg> = Vec::new();
        let mut verdict = Ok(Mapped::Placed);
        for &(via, s, e) in &segs {
            match one(via, s, e - s) {
                Ok(Mapped::Placed) => placed.push((via, s, e)),
                other => {
                    verdict = other;
                    break;
                }
            }
        }
        if verdict != Ok(Mapped::Placed) {
            // All or nothing: the pieces already placed go again (they were never acknowledged).
            let mut stray: Option<String> = None;
            for &(via, s, e) in placed.iter().rev() {
                let m = OwnMap {
                    len: e - s,
                    batch: None,
                    via,
                };
                if let Err(r) = self.unmap_entry(s, m, defer) {
                    // ★ Review fix 2026-10-10: the piece is still OURS on the host — recorded, so
                    // a later map there removes it first ([`BatchedVas::clear_strays`]) and a
                    // retire takes it down; never an unrecorded mapping of ours.
                    self.record(s, e - s, None, via);
                    eprintln!(
                        "kf-mem: map {va:#x}+{:#x}: rolling back piece {s:#x} refused ({r}) — kept in the ledger as a stray",
                        end - va
                    );
                    stray.get_or_insert(r);
                }
            }
            self.unpin(&segs);
            self.release_micro(va, end);
            // ★ Something of ours is left there: never "held by host" (the walker would commit a
            // placement it never asks us to take down) — refused, retried, the stray removed first.
            return match stray {
                Some(r) => Err(format!(
                    "map {va:#x}+{:#x}: not placed whole, and rolling back a placed piece was refused ({r})",
                    end - va
                )),
                None => verdict,
            };
        }
        self.record_all(&placed);
        self.unpin(&segs);
        Ok(Mapped::Placed)
    }

    /// ★ One row, mapped per run (the per-run verb), recorded as ours when placed. A row over a
    /// live micro reservation goes THROUGH it (a map through the `NV01` range there would find the
    /// VA held by the reservation).
    ///
    /// # Errors
    /// The host's refusal, by name.
    pub fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let end = d.va.saturating_add(d.len);
        self.map_segments(d.va, end, d.leaf, defer, &|via, s, l| {
            let piece = Desired {
                va: s,
                len: l,
                off: d.off + (s - d.va),
                ..*d
            };
            match via {
                Some(h) => self.vas.map_row_in(h, &piece, defer),
                None => self.vas.map_row(&piece, defer),
            }
        })
    }

    /// ★ One SKED-reflected row (message kind), recorded as ours when placed.
    ///
    /// # Errors
    /// The host's refusal, by name.
    pub fn map_sked(&self, sk: &SkedRow, defer: bool) -> Result<Mapped, String> {
        let end = sk.va.saturating_add(sk.len);
        self.map_segments(sk.va, end, BATCH_PAGE, defer, &|via, s, l| {
            let piece = SkedRow {
                va: s,
                len: l,
                off: sk.off + (s - sk.va),
                ..*sk
            };
            match via {
                Some(h) => self.vas.map_sked_in(h, &piece, defer),
                None => self.vas.map_sked_row(&piece, defer),
            }
        })
    }

    /// ★ Place VA-contiguous guest-RAM `rows` as ONE batch stitched from `ram_fd` and book its
    /// object. `Ok` ⇔ every row is our mapping; `Err` ⇔ none is (an object the book cannot
    /// account for is freed — which unmaps it — before the refusal returns).
    ///
    /// ★ 2026-10-09 (rule 2): a batch lives inside ONE VA-reserving `hDma`: wholly inside a guest
    /// reservation; or wholly inside one live micro reservation (a batch re-using its dead pages);
    /// or, with [`BatchedVas::low_reserve`] and at least [`LOW_RANGE_MIN_RUNS`] rows, inside a NEW
    /// micro reservation made over exactly its VA. Anything else answers
    /// [`crate::ledger::NOT_BATCHED`] — the per-run path, silently (not a fallback). ★ D3: a
    /// reservation host RM refuses is exactly that answer — the rows then go per run at the 4 KiB
    /// grain, nothing placed or lost.
    ///
    /// # Errors
    /// The host's refusal or the book's, by name.
    pub fn place(
        &self,
        ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<(), String> {
        let (va, len) = match (rows.first(), rows.last()) {
            (Some(a), Some(b)) => (
                a.va,
                b.va.checked_add(b.len)
                    .and_then(|e| e.checked_sub(a.va))
                    .ok_or("batch rows out of order")?,
            ),
            _ => return Err("empty batch".into()),
        };
        let end = va + len;
        self.clear_strays(va, end, defer)?;
        let segs = self.segments(va, end);
        let mut pinned: Vec<Seg> = segs.clone();
        let mapped: Result<(u32, Option<u32>), String> = match segs.as_slice() {
            [(None, ..)] if self.vas.splits_safely(va, len) => self
                .vas
                .map_scattered(ram_fd, rows, defer)
                .map(|h| (h, None)),
            [(Some(h), ..)] => self
                .vas
                .map_scattered_in(*h, ram_fd, rows, defer)
                .map(|o| (o, Some(*h))),
            [(None, ..)] if self.low_reserve && rows.len() >= LOW_RANGE_MIN_RUNS => {
                match self.vas.reserve(va, len) {
                    Ok(h) => {
                        self.add_micro(va, end, h);
                        pinned.push((Some(h), va, end));
                        self.vas
                            .map_scattered_in(h, ram_fd, rows, defer)
                            .map(|o| (o, Some(h)))
                    }
                    Err(e) => {
                        self.reserve_refused(va, len, &e);
                        Err(crate::ledger::NOT_BATCHED.into())
                    }
                }
            }
            _ => Err(crate::ledger::NOT_BATCHED.into()),
        };
        let (handle, via) = match mapped {
            Ok(x) => x,
            Err(e) => {
                self.unpin(&pinned);
                self.release_micro(va, end);
                return Err(e);
            }
        };
        // The liveness bitmap is built OUTSIDE the lock (D2).
        let booked = BatchBook::prepare(va, len, handle)
            .and_then(|p| self.hold(&self.book, |b| (b.insert_prepared(p), 1)))
            .map_err(|e| format!("{e:?}"));
        if let Err(e) = booked {
            self.free(vec![handle]);
            self.unpin(&pinned);
            self.release_micro(va, end);
            return Err(format!(
                "batch {va:#x}+{len:#x}: its object could not be booked ({e}) — freed, nothing placed"
            ));
        }
        self.record(va, len, Some(handle), via);
        self.unpin(&pinned);
        Ok(())
    }

    /// ★ Unmap every mapping of OURS in `[va, va+len)` — one host range per maximal owned span and
    /// `hDma` (O(spans), never a gap) — then free the batch objects and micro reservations left
    /// empty. Nothing of ours there: `Ok` with no host call.
    ///
    /// # Errors
    /// [`SPLIT_OUTSIDE_RESERVATION`] before any host call (the caller unmaps run by run); else the
    /// host's refusal (the spans before it are recorded as gone; the rest is untouched).
    pub fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        let r = self.unmap_owned(va, len, defer);
        // ★ Review fix 2026-10-10 (finding 5): ⊘ an erroring span used to return BEFORE this, so a
        // micro reservation emptied by the spans that did land was never released.
        self.release_micro(va, va.saturating_add(len));
        r
    }

    /// ★ Review fix 2026-10-10: [`BatchedVas::unmap_range`] without the release of the micro
    /// reservations it empties (the caller releases them).
    ///
    /// ★ D2: the ledger is read in chunks and the spans are formed — and unmapped — as the chunks
    /// stream by, so no lock is held for more than [`LEDGER_CHUNK`] mappings, however many the
    /// range holds; a span is one host call and O(1) memory whatever its length.
    ///
    /// # Errors
    /// As [`BatchedVas::unmap_range`]. The ledger is cut for exactly the spans host RM answered
    /// `Ok` for; a span that failed stays recorded (a range over it is idempotent: retried later).
    pub fn unmap_owned(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        let end = va
            .checked_add(len)
            .ok_or_else(|| format!("unmap range {va:#x}+{len:#x} wraps"))?;
        if end <= va {
            return Ok(());
        }
        // Only the first and the last mapping of the range can cross an edge of it.
        let (lo_edge, hi_edge) = self.hold(&self.own, |o| {
            (
                (
                    o.containing(va).filter(|&(s, _)| s < va),
                    o.containing(end - 1)
                        .filter(|&(s, m)| s.saturating_add(m.len) > end),
                ),
                2,
            )
        });
        if let Some((s, m)) = [lo_edge, hi_edge]
            .into_iter()
            .flatten()
            .find(|&(s, m)| m.via.is_none() && !self.vas.splits_safely(s, m.len))
        {
            bump(&self.unsafe_splits);
            return Err(format!(
                "unmap range {va:#x}+{len:#x}: our mapping {s:#x}+{:#x}: {SPLIT_OUTSIDE_RESERVATION}",
                m.len
            ));
        }
        let mut open: Option<(u64, u64, Option<u32>)> = None;
        let mut cur = va;
        while cur < end {
            let chunk = self.hold(&self.own, |o| {
                let c = o.within_limited(cur, end, LEDGER_CHUNK);
                let n = c.len();
                (c, n)
            });
            let full = chunk.len() == LEDGER_CHUNK;
            for &(s, m) in &chunk {
                let e = s.saturating_add(m.len);
                let (cs, ce) = (s.max(va), e.min(end));
                match open.as_mut() {
                    Some((_, oe, via)) if *oe == cs && *via == m.via => *oe = ce,
                    _ => {
                        if let Some(span) = open.replace((cs, ce, m.via)) {
                            self.unmap_span(span, defer)?;
                        }
                    }
                }
                cur = cur.max(e);
            }
            if !full {
                break;
            }
        }
        if let Some(span) = open {
            self.unmap_span(span, defer)?;
        }
        Ok(())
    }

    /// One maximal owned span: the host call (no lock held), then the ledger and the book.
    fn unmap_span(&self, (s, e, via): (u64, u64, Option<u32>), defer: bool) -> Result<(), String> {
        match via {
            Some(h) => self.vas.unmap_in(h, s, e - s, defer)?,
            None => self.vas.unmap_range(s, e - s, defer)?,
        }
        self.cut_own(s, e);
        self.retired(s, e - s);
        Ok(())
    }

    /// ★ Unmap ONE committed run at `va` (`len` when known). A run that is exactly our host
    /// mapping(s) — one, or one per `hDma` piece — goes by the whole-mapping verb; a PIECE of a
    /// larger mapping of ours (a batch) goes by range ([`BatchedVas::unmap_range`]); nothing of
    /// ours in the run is answered with no host call.
    ///
    /// ★ Review fix 2026-10-10 (finding 4): with a known `len` the WHOLE run `[va, va+len)` is
    /// asked, never its start alone. ⊘ Before, "nothing of ours here" was decided from `va`: after a
    /// range that cut the run's start and then failed on its tail, the fallback answered `Ok` with
    /// no host call and the tail outlived the guest's unmap, still mapping the old guest page
    /// (`[model]` adversarial seed 36 step 7 call 2).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unmap_run(&self, va: u64, len: Option<u64>, defer: bool) -> Result<(), String> {
        let end = match len {
            Some(l) => va.saturating_add(l),
            None => match self.hold(&self.own, |o| (o.containing(va), 1)) {
                Some((s, m)) if s == va => va.saturating_add(m.len),
                Some(_) => {
                    return Err(format!(
                        "unmap {va:#x}: a piece of a larger mapping of ours with no known length — refused (a whole-mapping unmap would take the rest)"
                    ));
                }
                None => va,
            },
        };
        let entries = self.collect_within(va, end);
        if entries.is_empty() {
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 16 {
                eprintln!(
                    "kf-mem: unmap {va:#x}+{:#x}: no host mapping of ours there — no host call (nothing of ours to take down)",
                    end - va
                );
            }
            return Ok(());
        }
        // Whole ⇔ the entries tile [va, end) exactly, each starting and ending inside it.
        let mut cur = va;
        let whole = entries.iter().all(|&(s, m)| {
            let ok = s == cur && s.saturating_add(m.len) <= end;
            cur = s.saturating_add(m.len);
            ok
        }) && cur == end;
        if !whole {
            return match len {
                Some(l) => self.unmap_range(va, l, defer),
                None => Err(format!(
                    "unmap {va:#x}: a piece of a larger mapping of ours with no known length — refused (a whole-mapping unmap would take the rest)"
                )),
            };
        }
        let mut r = Ok(());
        let mut done_end = va;
        for (s, m) in entries {
            if let Err(e) = self.unmap_entry(s, m, defer) {
                r = Err(e);
                break;
            }
            self.hold(&self.own, |o| {
                o.cut(s, s.saturating_add(m.len));
                ((), 1)
            });
            done_end = s.saturating_add(m.len);
        }
        if done_end > va {
            // The entries that landed tile [va, done_end): one pass over the book for all of them.
            self.retired(va, done_end - va);
        }
        self.release_micro(va, end);
        r
    }

    /// Free every batch object and micro reservation left (the space is being retired). Returns
    /// how many batch objects.
    pub fn drain(&self) -> usize {
        let taken = self.hold(&self.book, |b| (b.take_all(), 1));
        let rest = taken.extents();
        drop(taken); // the bitmaps go OUTSIDE the lock
        for &(h, s, l) in &rest {
            self.forget_batch_range(h, s, s.saturating_add(l));
        }
        let n = rest.len();
        self.free(rest.into_iter().map(|x| x.0).collect());
        // ★ Review fix 2026-10-10 (finding 5): a batch object whose free was refused earlier is
        // retried here, once more.
        let stuck = self.hold(&self.stuck_objects, |s| (core::mem::take(s), 1));
        self.free(stuck);
        let micro: Vec<(u64, MicroResv)> = self
            .hold(&self.micro, |m| (core::mem::take(m), 1))
            .into_iter()
            .collect();
        for (lo, r) in micro {
            if self.free_micro(r.handle) {
                // Freeing a reservation unmaps everything inside it.
                self.cut_own(lo, r.hi);
            } else {
                // ★ Still OURS: kept tracked ([`BatchedVas::leftovers`]) — the caller frees the
                // whole space instead of recycling it.
                self.hold(&self.micro, |m| {
                    m.insert(lo, r);
                    ((), 1)
                });
            }
        }
        n
    }

    /// Free every micro reservation intersecting `[va, end)` that holds nothing of ours any more.
    ///
    /// ★ Review fix 2026-10-10 (finding 5): a reservation leaves `micro` only when host RM freed it.
    /// ⊘ Before, it was dropped from `micro` BEFORE the free was tried, so a refused free left its
    /// VA reserved for the space's life with nothing tracking it: every later map there found the
    /// VA held (`HeldByHost` → committed HELD → never mapped; `[model]` adversarial seed 7 step 7
    /// call 4), and the retire never retried it. Now a refused free stays tracked: a later map
    /// there goes THROUGH it ([`BatchedVas::segments`]), the next release over it retries the free,
    /// and the retire retries once more and reports it ([`BatchedVas::leftovers`]).
    ///
    /// ★ D2: chunked; only the predecessor can reach into `va` (reservations never overlap), and
    /// the emptiness test is O(log n) ([`OwnMaps::any_in`]).
    fn release_micro(&self, va: u64, end: u64) {
        if end <= va {
            return;
        }
        let (mut from, mut first) = (va, true);
        loop {
            // Chosen and taken out under the `micro` lock (then `own`, always in that order), so
            // only one caller frees each, and none is taken while a map through it is in flight
            // (`pins`).
            let (taken, next) = self.hold2(&self.micro, &self.own, |m, o| {
                let mut cands: Vec<u64> = Vec::new();
                if first
                    && let Some((&lo, r)) = m.range(..va).next_back()
                    && r.hi > va
                {
                    cands.push(lo);
                }
                let (mut n, mut last) = (0usize, 0u64);
                for (&lo, _) in m.range(from..end).take(LEDGER_CHUNK) {
                    cands.push(lo);
                    n += 1;
                    last = lo;
                }
                let touched = cands.len();
                let mut taken: Vec<(u64, MicroResv)> = Vec::new();
                for lo in cands {
                    if let Some(&r) = m.get(&lo)
                        && r.pins == 0
                        && !o.any_in(lo, r.hi)
                    {
                        m.remove(&lo);
                        taken.push((lo, r));
                    }
                }
                let next = (n == LEDGER_CHUNK).then(|| last + 1);
                ((taken, next), touched)
            });
            for (lo, r) in taken {
                if !self.free_micro(r.handle) {
                    self.hold(&self.micro, |m| {
                        m.insert(lo, r);
                        ((), 1)
                    });
                }
            }
            first = false;
            match next {
                Some(n) if n < end => from = n,
                _ => break,
            }
        }
    }

    /// Free one micro reservation; `true` when host RM freed it.
    fn free_micro(&self, h: u32) -> bool {
        match self.vas.free(h) {
            Ok(()) => {
                bump(&self.micro_freed);
                true
            }
            Err(e) => {
                eprintln!(
                    "kf-mem: micro reservation {h:#x} free refused: {e} — kept tracked; retried at the next release over it and at retire"
                );
                false
            }
        }
    }

    /// `[va, va+len)` is no longer mapped by us: clear it in the book ([`LEDGER_CHUNK`] units per
    /// hold), forget the ledger entries of the batch objects that emptied, and free those objects
    /// (host calls, no lock held).
    fn retired(&self, va: u64, len: u64) {
        let end = va.saturating_add(len);
        let mut cur = BookCursor::default();
        let mut emptied: Vec<(u32, u64, u64)> = Vec::new();
        loop {
            let (step, finished) = self.hold(&self.book, |b| {
                let mut step = Vec::new();
                let (t, f) = b.unmapped_step(va, end, &mut cur, LEDGER_CHUNK, &mut step);
                ((step, f), t)
            });
            emptied.extend(step);
            if finished {
                break;
            }
        }
        for &(h, s, l) in &emptied {
            self.forget_batch_range(h, s, s.saturating_add(l));
        }
        self.free(emptied.into_iter().map(|(h, _, _)| h).collect());
    }

    fn free(&self, handles: Vec<u32>) {
        for h in handles {
            match self.vas.free(h) {
                Ok(()) => bump(&self.frees),
                // ⊘ Not fatal to the unmap that emptied it (its mappings ARE gone); the object and
                // its pinned pages live on — named. ★ Review fix 2026-10-10: and TRACKED
                // (`stuck_objects`), retried at retire and reported by [`BatchedVas::leftovers`].
                Err(e) => {
                    eprintln!(
                        "kf-mem: batch object {h:#x} free refused: {e} — kept tracked, retried at retire"
                    );
                    self.hold(&self.stuck_objects, |s| {
                        s.push(h);
                        ((), 1)
                    });
                }
            }
        }
    }
}

/// `[s, e)` as one 4 KiB-grain piece per page (rule 2: the unit nothing can split).
fn push_grains(out: &mut Vec<Seg>, s: u64, e: u64) {
    let mut cur = s;
    while cur < e {
        let next = cur.saturating_add(BATCH_PAGE).min(e);
        out.push((None, cur, next));
        cur = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: u64 = BATCH_PAGE;

    #[test]
    fn a_batch_is_freed_exactly_when_its_last_page_goes() {
        let mut b = BatchBook::default();
        b.insert(0x10_0000, 4 * P, 7).unwrap();
        assert!(
            b.unmapped(0x10_0000 + P, P).is_empty(),
            "one page gone, three live"
        );
        assert!(
            b.unmapped(0x10_0000 + P, P).is_empty(),
            "a repeated unmap does not double-count"
        );
        assert!(b.unmapped(0x10_0000, P).is_empty());
        assert!(b.unmapped(0x10_0000 + 3 * P, P).is_empty());
        assert_eq!(b.unmapped(0x10_0000 + 2 * P, P), vec![7]);
        assert!(b.is_empty());
    }

    #[test]
    fn one_range_can_empty_several_batches_and_spare_its_neighbours() {
        let mut b = BatchBook::default();
        b.insert(0x1000_0000, 2 * P, 1).unwrap();
        b.insert(0x1000_0000 + 2 * P, 3 * P, 2).unwrap();
        b.insert(0x1000_0000 + 5 * P, 2 * P, 3).unwrap();
        let mut got = b.unmapped(0x1000_0000, 5 * P);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
        assert_eq!(b.len(), 1, "the neighbour past the range is untouched");
        // A range that only grazes it (ends at its start) leaves it live.
        assert!(b.unmapped(0x1000_0000 + 4 * P, P).is_empty());
        assert_eq!(b.unmapped(0x1000_0000 + 5 * P, 64 * P), vec![3]);
    }

    #[test]
    fn a_later_batch_over_an_earlier_ones_dead_pages_keeps_both_accounts() {
        let mut b = BatchBook::default();
        b.insert(0x20_0000, 4 * P, 1).unwrap();
        assert!(b.unmapped(0x20_0000 + P, 2 * P).is_empty());
        // Pages 1-2 of batch 1 are dead; batch 2 is placed over them.
        b.insert(0x20_0000 + P, 2 * P, 2).unwrap();
        assert_eq!(
            b.unmapped(0x20_0000 + P, 2 * P),
            vec![2],
            "batch 1's pages there were already dead"
        );
        let mut got = b.unmapped(0x20_0000, 4 * P);
        got.sort_unstable();
        assert_eq!(got, vec![1]);
    }

    #[test]
    fn page_counts_that_are_not_a_multiple_of_64_track_exactly() {
        let mut b = BatchBook::default();
        b.insert(0, 65 * P, 9).unwrap();
        assert!(b.unmapped(0, 64 * P).is_empty(), "page 64 is still live");
        assert_eq!(b.unmapped(64 * P, P), vec![9]);
        b.insert(0, 3 * P, 4).unwrap();
        assert_eq!(b.drain(), vec![4]);
    }

    #[test]
    fn shapes_it_cannot_account_for_are_refused() {
        let mut b = BatchBook::default();
        assert_eq!(b.insert(0x1000, 0, 1), Err(BookRefusal::Shape));
        assert_eq!(b.insert(0x1001, P, 1), Err(BookRefusal::Shape));
        assert_eq!(
            b.insert(u64::MAX - P + 1, 2 * P, 1),
            Err(BookRefusal::Shape)
        );
        b.insert(0x1000, P, 1).unwrap();
        assert_eq!(b.insert(0x1000, P, 1), Err(BookRefusal::Occupied));
        b.insert(0x1000, P, 2).unwrap();
        assert_eq!(b.len(), 2, "two handles may start at one VA");
    }

    /// ★ 2026-10-09: a range plans only OUR bytes — maximal owned spans per `hDma`, never a gap —
    /// and names the mappings it would split.
    #[test]
    fn own_maps_plan_owned_spans_and_name_splits() {
        let mut o = OwnMaps::default();
        o.insert(0x10_0000, 2 * P, None, None);
        o.insert(0x10_0000 + 2 * P, P, Some(7), None);
        o.insert(0x10_0000 + 5 * P, 4 * P, Some(8), Some(0x99));
        let plan = o.plan(0x10_0000 + P, 0x10_0000 + 6 * P);
        assert_eq!(
            plan.spans,
            vec![
                (0x10_0000 + P, 0x10_0000 + 3 * P, None),
                (0x10_0000 + 5 * P, 0x10_0000 + 6 * P, Some(0x99))
            ],
            "the gap [3P, 5P) is never part of a span"
        );
        assert_eq!(
            plan.split,
            vec![
                (0x10_0000, 2 * P, None),
                (0x10_0000 + 5 * P, 4 * P, Some(0x99))
            ]
        );
        assert_eq!(o.plan(0x20_0000, 0x30_0000), OwnPlan::default());
        // Adjacent mappings through different `hDma`s are two spans.
        let mut o = OwnMaps::default();
        o.insert(0x10_0000, P, None, Some(1));
        o.insert(0x10_0000 + P, P, None, None);
        assert_eq!(o.plan(0x10_0000, 0x10_0000 + 2 * P).spans.len(), 2);
    }

    /// ★ The ledger is cut exactly as host RM cuts: inside goes, straddlers keep their outside.
    #[test]
    fn own_maps_cut_like_host_rm() {
        let mut o = OwnMaps::default();
        o.insert(0x10_0000, 8 * P, Some(3), Some(5));
        o.cut(0x10_0000 + 2 * P, 0x10_0000 + 3 * P);
        let m = |len| OwnMap {
            len,
            batch: Some(3),
            via: Some(5),
        };
        assert_eq!(o.containing(0x10_0000 + P), Some((0x10_0000, m(2 * P))));
        assert_eq!(o.containing(0x10_0000 + 2 * P), None);
        assert_eq!(
            o.containing(0x10_0000 + 7 * P),
            Some((0x10_0000 + 3 * P, m(5 * P)))
        );
        o.forget_batch(3);
        assert!(o.is_empty());
    }

    #[test]
    fn covers_answers_for_live_pages_only() {
        let mut b = BatchBook::default();
        b.insert(0x40_0000, 3 * P, 1).unwrap();
        assert!(b.covers(0x40_0000) && b.covers(0x40_0000 + 2 * P + 5));
        assert!(!b.covers(0x40_0000 + 3 * P) && !b.covers(0x40_0000 - 1));
        assert!(b.unmapped(0x40_0000 + P, P).is_empty());
        assert!(!b.covers(0x40_0000 + P), "a dead page is not covered");
    }

    // ─── D2 (2026-10-10): the ledger locks are held for a bounded time ─────────────────────────

    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

    /// A host that accepts every verb and remembers nothing — the ledger code alone is under test,
    /// at sizes the host model (`crate::sim`, linear scans) could not carry.
    struct NullHost {
        next: AtomicU32,
        /// Reservations longer than this are refused (0 = all refused).
        reserve_max: AtomicU64,
        host_calls: AtomicU64,
    }

    impl NullHost {
        fn new(reserve_max: u64) -> Self {
            NullHost {
                next: AtomicU32::new(0x100),
                reserve_max: AtomicU64::new(reserve_max),
                host_calls: AtomicU64::new(0),
            }
        }
        fn call(&self) {
            self.host_calls.fetch_add(1, Relaxed);
        }
    }

    impl SpaceVerbs for &NullHost {
        fn map_row(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
            self.call();
            Ok(Mapped::Placed)
        }
        fn map_sked_row(&self, _: &SkedRow, _: bool) -> Result<Mapped, String> {
            self.call();
            Ok(Mapped::Placed)
        }
        fn map_scattered(
            &self,
            _: std::os::fd::BorrowedFd<'_>,
            _: &[Desired],
            _: bool,
        ) -> Result<u32, String> {
            self.call();
            Ok(self.next.fetch_add(1, Relaxed))
        }
        fn unmap_whole(&self, _: u64, _: bool) -> Result<(), String> {
            self.call();
            Ok(())
        }
        fn unmap_row(&self, _: u64, _: u64, _: bool) -> Result<(), String> {
            self.call();
            Ok(())
        }
        fn unmap_range(&self, _: u64, _: u64, _: bool) -> Result<(), String> {
            self.call();
            Ok(())
        }
        fn free(&self, _: u32) -> Result<(), String> {
            self.call();
            Ok(())
        }
        fn splits_safely(&self, _: u64, _: u64) -> bool {
            false
        }
        fn reserve(&self, va: u64, len: u64) -> Result<u32, String> {
            self.call();
            if len > self.reserve_max.load(Relaxed) {
                return Err(format!("reserve {va:#x}+{len:#x}: refused"));
            }
            Ok(self.next.fetch_add(1, Relaxed))
        }
        fn map_row_in(&self, _: u32, _: &Desired, _: bool) -> Result<Mapped, String> {
            self.call();
            Ok(Mapped::Placed)
        }
        fn map_sked_in(&self, _: u32, _: &SkedRow, _: bool) -> Result<Mapped, String> {
            self.call();
            Ok(Mapped::Placed)
        }
        fn map_scattered_in(
            &self,
            _: u32,
            _: std::os::fd::BorrowedFd<'_>,
            _: &[Desired],
            _: bool,
        ) -> Result<u32, String> {
            self.call();
            Ok(self.next.fetch_add(1, Relaxed))
        }
        fn unmap_in(&self, _: u32, _: u64, _: u64, _: bool) -> Result<(), String> {
            self.call();
            Ok(())
        }
    }

    fn row(va: u64, len: u64, leaf: u64) -> Desired {
        Desired {
            va,
            len,
            off: va,
            ram: true,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf,
        }
    }

    /// The bound every hold must keep: a chunk, plus the one predecessor a range scan may add.
    const BOUND: u64 = LEDGER_CHUNK as u64 + 1;

    /// ★ D2 — **the 2^20-piece row.** The largest row the ledger is ever asked to hold outside a
    /// reservation (`MAX_LEAF_PIECES` 4 KiB grains, 4 GiB — the whole Windows low range) is placed,
    /// read (`own_view`), handed to host RM in part (the act thread's path), unmapped by range and
    /// by run; and no ledger lock hold, on any of those paths, touches more than one chunk.
    #[test]
    fn a_2_pow_20_piece_row_never_holds_a_ledger_lock_beyond_one_chunk() {
        let host = NullHost::new(0); // no reservation: the 4 KiB grain
        let bv = BatchedVas::with_low_reserve(&host, false);
        let (va, len) = (0x1_0000_0000u64, MAX_LEAF_PIECES * BATCH_PAGE);
        let end = va + len;
        let d = row(va, len, BATCH_PAGE);
        assert_eq!(bv.map(&d, true), Ok(Mapped::Placed));
        assert_eq!(bv.own.lock().unwrap().len() as u64, MAX_LEAF_PIECES);
        let v = bv.own_view(va, end);
        assert_eq!(v.owned, vec![(va, end)]);
        assert!(v.rigid.is_empty());
        // The steer: hand a quarter of it to host RM, then the rest.
        assert_eq!(bv.hand_to_host(va + len / 4, len / 4), HandOver::Free);
        assert_eq!(
            bv.own.lock().unwrap().len() as u64,
            MAX_LEAF_PIECES - MAX_LEAF_PIECES / 4
        );
        assert_eq!(bv.hand_to_host(va, len), HandOver::Free);
        assert!(bv.own.lock().unwrap().is_empty());
        // By run (the whole-mapping verb, one entry at a time) and by range.
        assert_eq!(bv.map(&d, true), Ok(Mapped::Placed));
        assert_eq!(bv.unmap_run(va, Some(len), true), Ok(()));
        assert!(bv.own.lock().unwrap().is_empty());
        assert_eq!(bv.map(&d, true), Ok(Mapped::Placed));
        assert_eq!(bv.unmap_range(va, len, true), Ok(()));
        assert!(bv.own.lock().unwrap().is_empty());
        assert_eq!(bv.leftovers(), 0);
        let (touched, hold_us) = bv.hold_stats();
        eprintln!(
            "2^20-piece row: max entries per lock hold = {touched} (chunk {LEDGER_CHUNK}), longest hold = {hold_us} us, {} holds",
            bv.holds.holds.load(Relaxed)
        );
        assert!(touched <= BOUND, "a hold touched {touched} entries");
        assert!(
            touched >= LEDGER_CHUNK as u64 / 2,
            "the chunking was exercised (a hold touched {touched})"
        );
        assert!(bv.holds.holds.load(Relaxed) > MAX_LEAF_PIECES / BOUND);
    }

    /// ★ D1/D2 — a big-leaf row far beyond the grain bound is ONE ledger entry through ONE
    /// reservation; when the host refuses the whole-row reservation, one reservation per leaf
    /// ((a2), every hold still one chunk); when it refuses all, refused by name with nothing left.
    #[test]
    fn a_huge_big_leaf_row_is_reserved_whole_per_leaf_or_refused_by_name() {
        let (va, leaf) = (0x1_0000_0000u64, 0x20_0000u64);
        let len = 2 * MAX_LEAF_PIECES * BATCH_PAGE; // 8 GiB: 2^21 grains, 4096 leaves
        let end = va + len;
        // (a) accepted.
        let host = NullHost::new(u64::MAX);
        let bv = BatchedVas::with_low_reserve(&host, true);
        assert_eq!(bv.map(&row(va, len, leaf), true), Ok(Mapped::Placed));
        assert_eq!(
            (bv.own.lock().unwrap().len(), bv.micro.lock().unwrap().len()),
            (1, 1)
        );
        assert_eq!(
            bv.unmap_range(va + leaf, leaf, true),
            Ok(()),
            "exact, inside"
        );
        assert_eq!(bv.hand_to_host(va, len), HandOver::Free);
        assert_eq!((bv.leftovers(), bv.leaf_reserved.load(Relaxed)), (0, 1));
        // (a2) the whole row refused, one leaf accepted.
        let host = NullHost::new(leaf);
        let bv = BatchedVas::with_low_reserve(&host, true);
        assert_eq!(bv.map(&row(va, len, leaf), true), Ok(Mapped::Placed));
        assert_eq!(bv.micro.lock().unwrap().len() as u64, len / leaf);
        assert_eq!(bv.own.lock().unwrap().len() as u64, len / leaf);
        assert_eq!(bv.own_view(va, end).owned, vec![(va, end)]);
        assert_eq!(bv.hand_to_host(va, len), HandOver::Free);
        assert_eq!(bv.leftovers(), 0);
        assert!(bv.hold_stats().0 <= BOUND, "{:?}", bv.hold_stats());
        // Everything refused: by name, nothing left, pins released.
        let host = NullHost::new(0);
        let bv = BatchedVas::with_low_reserve(&host, true);
        let e = bv.map(&row(va, len, leaf), true).unwrap_err();
        assert!(e.contains(HUGE_ROW_OUTSIDE_RESERVATION), "{e}");
        assert_eq!(bv.leftovers(), 0);
        assert_eq!(bv.huge_refused.load(Relaxed), 1);
    }

    /// ★ D1 (a2) — a host that refuses some leaf-sized reservations too: those leaves go at 4 KiB
    /// grain within the same grain budget; past it the row is refused by name and nothing is left.
    #[test]
    fn per_leaf_reservations_that_are_refused_spend_the_grain_budget() {
        struct Flaky(NullHost, AtomicU32);
        impl SpaceVerbs for &Flaky {
            fn map_row(&self, d: &Desired, f: bool) -> Result<Mapped, String> {
                (&self.0).map_row(d, f)
            }
            fn map_sked_row(&self, s: &SkedRow, f: bool) -> Result<Mapped, String> {
                (&self.0).map_sked_row(s, f)
            }
            fn map_scattered(
                &self,
                fd: std::os::fd::BorrowedFd<'_>,
                r: &[Desired],
                f: bool,
            ) -> Result<u32, String> {
                (&self.0).map_scattered(fd, r, f)
            }
            fn unmap_whole(&self, v: u64, f: bool) -> Result<(), String> {
                (&self.0).unmap_whole(v, f)
            }
            fn unmap_row(&self, v: u64, l: u64, f: bool) -> Result<(), String> {
                (&self.0).unmap_row(v, l, f)
            }
            fn unmap_range(&self, v: u64, l: u64, f: bool) -> Result<(), String> {
                (&self.0).unmap_range(v, l, f)
            }
            fn free(&self, h: u32) -> Result<(), String> {
                (&self.0).free(h)
            }
            fn splits_safely(&self, v: u64, l: u64) -> bool {
                (&self.0).splits_safely(v, l)
            }
            // Whole rows refused; every 3rd leaf-sized one refused too.
            fn reserve(&self, v: u64, l: u64) -> Result<u32, String> {
                if self.1.fetch_add(1, Relaxed).is_multiple_of(3) {
                    return Err(format!("reserve {v:#x}+{l:#x}: refused (flaky)"));
                }
                (&self.0).reserve(v, l)
            }
            fn map_row_in(&self, h: u32, d: &Desired, f: bool) -> Result<Mapped, String> {
                (&self.0).map_row_in(h, d, f)
            }
            fn map_sked_in(&self, h: u32, s: &SkedRow, f: bool) -> Result<Mapped, String> {
                (&self.0).map_sked_in(h, s, f)
            }
            fn map_scattered_in(
                &self,
                h: u32,
                fd: std::os::fd::BorrowedFd<'_>,
                r: &[Desired],
                f: bool,
            ) -> Result<u32, String> {
                (&self.0).map_scattered_in(h, fd, r, f)
            }
            fn unmap_in(&self, h: u32, v: u64, s: u64, f: bool) -> Result<(), String> {
                (&self.0).unmap_in(h, v, s, f)
            }
        }
        let (va, leaf) = (0x1_0000_0000u64, 0x1_0000u64);
        let len = 96 * leaf; // 96 leaves = 1536 grains
        let flaky = Flaky(NullHost::new(leaf), AtomicU32::new(0));
        let mut bv = BatchedVas::with_low_reserve(&flaky, true);
        bv.max_leaf_pieces = 1024; // the whole row (1536 grains) is over the bound
        // The whole-row reservation is refused (len > leaf), then 1 leaf in 3 is refused: 32 leaves
        // = 512 grains, within the budget of 1024.
        assert_eq!(bv.map(&row(va, len, leaf), true), Ok(Mapped::Placed));
        let reserved = bv.micro.lock().unwrap().len() as u64;
        let grained = bv.own.lock().unwrap().len() as u64 - reserved;
        assert!(
            reserved > 50 && grained > 400,
            "{reserved} reserved, {grained} grains"
        );
        assert_eq!(bv.hand_to_host(va, len), HandOver::Free);
        assert_eq!(bv.leftovers(), 0);
        // A budget of 256 grains cannot take 512: refused by name, nothing left behind.
        let flaky = Flaky(NullHost::new(leaf), AtomicU32::new(0));
        let mut bv = BatchedVas::with_low_reserve(&flaky, true);
        bv.max_leaf_pieces = 256;
        let e = bv.map(&row(va, len, leaf), true).unwrap_err();
        assert!(e.contains(HUGE_ROW_OUTSIDE_RESERVATION), "{e}");
        assert_eq!(
            bv.leftovers(),
            0,
            "the reservations made on the way are released"
        );
        assert!(bv.micro.lock().unwrap().values().all(|r| r.pins == 0));
    }

    /// ★ D2 — many reservations in one row's range: placement over all of them, hand-over and
    /// release walk them in chunks (the lock is dropped between), pins are all released, and no
    /// hold exceeds one chunk.
    #[test]
    fn thousands_of_micro_reservations_are_walked_in_bounded_holds() {
        let host = NullHost::new(u64::MAX);
        let bv = BatchedVas::with_low_reserve(&host, true);
        let n = 3 * LEDGER_CHUNK as u64 + 7;
        let (base, leaf) = (0x2_0000_0000u64, 0x1_0000u64);
        // One reserved 64 KiB leaf every other 64 KiB.
        for i in 0..n {
            let r = row(base + 2 * i * leaf, leaf, leaf);
            assert_eq!(bv.map(&r, true), Ok(Mapped::Placed));
        }
        assert_eq!(bv.micro.lock().unwrap().len() as u64, n);
        // One 4 KiB-leaf row over the lot: routed through every reservation, grains in the gaps.
        let span = 2 * n * leaf;
        // (The gaps are free, the reserved leaves are ours: mapping a row over our own mappings
        // would be a stray; unmap them first through the steer, which must leave the
        // reservations in place.)
        assert_eq!(bv.hand_to_host(base, span), HandOver::Free);
        assert!(bv.own.lock().unwrap().is_empty());
        assert!(
            bv.micro.lock().unwrap().is_empty(),
            "emptied reservations released"
        );
        assert_eq!(bv.leftovers(), 0);
        // Again, this time a row over live reservations (pinned while it maps, then released).
        for i in 0..n {
            let r = row(base + 2 * i * leaf, leaf, leaf);
            assert_eq!(bv.map(&r, true), Ok(Mapped::Placed));
        }
        let segs = bv.segments(base, base + span);
        assert_eq!(segs.iter().filter(|x| x.0.is_some()).count() as u64, n);
        assert!(bv.micro.lock().unwrap().values().all(|r| r.pins == 1));
        bv.unpin(&segs);
        assert!(bv.micro.lock().unwrap().values().all(|r| r.pins == 0));
        assert!(bv.micro_covers(base, base + span));
        assert_eq!(bv.unmap_range(base, span, true), Ok(()));
        assert!(!bv.micro_covers(base, base + span));
        assert_eq!(bv.leftovers(), 0);
        assert!(bv.hold_stats().0 <= BOUND, "{:?}", bv.hold_stats());
    }

    /// ★ D2 — the chunked book step is the unbounded one: on random batches and unmaps, stepping
    /// with a tiny budget empties exactly the same objects at the same time and leaves the same
    /// pages live; no step touches more than its budget.
    #[test]
    fn the_book_step_equals_the_unbounded_clear_at_any_budget() {
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for round in 0..200u32 {
            let (mut a, mut b) = (BatchBook::default(), BatchBook::default());
            let n = 1 + next() % 12;
            for h in 0..n {
                let (va, len) = ((next() % 64) * P, (1 + next() % 200) * P);
                let _ = a.insert(va, len, h as u32);
                let _ = b.insert(va, len, h as u32);
            }
            for _ in 0..8 {
                let (va, len) = ((next() % 80) * P, (1 + next() % 120) * P);
                let want = a.unmapped_extents(va, len);
                let budget = 1 + (next() % 5) as usize;
                let mut cur = BookCursor::default();
                let mut got = Vec::new();
                loop {
                    let mut step = Vec::new();
                    let (t, done) =
                        b.unmapped_step(va, va.saturating_add(len), &mut cur, budget, &mut step);
                    assert!(t <= budget + 1, "round {round}: {t} > {budget} + 1");
                    got.extend(step);
                    if done {
                        break;
                    }
                }
                let (mut w, mut g) = (want, got);
                w.sort_unstable();
                g.sort_unstable();
                assert_eq!(w, g, "round {round}");
                assert_eq!(a, b, "round {round}: same live pages afterwards");
            }
        }
    }

    /// ★ D2 — chunked ledger cuts equal the unbounded cut, and a chunk always makes progress.
    #[test]
    fn the_ledger_cut_in_chunks_equals_the_unbounded_cut() {
        let mut rng = 0xC0FF_EE11_u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for round in 0..200u32 {
            let (mut a, mut b) = (OwnMaps::default(), OwnMaps::default());
            let mut at = 0u64;
            for _ in 0..(1 + next() % 40) {
                at += (next() % 3) * P;
                let len = (1 + next() % 4) * P;
                let (batch, via) = (Some((next() % 3) as u32), Some((next() % 2) as u32));
                a.insert(at, len, batch, via);
                b.insert(at, len, batch, via);
                at += len;
            }
            let (va, end) = ((next() % 30) * P, (30 + next() % 100) * P);
            a.cut(va, end);
            let limit = 1 + (next() % 4) as usize;
            let mut guard = 0;
            loop {
                let (n, done) = b.cut_chunk(va, end, limit);
                assert!(n <= limit);
                if done {
                    break;
                }
                guard += 1;
                assert!(guard < 1000, "round {round}: no progress");
            }
            assert_eq!(a, b, "round {round}");
            assert!(!b.any_in(va, end));
        }
    }
}
