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
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let mut at = start;
    for &(va, foff, rlen) in rows {
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
    let mut plan = ReconcilePlan::default();
    let mut kept: Vec<(u64, u64, u64, bool)> = Vec::new();
    for &(va, len, off, ram) in ledger {
        let want = if ram { &want_ram } else { &want_store };
        if ram_slice_backed(va, off, len, want) {
            kept.push((va, len, off, ram));
        } else {
            plan.unmap.push((va, len));
        }
    }
    kept.sort_unstable();
    let kept_rows = |ram: bool| -> Vec<(u64, u64, u64)> {
        kept.iter()
            .filter(|k| k.3 == ram)
            .map(|&(va, len, off, _)| (va, off, len))
            .collect()
    };
    let (have_store, have_ram) = (kept_rows(false), kept_rows(true));
    let mut dropped: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for d in desired {
        let have = if d.ram { &have_ram } else { &have_store };
        if ram_slice_backed(d.va, d.off, d.len, have) {
            continue;
        }
        for &(kva, klen, _, _) in &kept {
            let overlaps = kva < d.va.saturating_add(d.len) && d.va < kva.saturating_add(klen);
            if overlaps && dropped.insert(kva) {
                plan.unmap.push((kva, klen));
            }
        }
        plan.map.push(*d);
    }
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
    pub fn apply(
        &mut self,
        rm: &kf_host::HostRm,
        space: kf_host::VaSpace,
        store: u32,
        ram_obj: Option<u32>,
        plan: &ReconcilePlan,
    ) -> Applied {
        let mut out = Applied::default();
        let refuse = |out: &mut Applied, what: String| {
            out.refused += 1;
            out.first_refusal.get_or_insert(what);
        };
        for &(va, len) in &plan.unmap {
            match rm.unmap(space, va, true) {
                Ok(()) => {
                    self.placed.remove(&va);
                    out.unmapped += 1;
                }
                Err(e) => refuse(&mut out, format!("unmap {va:#x}+{len:#x}: {e:?}")),
            }
        }
        for d in &plan.map {
            let obj = if d.ram {
                match ram_obj {
                    Some(o) => o,
                    None => {
                        refuse(&mut out, format!("map {:#x}: guest-RAM row and no RAM object", d.va));
                        continue;
                    }
                }
            } else {
                store
            };
            match rm.map(space, obj, kf_host::MapBacking::SharedSlice, d.off, d.len, Some(d.va), true) {
                Ok(_) => {
                    self.placed.insert(d.va, Placed { len: d.len, off: d.off, ram: d.ram });
                    out.mapped += 1;
                }
                Err(e) => refuse(&mut out, format!("map {:#x}+{:#x}: {e:?}", d.va, d.len)),
            }
        }
        if out.mapped + out.unmapped > 0 {
            match rm.invalidate_tlb(space) {
                Ok(()) => out.invalidated = true,
                Err(e) => refuse(&mut out, format!("invalidate: {e:?}")),
            }
        }
        out
    }
}
