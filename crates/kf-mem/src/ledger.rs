//! ★★★ **The ledger of OUR OWN host mappings, and the planner that diffs the walk against it.**
//!
//! v3 §4.2 (w825 box): *"Diff the guest's live tables against our ledger. Nothing of the guest's is
//! copied; the only previous state is a record of our own actions."* The walk says what the guest's
//! tables say NOW; [`Ledger`] is what WE have mapped; [`plan_reconcile`] is the difference; the apply
//! is deferred maps/unmaps and ONE TLB invalidate.
//!
//! The planners are copied from the old tree's `storemap.rs` (pure, tested there); the ledger is v3's.

/// One mapping a walk says the guest's tables currently express, as a slice of one of the two
/// ground truths: the reserved store (`ram == false`, `off` = GPGA offset) or the guest-RAM
/// object (`ram == true`, `off` = memfd file offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Desired {
    /// Guest VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// Offset into the object named by `ram`.
    pub off: u64,
    /// Which ground truth.
    pub ram: bool,
}

/// A walked leaf's aperture, as the walk kernel reports it (`KFWR_RF_AP_*`, `cuda/walk/kf_walk.h:92-97`).
pub const AP_VIDMEM: u8 = 0;
/// Peer memory — meaningless for a single-GPU guest; the walker refuses it too.
pub const AP_PEER: u8 = 1;
/// System memory, coherent.
pub const AP_SYS_COHERENT: u8 = 2;
/// System memory, non-coherent.
pub const AP_SYS_NONCOHERENT: u8 = 3;

/// Why a walked leaf could not become a [`Desired`] row. Refused by name, never clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafRefusal {
    /// A vidmem leaf outside the store (the walker bounds these too; this is the host's copy).
    OutsideStore {
        /// Guest VA.
        va: u64,
        /// GPGA offset.
        gpga: u64,
        /// Bytes.
        len: u64,
    },
    /// A sysmem leaf naming guest-physical memory the VMM's layout does not back contiguously.
    NotGuestRam {
        /// Guest VA.
        va: u64,
        /// Guest-physical address.
        gpa: u64,
        /// Bytes.
        len: u64,
    },
    /// Peer or an unknown aperture.
    Aperture {
        /// Guest VA.
        va: u64,
        /// The code.
        ap: u8,
    },
}

/// ★ **Classify walked leaves by APERTURE** into rows of the two ground truths. A vidmem leaf is a
/// slice of the store (`off` = GPGA); a sysmem leaf is a slice of the guest-RAM object, at the
/// memfd offset `ram_offset(gpa, len)` gives — the VMM's own guest-physical layout, which is NOT
/// the identity once there is a PCI hole. ⊘ The walker deliberately leaves sysmem leaves unbounded
/// (w825: it cannot know the layout); THIS is the bound.
///
/// # Errors
/// The first leaf refused, by name.
pub fn desired_from_leaves(
    leaves: impl IntoIterator<Item = (u64, u64, u64, u8)>,
    store_bytes: u64,
    ram_offset: &dyn Fn(u64, u64) -> Option<u64>,
) -> Result<Vec<Desired>, LeafRefusal> {
    leaves
        .into_iter()
        .map(|(va, at, len, ap)| match ap {
            AP_VIDMEM => at
                .checked_add(len)
                .filter(|&e| e <= store_bytes)
                .map(|_| Desired { va, len, off: at, ram: false })
                .ok_or(LeafRefusal::OutsideStore { va, gpga: at, len }),
            AP_SYS_COHERENT | AP_SYS_NONCOHERENT => ram_offset(at, len)
                .map(|off| Desired { va, len, off, ram: true })
                .ok_or(LeafRefusal::NotGuestRam { va, gpa: at, len }),
            _ => Err(LeafRefusal::Aperture { va, ap }),
        })
        .collect()
}

/// Sort and merge half-open `[a, b)` ranges; touching ranges merge; empty ones are dropped.
#[must_use]
pub fn merge_ranges(mut r: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    r.retain(|(a, b)| b > a);
    r.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(r.len());
    for (a, b) in r {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// What [`plan_reconcile`] asks for. Unmaps are applied FIRST.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// `(va, len)` of ledger slices to take down.
    pub unmap: Vec<(u64, u64)>,
    /// Desired runs to map.
    pub map: Vec<Desired>,
    /// Ledger slices left exactly as they are.
    pub kept: usize,
}

/// Whether `[start, start+len)` mapped to `[off, off+len)` is fully covered by `rows`
/// (sorted `(va, file_offset, len)`), each row agreeing on the offset. See
/// [`StoreMapPort::ram_stale`].
#[must_use]
pub fn ram_slice_backed(start: u64, off: u64, len: u64, rows: &[(u64, u64, u64)]) -> bool {
    backed_from(start, off, len, rows, first_reaching(rows, start, ends_sorted(rows)))
}

/// ★ P5c: whether the rows' ENDS ascend with their starts — true of every disjoint sorted set (a
/// walk's leaves, our ledger's rows). Only then may a search skip the rows that end at or before a
/// point; otherwise the scan starts at 0, exactly as before.
fn ends_sorted(rows: &[(u64, u64, u64)]) -> bool {
    rows.windows(2).all(|w| w[0].0.saturating_add(w[0].2) <= w[1].0.saturating_add(w[1].2))
}

/// ★ P5c: the first row that ends AFTER `start` (every row before it would be skipped by the scan
/// anyway) — `O(log n)` where the linear scan was `O(n)`, which made the reconcile `O(n²)` in the
/// space's rows: `[measured c2, 9a3e499d]` `plan_avg_us` 3 108 at ~2 400 rows, and the raw client's
/// `--ce-client-guest-ram` declares 13 000.
fn first_reaching(rows: &[(u64, u64, u64)], start: u64, sorted_ends: bool) -> usize {
    if sorted_ends { rows.partition_point(|&(va, _, rlen)| va.saturating_add(rlen) <= start) } else { 0 }
}

/// [`ram_slice_backed`]'s scan, from row `from` on.
fn backed_from(start: u64, off: u64, len: u64, rows: &[(u64, u64, u64)], from: usize) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let mut at = start;
    for &(va, foff, rlen) in &rows[from.min(rows.len())..] {
        if at >= end {
            break;
        }
        let Some(rend) = va.checked_add(rlen) else {
            return false;
        };
        if rend <= at {
            continue;
        }
        if va > at {
            return false;
        }
        let delta = at - va;
        if foff.checked_add(delta) != off.checked_add(at - start) {
            return false;
        }
        at = rend.min(end);
    }
    at >= end
}

/// ★★★★★ **The pure half of the reconcile — no GPU, fully testable.**
///
/// `ledger` is every slice this port holds in one VA space, `(va, len, off, ram)`; `desired`
/// is the COMPLETE state a walk reported for that space. A ledger slice is kept iff desired
/// runs of the SAME ground truth back every byte of it at the same offsets; every other slice
/// is unmapped. A desired run already backed by kept slices is left alone; otherwise every kept
/// slice that overlaps it is unmapped too and the run is mapped whole — a FIXED map over a live
/// slice is refused by RM, so overlap is resolved here, not discovered there.
#[must_use]
pub fn plan_reconcile(ledger: &[(u64, u64, u64, bool)], desired: &[Desired]) -> ReconcilePlan {
    let rows = |ram: bool| -> Vec<(u64, u64, u64)> {
        let mut v: Vec<(u64, u64, u64)> = desired
            .iter()
            .filter(|d| d.ram == ram)
            .map(|d| (d.va, d.off, d.len))
            .collect();
        v.sort_unstable();
        v
    };
    let (want_store, want_ram) = (rows(false), rows(true));
    let (ws_sorted, wr_sorted) = (ends_sorted(&want_store), ends_sorted(&want_ram));
    let mut plan = ReconcilePlan::default();
    let mut kept: Vec<(u64, u64, u64, bool)> = Vec::new();
    for &(va, len, off, ram) in ledger {
        let (want, sorted) = if ram { (&want_ram, wr_sorted) } else { (&want_store, ws_sorted) };
        if backed_from(va, off, len, want, first_reaching(want, va, sorted)) {
            kept.push((va, len, off, ram));
        } else {
            plan.unmap.push((va, len));
        }
    }
    kept.sort_unstable();
    // Kept rows, as `(va, end)` for the overlap search (ledger rows are disjoint: ends ascend).
    let kept_spans: Vec<(u64, u64)> = kept.iter().map(|&(va, len, _, _)| (va, va.saturating_add(len))).collect();
    let spans_sorted = kept_spans.windows(2).all(|w| w[0].1 <= w[1].1);
    let mut dropped: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut mapped = vec![false; desired.len()];
    // ⊘ P5c: iterate to a fixed point. The old single pass checked every desired run against ALL
    // kept rows, then dropped kept rows overlapping an unbacked run — so a run that only a
    // since-dropped row backed was skipped AND left unmapped. A second pass checks the survivors.
    loop {
        let alive = |ram: bool| -> Vec<(u64, u64, u64)> {
            kept.iter()
                .filter(|k| k.3 == ram && !dropped.contains(&k.0))
                .map(|&(va, len, off, _)| (va, off, len))
                .collect()
        };
        let (have_store, have_ram) = (alive(false), alive(true));
        let (hs_sorted, hr_sorted) = (ends_sorted(&have_store), ends_sorted(&have_ram));
        let mut changed = false;
        for (i, d) in desired.iter().enumerate() {
            if mapped[i] {
                continue;
            }
            let (have, sorted) = if d.ram { (&have_ram, hr_sorted) } else { (&have_store, hs_sorted) };
            if backed_from(d.va, d.off, d.len, have, first_reaching(have, d.va, sorted)) {
                continue;
            }
            mapped[i] = true;
            changed = true;
            let d_end = d.va.saturating_add(d.len);
            let from = if spans_sorted { kept_spans.partition_point(|&(_, e)| e <= d.va) } else { 0 };
            for (j, &(kva, kend)) in kept_spans.iter().enumerate().skip(from) {
                if spans_sorted && kva >= d_end {
                    break;
                }
                let overlaps = kva < d_end && d.va < kend;
                if overlaps && dropped.insert(kva) {
                    plan.unmap.push((kva, kept[j].1));
                }
            }
        }
        if !changed {
            break;
        }
    }
    plan.map = desired.iter().zip(&mapped).filter(|(_, m)| **m).map(|(d, _)| *d).collect();
    plan.kept = kept.len() - dropped.len();
    plan
}

/// ★★★★★ **w826 — a server row, made mappable and made SUBORDINATE to the walk.**
///
/// A `GPU_PROMOTE_CTX` row is what RM asked US to map; the walk is what the guest's own
/// tables say. Two rules, both measured `[w826 m2 cup3: cuCtxCreate 719]`:
/// 1. **Whole pages.** RM refuses a mapping that is not a page multiple (`0x20409d000+0x8600`
///    → `Other(19305)`); the C rounds every promote mapping (`nvkvm_gpu_emul.c:7920`). A row
///    that is 64 KiB-aligned on both sides takes the C's 64 KiB round-up; any other row rounds
///    to 4 KiB. A row whose VA and backing disagree inside a page cannot be expressed → none.
/// 2. **The walk wins where both speak.** A row overlapping walked runs contributes only the
///    pieces the walk left empty — otherwise the two fight over one slice and every pass
///    unmaps and remaps it (`mapped=1 unmapped=1`, measured).
///
/// Returns `(va, len, backing offset)` pieces, page-aligned, ascending.
#[must_use]
pub fn server_row_pieces(
    va: u64,
    len: u64,
    phys: u64,
    walked: &[(u64, u64)],
) -> Vec<(u64, u64, u64)> {
    const PAGE: u64 = 0x1000;
    const BIG: u64 = 0x1_0000;
    if len == 0 || (va % PAGE) != (phys % PAGE) {
        return Vec::new();
    }
    let lead = va % PAGE;
    let (va, phys, len) = (va - lead, phys - lead, len + lead);
    let grain = if va % BIG == 0 && phys % BIG == 0 { BIG } else { PAGE };
    let Some(end) = len.checked_next_multiple_of(grain).and_then(|l| va.checked_add(l)) else {
        return Vec::new();
    };
    let mut cover: Vec<(u64, u64)> = walked
        .iter()
        .filter(|(w, l)| *l > 0 && *w < end && w.saturating_add(*l) > va)
        .map(|&(w, l)| (w, w.saturating_add(l)))
        .collect();
    cover.sort_unstable();
    let mut out = Vec::new();
    let mut at = va;
    for (s, e) in cover {
        if s > at {
            out.push((at, s.min(end) - at, phys + (at - va)));
        }
        at = at.max(e);
        if at >= end {
            break;
        }
    }
    if at < end {
        out.push((at, end - at, phys + (at - va)));
    }
    out
}

/// One mapping WE placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Length in bytes.
    pub len: u64,
    /// Offset in the backing object (store offset, or guest-RAM file offset).
    pub off: u64,
    /// Backed by guest RAM rather than the store.
    pub ram: bool,
}

/// What one [`Ledger::apply`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Maps placed.
    pub mapped: usize,
    /// Maps removed.
    pub unmapped: usize,
    /// Operations the host refused (the ledger reflects only what landed).
    pub refused: usize,
    /// The first refusal, by name.
    pub first_refusal: Option<String>,
    /// Whether the batch's single invalidate ran.
    pub invalidated: bool,
}

/// ★★★ **Where a reconcile's operations land** — `V3_P4_PORT_MAP.md` §2.3(b).
///
/// [`Ledger::apply`] was hard-wired to [`kf_host::HostRm`]. The P4 composition (the invalidate →
/// walk → reconcile → clear step, [`crate::vasmgr`]) must be testable with no GPU, and §2.3's
/// BAR windows are a second target of the same plan (`CpuWindow`), so the verbs are a trait.
/// ⊘ Every verb is one WE author (§9): a guest value never reaches a host flag word here —
/// `defer` is ours, the backing is decided by `Desired::ram`.
pub trait MapTarget {
    /// Map `d` at `d.va`, deferring the TLB invalidate when `defer`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map(&self, d: &Desired, defer: bool) -> Result<(), String>;
    /// Unmap the mapping WE placed at `va`, deferring the TLB invalidate when `defer`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String>;
    /// ONE invalidate for everything deferred since the last one.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn invalidate(&self) -> Result<(), String>;

    /// ★ P4: the VA extent `[0, extent)` this target can express, or `None` for a whole GPU VA
    /// space. A CPU window (the guest's BAR2 aperture) shows only the VAs its PCI BAR decodes:
    /// a walked leaf above that is real in the guest's tables but has no CPU address, so the VA
    /// manager CLIPS it (counted in `VaStats::clipped_bytes`) instead of refusing the space.
    fn va_extent(&self) -> Option<u64> {
        None
    }
}

/// ★ Cut walked leaves `(va, at, len, ap)` to `[0, extent)`: a leaf wholly above is dropped, a
/// leaf crossing the end is shortened (its backing offset is unchanged — it starts at the same
/// VA). Returns the kept leaves and the bytes cut.
#[must_use]
pub fn clip_leaves(leaves: &[(u64, u64, u64, u8)], extent: u64) -> (Vec<(u64, u64, u64, u8)>, u64) {
    let mut cut = 0u64;
    let mut out = Vec::with_capacity(leaves.len());
    for &(va, at, len, ap) in leaves {
        let end = va.saturating_add(len);
        if va >= extent {
            cut = cut.saturating_add(len);
        } else if end > extent {
            cut = cut.saturating_add(end - extent);
            out.push((va, at, extent - va, ap));
        } else {
            out.push((va, at, len, ap));
        }
    }
    (out, cut)
}

/// ★ The GPU VA-space target: one host VA space, the store object, and the guest-RAM object.
#[derive(Debug, Clone, Copy)]
pub struct HostVas<'rm> {
    /// The in-process host RM session.
    pub rm: &'rm kf_host::HostRm,
    /// The host VA space that mirrors the guest's.
    pub space: kf_host::VaSpace,
    /// The store (guest VRAM) object.
    pub store: u32,
    /// The guest-RAM object, if one is registered.
    pub ram_obj: Option<u32>,
}

impl MapTarget for HostVas<'_> {
    fn map(&self, d: &Desired, defer: bool) -> Result<(), String> {
        let obj = if d.ram {
            self.ram_obj
                .ok_or_else(|| format!("map {:#x}: guest-RAM row and no RAM object", d.va))?
        } else {
            self.store
        };
        self.rm
            .map(self.space, obj, kf_host::MapBacking::SharedSlice, d.off, d.len, Some(d.va), defer)
            .map(|_| ())
            .map_err(|e| format!("map {:#x}+{:#x}: {e:?}", d.va, d.len))
    }

    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        self.rm
            .unmap(self.space, va, defer)
            .map_err(|e| format!("unmap {va:#x}: {e:?}"))
    }

    fn invalidate(&self) -> Result<(), String> {
        self.rm
            .invalidate_tlb(self.space)
            .map_err(|e| format!("invalidate: {e:?}"))
    }
}

/// ★ The ledger of OUR OWN mappings in one host VA space — never a copy of the guest's tables.
#[derive(Debug, Default)]
pub struct Ledger {
    placed: std::collections::BTreeMap<u64, Placed>,
}

impl Ledger {
    /// `(va, len, off, ram)` rows, for [`plan_reconcile`].
    #[must_use]
    pub fn rows(&self) -> Vec<(u64, u64, u64, bool)> {
        self.placed.iter().map(|(&va, p)| (va, p.len, p.off, p.ram)).collect()
    }

    /// ★ Where `[va, va+len)` lives in OUR mappings: `(ram, offset)` — the store offset (or
    /// guest-RAM file offset) of `va`, when ONE placed row covers the whole range. This is how a
    /// Translated channel finds the bytes behind a guest VA (its GPFIFO, a pushbuffer segment):
    /// through what WE mapped, never a stored copy of the guest's tables (§24.2). `None` for a
    /// range we did not map, or that crosses rows (the caller reads row by row).
    #[must_use]
    pub fn resolve(&self, va: u64, len: u64) -> Option<(bool, u64)> {
        let (&start, p) = self.placed.range(..=va).next_back()?;
        let end = va.checked_add(len)?;
        (end <= start.checked_add(p.len)?).then(|| (p.ram, p.off + (va - start)))
    }

    /// Mappings held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.placed.len()
    }

    /// Nothing held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// ★ Execute `plan` in `space`: every unmap and map DEFERS its TLB invalidate, and the batch
    /// ends with ONE [`kf_host::HostRm::invalidate_tlb`] (v3 §4.2). Unmaps run first. The ledger
    /// records only what the host actually did; a refusal is counted and named, never assumed.
    /// ⊘ Now a thin wrapper over [`Ledger::apply_to`] with a [`HostVas`] target.
    pub fn apply(
        &mut self,
        rm: &kf_host::HostRm,
        space: kf_host::VaSpace,
        store: u32,
        ram_obj: Option<u32>,
        plan: &ReconcilePlan,
    ) -> Applied {
        self.apply_to(&HostVas { rm, space, store, ram_obj }, plan)
    }

    /// ★ Execute `plan` against any [`MapTarget`]: deferred unmaps, then deferred maps, then ONE
    /// invalidate when anything changed. The ledger records only what the target accepted.
    pub fn apply_to(&mut self, target: &dyn MapTarget, plan: &ReconcilePlan) -> Applied {
        let mut out = Applied::default();
        let refuse = |out: &mut Applied, what: String| {
            out.refused += 1;
            out.first_refusal.get_or_insert(what);
        };
        for &(va, len) in &plan.unmap {
            match target.unmap(va, true) {
                Ok(()) => {
                    self.placed.remove(&va);
                    out.unmapped += 1;
                }
                Err(e) => refuse(&mut out, format!("{e} (len {len:#x})")),
            }
        }
        for d in &plan.map {
            match target.map(d, true) {
                Ok(()) => {
                    self.placed.insert(d.va, Placed { len: d.len, off: d.off, ram: d.ram });
                    out.mapped += 1;
                }
                Err(e) => refuse(&mut out, e),
            }
        }
        if out.mapped + out.unmapped > 0 {
            match target.invalidate() {
                Ok(()) => out.invalidated = true,
                Err(e) => refuse(&mut out, e),
            }
        }
        out
    }
}


#[cfg(test)]
mod plan_tests {
    use super::*;

    /// Apply `plan` to `ledger` and check the result expresses `desired` exactly: rows disjoint,
    /// every row agrees with the desired run under it (same ground truth, same offset), and every
    /// desired byte is covered.
    fn check(ledger: &[(u64, u64, u64, bool)], desired: &[Desired], plan: &ReconcilePlan) {
        let mut rows: std::collections::BTreeMap<u64, (u64, u64, bool)> =
            ledger.iter().map(|&(va, len, off, ram)| (va, (len, off, ram))).collect();
        for &(va, _) in &plan.unmap {
            assert!(rows.remove(&va).is_some(), "unmap of {va:#x}, which is not held");
        }
        for d in &plan.map {
            assert!(rows.insert(d.va, (d.len, d.off, d.ram)).is_none(), "map over a held row at {:#x}", d.va);
        }
        let v: Vec<(u64, u64, u64, bool)> = rows.iter().map(|(&va, &(len, off, ram))| (va, len, off, ram)).collect();
        for w in v.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "overlapping rows {w:x?}");
        }
        for &(va, len, off, ram) in &v {
            let want: Vec<(u64, u64, u64)> =
                desired.iter().filter(|d| d.ram == ram).map(|d| (d.va, d.off, d.len)).collect();
            let mut want = want;
            want.sort_unstable();
            assert!(ram_slice_backed(va, off, len, &want), "row {va:#x}+{len:#x} is not what the walk says");
        }
        for d in desired {
            let have: Vec<(u64, u64, u64)> =
                v.iter().filter(|r| r.3 == d.ram).map(|&(va, len, off, _)| (va, off, len)).collect();
            assert!(ram_slice_backed(d.va, d.off, d.len, &have), "desired {d:x?} left unmapped");
        }
    }

    /// ⊘ The single-pass planner's hole: a kept row backing TWO runs is dropped for the second,
    /// and the first — skipped as backed — was left unmapped.
    #[test]
    fn a_run_backed_only_by_a_dropped_row_is_remapped() {
        let ledger = [(0x0, 0x2000, 0x10_0000, false)];
        let desired = [
            Desired { va: 0x0, len: 0x1000, off: 0x10_0000, ram: false },
            Desired { va: 0x1000, len: 0x2000, off: 0x10_1000, ram: false },
        ];
        let plan = plan_reconcile(&ledger, &desired);
        check(&ledger, &desired, &plan);
        assert_eq!(plan.map.len(), 2);
    }

    /// A small deterministic generator (no dependency): random disjoint layouts, random edits.
    #[test]
    fn random_layouts_reconcile_exactly() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..400 {
            let n = (next() % 40) as usize;
            let mut desired = Vec::new();
            let mut va = 0u64;
            for _ in 0..n {
                va += (next() % 4) * 0x1000;
                let len = (1 + next() % 4) * 0x1000;
                desired.push(Desired { va, len, off: (next() % 64) * 0x1000, ram: next() % 3 == 0 });
                va += len;
            }
            // A ledger from an EARLIER layout: some rows equal, some split/merged/shifted.
            let mut ledger = Vec::new();
            let mut va = 0u64;
            for d in &desired {
                match next() % 5 {
                    0 => {}
                    1 if d.len > 0x1000 => {
                        ledger.push((d.va, 0x1000, d.off, d.ram));
                    }
                    2 => ledger.push((d.va, d.len, d.off + 0x1000, d.ram)),
                    _ => ledger.push((d.va, d.len, d.off, d.ram)),
                }
                va = va.max(d.va + d.len);
            }
            ledger.push((va + 0x10_0000, 0x1000, 0, false));
            ledger.sort_unstable();
            ledger.dedup_by_key(|r| r.0);
            let plan = plan_reconcile(&ledger, &desired);
            check(&ledger, &desired, &plan);
        }
    }

    /// ★ The throughput this exists for: 13 000 rows reconcile in far under a millisecond each.
    #[test]
    fn thirteen_thousand_rows_plan_in_near_linear_time() {
        let desired: Vec<Desired> =
            (0..13_000u64).map(|i| Desired { va: 0x1_0000_0000 + i * 0x1000, len: 0x1000, off: i * 0x3000, ram: true }).collect();
        let ledger: Vec<(u64, u64, u64, bool)> = desired[..12_999].iter().map(|d| (d.va, d.len, d.off, d.ram)).collect();
        let t = std::time::Instant::now();
        let plan = plan_reconcile(&ledger, &desired);
        let el = t.elapsed();
        assert_eq!((plan.map.len(), plan.unmap.len(), plan.kept), (1, 0, 12_999));
        assert!(el < std::time::Duration::from_millis(200), "{el:?}");
    }
}
