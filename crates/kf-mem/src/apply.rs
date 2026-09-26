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
}

impl Applied {
    fn refuse(&mut self, i: usize, why: String) {
        self.codes[i] = KFWR_ACK_FAILED;
        self.refused += 1;
        self.first_refusal.get_or_insert(why);
    }
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
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| r.unmap) {
        if r.held {
            out.held_retired += 1;
            continue;
        }
        match target.unmap(r.va, true) {
            Ok(()) => out.unmapped += 1,
            Err(e) => {
                failed_unmaps.push((r.va, r.va.saturating_add(r.len)));
                out.refuse(i, format!("{e} (len {:#x})", r.len));
            }
        }
    }
    let extent = target.va_extent();
    let reserved = target.reserved();
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
        match target.map(&d, true) {
            Ok(Mapped::Placed) => out.mapped += 1,
            Ok(Mapped::HeldByHost) => {
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
    if out.mapped + out.unmapped + out.usermode_trapped > 0 {
        match target.invalidate() {
            Ok(()) => out.invalidated = true,
            Err(e) => {
                // ⊘ The rows landed (their verdicts stand — the host holds them); the space is
                // not settled until an invalidate succeeds, so the caller must not clear.
                out.refused += 1;
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
    }
    impl MapTarget for Rec {
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
