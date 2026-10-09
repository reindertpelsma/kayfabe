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

impl BatchBook {
    /// Record a batch object `handle` mapped at `[va, va+len)`, every page live.
    ///
    /// # Errors
    /// [`BookRefusal`] — the caller must then free `handle` itself (it cannot be tracked).
    pub fn insert(&mut self, va: u64, len: u64, handle: u32) -> Result<(), BookRefusal> {
        if len == 0 || !(va | len).is_multiple_of(BATCH_PAGE) || va.checked_add(len).is_none() {
            return Err(BookRefusal::Shape);
        }
        if self.by_va.contains_key(&(va, handle)) {
            return Err(BookRefusal::Occupied);
        }
        let pages = len / BATCH_PAGE;
        let words = usize::try_from(pages.div_ceil(64)).map_err(|_| BookRefusal::Shape)?;
        let mut live = vec![u64::MAX; words];
        if !pages.is_multiple_of(64)
            && let Some(last) = live.last_mut()
        {
            *last = (1u64 << (pages % 64)) - 1;
        }
        self.by_va.insert(
            (va, handle),
            Entry {
                handle,
                len,
                live,
                live_pages: pages,
            },
        );
        self.max_len = self.max_len.max(len);
        Ok(())
    }

    /// `[va, va+len)` is no longer mapped by us: clear those pages in every batch they touch, and
    /// return (and forget) the handles with no live page left — the caller frees them.
    #[must_use]
    pub fn unmapped(&mut self, va: u64, len: u64) -> Vec<u32> {
        let end = va.saturating_add(len);
        let floor = va.saturating_sub(self.max_len);
        let mut emptied = Vec::new();
        for (&(start, h), e) in self.by_va.range_mut((floor, 0)..(end, 0)) {
            let e_end = start + e.len;
            if e_end <= va {
                continue;
            }
            let lo = (va.max(start) - start) / BATCH_PAGE;
            let hi = (end.min(e_end) - start).div_ceil(BATCH_PAGE);
            for p in lo..hi {
                let (w, b) = ((p / 64) as usize, p % 64);
                if e.live[w] & (1 << b) != 0 {
                    e.live[w] &= !(1 << b);
                    e.live_pages -= 1;
                }
            }
            if e.live_pages == 0 {
                emptied.push((start, h));
            }
        }
        emptied
            .into_iter()
            .filter_map(|s| self.by_va.remove(&s))
            .map(|e| e.handle)
            .collect()
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
        self.max_len = 0;
        core::mem::take(&mut self.by_va)
            .into_values()
            .map(|e| e.handle)
            .collect()
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

    /// Our mappings intersecting `[va, end)`, in VA order.
    #[must_use]
    pub fn within(&self, va: u64, end: u64) -> Vec<(u64, OwnMap)> {
        let first = self.containing(va).map_or(va, |(s, _)| s);
        self.by_va
            .range(first..end)
            .map(|(&s, &m)| (s, m))
            .filter(|&(s, m)| s.saturating_add(m.len) > va)
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

    /// Forget every mapping of batch object `h` (freeing it unmapped them).
    pub fn forget_batch(&mut self, h: u32) {
        self.by_va.retain(|_, m| m.batch != Some(h));
    }

    /// Whether any mapping of ours intersects `[va, end)`.
    #[must_use]
    pub fn any_in(&self, va: u64, end: u64) -> bool {
        !self.within(va, end).is_empty()
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

/// ★ 2026-10-09: the most leaf mappings ONE row is split into outside a reservation (a 512 MiB
/// guest row of 4 KiB leaves) — a bound on host calls per row, never sized by an unchecked guest
/// value. A longer row stays one mapping (its later partial change is then refused by name,
/// [`SPLIT_OUTSIDE_RESERVATION`], never split).
pub const MAX_LEAF_PIECES: u64 = 1 << 17;

/// ★★★ **A host VA space that places batches and keeps their objects' books** — the one
/// implementation production (`kf_qemu::mem::GpuMirror`) and the hardware gate share.
///
/// Every verb here is one WE author on OUR host space (§9); no lock is ever held across a host
/// call (the VA thread is the only caller; plan under the lock, call, then record).
///
/// ★★★ **STATUS (2026-10-09): three rules, from host RM's own source, enforced HERE so no caller
/// can break them** (`V3_BATCHED_MAP.md` §8):
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
///    `KF3_NO_BATCHED_MAP=1` (run 244). A batch therefore lives inside a VA-RESERVING `hDma`
///    (`NV50_MEMORY_VIRTUAL`, whose unmap invalidates exactly the unmapped PTEs,
///    `virt_mem_allocator_gm107.c:1578-1633`): a guest reservation, or — outside them, with
///    [`BatchedVas::low_reserve`] — a MICRO reservation made over exactly the batch's VA.
/// 3. **No remap, ever.** A VA whose guest mapping did not change is never transiently unmapped
///    (owner, 2026-10-09): partial unmaps are made exact instead.
#[derive(Debug)]
pub struct BatchedVas<'rm, V: SpaceVerbs = HostVas<'rm>> {
    /// The host space.
    pub vas: V,
    /// Our batch objects in it.
    pub book: std::sync::Mutex<BatchBook>,
    /// ★ Every host mapping of ours in it ([`OwnMaps`]).
    pub own: std::sync::Mutex<OwnMaps>,
    /// ★ Our micro reservations, `lo → (hi, handle)` (rule 2).
    pub micro: std::sync::Mutex<BTreeMap<u64, (u64, u32)>>,
    /// ★ Batch outside the guest reservations through micro reservations (rule 2). Off: rows
    /// there go per run.
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
    /// (the caller then unmaps run by run) — expected 0; counted, never silent.
    pub unsafe_splits: std::sync::atomic::AtomicU64,
    _rm: std::marker::PhantomData<&'rm ()>,
}

/// ★ Why [`BatchedVas::unmap_range`] refused before any host call.
pub const SPLIT_OUTSIDE_RESERVATION: &str = "range would split one of our mappings outside a VA-reserving hDma (host RM would free its whole VA block) — refused, unmap run by run";

fn bump(c: &std::sync::atomic::AtomicU64) {
    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

impl<V: SpaceVerbs> BatchedVas<'_, V> {
    /// Batches over `vas`, none yet; outside the guest reservations rows go per run.
    #[must_use]
    pub fn new(vas: V) -> Self {
        Self::with_low_reserve(vas, false)
    }

    /// Batches over `vas`; `low_reserve` batches outside the guest reservations through micro
    /// reservations (rule 2).
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
            _rm: std::marker::PhantomData,
        }
    }

    /// `[va, end)` cut at our micro reservations' edges: `(via, start, end)` in VA order.
    fn segments(&self, va: u64, end: u64) -> Vec<(Option<u32>, u64, u64)> {
        let micro = self.micro.lock().map(|m| m.clone()).unwrap_or_default();
        let mut out = Vec::new();
        let mut cur = va;
        let covering = micro
            .range(..end)
            .filter(|&(_, &(hi, _))| hi > va)
            .map(|(&lo, &(hi, h))| (lo, hi, h));
        for (lo, hi, h) in covering {
            if lo > cur {
                out.push((None, cur, lo.min(end)));
            }
            let (s, e) = (lo.max(cur), hi.min(end));
            if s < e {
                out.push((Some(h), s, e));
                cur = e;
            }
        }
        if cur < end {
            out.push((None, cur, end));
        }
        out
    }

    /// ★★★ 2026-10-09 — **[`BatchedVas::segments`], then one segment per guest LEAF wherever the
    /// mapping would go through the space's `NV01` range** (no reservation of any kind over it).
    /// There each map owns its own VA block and only a whole-mapping unmap is exact (rule 2), and a
    /// guest can change any single leaf on its own (its PTEs are not coalesced), so the host
    /// mapping unit is the leaf: `leaf` bytes when the segment is whole aligned leaves of it, else
    /// the 4 KiB grain. Inside a reservation a coalesced row stays one mapping (partial unmaps are
    /// exact there). Bounded: a hostile row is at most [`MAX_LEAF_PIECES`] pieces, else one piece
    /// per segment is refused by the caller's own bounds (see `crate::apply`'s whole-page check).
    fn leaf_segments(&self, va: u64, end: u64, leaf: u64) -> Vec<(Option<u32>, u64, u64)> {
        let mut out = Vec::new();
        for (via, s, e) in self.segments(va, end) {
            if via.is_some() || self.vas.splits_safely(s, e - s) {
                out.push((via, s, e));
                continue;
            }
            let l = if leaf.is_power_of_two() && leaf >= BATCH_PAGE && (s | e).is_multiple_of(leaf)
            {
                leaf
            } else {
                BATCH_PAGE
            };
            if (e - s) / l > MAX_LEAF_PIECES {
                out.push((None, s, e));
                continue;
            }
            let mut cur = s;
            while cur < e {
                let next = cur.saturating_add(l).min(e);
                out.push((None, cur, next));
                cur = next;
            }
        }
        out
    }

    fn record(&self, va: u64, len: u64, batch: Option<u32>, via: Option<u32>) {
        if let Ok(mut o) = self.own.lock() {
            o.insert(va, len, batch, via);
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

    /// Map `[va, end)` piece by piece (one host map per micro-reservation segment), all or
    /// nothing; records each placed piece. `one(via, start, len)` performs one piece.
    fn map_segments(
        &self,
        va: u64,
        end: u64,
        leaf: u64,
        defer: bool,
        one: &dyn Fn(Option<u32>, u64, u64) -> Result<Mapped, String>,
    ) -> Result<Mapped, String> {
        let segs = self.leaf_segments(va, end, leaf);
        let mut placed: Vec<(Option<u32>, u64, u64)> = Vec::new();
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
            for &(via, s, e) in placed.iter().rev() {
                let m = OwnMap {
                    len: e - s,
                    batch: None,
                    via,
                };
                if let Err(r) = self.unmap_entry(s, m, defer) {
                    eprintln!(
                        "kf-mem: map {va:#x}+{:#x}: rolling back piece {s:#x} refused ({r})",
                        end - va
                    );
                }
            }
            return verdict;
        }
        for (via, s, e) in placed {
            self.record(s, e - s, None, via);
        }
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
    /// [`crate::ledger::NOT_BATCHED`] — the per-run path, silently (not a fallback).
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
        let segs = self.segments(va, end);
        let (handle, via) = match segs.as_slice() {
            [(None, ..)] if self.vas.splits_safely(va, len) => {
                (self.vas.map_scattered(ram_fd, rows, defer)?, None)
            }
            [(Some(h), ..)] => (
                self.vas.map_scattered_in(*h, ram_fd, rows, defer)?,
                Some(*h),
            ),
            [(None, ..)] if self.low_reserve && rows.len() >= LOW_RANGE_MIN_RUNS => {
                let h = match self.vas.reserve(va, len) {
                    Ok(h) => h,
                    Err(e) => {
                        if self
                            .micro_refused
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            < 8
                        {
                            eprintln!(
                                "kf-mem: micro reservation {va:#x}+{len:#x} refused ({e}) — the rows go per run"
                            );
                        }
                        return Err(crate::ledger::NOT_BATCHED.into());
                    }
                };
                bump(&self.micro_made);
                if let Ok(mut m) = self.micro.lock() {
                    m.insert(va, (end, h));
                }
                match self.vas.map_scattered_in(h, ram_fd, rows, defer) {
                    Ok(obj) => (obj, Some(h)),
                    Err(e) => {
                        self.release_micro(va, end);
                        return Err(e);
                    }
                }
            }
            _ => return Err(crate::ledger::NOT_BATCHED.into()),
        };
        let booked = self
            .book
            .lock()
            .map_err(|_| "batch book poisoned".to_string())
            .and_then(|mut b| b.insert(va, len, handle).map_err(|e| format!("{e:?}")));
        if let Err(e) = booked {
            self.free(vec![handle]);
            self.release_micro(va, end);
            return Err(format!(
                "batch {va:#x}+{len:#x}: its object could not be booked ({e}) — freed, nothing placed"
            ));
        }
        self.record(va, len, Some(handle), via);
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
        let end = va
            .checked_add(len)
            .ok_or_else(|| format!("unmap range {va:#x}+{len:#x} wraps"))?;
        let plan = self
            .own
            .lock()
            .map_err(|_| "own-mapping ledger poisoned".to_string())?
            .plan(va, end);
        if let Some(&(s, l, _)) = plan
            .split
            .iter()
            .find(|&&(s, l, via)| via.is_none() && !self.vas.splits_safely(s, l))
        {
            bump(&self.unsafe_splits);
            return Err(format!(
                "unmap range {va:#x}+{len:#x}: our mapping {s:#x}+{l:#x}: {SPLIT_OUTSIDE_RESERVATION}"
            ));
        }
        for (s, e, via) in plan.spans {
            match via {
                Some(h) => self.vas.unmap_in(h, s, e - s, defer)?,
                None => self.vas.unmap_range(s, e - s, defer)?,
            }
            if let Ok(mut o) = self.own.lock() {
                o.cut(s, e);
            }
            self.retired(s, e - s);
        }
        self.release_micro(va, end);
        Ok(())
    }

    /// ★ Unmap ONE committed run at `va` (`len` when known). A run that is exactly our host
    /// mapping(s) — one, or one per `hDma` piece — goes by the whole-mapping verb; a PIECE of a
    /// larger mapping of ours (a batch) goes by range ([`BatchedVas::unmap_range`]); nothing of
    /// ours at `va` is answered with no host call.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unmap_run(&self, va: u64, len: Option<u64>, defer: bool) -> Result<(), String> {
        let hit = self.own.lock().ok().and_then(|o| o.containing(va));
        let Some((start, first)) = hit else {
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 16 {
                eprintln!(
                    "kf-mem: unmap {va:#x}: no host mapping of ours there — no host call (nothing of ours to take down)"
                );
            }
            return Ok(());
        };
        let end = va.saturating_add(len.unwrap_or(first.len));
        let entries = self
            .own
            .lock()
            .map(|o| o.within(va, end))
            .unwrap_or_default();
        // Whole ⇔ the entries tile [va, end) exactly, each starting and ending inside it.
        let mut cur = va;
        let whole = start == va
            && entries.iter().all(|&(s, m)| {
                let ok = s == cur && s.saturating_add(m.len) <= end;
                cur = s.saturating_add(m.len);
                ok
            })
            && cur == end;
        if whole {
            for (s, m) in entries {
                self.unmap_entry(s, m, defer)?;
                if let Ok(mut o) = self.own.lock() {
                    o.cut(s, s.saturating_add(m.len));
                }
                self.retired(s, m.len);
            }
            self.release_micro(va, end);
            return Ok(());
        }
        match len {
            Some(l) => self.unmap_range(va, l, defer),
            None => Err(format!(
                "unmap {va:#x}: a piece of a larger mapping of ours with no known length — refused (a whole-mapping unmap would take the rest)"
            )),
        }
    }

    /// Free every batch object and micro reservation left (the space is being retired). Returns
    /// how many batch objects.
    pub fn drain(&self) -> usize {
        let rest = self.book.lock().map(|mut b| b.drain()).unwrap_or_default();
        if let Ok(mut o) = self.own.lock() {
            for &h in &rest {
                o.forget_batch(h);
            }
        }
        let n = rest.len();
        self.free(rest);
        let micro: Vec<(u64, u64, u32)> = self
            .micro
            .lock()
            .map(|mut m| {
                core::mem::take(&mut *m)
                    .into_iter()
                    .map(|(lo, (hi, h))| (lo, hi, h))
                    .collect()
            })
            .unwrap_or_default();
        for (lo, hi, h) in micro {
            if let Ok(mut o) = self.own.lock() {
                o.cut(lo, hi);
            }
            self.free_micro(h);
        }
        n
    }

    /// Free every micro reservation intersecting `[va, end)` that holds nothing of ours any more.
    fn release_micro(&self, va: u64, end: u64) {
        let candidates: Vec<(u64, u64, u32)> = self
            .micro
            .lock()
            .map(|m| {
                m.range(..end)
                    .filter(|&(_, &(hi, _))| hi > va)
                    .map(|(&lo, &(hi, h))| (lo, hi, h))
                    .collect()
            })
            .unwrap_or_default();
        for (lo, hi, h) in candidates {
            let empty = self.own.lock().is_ok_and(|o| !o.any_in(lo, hi));
            if empty {
                if let Ok(mut m) = self.micro.lock() {
                    m.remove(&lo);
                }
                self.free_micro(h);
            }
        }
    }

    fn free_micro(&self, h: u32) {
        match self.vas.free(h) {
            Ok(()) => bump(&self.micro_freed),
            Err(e) => eprintln!(
                "kf-mem: micro reservation {h:#x} free refused: {e} — its VA stays reserved until the space is freed"
            ),
        }
    }

    fn retired(&self, va: u64, len: u64) {
        let emptied = self
            .book
            .lock()
            .map(|mut b| b.unmapped(va, len))
            .unwrap_or_default();
        if let Ok(mut o) = self.own.lock() {
            for &h in &emptied {
                o.forget_batch(h);
            }
        }
        self.free(emptied);
    }

    fn free(&self, handles: Vec<u32>) {
        for h in handles {
            match self.vas.free(h) {
                Ok(()) => bump(&self.frees),
                // ⊘ Not fatal to the unmap that emptied it (its mappings ARE gone); the object and
                // its pinned pages live until the host client closes — named.
                Err(e) => eprintln!(
                    "kf-mem: batch object {h:#x} free refused: {e} — its pages stay pinned until the host client closes"
                ),
            }
        }
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
}
