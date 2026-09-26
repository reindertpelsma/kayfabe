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

use crate::ledger::{Desired, MapTarget, Mapped, desired_from_leaves};
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
}

/// How one entry's runs are turned into host rows.
pub struct ApplyCfg<'a> {
    /// The store's length (vidmem leaves are bounded by it).
    pub store_bytes: u64,
    /// The family's smallest GMMU page: every row is whole pages of it.
    pub grain: u64,
    /// The VMM's guest-RAM layout: the memfd offset of guest-physical `[gpa, gpa+len)`.
    pub ram_offset: &'a dyn Fn(u64, u64) -> Option<u64>,
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
        let mut d = match desired_from_leaves([(r.va, r.at, r.len, r.ap)], cfg.store_bytes, cfg.ram_offset) {
            Ok(v) if v.len() == 1 => v[0],
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
    if out.mapped + out.unmapped > 0 {
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
    }
    fn cfg() -> ApplyCfg<'static> {
        ApplyCfg { store_bytes: 1 << 30, grain: 0x1000, ram_offset: &|gpa, _| Some(gpa) }
    }
    fn m(va: u64, at: u64, len: u64) -> DiffRun {
        DiffRun { unmap: false, va, len, at, ap: 0, held: false }
    }
    fn u(va: u64, len: u64) -> DiffRun {
        DiffRun { unmap: true, va, len, at: 0, ap: 0, held: false }
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
}
