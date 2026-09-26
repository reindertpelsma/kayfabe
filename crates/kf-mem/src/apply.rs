//! ★★★★★ **APPLY ONE ENTRY'S DIFF, AND SAY EXACTLY WHAT LANDED** — the host half of the
//! commit-on-ack protocol (`kf_cuda::diffmodel`; owner design + ruling 2026-09-25).
//!
//! The walk kernel reports, per VA-space object, the diff of the guest's live tables against the
//! placements the host confirmed: UNMAPs of whole placements, MAPs of the pieces to place. This
//! module applies one entry's diff through a [`MapTarget`] — deferred unmaps, then deferred maps,
//! then ONE invalidate — and returns one verdict per run. The walker commits exactly the runs
//! acknowledged `Applied`/`Held`; a refused run stays a difference and the next diff retries it.
//! There is no rollback and no CPU copy of what was placed: the GPU's slot is the record, and the
//! host kernel is where the mappings actually live.
//!
//! ⊘ **No O(placements) work here.** Everything below is proportional to the DIFF.

use crate::ledger::{Desired, MapTarget, Mapped, UsermodeRow, desired_from_leaves};
use kf_chip::usermode::{UsermodeLeaf, UsermodeMmio};
use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD};

/// One run of a diff, decoded from the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRun {
    /// UNMAP (a whole committed placement) or MAP (a piece to place).
    pub unmap: bool,
    /// Guest VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// GPGA (vidmem) or guest-physical address (sysmem).
    pub at: u64,
    /// The leaf aperture code (`kf_mem::ledger::AP_*`).
    pub ap: u8,
    /// An UNMAP of a placement the host answered "already held": retired without a host call.
    pub held: bool,
    /// ★ v3-gfx: the guest PTE's KIND (run flags bits 16..23, `KFWR_RF_KIND_SHIFT`) — part of run
    /// identity in the walker, carried to the host map as its UNCOMPRESSED equivalent.
    pub kind: u8,
}

/// How one entry's runs are turned into host rows.
pub struct ApplyCfg<'a> {
    /// The store's length (vidmem leaves are bounded by it).
    pub store_bytes: u64,
    /// The family's smallest GMMU page: every row is whole pages of it.
    pub grain: u64,
    /// The VMM's guest-RAM layout: the memfd offset of guest-physical `[gpa, gpa+len)`.
    pub ram_offset: &'a dyn Fn(u64, u64) -> Option<u64>,
    /// ★ The family's internal-MMIO usermode page (`kf_chip::Family::usermode_mmio`): `None` on
    /// Turing … Ada, where no such leaf exists and nothing below changes.
    pub usermode: Option<UsermodeMmio>,
}

/// What one [`apply_entry`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// One `KFWR_ACK_*` per run, in run order.
    pub codes: Vec<u8>,
    /// Maps placed.
    pub mapped: usize,
    /// Placements removed (host calls).
    pub unmapped: usize,
    /// ★ P6b (a): maps the host answered `HeldByHost` — satisfied, never ours.
    pub held: usize,
    /// Held placements retired without a host call.
    pub held_retired: usize,
    /// Runs NOT applied (every one named in `first_refusal` or counted).
    pub refused: usize,
    /// The first refusal, by name.
    pub first_refusal: Option<String>,
    /// ★ v3-mapfix: of `refused`, the UNMAPs the host refused — a placement that may still be
    /// live on the host after the guest dropped it (the one refusal that is not mere absence).
    pub unmap_refused: usize,
    /// ★ v3-mapfix: the entry's invalidate was refused (its rows landed; the host TLB may not
    /// have seen them) — counted in `refused` too.
    pub invalidate_refused: bool,
    /// Whether the entry's single invalidate ran.
    pub invalidated: bool,
    /// Walked bytes above a CPU window's extent (satisfied: nothing to show them at).
    pub clipped_bytes: u64,
    /// Map runs refused because they overlap one of OUR VMM placements.
    pub vmm_overlaps: usize,
    /// ★ Usermode-page views the target placed something of its own for (BAR1: a trap overlay).
    pub usermode_trapped: usize,
    /// ★ Usermode-page views satisfied WITHOUT a host mapping (a GPU VA view — see
    /// [`MapTarget::map_usermode`]'s default).
    pub usermode_unmirrored: usize,
    /// ★ `V3_BATCHED_MAP.md`: map verbs issued to the target (a batch counts ONE).
    pub map_calls: usize,
    /// Unmap verbs issued to the target (a range counts ONE).
    pub unmap_calls: usize,
    /// Batched maps the target placed, and the runs they carried.
    pub batches: usize,
    /// Runs placed by a batch (included in `mapped`).
    pub batched_runs: usize,
    /// Range unmaps the target performed, and the runs they removed (included in `unmapped`).
    pub range_unmaps: usize,
    /// Runs removed by a range unmap.
    pub range_unmapped_runs: usize,
    /// Batches / ranges the target refused whose runs then went one by one (not refusals: every
    /// run still got its own verdict).
    pub batch_fallbacks: usize,
    /// The first such fallback's reason.
    pub first_batch_fallback: Option<String>,
}

impl Applied {
    /// ★ v3-mapfix — **every refusal here is ABSENCE**: only MAP runs (or usermode views) were
    /// refused; every unmap landed and the invalidate ran. The refused leaves are simply not on
    /// the host — a GPU access to one faults on OUR twin, contained to the space that named it —
    /// and nothing the guest dropped is still reachable.
    #[must_use]
    pub fn refusals_are_absence(&self) -> bool {
        self.unmap_refused == 0 && !self.invalidate_refused
    }

    fn refuse(&mut self, i: usize, why: String) {
        self.codes[i] = KFWR_ACK_FAILED;
        self.refused += 1;
        self.first_refusal.get_or_insert(why);
    }

    fn fallback(&mut self, why: String) {
        // A target that does not batch at all is not a fallback — it is the per-run path.
        if why != crate::ledger::NOT_BATCHED {
            self.batch_fallbacks += 1;
            self.first_batch_fallback.get_or_insert(why);
        }
    }
}

/// ★ The most runs one batched map carries (`V3_BATCHED_MAP.md` §3.3): the stitched host view
/// holds one VMA per file-discontiguous piece while the descriptor is built, and Linux caps a
/// process at `vm.max_map_count` (65 530 by default) VMAs — QEMU's own included.
pub const BATCH_MAX_RUNS: usize = 4096;

/// Split `items` (already sorted by VA) into maximal groups whose members are VA-adjacent
/// (`va + len` of one is the next one's `va`), pairwise `compatible` with the group's first, and at
/// most `cap` long.
fn contiguous_groups(
    items: &[usize],
    span: impl Fn(usize) -> (u64, u64),
    compatible: impl Fn(usize, usize) -> bool,
    cap: usize,
) -> Vec<&[usize]> {
    let mut out = Vec::new();
    let mut start = 0;
    for k in 1..=items.len() {
        let breaks = k == items.len() || k - start >= cap || {
            let (va, len) = span(items[k - 1]);
            va.checked_add(len) != Some(span(items[k]).0) || !compatible(items[start], items[k])
        };
        if breaks {
            out.push(&items[start..k]);
            start = k;
        }
    }
    out
}

/// Whether `d` is whole `grain` pages on both sides (`grain` a power of two).
fn whole_pages(d: &Desired, grain: u64) -> bool {
    let m = grain.wrapping_sub(1);
    d.len > 0 && (d.va | d.len | d.off) & m == 0
}

/// ★ v3-gfx — **the PTE kind a host row carries**: the guest's kind with compression stripped
/// (`kf_chip::uncompressed_pte_kind`; this device never backs comptags, so a compressible kind would
/// point the engine at compression state that does not exist). Guest RAM takes only PITCH or
/// GENERIC (sysmem holds no depth/stencil surface kinds); anything unknown stays PITCH, the
/// pre-v3-gfx mapping. `[measured vgfx 2026-09-26, gfx9]` a GL depth buffer mapped PITCH raised
/// host Xid 13 "3D-Z KIND Violation" on every draw.
#[must_use]
pub fn host_pte_kind(guest: u8, ram: bool) -> u8 {
    match kf_chip::uncompressed_pte_kind(guest) {
        Some(k) if !ram => k,
        Some(k @ (kf_chip::PTE_KIND_PITCH | kf_chip::PTE_KIND_GENERIC)) => k,
        _ => kf_chip::PTE_KIND_PITCH,
    }
}

/// ★★★★★ **Apply `runs` (one entry's diff) through `target`.** Unmaps first (a held placement
/// is retired without a host call), then maps, then ONE invalidate if anything changed.
///
/// A map is not attempted — and is acknowledged FAILED, so the next diff retries it — when it
/// cannot become a host row (outside the store, not guest RAM, not whole pages), when it overlaps
/// one of OUR VMM placements (a guest VA may never alias a VMM address), or when it overlaps a
/// placement whose unmap was just refused (the two would overlap on the host). A walked piece
/// wholly above a CPU window's extent has no CPU address: it is satisfied as HELD (nothing of
/// ours placed); one crossing the extent is placed up to it.
pub fn apply_entry(target: &dyn MapTarget, runs: &[DiffRun], cfg: &ApplyCfg<'_>) -> Applied {
    let mut out = Applied { codes: vec![KFWR_ACK_APPLIED; runs.len()], ..Applied::default() };
    let mut failed_unmaps: Vec<(u64, u64)> = Vec::new();
    // ★ `V3_BATCHED_MAP.md` §4: VA-adjacent unmaps go as ONE range (the union of exactly the
    // placements being removed, nothing else); a refused range falls back to one call per run,
    // so every run still gets its own verdict.
    let mut unmaps: Vec<usize> = Vec::new();
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| r.unmap) {
        if r.held {
            out.held_retired += 1;
        } else {
            unmaps.push(i);
        }
    }
    unmaps.sort_by_key(|&i| runs[i].va);
    for group in contiguous_groups(&unmaps, |i| (runs[i].va, runs[i].len), |_, _| true, usize::MAX) {
        if group.len() >= 2 {
            let (va, end) = (runs[group[0]].va, runs[group[group.len() - 1]].va + runs[group[group.len() - 1]].len);
            out.unmap_calls += 1;
            match target.unmap_range(va, end - va, true) {
                Ok(()) => {
                    out.unmapped += group.len();
                    out.range_unmaps += 1;
                    out.range_unmapped_runs += group.len();
                    continue;
                }
                Err(e) => out.fallback(e),
            }
        }
        for &i in group {
            let r = &runs[i];
            out.unmap_calls += 1;
            match target.unmap(r.va, true) {
                Ok(()) => out.unmapped += 1,
                Err(e) => {
                    failed_unmaps.push((r.va, r.va.saturating_add(r.len)));
                    out.unmap_refused += 1;
                    out.refuse(i, format!("{e} (len {:#x})", r.len));
                }
            }
        }
    }
    let extent = target.va_extent();
    let reserved = target.reserved();
    let mut pending: Vec<(usize, Desired)> = Vec::new();
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| !r.unmap) {
        // ★★★ Hopper+ internal MMIO FIRST: a usermode-page view is never a memory row.
        if let Some(leaf) = cfg.usermode.and_then(|u| u.classify(r.ap, r.kind, r.at, r.len)) {
            apply_usermode(target, r, leaf, extent, &reserved, &failed_unmaps, i, &mut out);
            continue;
        }
        let mut d = match desired_from_leaves([(r.va, r.at, r.len, r.ap)], cfg.store_bytes, cfg.ram_offset) {
            // ★ v3-gfx: the host maps it with the guest's kind, uncompressed (`Desired::kind`).
            Ok(v) if v.len() == 1 => Desired { kind: host_pte_kind(r.kind, v[0].ram), ..v[0] },
            Ok(_) => {
                out.refuse(i, format!("map {:#x}: no row", r.va));
                continue;
            }
            Err(e) => {
                out.refuse(i, format!("leaf refused: {e:?}"));
                continue;
            }
        };
        if !whole_pages(&d, cfg.grain) {
            out.refuse(
                i,
                format!(
                    "leaf {:#x}+{:#x} (backing {:#x}) is not whole {:#x}-byte pages: a sub-page row would leave a hole",
                    d.va, d.len, d.off, cfg.grain
                ),
            );
            continue;
        }
        if let Some(ext) = extent {
            if d.va >= ext {
                out.clipped_bytes += d.len;
                out.codes[i] = KFWR_ACK_HELD;
                continue;
            }
            let end = d.va.saturating_add(d.len);
            if end > ext {
                out.clipped_bytes += end - ext;
                d.len = ext - d.va;
            }
        }
        let end = d.va.saturating_add(d.len);
        if let Some(&(a, b)) = reserved.iter().find(|&&(a, b)| d.va < b && a < end) {
            out.vmm_overlaps += 1;
            out.refuse(
                i,
                format!(
                    "leaf {:#x}+{:#x} overlaps OUR placement [{a:#x}, {b:#x}) — a guest VA may never alias a VMM address; this leaf alone is refused (Q11)",
                    d.va, d.len
                ),
            );
            continue;
        }
        if failed_unmaps.iter().any(|&(a, b)| d.va < b && a < end) {
            out.refuse(i, format!("map {:#x}+{:#x}: over a placement whose unmap was refused", d.va, d.len));
            continue;
        }
        pending.push((i, d));
    }
    // ★ `V3_BATCHED_MAP.md` §3: VA-contiguous guest-RAM rows of one kind go as ONE batched
    // placement; a refused batch placed nothing, so its rows go one by one and each gets its own
    // verdict (HELD included).
    pending.sort_by_key(|&(_, d)| d.va);
    let idx: Vec<usize> = (0..pending.len()).collect();
    let same = |a: usize, b: usize| pending[a].1.ram && pending[b].1.ram && pending[a].1.kind == pending[b].1.kind;
    for group in contiguous_groups(&idx, |k| (pending[k].1.va, pending[k].1.len), same, BATCH_MAX_RUNS) {
        if group.len() >= 2 && pending[group[0]].1.ram {
            let rows: Vec<Desired> = group.iter().map(|&k| pending[k].1).collect();
            out.map_calls += 1;
            match target.map_batch(&rows, true) {
                Ok(()) => {
                    out.mapped += rows.len();
                    out.batches += 1;
                    out.batched_runs += rows.len();
                    continue;
                }
                Err(e) => out.fallback(e),
            }
        }
        for &k in group {
            let (i, d) = pending[k];
            out.map_calls += 1;
            match target.map(&d, true) {
                Ok(Mapped::Placed) => out.mapped += 1,
                Ok(Mapped::HeldByHost) => {
                    // Rare (a host-RM placement in the twin's VAS at the guest's VA): named per leaf.
                    eprintln!("kf3: mem leaf {:#x}+{:#x} HELD BY HOST (host RM placed its own buffer there)", d.va, d.len);
                    out.held += 1;
                    out.codes[i] = KFWR_ACK_HELD;
                    // ★ v3-gfx: name WHERE (bounded) — a held row is a guest VA host RM already owns.
                    static HELD_LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if HELD_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 64 {
                        eprintln!("kf-mem: HELD-BY-HOST guest row {:#x}+{:#x} (ram={}) — host RM already maps that VA", d.va, d.len, d.ram);
                    }
                }
                Err(e) => out.refuse(i, e),
            }
        }
    }
    if out.mapped + out.unmapped + out.usermode_trapped > 0 {
        match target.invalidate() {
            Ok(()) => out.invalidated = true,
            Err(e) => {
                // ⊘ The rows landed (their verdicts stand — the host holds them); the space is
                // not settled until an invalidate succeeds, so the caller must not clear.
                out.refused += 1;
                out.invalidate_refused = true;
                out.first_refusal.get_or_insert(e);
            }
        }
    }
    out
}

/// ★★★ One usermode-page view (`V3_BAR1_DOORBELL.md` §4): bounded like any row, then handed to
/// the target's [`MapTarget::map_usermode`]. A PRIV-page or stray internal-MMIO leaf is refused by
/// name — never turned into guest RAM.
#[allow(clippy::too_many_arguments)]
fn apply_usermode(
    target: &dyn MapTarget,
    r: &DiffRun,
    leaf: UsermodeLeaf,
    extent: Option<u64>,
    reserved: &[(u64, u64)],
    failed_unmaps: &[(u64, u64)],
    i: usize,
    out: &mut Applied,
) {
    let vf_rel = match leaf {
        UsermodeLeaf::User { vf_rel } => vf_rel,
        UsermodeLeaf::Priv { priv_off } => {
            out.refuse(
                i,
                format!(
                    "internal-MMIO leaf {:#x}+{:#x} views the kernel-only PRIV VF page at {priv_off:#x} (bPriv, usermode_api.c:67-73) — refused, never mapped",
                    r.va, r.len
                ),
            );
            return;
        }
        UsermodeLeaf::Stray => {
            out.refuse(
                i,
                format!(
                    "internal-MMIO leaf {:#x}+{:#x} (SYS_COH + message kind) names register {:#x}, not the usermode page — refused, never guest RAM",
                    r.va, r.len, r.at
                ),
            );
            return;
        }
    };
    let mut u = UsermodeRow { va: r.va, len: r.len, vf_rel };
    if u.len == 0 || (u.va | u.len | u.vf_rel) & 0xFFF != 0 {
        out.refuse(i, format!("usermode view {:#x}+{:#x} (page {vf_rel:#x}) is not whole 4 KiB pages", u.va, u.len));
        return;
    }
    if let Some(ext) = extent {
        if u.va >= ext {
            out.clipped_bytes += u.len;
            out.codes[i] = KFWR_ACK_HELD;
            return;
        }
        let end = u.va.saturating_add(u.len);
        if end > ext {
            out.clipped_bytes += end - ext;
            u.len = ext - u.va;
        }
    }
    let end = u.va.saturating_add(u.len);
    if let Some(&(a, b)) = reserved.iter().find(|&&(a, b)| u.va < b && a < end) {
        out.vmm_overlaps += 1;
        out.refuse(i, format!("usermode view {:#x}+{:#x} overlaps OUR placement [{a:#x}, {b:#x}) (Q11)", u.va, u.len));
        return;
    }
    if failed_unmaps.iter().any(|&(a, b)| u.va < b && a < end) {
        out.refuse(i, format!("usermode view {:#x}+{:#x}: over a placement whose unmap was refused", u.va, u.len));
        return;
    }
    match target.map_usermode(&u) {
        Ok(Mapped::Placed) => out.usermode_trapped += 1,
        Ok(Mapped::HeldByHost) => {
            out.usermode_unmirrored += 1;
            out.codes[i] = KFWR_ACK_HELD;
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 16 {
                eprintln!(
                    "kf-mem: USERMODE-VIEW-NOT-MIRRORED {:#x}+{:#x} (page {:#x}) — a GPU-originated doorbell through this VA is refused by name (no host mapping; it faults on the twin)",
                    u.va, u.len, u.vf_rel
                );
            }
        }
        Err(e) => out.refuse(i, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Rec {
        ops: RefCell<Vec<String>>,
        refuse_unmap: Option<u64>,
        refuse_map: Option<u64>,
        held_at: Option<u64>,
        extent: Option<u64>,
        /// Behave like the BAR1 window: place a trap for a usermode view.
        traps_usermode: bool,
        /// ★ Batch: `Some(refuse_at)` = batches and range unmaps supported; a batch containing
        /// `refuse_at` (or a range containing it) is refused (nothing placed / removed).
        batching: Option<Option<u64>>,
    }
    impl MapTarget for Rec {
        fn map_batch(&self, rows: &[Desired], _: bool) -> Result<(), String> {
            let Some(refuse) = self.batching else { return Err(crate::ledger::NOT_BATCHED.into()) };
            if rows.iter().any(|d| Some(d.va) == refuse) {
                return Err("batch refused (fake)".into());
            }
            let len: u64 = rows.iter().map(|d| d.len).sum();
            self.ops.borrow_mut().push(format!("batch {:#x}+{len:#x} x{}", rows[0].va, rows.len()));
            Ok(())
        }
        fn unmap_range(&self, va: u64, len: u64, _: bool) -> Result<(), String> {
            let Some(refuse) = self.batching else { return Err(crate::ledger::NOT_BATCHED.into()) };
            if refuse.is_some_and(|r| r >= va && r < va + len) {
                return Err("range refused (fake)".into());
            }
            self.ops.borrow_mut().push(format!("unmap-range {va:#x}+{len:#x}"));
            Ok(())
        }
        fn map(&self, d: &Desired, _: bool) -> Result<Mapped, String> {
            if self.refuse_map == Some(d.va) {
                return Err("no (fake)".into());
            }
            self.ops.borrow_mut().push(format!("map {:#x}+{:#x}", d.va, d.len));
            Ok(if self.held_at == Some(d.va) { Mapped::HeldByHost } else { Mapped::Placed })
        }
        fn unmap(&self, va: u64, _: bool) -> Result<(), String> {
            if self.refuse_unmap == Some(va) {
                return Err("no (fake)".into());
            }
            self.ops.borrow_mut().push(format!("unmap {va:#x}"));
            Ok(())
        }
        fn invalidate(&self) -> Result<(), String> {
            self.ops.borrow_mut().push("inval".into());
            Ok(())
        }
        fn va_extent(&self) -> Option<u64> {
            self.extent
        }
        fn map_usermode(&self, u: &UsermodeRow) -> Result<Mapped, String> {
            if !self.traps_usermode {
                return Ok(Mapped::HeldByHost); // the trait default's answer, recorded nowhere
            }
            self.ops.borrow_mut().push(format!("trap {:#x}+{:#x} vf{:#x}", u.va, u.len, u.vf_rel));
            Ok(Mapped::Placed)
        }
    }
    fn cfg() -> ApplyCfg<'static> {
        ApplyCfg { store_bytes: 1 << 30, grain: 0x1000, ram_offset: &|gpa, _| Some(gpa), usermode: None }
    }
    fn m(va: u64, at: u64, len: u64) -> DiffRun {
        DiffRun { unmap: false, va, len, at, ap: 0, held: false, kind: 0 }
    }
    fn u(va: u64, len: u64) -> DiffRun {
        DiffRun { unmap: true, va, len, at: 0, ap: 0, held: false, kind: 0 }
    }

    fn ram(va: u64, gpa: u64, len: u64) -> DiffRun {
        DiffRun { ap: crate::ledger::AP_SYS_COHERENT, ..m(va, gpa, len) }
    }

    /// ★★★ `V3_BATCHED_MAP.md`: VA-contiguous guest-RAM runs (scattered in guest-physical memory)
    /// are ONE batched placement; a VA gap, a vidmem row or a kind change starts a new group, and
    /// a lone run keeps the per-run verb. Every run is acknowledged APPLIED.
    #[test]
    fn va_contiguous_guest_ram_runs_map_as_one_batch() {
        let t = Rec { batching: Some(None), ..Rec::default() };
        let runs = [
            ram(0x2_0000_2000, 0x7000, 0x1000), // out of VA order on purpose
            ram(0x2_0000_0000, 0x9000, 0x1000),
            ram(0x2_0000_1000, 0x3000, 0x1000),
            ram(0x2_0000_4000, 0x5000, 0x1000), // VA gap at 0x3000
            m(0x2_0000_5000, 0x10_0000, 0x1000), // vidmem: never batched
            DiffRun { kind: 0x06, ..ram(0x2_0000_6000, 0xB000, 0x1000) },
            DiffRun { kind: 0x06, ..ram(0x2_0000_7000, 0x1000, 0x1000) },
        ];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; runs.len()]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "batch 0x200000000+0x3000 x3",
                "map 0x200004000+0x1000",
                "map 0x200005000+0x1000",
                "batch 0x200006000+0x2000 x2",
                "inval"
            ]
        );
        assert_eq!((a.mapped, a.batches, a.batched_runs, a.map_calls), (7, 2, 5, 4));
    }

    /// ★★★ A refused batch placed NOTHING, so its runs go one by one — and each gets its own
    /// verdict: the one the host already holds is HELD, the one it refuses is FAILED, the rest
    /// APPLIED. Commit-on-ack is unchanged by batching.
    #[test]
    fn a_refused_batch_falls_back_to_exact_per_run_verdicts() {
        let t = Rec { batching: Some(Some(0x1000_1000)), held_at: Some(0x1000_2000), refuse_map: Some(0x1000_1000), ..Rec::default() };
        let runs = [ram(0x1000_0000, 0x4000, 0x1000), ram(0x1000_1000, 0x9000, 0x1000), ram(0x1000_2000, 0x2000, 0x1000)];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD]);
        assert_eq!(*t.ops.borrow(), vec!["map 0x10000000+0x1000", "map 0x10002000+0x1000", "inval"]);
        assert_eq!((a.mapped, a.held, a.refused, a.batch_fallbacks), (1, 1, 1, 1));
        assert!(a.first_batch_fallback.unwrap().contains("batch refused"));
    }

    /// ★★★ VA-adjacent unmaps are ONE range; a held run is never part of one (it never reaches
    /// the host); a refused range is retried run by run so every run is named.
    #[test]
    fn adjacent_unmaps_are_one_range_and_a_refused_range_goes_run_by_run() {
        let t = Rec { batching: Some(None), ..Rec::default() };
        let runs = [u(0x3000, 0x1000), u(0x1000, 0x2000), DiffRun { held: true, ..u(0x4000, 0x1000) }, u(0x9000, 0x1000)];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 4]);
        assert_eq!(*t.ops.borrow(), vec!["unmap-range 0x1000+0x3000", "unmap 0x9000", "inval"]);
        assert_eq!((a.unmapped, a.range_unmaps, a.range_unmapped_runs, a.unmap_calls, a.held_retired), (3, 1, 2, 2, 1));

        let t = Rec { batching: Some(Some(0x2000)), refuse_unmap: Some(0x2000), ..Rec::default() };
        let a = apply_entry(&t, &[u(0x1000, 0x1000), u(0x2000, 0x1000), u(0x3000, 0x1000), m(0x2000, 0x5000, 0x1000)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_APPLIED, KFWR_ACK_FAILED]);
        assert_eq!(*t.ops.borrow(), vec!["unmap 0x1000", "unmap 0x3000", "inval"], "the map over the refused unmap is still blocked");
        assert_eq!(a.batch_fallbacks, 1);
    }

    /// A batch never exceeds [`BATCH_MAX_RUNS`] runs.
    #[test]
    fn a_batch_is_capped() {
        let t = Rec { batching: Some(None), ..Rec::default() };
        let n = BATCH_MAX_RUNS + 3;
        let runs: Vec<DiffRun> = (0..n as u64).map(|k| ram(0x4_0000_0000 + k * 0x1000, (n as u64 - k) * 0x2000, 0x1000)).collect();
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.batches, 2);
        assert_eq!(t.ops.borrow()[0], format!("batch 0x400000000+{:#x} x{BATCH_MAX_RUNS}", BATCH_MAX_RUNS * 0x1000));
        assert_eq!(t.ops.borrow()[1], format!("batch {:#x}+0x3000 x3", 0x4_0000_0000u64 + BATCH_MAX_RUNS as u64 * 0x1000));
    }

    #[test]
    fn unmaps_then_maps_then_one_invalidate() {
        let t = Rec::default();
        let a = apply_entry(&t, &[m(0x2000, 0x10_0000, 0x1000), u(0x5000, 0x1000)], &cfg());
        assert_eq!(*t.ops.borrow(), vec!["unmap 0x5000", "map 0x2000+0x1000", "inval"]);
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
    }

    #[test]
    fn a_refused_unmap_blocks_the_map_over_it_and_both_stay_differences() {
        let t = Rec { refuse_unmap: Some(0x2000), ..Rec::default() };
        let a = apply_entry(&t, &[u(0x2000, 0x2000), m(0x3000, 0x10_0000, 0x1000), m(0x8000, 0x20_0000, 0x1000)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED, KFWR_ACK_APPLIED]);
        assert_eq!(*t.ops.borrow(), vec!["map 0x8000+0x1000", "inval"], "the blocked map was never attempted");
        assert_eq!(a.refused, 2);
    }

    #[test]
    fn held_is_acknowledged_held_and_a_held_unmap_never_reaches_the_host() {
        let t = Rec { held_at: Some(0x2000), ..Rec::default() };
        let a = apply_entry(&t, &[DiffRun { held: true, ..u(0x9000, 0x1000) }, m(0x2000, 0x10_0000, 0x1000)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
        assert_eq!(*t.ops.borrow(), vec!["map 0x2000+0x1000"], "no unmap call and no invalidate: nothing of ours changed");
    }

    #[test]
    fn a_window_clips_at_its_extent_and_satisfies_what_lies_above() {
        let t = Rec { extent: Some(0x10_0000), ..Rec::default() };
        let a = apply_entry(&t, &[m(0xF_F000, 0x1000, 0x2000), m(0x20_0000, 0x4000, 0x1000)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
        assert_eq!(t.ops.borrow()[0], "map 0xff000+0x1000");
        assert_eq!(a.clipped_bytes, 0x2000);
    }

    #[test]
    fn a_leaf_that_cannot_be_a_row_is_refused_by_name() {
        let t = Rec::default();
        let a = apply_entry(&t, &[m(0x1000, (1 << 30) - 0x1000, 0x2000), m(0x1010, 0x1000, 0x1000)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
        assert!(a.first_refusal.unwrap().contains("OutsideStore"));
        assert!(t.ops.borrow().is_empty());
    }
    // ★★★ GH100-shaped fixture (`V3_BAR1_DOORBELL.md` §2): the BAR1 PTEs RM writes for `pBar1VF`
    // after `kbusMapFbAperture_GM107` placed the 64 KiB view at BAR1 VA 0x0123_0000 — aperture
    // SYS_COHERENT (2), kind SMSKED_MESSAGE (0xF), address = NV_VIRTUAL_FUNCTION base 0x30000.
    const GH100_DB_VA: u64 = 0x0123_0000;
    fn gh100_db_leaf() -> DiffRun {
        DiffRun { unmap: false, va: GH100_DB_VA, len: 0x1_0000, at: 0x3_0000, ap: 2, held: false, kind: 0x0F }
    }
    fn hopper() -> ApplyCfg<'static> {
        ApplyCfg { usermode: kf_chip::Family::Hopper.usermode_mmio(), ..cfg() }
    }

    #[test]
    fn gh100_bar1_doorbell_view_becomes_a_trap_never_guest_ram() {
        let t = Rec { traps_usermode: true, ..Rec::default() };
        let a = apply_entry(&t, &[gh100_db_leaf(), m(0x10_0000, 0x40_0000, 0x1000)], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
        assert_eq!(
            *t.ops.borrow(),
            vec!["trap 0x1230000+0x10000 vf0x0", "map 0x100000+0x1000", "inval"],
            "the view is a trap; ordinary memory beside it still maps"
        );
        assert_eq!((a.usermode_trapped, a.mapped), (1, 1));
        // ⊘ And its UNMAP (the guest's last munmap freed the BAR1 VA) reaches the target at the VA.
        let a = apply_entry(&t, &[u(GH100_DB_VA, 0x1_0000)], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(t.ops.borrow()[3], "unmap 0x1230000");
    }

    #[test]
    fn gh100_gpu_va_doorbell_view_is_satisfied_unmirrored_by_default() {
        let t = Rec::default(); // a GPU VA space: the trait default
        let a = apply_entry(&t, &[gh100_db_leaf()], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_HELD], "satisfied (the invalidate clears) but not ours");
        assert_eq!(a.usermode_unmirrored, 1);
        assert!(t.ops.borrow().is_empty(), "no host mapping — above all not guest RAM at 0x30000");
    }

    #[test]
    fn ga10x_config_leaves_every_leaf_on_the_memory_path() {
        // `usermode: None` (Turing … Ada): byte-for-byte the pre-2026-09-26 behaviour.
        let t = Rec { traps_usermode: true, ..Rec::default() };
        let a = apply_entry(&t, &[gh100_db_leaf()], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(*t.ops.borrow(), vec!["map 0x1230000+0x10000", "inval"]);
        assert_eq!(a.usermode_trapped + a.usermode_unmirrored, 0);
    }

    #[test]
    fn priv_and_stray_internal_mmio_are_refused_and_plain_ram_at_0x30000_still_maps() {
        let t = Rec { traps_usermode: true, ..Rec::default() };
        let priv_leaf = DiffRun { at: 0x2000, len: 0x1000, ..gh100_db_leaf() };
        let stray = DiffRun { va: 0x200_0000, at: 0x50_0000, len: 0x1000, ..gh100_db_leaf() };
        let ram = DiffRun { va: 0x300_0000, at: 0x3_0000, len: 0x1000, ap: 2, kind: 0x00, ..gh100_db_leaf() };
        let a = apply_entry(&t, &[priv_leaf, stray, ram], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED, KFWR_ACK_APPLIED]);
        assert!(a.first_refusal.unwrap().contains("PRIV"));
        assert_eq!(*t.ops.borrow(), vec!["map 0x3000000+0x1000", "inval"]);
    }

    #[test]
    fn a_doorbell_view_above_the_bar1_extent_is_clipped_like_any_row() {
        let t = Rec { traps_usermode: true, extent: Some(GH100_DB_VA + 0x8000), ..Rec::default() };
        let a = apply_entry(&t, &[gh100_db_leaf()], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(*t.ops.borrow(), vec!["trap 0x1230000+0x8000 vf0x0", "inval"]);
        assert_eq!(a.clipped_bytes, 0x8000);
    }
}
