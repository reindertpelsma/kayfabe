//! ★★★★★ **THE DIFF/ACK PROTOCOL, STATED IN RUST** — the specification the walk kernel's
//! `kf_diff_slots` / `kf_commit_kernel` implement on the GPU, and the oracle `kf-gate9` holds the
//! GPU to, report for report, on hardware.
//!
//! # The protocol (owner design, 2026-09-25; COMMIT-ON-ACK ruling)
//!
//! > *"The GPU only sends a diff. … the copy the PTX holds, the last snapshot, is in vidmem,
//! > maintained by the PTX for compare."* · *"tell the PTX about it so it can update its own table,
//! > or only update its table for successful ones."*
//!
//! 1. The GPU holds, per **slot** (one per VA-space OBJECT, never per PDB — a root move keeps the
//!    slot), the **committed placements**: the mappings the host has confirmed it made, exactly
//!    as they were made (one entry per host map call). Grouped by page-size class, each class
//!    sorted by VA and disjoint.
//! 2. A walk reads the guest's live tables (`W`) and emits, per walked entry, the diff of `W`
//!    against the slot's committed placements `P` ([`diff`]):
//!    - `UNMAP p` for every placement `W` no longer backs byte for byte (same ground truth, same
//!      linear offset) — **whole placements only**, because the host's unmap takes the VA a map
//!      was placed at and nothing else;
//!    - `MAP` for every piece of `W` not covered by a KEPT placement.
//! 3. The host applies the diff with authored, unprivileged verbs and answers one [`AckCode`] per
//!    report run.
//! 4. [`commit`] folds ONLY the acknowledged entries into the slot. A refused map stays a
//!    difference and is re-emitted by the next diff; a refused unmap stays a placement. There is
//!    no rollback, and nothing is committed that the host did not confirm.
//!
//! # Why this is not the shadow `V3_BUILD.md` ruled out
//!
//! The walk kernel's old delta snapshot was the PREVIOUS WALK — a copy of the guest's table
//! content, committed whether or not the host acted on it (`THE_TRANSLATED_PLANE.md` §18.2's
//! shadow). The committed placements are a record of **our own host actions**, written only on
//! the host's confirmation — the "ledger of our own map handles" `THE_ARCHITECTURE_v3.md` §4.2
//! (w825) sanctions, held in vidmem beside the walker instead of in the VMM. Nothing is ever read
//! from it in place of the guest's tables, and a stale entry can only cost an extra diff line,
//! never a wrong translation.

use crate::abi::{KfMapRun, KFWR_OP_MAP, KFWR_OP_UNMAP, KFWR_RF_HELD, KFWR_RF_HOST_PERM};

/// Page-size classes.
pub const CLASSES: usize = 4;

/// A run's page-size class (`flags` bits 8..12, clamped to the four codes the format has).
#[must_use]
pub fn class_of(flags: u32) -> usize {
    ((flags >> 8) & 0x3) as usize
}

/// ★ The ground truth a run's backing names, as the HOST maps it: `0` the store (vidmem), `1`
/// guest RAM (both system apertures — the host maps one guest-RAM object either way), `2` any
/// other aperture (peer; the walker refuses it, so it never matches anything) — ★ v3-gfx: plus the
/// PTE KIND in bits `2..10`, because the host mapping now carries it (`kf_mem::apply::host_pte_kind`)
/// and a page re-kinded at the same backing must be re-mapped — ★★★ v3-roperm: plus the host
/// PERMISSIONS ([`KFWR_RF_HOST_PERM`]: read-only, atomic-disable, volatile) in bits `10..13`,
/// because the host mapping carries them too (`kf_host::MapPerm`). ⊘ Without them a guest RW→RO
/// downgrade over the same backing was KEPT: the host twin stayed read-write and a GPU write to a
/// UVM read-duplicate landed silently in a stale copy (`V3_UVM_DEMAND_PAGING.md` §6). Page size
/// and `PRIVILEGE` (no unprivileged host verb can place it) are not part of what the host places.
/// Mirrors `kf_hkey` (`cuda/walk/kf_walk.cu`).
#[must_use]
pub fn host_key(flags: u32) -> u32 {
    let ap = match flags & 0x7 {
        0 => 0,
        2 | 3 => 1,
        _ => 2,
    };
    ap | (((flags >> 16) & 0xff) << 2) | (((flags & KFWR_RF_HOST_PERM) >> 3) << 10)
}

/// One host verdict on one report run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AckCode {
    /// Not applied (refused, skipped, or never attempted): commit nothing for it.
    Failed = 0,
    /// Applied: an UNMAP retired its placement, a MAP placed ours.
    Applied = 1,
    /// ★ A MAP the host answered "already held" (P6b ruling (a)): the guest's statement is
    /// satisfied but the mapping is NOT ours. Committed with [`KFWR_RF_HELD`], so its later
    /// UNMAP is retired without asking the host to take down what it never placed for us.
    Held = 2,
}

/// The committed placements of one slot, per class, each sorted by VA and disjoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Committed {
    /// Per class.
    pub cls: [Vec<KfMapRun>; CLASSES],
}

impl Committed {
    /// Total placements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cls.iter().map(Vec::len).sum()
    }
    /// No placements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The class-grouped flat table, as the GPU stores it.
    #[must_use]
    pub fn flat(&self) -> Vec<KfMapRun> {
        self.cls.iter().flatten().copied().collect()
    }
}

/// What one entry's diff produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryDiff {
    /// Per class: the class's UNMAPs, then its MAPs.
    pub runs: Vec<KfMapRun>,
    /// ★ The maps were WITHHELD because committing them could overflow the slot: only the
    /// UNMAPs are emitted, and the caller re-walks as soon as they are applied.
    pub partial: bool,
    /// The slot is full and nothing can be retired: no progress is possible for this entry.
    pub overflow: bool,
}

/// The walk's runs of one class, in order (the walk emits each class VA-ascending and disjoint).
fn walk_class(walk: &[KfMapRun], c: usize) -> Vec<KfMapRun> {
    walk.iter().filter(|r| class_of(r.flags) == c).copied().collect()
}

/// Whether `w` backs `p` byte for byte: same host ground truth, same linear offset, no hole.
/// `w` is one class's walk runs, sorted and disjoint.
fn covered(p: &KfMapRun, w: &[KfMapRun]) -> bool {
    let Some(end) = p.va.checked_add(p.len) else {
        return false;
    };
    // The last run starting at or before p.va.
    let mut i = w.partition_point(|r| r.va <= p.va);
    if i == 0 {
        return false;
    }
    i -= 1;
    let mut at = p.va;
    let key = host_key(p.flags);
    while i < w.len() {
        let r = &w[i];
        let rend = r.va.saturating_add(r.len);
        if !(r.va <= at && at < rend) || host_key(r.flags) != key {
            return false;
        }
        if r.gpga.wrapping_add(at - r.va) != p.gpga.wrapping_add(at - p.va) {
            return false;
        }
        at = rend;
        if at >= end {
            return true;
        }
        i += 1;
    }
    false
}

/// ★★★★★ **The diff of one walked entry against its slot** (see the module docs). `cap` is the
/// slot's capacity: a diff whose maps, committed on top of every current placement, could exceed
/// it withholds the maps ([`EntryDiff::partial`]) — or, with nothing to retire, reports
/// [`EntryDiff::overflow`].
#[must_use]
pub fn diff(com: &Committed, walk: &[KfMapRun], cap: usize) -> EntryDiff {
    let mut unmaps: [Vec<KfMapRun>; CLASSES] = Default::default();
    let mut maps: [Vec<KfMapRun>; CLASSES] = Default::default();
    for c in 0..CLASSES {
        let w = walk_class(walk, c);
        let p = &com.cls[c];
        let mut kept: Vec<&KfMapRun> = Vec::new();
        for x in p {
            if covered(x, &w) {
                kept.push(x);
            } else {
                unmaps[c].push(KfMapRun { op: KFWR_OP_UNMAP, pdb_index: 0, ..*x });
            }
        }
        // Gap g lies between kept[g-1] and kept[g].
        for g in 0..=kept.len() {
            let lo = if g == 0 { 0 } else { kept[g - 1].va + kept[g - 1].len };
            let hi = if g == kept.len() { u64::MAX } else { kept[g].va };
            if lo >= hi {
                continue;
            }
            let mut i = w.partition_point(|r| r.va.saturating_add(r.len) <= lo);
            while i < w.len() && w[i].va < hi {
                let r = &w[i];
                let s = r.va.max(lo);
                let e = r.va.saturating_add(r.len).min(hi);
                if s < e {
                    maps[c].push(KfMapRun {
                        va: s,
                        gpga: r.gpga.wrapping_add(s - r.va),
                        len: e - s,
                        flags: r.flags & !KFWR_RF_HELD,
                        op: KFWR_OP_MAP,
                        pdb_index: 0,
                    });
                }
                i += 1;
            }
        }
    }
    let n_unmap: usize = unmaps.iter().map(Vec::len).sum();
    let n_map: usize = maps.iter().map(Vec::len).sum();
    let mut out = EntryDiff::default();
    if com.len() + n_map > cap {
        if n_unmap > 0 {
            out.partial = true;
            maps = Default::default();
        } else {
            out.overflow = true;
            return out;
        }
    }
    for c in 0..CLASSES {
        out.runs.extend_from_slice(&unmaps[c]);
        out.runs.extend_from_slice(&maps[c]);
    }
    out
}

/// ★★★★★ **Commit the host's verdict** on one entry's diff (`runs`, `codes` parallel) into its
/// slot: remove the placements whose UNMAP was applied, insert the MAPs that were applied (a
/// `Held` one carries [`KFWR_RF_HELD`]). Everything not acknowledged leaves the slot as it was.
#[must_use]
pub fn commit(com: &Committed, runs: &[KfMapRun], codes: &[AckCode]) -> Committed {
    let mut out = Committed::default();
    for c in 0..CLASSES {
        let p = &com.cls[c];
        let mut rm = vec![false; p.len()];
        let mut add: Vec<KfMapRun> = Vec::new();
        for (r, &code) in runs.iter().zip(codes) {
            if class_of(r.flags) != c || code == AckCode::Failed {
                continue;
            }
            if r.op == KFWR_OP_UNMAP {
                let i = p.partition_point(|x| x.va < r.va);
                if i < p.len() && p[i].va == r.va {
                    rm[i] = true;
                }
            } else if r.op == KFWR_OP_MAP {
                let mut m = KfMapRun { op: KFWR_OP_MAP, pdb_index: 0, ..*r };
                m.flags &= !KFWR_RF_HELD;
                if code == AckCode::Held {
                    m.flags |= KFWR_RF_HELD;
                }
                add.push(m);
            }
        }
        let kept: Vec<KfMapRun> = p.iter().zip(&rm).filter(|(_, r)| !**r).map(|(x, _)| *x).collect();
        // Merge by VA (both sorted).
        let (mut i, mut j) = (0, 0);
        let v = &mut out.cls[c];
        while i < kept.len() || j < add.len() {
            if j >= add.len() || (i < kept.len() && kept[i].va < add[j].va) {
                v.push(kept[i]);
                i += 1;
            } else {
                v.push(add[j]);
                j += 1;
            }
        }
    }
    out
}

/// The host-relevant mapping a set of runs expresses, per class, as sorted disjoint
/// `(va, len, key, gpga)` pieces with adjacent linear pieces merged — for comparing a slot with a
/// walk (closure).
#[must_use]
pub fn coverage(runs: &[KfMapRun]) -> [Vec<(u64, u64, u32, u64)>; CLASSES] {
    let mut out: [Vec<(u64, u64, u32, u64)>; CLASSES] = Default::default();
    for (c, slot) in out.iter_mut().enumerate() {
        let mut v: Vec<(u64, u64, u32, u64)> = runs
            .iter()
            .filter(|r| class_of(r.flags) == c)
            .map(|r| (r.va, r.len, host_key(r.flags), r.gpga))
            .collect();
        v.sort_unstable();
        let mut m: Vec<(u64, u64, u32, u64)> = Vec::new();
        for x in v {
            if let Some(l) = m.last_mut()
                && l.0 + l.1 == x.0
                && l.2 == x.2
                && l.3.wrapping_add(l.1) == x.3
            {
                l.1 += x.1;
                continue;
            }
            m.push(x);
        }
        *slot = m;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{KFWR_RF_ATOMIC_DISABLE, KFWR_RF_PRIVILEGE, KFWR_RF_READ_ONLY, KFWR_RF_VOLATILE};

    const PAGE: u64 = 0x1000;

    fn run(va: u64, gpga: u64, len: u64, flags: u32) -> KfMapRun {
        KfMapRun { va, gpga, len, flags, op: KFWR_OP_MAP, pdb_index: 0 }
    }
    const SYS: u32 = 2;
    const BIG: u32 = 1 << 8;

    fn ok(n: usize) -> Vec<AckCode> {
        vec![AckCode::Applied; n]
    }

    /// Apply a diff fully, then return the new slot.
    fn settle(com: &Committed, walk: &[KfMapRun]) -> Committed {
        let d = diff(com, walk, usize::MAX);
        commit(com, &d.runs, &ok(d.runs.len()))
    }

    fn assert_sound(c: &Committed) {
        for v in &c.cls {
            for w in v.windows(2) {
                assert!(w[0].va + w[0].len <= w[1].va, "committed not sorted/disjoint: {w:x?}");
            }
        }
    }

    #[test]
    fn a_first_walk_maps_every_run_and_then_is_quiet() {
        let walk = vec![run(0x10_0000, 0x20_0000, 3 * PAGE, 0), run(0x20_0000, 0x5000, PAGE, SYS)];
        let d = diff(&Committed::default(), &walk, 1 << 14);
        assert_eq!(d.runs.len(), 2);
        assert!(d.runs.iter().all(|r| r.op == KFWR_OP_MAP));
        let c = commit(&Committed::default(), &d.runs, &ok(2));
        assert!(diff(&c, &walk, 1 << 14).runs.is_empty(), "a settled slot diffs empty");
        assert_eq!(coverage(&c.flat()), coverage(&walk));
    }

    /// ★ Commit-on-ack: a FAILED map is still a difference next time; a successful one is never
    /// re-emitted.
    #[test]
    fn a_failed_map_is_retried_and_a_successful_one_never_re_emitted() {
        let walk = vec![run(0x1000, 0xA000, PAGE, 0), run(0x3000, 0xC000, PAGE, 0)];
        let d = diff(&Committed::default(), &walk, 64);
        let c = commit(&Committed::default(), &d.runs, &[AckCode::Applied, AckCode::Failed]);
        let d2 = diff(&c, &walk, 64);
        assert_eq!(d2.runs, vec![KfMapRun { op: KFWR_OP_MAP, ..walk[1] }], "only the failed map, again");
    }

    /// An unmap names a WHOLE placement; the still-backed parts of it are mapped again.
    #[test]
    fn a_partly_changed_placement_is_unmapped_whole_and_its_rest_remapped() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 4 * PAGE, 0)]);
        // The guest re-points page 2.
        let walk = vec![run(0, 0x10_0000, 2 * PAGE, 0), run(2 * PAGE, 0x90_0000, PAGE, 0), run(3 * PAGE, 0x10_3000, PAGE, 0)];
        let d = diff(&c, &walk, 64);
        assert_eq!(d.runs[0], KfMapRun { op: KFWR_OP_UNMAP, ..c.cls[0][0] });
        assert_eq!(d.runs.len(), 4, "{:x?}", d.runs);
        let c2 = commit(&c, &d.runs, &ok(4));
        assert_eq!(coverage(&c2.flat()), coverage(&walk));
    }

    #[test]
    fn growth_maps_only_the_new_page_and_keeps_the_old_placements() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 2 * PAGE, 0)]);
        let walk = vec![run(0, 0x10_0000, 3 * PAGE, 0)]; // coalesced with a new page
        let d = diff(&c, &walk, 64);
        assert_eq!(d.runs, vec![run(2 * PAGE, 0x10_2000, PAGE, 0)]);
    }

    /// ★ A root move: the slot is the OBJECT's; the new root's walk is diffed against what we
    /// placed under the old one — migrated entries are kept, the rest retired.
    #[test]
    fn a_root_move_keeps_what_the_new_root_still_maps() {
        let old = vec![run(0x1210_1000, 0x20_0000, PAGE, 0), run(0x1210_2000, 0x21_0000, PAGE, 0)];
        let c = settle(&Committed::default(), &old);
        let new_root_walk = vec![run(0x1210_1000, 0x20_0000, PAGE, 0), run(0x5000_0000, 0x40_0000, PAGE, 0)];
        let d = diff(&c, &new_root_walk, 64);
        assert_eq!(
            d.runs,
            vec![
                KfMapRun { op: KFWR_OP_UNMAP, ..old[1] },
                run(0x5000_0000, 0x40_0000, PAGE, 0),
            ]
        );
    }

    #[test]
    fn a_held_map_is_committed_held_and_retired_as_held() {
        let walk = vec![run(0x1000, 0xA000, PAGE, 0)];
        let d = diff(&Committed::default(), &walk, 64);
        let c = commit(&Committed::default(), &d.runs, &[AckCode::Held]);
        assert_ne!(c.cls[0][0].flags & KFWR_RF_HELD, 0);
        assert!(diff(&c, &walk, 64).runs.is_empty(), "a held placement is kept while the walk agrees");
        let d = diff(&c, &[], 64);
        assert_eq!(d.runs.len(), 1);
        assert_ne!(d.runs[0].flags & KFWR_RF_HELD, 0, "its unmap says it was never ours");
    }

    /// ★★★★★ v3-roperm — **A GUEST RW→RO DOWNGRADE OVER THE SAME BACKING IS A CHANGE.** Stock UVM
    /// read duplication revokes GPU write IN PLACE (`block_revoke_prot`, `uvm_va_block.c:9010-9060`):
    /// same VA, same physical page, only READ_ONLY flips. ⊘ The key used to omit it, so this diff
    /// was EMPTY, the host twin stayed read-write, and a GPU write to the duplicate landed silently
    /// in a stale copy (`V3_UVM_DEMAND_PAGING.md` §6). It must UNMAP the RW placement and MAP the
    /// same bytes again read-only — and the upgrade back must do the same the other way.
    #[test]
    fn a_rw_to_ro_downgrade_over_the_same_backing_unmaps_and_remaps() {
        let rw = run(0x10_0000, 0x20_0000, 4 * PAGE, SYS);
        let c = settle(&Committed::default(), &[rw]);
        assert!(diff(&c, &[rw], 64).runs.is_empty(), "control: an unchanged walk is quiet");
        let ro = run(0x10_0000, 0x20_0000, 4 * PAGE, SYS | KFWR_RF_READ_ONLY);
        let d = diff(&c, &[ro], 64);
        assert_eq!(
            d.runs,
            vec![KfMapRun { op: KFWR_OP_UNMAP, ..rw }, ro],
            "a permission downgrade must retire the RW placement and place the RO one"
        );
        let c = commit(&c, &d.runs, &ok(d.runs.len()));
        assert_eq!(c.cls[0], vec![ro], "the slot now records the placement as read-only");
        assert!(diff(&c, &[ro], 64).runs.is_empty(), "and is quiet once it landed");
        // The upgrade back (a collapse re-grants write) is a change too.
        let d = diff(&c, &[rw], 64);
        assert_eq!(d.runs, vec![KfMapRun { op: KFWR_OP_UNMAP, ..ro }, rw]);
        // ⊘ A refused remap stays a difference: the RW placement is still what the host holds.
        let c2 = settle(&Committed::default(), &[rw]);
        let d = diff(&c2, &[ro], 64);
        let c2 = commit(&c2, &d.runs, &[AckCode::Applied, AckCode::Failed]);
        assert!(c2.is_empty(), "the unmap landed, the RO map did not: nothing is placed");
        assert_eq!(diff(&c2, &[ro], 64).runs, vec![ro], "and the RO map is retried");
    }

    /// Every carried permission is part of the key — and `PRIVILEGE`, which no unprivileged host
    /// verb can place, is not (keying on it would only churn the host).
    #[test]
    fn each_carried_permission_is_part_of_the_key_and_privilege_is_not() {
        let base = run(0, 0x40_0000, 2 * PAGE, 0);
        let c = settle(&Committed::default(), &[base]);
        for bit in [KFWR_RF_READ_ONLY, KFWR_RF_ATOMIC_DISABLE, KFWR_RF_VOLATILE] {
            let w = KfMapRun { flags: base.flags | bit, ..base };
            assert_eq!(diff(&c, &[w], 64).runs.len(), 2, "flag {bit:#x} must re-map");
            assert_ne!(host_key(w.flags), host_key(base.flags));
        }
        let w = KfMapRun { flags: base.flags | KFWR_RF_PRIVILEGE, ..base };
        assert!(diff(&c, &[w], 64).runs.is_empty(), "PRIVILEGE is not placed by the host");
    }

    /// A downgrade of PART of a placement retires the whole placement (the host unmaps by the VA
    /// a map was placed at) and maps both halves with their own permissions.
    #[test]
    fn a_partial_downgrade_splits_the_placement_by_permission() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 4 * PAGE, 0)]);
        let walk = vec![run(0, 0x10_0000, 2 * PAGE, 0), run(2 * PAGE, 0x10_2000, 2 * PAGE, KFWR_RF_READ_ONLY)];
        let d = diff(&c, &walk, 64);
        assert_eq!(d.runs.len(), 3);
        assert_eq!(d.runs[0].op, KFWR_OP_UNMAP);
        assert_eq!(d.runs[0].len, 4 * PAGE);
        assert_eq!(&d.runs[1..], &walk[..]);
        let c = commit(&c, &d.runs, &ok(3));
        assert!(diff(&c, &walk, 64).runs.is_empty());
    }

    #[test]
    fn classes_are_diffed_separately() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 16 * PAGE, 0)]);
        // The same VAs, now one 64 KiB page with the same backing.
        let walk = vec![run(0, 0x10_0000, 16 * PAGE, BIG)];
        let d = diff(&c, &walk, 64);
        assert_eq!(d.runs.iter().map(|r| r.op).collect::<Vec<_>>(), vec![KFWR_OP_UNMAP, KFWR_OP_MAP]);
    }

    #[test]
    fn a_diff_that_could_overflow_the_slot_withholds_its_maps() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, PAGE, 0), run(2 * PAGE, 0x20_0000, PAGE, 0)]);
        let walk = vec![run(2 * PAGE, 0x30_0000, PAGE, 0), run(4 * PAGE, 0x40_0000, PAGE, 0)];
        let d = diff(&c, &walk, 3);
        assert!(d.partial);
        assert!(d.runs.iter().all(|r| r.op == KFWR_OP_UNMAP));
        let c = commit(&c, &d.runs, &ok(d.runs.len()));
        let d = diff(&c, &walk, 3);
        assert!(!d.partial && d.runs.len() == 2);
        let full = settle(&Committed::default(), &[run(0, 0, PAGE, 0), run(PAGE * 2, 0, PAGE, 0)]);
        assert!(diff(&full, &[run(0, 0, PAGE, 0), run(PAGE * 2, 0, PAGE, 0), run(PAGE * 9, 0, PAGE, 0)], 2).overflow);
    }

    // ── property tests: a deterministic generator, no dependency ─────────────────────────────

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A random walk: per class, disjoint VA-sorted runs, coalesced as the walker coalesces
    /// (never two adjacent linear runs with equal flags).
    fn random_walk(r: &mut Rng, n: usize) -> Vec<KfMapRun> {
        let mut out = Vec::new();
        for c in 0..2u32 {
            let mut va = 0x1_0000_0000u64 * u64::from(c + 1);
            let mut last: Option<KfMapRun> = None;
            for _ in 0..n {
                va += r.below(3) * PAGE;
                let len = (1 + r.below(3)) * PAGE;
                let flags = (c << 8) | if r.below(4) == 0 { SYS } else { 0 } | if r.below(5) == 0 { 1 << 3 } else { 0 };
                let gpga = r.below(64) * PAGE;
                let x = run(va, gpga, len, flags);
                if let Some(l) = last.as_mut()
                    && l.va + l.len == x.va
                    && l.gpga + l.len == x.gpga
                    && l.flags == x.flags
                {
                    l.len += x.len;
                } else {
                    if let Some(l) = last.take() {
                        out.push(l);
                    }
                    last = Some(x);
                }
                va += len;
            }
            if let Some(l) = last {
                out.push(l);
            }
        }
        out
    }

    /// Edit a walk the way a guest does: re-point, drop, add, grow.
    fn mutate(r: &mut Rng, walk: &[KfMapRun]) -> Vec<KfMapRun> {
        let mut v: Vec<KfMapRun> = Vec::new();
        for x in walk {
            match r.below(10) {
                0 => {}
                1 => v.push(KfMapRun { gpga: r.below(64) * PAGE, ..*x }),
                // ★ v3-roperm: a permission flip at the same backing (RW↔RO, atomics, cache,
                // privilege) — the in-place downgrade UVM's read duplication performs.
                3 => v.push(KfMapRun { flags: x.flags ^ [1u32 << 3, 1 << 4, 1 << 5, 1 << 6][r.below(4) as usize], ..*x }),
                2 if x.len > PAGE => {
                    v.push(KfMapRun { len: PAGE, ..*x });
                    v.push(KfMapRun { va: x.va + PAGE, gpga: r.below(64) * PAGE, len: x.len - PAGE, ..*x });
                }
                _ => v.push(*x),
            }
        }
        v
    }

    /// ★★★★★ THE PROTOCOL PROPERTIES, over random tables, random edits and random host refusals:
    /// - every UNMAP names exactly one committed placement (the host unmaps by placed VA);
    /// - the committed table stays sorted and disjoint per class, within capacity;
    /// - an acknowledged entry is never re-emitted while the walk is unchanged;
    /// - a refused entry IS re-emitted (retry), unchanged;
    /// - once everything is acknowledged the slot expresses exactly the walk (closure), and the
    ///   next diff is empty.
    #[test]
    fn random_sequences_hold_every_protocol_property() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..300 {
            let mut com = Committed::default();
            let n = 1 + r.below(30) as usize;
            let mut walk = random_walk(&mut r, n);
            for _step in 0..12 {
                if r.below(3) == 0 {
                    walk = mutate(&mut r, &walk);
                }
                let d = diff(&com, &walk, 1 << 14);
                assert!(!d.partial && !d.overflow);
                for u in d.runs.iter().filter(|x| x.op == KFWR_OP_UNMAP) {
                    let p = &com.cls[class_of(u.flags)];
                    assert!(p.iter().any(|x| x.va == u.va && x.len == u.len), "unmap of a non-placement {u:x?}");
                }
                // The host refuses at random; a map overlapping a refused unmap is not attempted.
                let mut codes: Vec<AckCode> = Vec::with_capacity(d.runs.len());
                let mut failed_unmaps: Vec<(usize, u64, u64)> = Vec::new();
                for x in &d.runs {
                    let c = class_of(x.flags);
                    let blocked = x.op == KFWR_OP_MAP
                        && failed_unmaps.iter().any(|&(fc, a, b)| fc == c && x.va < b && a < x.va + x.len);
                    let code = if blocked || r.below(4) == 0 { AckCode::Failed } else { AckCode::Applied };
                    if x.op == KFWR_OP_UNMAP && code == AckCode::Failed {
                        failed_unmaps.push((c, x.va, x.va + x.len));
                    }
                    codes.push(code);
                }
                let next = commit(&com, &d.runs, &codes);
                assert_sound(&next);
                // Retry and no-re-emission, against the SAME walk.
                let d2 = diff(&next, &walk, 1 << 14);
                for (x, code) in d.runs.iter().zip(&codes) {
                    let again = d2.runs.iter().any(|y| y.op == x.op && y.va == x.va && y.len == x.len && y.gpga == x.gpga);
                    if *code == AckCode::Applied {
                        assert!(!again, "an applied entry was re-emitted: {x:x?}");
                    } else if x.op == KFWR_OP_UNMAP {
                        assert!(again, "a refused unmap was not retried: {x:x?}");
                    } else {
                        // A refused map's bytes are still unplaced: covered by the next diff's maps.
                        let cov = coverage(&d2.runs.iter().filter(|y| y.op == KFWR_OP_MAP).copied().collect::<Vec<_>>());
                        let mine = coverage(&[*x]);
                        let c = class_of(x.flags);
                        let (va, len, _, _) = mine[c][0];
                        assert!(cov[c].iter().any(|&(a, l, _, _)| a <= va && va + len <= a + l), "a refused map was not retried: {x:x?}");
                    }
                }
                com = next;
            }
            // Drain: everything acknowledged ⇒ closure.
            for _ in 0..3 {
                com = settle(&com, &walk);
            }
            assert_eq!(coverage(&com.flat()), coverage(&walk), "closure");
            assert!(diff(&com, &walk, 1 << 14).runs.is_empty(), "settled ⇒ quiet");
        }
    }

    /// ★ The throughput shape the protocol exists for: 13 000 separate pages, one added per
    /// invalidate — each diff is ONE map.
    #[test]
    fn thirteen_thousand_rows_one_added_per_step_is_one_map_each() {
        let mut walk: Vec<KfMapRun> = Vec::new();
        let mut com = Committed::default();
        for i in 0..2000u64 {
            walk.push(run(0x1_0000_0000 + i * PAGE, i * 3 * PAGE, PAGE, SYS));
            let d = diff(&com, &walk, 1 << 14);
            assert_eq!(d.runs.len(), 1);
            com = commit(&com, &d.runs, &ok(1));
        }
        assert_eq!(com.len(), 2000);
    }
}
