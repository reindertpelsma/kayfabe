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

use crate::abi::{
    AP_SYS, AP_SYS_NC, AP_VID, KFWR_OP_MAP, KFWR_OP_UNMAP, KFWR_RF_HELD, KFWR_RF_KEY_PERM_ALL,
    KFWR_RF_KEY_PERM_DEFAULT, KfMapRun, RF_AP, RF_CLASS, RF_KIND,
};

/// Page-size classes.
pub const CLASSES: usize = 4;

/// A run's page-size class: the page-size field (`KFWR_RF_PS_SHIFT`) clamped to the four codes the
/// format has, exactly as the kernel's `kf_pcls` reads it ([`RF_CLASS`], `& 3`).
#[must_use]
pub fn class_of(flags: u32) -> usize {
    RF_CLASS.get(flags) as usize
}

/// Where [`host_key_with`] puts the KIND (bits `2..10`) — `kf_hkey`'s `<< 2`.
const HKEY_KIND_LO: u32 = 2;
/// Where [`host_key_with`] puts the keyed permissions (bits `10..14`) — `kf_hkey`'s `<< 10`.
const HKEY_PERM_LO: u32 = 10;
/// The lowest permission bit of the run flags (`KFWR_RF_READ_ONLY`, bit 3) — `kf_hkey`'s `>> 3`.
const RF_PERM_LO: u32 = KFWR_RF_KEY_PERM_ALL.trailing_zeros();

/// ★ The ground truth a run's backing names, as the HOST maps it: `0` the store (vidmem), `1`
/// guest RAM (both system apertures — the host maps one guest-RAM object either way), `2` any
/// other aperture (peer; the walker refuses it, so it never matches anything) — ★ v3-gfx: plus the
/// PTE KIND in bits `2..10`, because the host mapping now carries it (`kf_mem::apply::host_pte_kind`)
/// and a page re-kinded at the same backing must be re-mapped — ★★★ v3-roperm: plus the
/// permission bits the host's policy keys on (`key_perm`, a subset of [`KFWR_RF_KEY_PERM_ALL`]) in
/// bits `10..14`. ⊘ Without them a guest RW→RO downgrade over the same backing was KEPT: the host
/// twin stayed read-write and a GPU write to a UVM read-duplicate landed silently in a stale copy
/// (`V3_UVM_DEMAND_PAGING.md` §6). Page size is not part of what the host places. Mirrors
/// `kf_hkey` (`cuda/walk/kf_walk.cu`), whose `kp` is `KfArgs::key_perm`.
#[must_use]
pub fn host_key_with(flags: u32, key_perm: u32) -> u32 {
    // `RF_AP` is 3 bits wide, so the narrowing is exact.
    let ap = match RF_AP.get(flags) as u8 {
        AP_VID => 0,
        AP_SYS | AP_SYS_NC => 1,
        _ => 2,
    };
    // ★ ABI 7: SPARSE joins the key (bit 14), as in `kf_hkey`.
    let sparse = if flags & crate::abi::KFWR_RF_SPARSE != 0 {
        1 << 14
    } else {
        0
    };
    ap | (RF_KIND.get(flags) << HKEY_KIND_LO)
        | (((flags & key_perm & KFWR_RF_KEY_PERM_ALL) >> RF_PERM_LO) << HKEY_PERM_LO)
        | sparse
}

/// [`host_key_with`] under the default policy ([`KFWR_RF_KEY_PERM_DEFAULT`]).
#[must_use]
pub fn host_key(flags: u32) -> u32 {
    host_key_with(flags, KFWR_RF_KEY_PERM_DEFAULT)
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
    walk.iter()
        .filter(|r| class_of(r.flags) == c)
        .copied()
        .collect()
}

/// Whether `w` backs `p` byte for byte: same host ground truth, same linear offset, no hole.
/// `w` is one class's walk runs, sorted and disjoint.
fn covered(p: &KfMapRun, w: &[KfMapRun], key_perm: u32) -> bool {
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
    let key = host_key_with(p.flags, key_perm);
    while i < w.len() {
        let r = &w[i];
        let rend = r.va.saturating_add(r.len);
        if !(r.va <= at && at < rend) || host_key_with(r.flags, key_perm) != key {
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
    diff_with(com, walk, cap, KFWR_RF_KEY_PERM_DEFAULT)
}

/// [`diff`] under an explicit key policy (`key_perm`, as the kernel's `KfArgs::key_perm`).
#[must_use]
pub fn diff_with(com: &Committed, walk: &[KfMapRun], cap: usize, key_perm: u32) -> EntryDiff {
    let mut unmaps: [Vec<KfMapRun>; CLASSES] = Default::default();
    let mut maps: [Vec<KfMapRun>; CLASSES] = Default::default();
    for c in 0..CLASSES {
        let w = walk_class(walk, c);
        let p = &com.cls[c];
        let mut kept: Vec<&KfMapRun> = Vec::new();
        for x in p {
            if covered(x, &w, key_perm) {
                kept.push(x);
            } else {
                unmaps[c].push(KfMapRun {
                    op: KFWR_OP_UNMAP,
                    pdb_index: 0,
                    ..*x
                });
            }
        }
        // Gap g lies between kept[g-1] and kept[g].
        for g in 0..=kept.len() {
            let lo = if g == 0 {
                0
            } else {
                kept[g - 1].va + kept[g - 1].len
            };
            let hi = if g == kept.len() {
                u64::MAX
            } else {
                kept[g].va
            };
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

/// ★★★★★ **The report's own capacity (M4, 2026-11-15) — the Rust spec of `kf_diff_emit`'s cut.**
///
/// `staged[t]` is entry `t`'s whole diff in STAGED order (the order [`diff`] returns: per class,
/// its UNMAPs then its MAPs — eight lists, each VA-ascending and disjoint within itself). A report
/// carries at most `cap` runs in all. A diff that does not fit is NOT a prefix of the staged order
/// (that separates an UNMAP from the overlapping MAP of another page-size class and from the MAP
/// that carries its unchanged pages — see `kf_walk.cu`'s cut) but a **window by address**: the first
/// entry whose running total passes `cap` reports every run that STARTS below the largest boundary
/// `v` that no run straddles (`va < v < va + len`) and that keeps the report within `cap`; entries
/// before it are whole, entries after it are withheld. Every run that overlaps a reported run is
/// itself reported, so the apply's pairing and ordering (which only ever relate overlapping runs)
/// sees exactly the window of the whole diff. When no such boundary reports anything of the FIRST
/// non-empty entry (one overlap-connected clump larger than the report) the legacy refusal stands:
/// [`CutReport::truncated`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CutReport {
    /// Per entry: the runs reported, in staged order.
    pub entries: Vec<Vec<KfMapRun>>,
    /// Per entry: `KFWR_V_CUT` — part of its diff is still to come.
    pub cut: Vec<bool>,
    /// The report is a window (`KFWR_HF_CUT`).
    pub is_cut: bool,
    /// Nothing can be reported without splitting a clump: the legacy `TRUNCATED` refusal.
    pub truncated: bool,
}

/// [`CutReport`] of `staged` at report capacity `cap`; the mirror of `kf_diff_emit`.
#[must_use]
pub fn cut_report(staged: &[Vec<KfMapRun>], cap: usize) -> CutReport {
    let n = staged.len();
    let mut out = CutReport {
        entries: staged.to_vec(),
        cut: vec![false; n],
        is_cut: false,
        truncated: false,
    };
    let (mut run, mut first) = (0usize, None);
    for (t, s) in staged.iter().enumerate() {
        if run + s.len() > cap {
            first = Some((t, run));
            break;
        }
        run += s.len();
    }
    let Some((f, fbase)) = first else {
        return out;
    };
    let s = &staged[f];
    let allowed = cap - fbase;
    let ends = |r: &KfMapRun| r.va.saturating_add(r.len);
    // The best straddle-free boundary: every run start (and 0) is a candidate, the most runs
    // within `allowed` wins (ties name the same set).
    let mut best: (usize, u64) = (0, 0);
    for v in s.iter().map(|r| r.va).chain(std::iter::once(0)) {
        let below = s.iter().filter(|r| r.va < v).count();
        let straddled = s.iter().any(|r| r.va < v && v < ends(r));
        if !straddled && below <= allowed && below >= best.0 {
            best = (below, v);
        }
    }
    if best.0 == 0 && fbase == 0 {
        out.entries = vec![Vec::new(); n];
        out.truncated = true;
        return out;
    }
    out.is_cut = true;
    for (t, st) in staged.iter().enumerate() {
        if t == f {
            out.entries[t] = s.iter().filter(|r| r.va < best.1).copied().collect();
        } else if t > f {
            out.entries[t] = Vec::new();
        }
        out.cut[t] = t >= f && out.entries[t].len() < st.len();
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
                let mut m = KfMapRun {
                    op: KFWR_OP_MAP,
                    pdb_index: 0,
                    ..*r
                };
                m.flags &= !KFWR_RF_HELD;
                if code == AckCode::Held {
                    m.flags |= KFWR_RF_HELD;
                }
                add.push(m);
            }
        }
        let kept: Vec<KfMapRun> = p
            .iter()
            .zip(&rm)
            .filter(|(_, r)| !**r)
            .map(|(x, _)| *x)
            .collect();
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
    coverage_with(runs, KFWR_RF_KEY_PERM_DEFAULT)
}

/// [`coverage`] under an explicit key policy.
#[must_use]
pub fn coverage_with(runs: &[KfMapRun], key_perm: u32) -> [Vec<(u64, u64, u32, u64)>; CLASSES] {
    let mut out: [Vec<(u64, u64, u32, u64)>; CLASSES] = Default::default();
    for (c, slot) in out.iter_mut().enumerate() {
        let mut v: Vec<(u64, u64, u32, u64)> = runs
            .iter()
            .filter(|r| class_of(r.flags) == c)
            .map(|r| (r.va, r.len, host_key_with(r.flags, key_perm), r.gpga))
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
    use crate::abi::{
        KFWR_RF_ATOMIC_DISABLE, KFWR_RF_KEY_PERM_ALL, KFWR_RF_PRIVILEGE, KFWR_RF_READ_ONLY,
        KFWR_RF_VOLATILE,
    };
    use crate::abi::{PS_64K, RF_PS};

    const PAGE: u64 = 0x1000;

    fn run(va: u64, gpga: u64, len: u64, flags: u32) -> KfMapRun {
        KfMapRun {
            va,
            gpga,
            len,
            flags,
            op: KFWR_OP_MAP,
            pdb_index: 0,
        }
    }
    const SYS: u32 = RF_AP.put(AP_SYS as u32);
    const BIG: u32 = RF_PS.put(PS_64K as u32);
    /// The four permission bits a guest can flip in place.
    const PERM_FLIPS: [u32; 4] = [
        KFWR_RF_READ_ONLY,
        KFWR_RF_ATOMIC_DISABLE,
        KFWR_RF_VOLATILE,
        KFWR_RF_PRIVILEGE,
    ];

    /// ★ The named-range rewrite of [`class_of`] and [`host_key_with`] (`STATUS_AND_HANDOFF.md` §4
    /// item 6) is BEHAVIOUR-PRESERVING: both still compute, bit for bit, the formulas they were
    /// written as — and that `kf_hkey`/`kf_pcls` spell in `kf_walk.cu` — over flags words that
    /// exercise every field, including page-size codes above 3 (the `& 3` class mask aliases
    /// them, as on the GPU) and bits outside every named range.
    #[test]
    fn the_named_ranges_compute_the_literal_formulas() {
        let literal_class = |f: u32| ((f >> 8) & 0x3) as usize;
        let literal_key = |f: u32, kp: u32| {
            let ap = match f & 0x7 {
                0 => 0,
                2 | 3 => 1,
                _ => 2,
            };
            // ★ ABI 7: `| ((flags & KFWR_RF_SPARSE) ? (1u << 14) : 0u)` (`kf_hkey`).
            ap | (((f >> 16) & 0xff) << 2) | (((f & kp & 0x78) >> 3) << 10) | (((f >> 7) & 1) << 14)
        };
        assert_eq!(SYS, 2);
        assert_eq!(BIG, 1 << 8);
        assert_eq!(PERM_FLIPS, [1 << 3, 1 << 4, 1 << 5, 1 << 6]);
        assert_eq!(KFWR_RF_KEY_PERM_ALL, 0x78, "the permission bits are 3..=6");
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut words: Vec<u32> = (0u32..=0xF)
            .flat_map(|ps| (0u32..8).map(move |ap| (ps << 8) | ap))
            .collect();
        words.extend([u32::MAX, KFWR_RF_HELD, 0xFF << 16, KFWR_RF_KEY_PERM_ALL]);
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            words.push(x as u32);
        }
        for &f in &words {
            assert_eq!(class_of(f), literal_class(f), "class_of({f:#x})");
            for kp in [
                0,
                KFWR_RF_KEY_PERM_DEFAULT,
                KFWR_RF_KEY_PERM_ALL,
                u32::MAX,
                f.rotate_left(7),
            ] {
                assert_eq!(
                    host_key_with(f, kp),
                    literal_key(f, kp),
                    "host_key_with({f:#x}, {kp:#x})"
                );
            }
        }
    }

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
                assert!(
                    w[0].va + w[0].len <= w[1].va,
                    "committed not sorted/disjoint: {w:x?}"
                );
            }
        }
    }

    #[test]
    fn a_first_walk_maps_every_run_and_then_is_quiet() {
        let walk = vec![
            run(0x10_0000, 0x20_0000, 3 * PAGE, 0),
            run(0x20_0000, 0x5000, PAGE, SYS),
        ];
        let d = diff(&Committed::default(), &walk, 1 << 14);
        assert_eq!(d.runs.len(), 2);
        assert!(d.runs.iter().all(|r| r.op == KFWR_OP_MAP));
        let c = commit(&Committed::default(), &d.runs, &ok(2));
        assert!(
            diff(&c, &walk, 1 << 14).runs.is_empty(),
            "a settled slot diffs empty"
        );
        assert_eq!(coverage(&c.flat()), coverage(&walk));
    }

    /// ★ Commit-on-ack: a FAILED map is still a difference next time; a successful one is never
    /// re-emitted.
    #[test]
    fn a_failed_map_is_retried_and_a_successful_one_never_re_emitted() {
        let walk = vec![run(0x1000, 0xA000, PAGE, 0), run(0x3000, 0xC000, PAGE, 0)];
        let d = diff(&Committed::default(), &walk, 64);
        let c = commit(
            &Committed::default(),
            &d.runs,
            &[AckCode::Applied, AckCode::Failed],
        );
        let d2 = diff(&c, &walk, 64);
        assert_eq!(
            d2.runs,
            vec![KfMapRun {
                op: KFWR_OP_MAP,
                ..walk[1]
            }],
            "only the failed map, again"
        );
    }

    /// An unmap names a WHOLE placement; the still-backed parts of it are mapped again.
    #[test]
    fn a_partly_changed_placement_is_unmapped_whole_and_its_rest_remapped() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 4 * PAGE, 0)]);
        // The guest re-points page 2.
        let walk = vec![
            run(0, 0x10_0000, 2 * PAGE, 0),
            run(2 * PAGE, 0x90_0000, PAGE, 0),
            run(3 * PAGE, 0x10_3000, PAGE, 0),
        ];
        let d = diff(&c, &walk, 64);
        assert_eq!(
            d.runs[0],
            KfMapRun {
                op: KFWR_OP_UNMAP,
                ..c.cls[0][0]
            }
        );
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
        let old = vec![
            run(0x1210_1000, 0x20_0000, PAGE, 0),
            run(0x1210_2000, 0x21_0000, PAGE, 0),
        ];
        let c = settle(&Committed::default(), &old);
        let new_root_walk = vec![
            run(0x1210_1000, 0x20_0000, PAGE, 0),
            run(0x5000_0000, 0x40_0000, PAGE, 0),
        ];
        let d = diff(&c, &new_root_walk, 64);
        assert_eq!(
            d.runs,
            vec![
                KfMapRun {
                    op: KFWR_OP_UNMAP,
                    ..old[1]
                },
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
        assert!(
            diff(&c, &walk, 64).runs.is_empty(),
            "a held placement is kept while the walk agrees"
        );
        let d = diff(&c, &[], 64);
        assert_eq!(d.runs.len(), 1);
        assert_ne!(
            d.runs[0].flags & KFWR_RF_HELD,
            0,
            "its unmap says it was never ours"
        );
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
        assert!(
            diff(&c, &[rw], 64).runs.is_empty(),
            "control: an unchanged walk is quiet"
        );
        let ro = run(0x10_0000, 0x20_0000, 4 * PAGE, SYS | KFWR_RF_READ_ONLY);
        let d = diff(&c, &[ro], 64);
        assert_eq!(
            d.runs,
            vec![
                KfMapRun {
                    op: KFWR_OP_UNMAP,
                    ..rw
                },
                ro
            ],
            "a permission downgrade must retire the RW placement and place the RO one"
        );
        let c = commit(&c, &d.runs, &ok(d.runs.len()));
        assert_eq!(
            c.cls[0],
            vec![ro],
            "the slot now records the placement as read-only"
        );
        assert!(
            diff(&c, &[ro], 64).runs.is_empty(),
            "and is quiet once it landed"
        );
        // The upgrade back (a collapse re-grants write) is a change too.
        let d = diff(&c, &[rw], 64);
        assert_eq!(
            d.runs,
            vec![
                KfMapRun {
                    op: KFWR_OP_UNMAP,
                    ..ro
                },
                rw
            ]
        );
        // ⊘ A refused remap stays a difference: the RW placement is still what the host holds.
        let c2 = settle(&Committed::default(), &[rw]);
        let d = diff(&c2, &[ro], 64);
        let c2 = commit(&c2, &d.runs, &[AckCode::Applied, AckCode::Failed]);
        assert!(
            c2.is_empty(),
            "the unmap landed, the RO map did not: nothing is placed"
        );
        assert_eq!(
            diff(&c2, &[ro], 64).runs,
            vec![ro],
            "and the RO map is retried"
        );
    }

    /// ★ The DEFAULT key: READ_ONLY and VOLATILE (carried to the host map) and PRIVILEGE (a user
    /// twin withholds a privileged leaf, so its flip re-decides the placement) each re-map;
    /// ATOMIC_DISABLE is keyed only when the host carries it (`KF3_CARRY_ATOMIC_DISABLE`), so by
    /// default its flip is quiet — and under [`KFWR_RF_KEY_PERM_ALL`] it re-maps too.
    #[test]
    fn the_key_is_the_hosts_policy() {
        let base = run(0, 0x40_0000, 2 * PAGE, 0);
        let c = settle(&Committed::default(), &[base]);
        for bit in [KFWR_RF_READ_ONLY, KFWR_RF_VOLATILE, KFWR_RF_PRIVILEGE] {
            let w = KfMapRun {
                flags: base.flags | bit,
                ..base
            };
            assert_eq!(
                diff(&c, &[w], 64).runs.len(),
                2,
                "flag {bit:#x} must re-map"
            );
            assert_ne!(host_key(w.flags), host_key(base.flags));
        }
        let ad = KfMapRun {
            flags: base.flags | KFWR_RF_ATOMIC_DISABLE,
            ..base
        };
        assert!(
            diff(&c, &[ad], 64).runs.is_empty(),
            "ATOMIC_DISABLE is not keyed by default"
        );
        assert_eq!(
            diff_with(&c, &[ad], 64, KFWR_RF_KEY_PERM_ALL).runs.len(),
            2,
            "…and is when the host carries it"
        );
        // A bit outside the permission set never joins the key, whatever the policy says.
        assert_eq!(
            host_key_with(base.flags | KFWR_RF_HELD, u32::MAX),
            host_key_with(base.flags, u32::MAX)
        );
    }

    /// A downgrade of PART of a placement retires the whole placement (the host unmaps by the VA
    /// a map was placed at) and maps both halves with their own permissions.
    #[test]
    fn a_partial_downgrade_splits_the_placement_by_permission() {
        let c = settle(&Committed::default(), &[run(0, 0x10_0000, 4 * PAGE, 0)]);
        let walk = vec![
            run(0, 0x10_0000, 2 * PAGE, 0),
            run(2 * PAGE, 0x10_2000, 2 * PAGE, KFWR_RF_READ_ONLY),
        ];
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
        assert_eq!(
            d.runs.iter().map(|r| r.op).collect::<Vec<_>>(),
            vec![KFWR_OP_UNMAP, KFWR_OP_MAP]
        );
    }

    #[test]
    fn a_diff_that_could_overflow_the_slot_withholds_its_maps() {
        let c = settle(
            &Committed::default(),
            &[
                run(0, 0x10_0000, PAGE, 0),
                run(2 * PAGE, 0x20_0000, PAGE, 0),
            ],
        );
        let walk = vec![
            run(2 * PAGE, 0x30_0000, PAGE, 0),
            run(4 * PAGE, 0x40_0000, PAGE, 0),
        ];
        let d = diff(&c, &walk, 3);
        assert!(d.partial);
        assert!(d.runs.iter().all(|r| r.op == KFWR_OP_UNMAP));
        let c = commit(&c, &d.runs, &ok(d.runs.len()));
        let d = diff(&c, &walk, 3);
        assert!(!d.partial && d.runs.len() == 2);
        let full = settle(
            &Committed::default(),
            &[run(0, 0, PAGE, 0), run(PAGE * 2, 0, PAGE, 0)],
        );
        assert!(
            diff(
                &full,
                &[
                    run(0, 0, PAGE, 0),
                    run(PAGE * 2, 0, PAGE, 0),
                    run(PAGE * 9, 0, PAGE, 0)
                ],
                2
            )
            .overflow
        );
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
                let flags = RF_PS.put(c)
                    | if r.below(4) == 0 { SYS } else { 0 }
                    | if r.below(5) == 0 {
                        KFWR_RF_READ_ONLY
                    } else {
                        0
                    };
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
                1 => v.push(KfMapRun {
                    gpga: r.below(64) * PAGE,
                    ..*x
                }),
                // ★ v3-roperm: a permission flip at the same backing (RW↔RO, atomics, cache,
                // privilege) — the in-place downgrade UVM's read duplication performs.
                3 => v.push(KfMapRun {
                    flags: x.flags ^ PERM_FLIPS[r.below(4) as usize],
                    ..*x
                }),
                2 if x.len > PAGE => {
                    v.push(KfMapRun { len: PAGE, ..*x });
                    v.push(KfMapRun {
                        va: x.va + PAGE,
                        gpga: r.below(64) * PAGE,
                        len: x.len - PAGE,
                        ..*x
                    });
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
                    assert!(
                        p.iter().any(|x| x.va == u.va && x.len == u.len),
                        "unmap of a non-placement {u:x?}"
                    );
                }
                // The host refuses at random; a map overlapping a refused unmap is not attempted.
                let mut codes: Vec<AckCode> = Vec::with_capacity(d.runs.len());
                let mut failed_unmaps: Vec<(usize, u64, u64)> = Vec::new();
                for x in &d.runs {
                    let c = class_of(x.flags);
                    let blocked = x.op == KFWR_OP_MAP
                        && failed_unmaps
                            .iter()
                            .any(|&(fc, a, b)| fc == c && x.va < b && a < x.va + x.len);
                    let code = if blocked || r.below(4) == 0 {
                        AckCode::Failed
                    } else {
                        AckCode::Applied
                    };
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
                    let again = d2.runs.iter().any(|y| {
                        y.op == x.op && y.va == x.va && y.len == x.len && y.gpga == x.gpga
                    });
                    if *code == AckCode::Applied {
                        assert!(!again, "an applied entry was re-emitted: {x:x?}");
                    } else if x.op == KFWR_OP_UNMAP {
                        assert!(again, "a refused unmap was not retried: {x:x?}");
                    } else {
                        // A refused map's bytes are still unplaced: covered by the next diff's maps.
                        let cov = coverage(
                            &d2.runs
                                .iter()
                                .filter(|y| y.op == KFWR_OP_MAP)
                                .copied()
                                .collect::<Vec<_>>(),
                        );
                        let mine = coverage(&[*x]);
                        let c = class_of(x.flags);
                        let (va, len, _, _) = mine[c][0];
                        assert!(
                            cov[c]
                                .iter()
                                .any(|&(a, l, _, _)| a <= va && va + len <= a + l),
                            "a refused map was not retried: {x:x?}"
                        );
                    }
                }
                com = next;
            }
            // Drain: everything acknowledged ⇒ closure.
            for _ in 0..3 {
                com = settle(&com, &walk);
            }
            assert_eq!(coverage(&com.flat()), coverage(&walk), "closure");
            assert!(
                diff(&com, &walk, 1 << 14).runs.is_empty(),
                "settled ⇒ quiet"
            );
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
    // ── ★ 2026-11-15 (M4): the report's own capacity — `cut_report`, the spec of kf_diff_emit's cut ──

    fn unmap(va: u64, len: u64, flags: u32) -> KfMapRun {
        KfMapRun {
            op: KFWR_OP_UNMAP,
            gpga: 0,
            ..run(va, 0, len, flags)
        }
    }
    fn overlaps(a: &KfMapRun, b: &KfMapRun) -> bool {
        a.va < b.va + b.len && b.va < a.va + a.len
    }

    /// The vectors `cuda/walk/kf_cpu_walk.cpp` (`emit_cut_cases`) runs on the REAL kernel source:
    /// the same staged diffs, the same capacities, the same expectations. Two statements of one spec.
    #[test]
    fn cut_report_matches_the_kernel_vectors() {
        // the M4 shape: one entry, twice the capacity, 200 maps 8 KiB apart
        let maps: Vec<KfMapRun> = (0..200u64)
            .map(|i| run(0x100_0000 + 0x2000 * i, 0x20_0000, PAGE, SYS))
            .collect();
        let c = cut_report(std::slice::from_ref(&maps), 100);
        assert!(c.is_cut && !c.truncated && c.cut == vec![true]);
        assert_eq!(c.entries[0], maps[..100].to_vec(), "the lowest 100 runs");
        // exactly full is whole
        let c = cut_report(&[maps[..100].to_vec()], 100);
        assert!(!c.is_cut && c.cut == vec![false] && c.entries[0].len() == 100);
        // a page-size change: the class-1 UNMAP travels with the class-0 maps that overlap it
        let staged = [
            unmap(0x10000, 0x10000, BIG | SYS),
            run(0x10000, 0x1000, PAGE, SYS),
            run(0x11000, 0x1000, PAGE, SYS),
            run(0x12000, 0x1000, PAGE, SYS),
            run(0x100000, 0x1000, PAGE, SYS),
            run(0x102000, 0x1000, PAGE, SYS),
        ];
        // (staged order is class 0 first, so put the class-0 maps first as `diff` would)
        let mut staged_order = vec![
            staged[1], staged[2], staged[3], staged[4], staged[5], staged[0],
        ];
        staged_order.sort_by_key(|r| (class_of(r.flags), r.op != KFWR_OP_UNMAP));
        let c = cut_report(&[staged_order.clone()], 4);
        assert!(c.is_cut && !c.truncated);
        assert_eq!(
            c.entries[0].len(),
            4,
            "the clump and the maps below the far ones"
        );
        assert!(
            c.entries[0].iter().any(|r| r.op == KFWR_OP_UNMAP),
            "the unmap is reported with the maps over it"
        );
        // ... where a staged-order PREFIX of 4 would not carry it
        assert!(staged_order[..4].iter().all(|r| r.op != KFWR_OP_UNMAP));
        // one overlap-connected clump larger than the report: the legacy refusal
        let mut clump = vec![unmap(0x10000, 0x10_0000, BIG | SYS)];
        clump.extend((0..8u64).map(|i| run(0x10000 + 0x1000 * i, 0x1000, PAGE, SYS)));
        clump.sort_by_key(|r| (class_of(r.flags), r.op != KFWR_OP_UNMAP));
        let c = cut_report(&[clump], 4);
        assert!(c.truncated && !c.is_cut && c.entries == vec![Vec::new()]);
        // three entries: whole, cut, withheld
        let e = |n: u64| -> Vec<KfMapRun> {
            (0..n)
                .map(|i| run(0x1000 * (2 * i + 1), 0x1000, PAGE, SYS))
                .collect()
        };
        let c = cut_report(&[e(4), e(20), e(3)], 10);
        assert_eq!(
            (c.entries[0].len(), c.entries[1].len(), c.entries[2].len()),
            (4, 6, 0)
        );
        assert_eq!(c.cut, vec![false, true, true]);
    }

    /// A random staged diff: eight sorted disjoint lists on a small page grid, so that lists
    /// overlap and straddle each other.
    fn random_staged(x: &mut u64, dense: u64) -> Vec<KfMapRun> {
        let mut rnd = |n: u64| {
            *x ^= *x << 13;
            *x ^= *x >> 7;
            *x ^= *x << 17;
            (*x >> 11) % n
        };
        let mut v = Vec::new();
        for cls in 0..4u32 {
            for m in 0..2 {
                let mut p = rnd(6);
                for _ in 0..rnd(2 + 4 * dense) {
                    if p >= 118 {
                        break;
                    }
                    let len = 1 + rnd(5);
                    let f = SYS | RF_PS.put(cls);
                    let r = if m == 1 {
                        run(0x10_0000 + PAGE * p, 0x20_0000, PAGE * len, f)
                    } else {
                        unmap(0x10_0000 + PAGE * p, PAGE * len, f)
                    };
                    v.push(r);
                    p += len + rnd(4) + if dense == 0 { 6 } else { 0 };
                }
            }
        }
        v
    }

    /// ★ The cut is a WINDOW: within the capacity, a boundary no run straddles, every run that
    /// overlaps a reported run reported with it — over random staged diffs and capacities. And the
    /// counter-model: a PREFIX of the staged order violates exactly that, often.
    #[test]
    fn a_cut_report_is_a_closed_window_and_a_staged_prefix_is_not() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let (mut cuts, mut whole, mut refused, mut prefix_breaks) = (0, 0, 0, 0);
        for _ in 0..4000 {
            let cap = 3 + (x % 30) as usize;
            let dense = (x >> 9) % 3;
            let s = random_staged(&mut x, dense);
            let c = cut_report(std::slice::from_ref(&s), cap);
            if s.len() <= cap {
                assert!(!c.is_cut && !c.truncated && c.entries[0] == s);
                whole += 1;
                continue;
            }
            // the staged-order prefix a naive cut would report
            let prefix = &s[..cap];
            if s[cap..]
                .iter()
                .any(|u| prefix.iter().any(|w| overlaps(u, w)))
            {
                prefix_breaks += 1;
            }
            if c.truncated {
                refused += 1;
                assert!(c.entries[0].is_empty() && !c.is_cut);
                continue;
            }
            cuts += 1;
            let rep = &c.entries[0];
            assert!(c.is_cut && c.cut[0] && !rep.is_empty() && rep.len() <= cap);
            let v = s
                .iter()
                .filter(|r| !rep.contains(r))
                .map(|r| r.va)
                .min()
                .unwrap();
            assert!(
                rep.iter().all(|r| r.va < v),
                "a window: reported runs start below the boundary"
            );
            for u in s.iter().filter(|r| !rep.contains(r)) {
                assert!(
                    rep.iter().all(|w| !overlaps(u, w)),
                    "closed: {u:x?} overlaps a reported run"
                );
            }
            // maximal: no larger straddle-free boundary reports more within the capacity
            for b in s.iter().map(|r| r.va).filter(|&b| b > v) {
                let below = s.iter().filter(|r| r.va < b).count();
                let straddled = s.iter().any(|r| r.va < b && b < r.va + r.len);
                assert!(
                    straddled || below > cap,
                    "boundary {b:#x} would report {below}"
                );
            }
        }
        assert!(
            cuts > 500 && whole > 200 && refused > 20,
            "{whole}/{cuts}/{refused}"
        );
        assert!(
            prefix_breaks > cuts / 4,
            "a staged-order prefix separates overlapping runs in {prefix_breaks} of {cuts} cuts"
        );
    }

    /// Across entries: the report never carries more than `cap`; entries before the cut entry are
    /// whole, the cut entry is a window, entries after it are withheld and say so.
    #[test]
    fn a_cut_report_across_entries_stays_within_the_capacity() {
        let mut x = 0xA076_1D64_78BD_642Fu64;
        let mut multi = 0;
        for _ in 0..3000 {
            let n = 2 + (x % 3) as usize;
            let staged: Vec<Vec<KfMapRun>> = (0..n)
                .map(|_| {
                    let dense = (x >> 5) % 2;
                    random_staged(&mut x, dense)
                })
                .collect();
            let cap = 3 + ((x >> 13) % 30) as usize;
            let c = cut_report(&staged, cap);
            let total: usize = staged.iter().map(Vec::len).sum();
            if total <= cap {
                assert!(!c.is_cut && !c.truncated && c.entries == staged);
                continue;
            }
            if c.truncated {
                assert!(c.entries.iter().all(Vec::is_empty));
                continue;
            }
            multi += 1;
            let sent: usize = c.entries.iter().map(Vec::len).sum();
            assert!(sent <= cap, "{sent} runs in a report of {cap}");
            let f = c.cut.iter().position(|&b| b).expect("some entry is cut");
            for (t, st) in staged.iter().enumerate() {
                if t < f {
                    assert_eq!(&c.entries[t], st, "entries before the cut are whole");
                } else if t > f {
                    assert!(
                        c.entries[t].is_empty(),
                        "entries after the cut are withheld"
                    );
                    assert_eq!(c.cut[t], !st.is_empty());
                }
            }
        }
        assert!(multi > 500, "{multi}");
    }

    /// ★ Review of PR #424, F3: a brick CHAIN of overlapping runs (UNMAP `[2k, 2k+2)` pages, MAP
    /// `[2k+1, 2k+3)` pages) is ONE clump however long. The first kernel walked down one straddler at a
    /// time and gave up after 4096 steps, refusing a report that had a window; the spec never had a
    /// step limit and the kernel has none now. (The kernel's 12 000-run case at capacity 5 000 is in
    /// `kf_cpu_walk.cpp`; the spec's reference is quadratic, so the chain here is shorter.)
    #[test]
    fn a_chain_of_overlapping_runs_is_one_clump_however_long() {
        let chain = |len: u64, iso: u64| -> Vec<KfMapRun> {
            let mut v: Vec<KfMapRun> = (0..iso)
                .map(|i| run(0x10_0000 + 0x4000 * i, 0x20_0000, PAGE, SYS))
                .collect();
            for k in 0..len {
                v.push(unmap(0x1000_0000 + 0x2000 * k, 0x2000, SYS));
                v.push(run(0x1000_0000 + 0x2000 * k + PAGE, 0x20_0000, 0x2000, SYS));
            }
            v.sort_by_key(|r| (class_of(r.flags), r.op != KFWR_OP_UNMAP, r.va));
            v
        };
        // longer than any step limit a kernel might carry, capacity above it: the 10 isolated runs
        let s = chain(1500, 10);
        let c = cut_report(std::slice::from_ref(&s), 700);
        assert!(c.is_cut && !c.truncated);
        assert_eq!(c.entries[0].len(), 10, "the window below the chain");
        assert!(c.entries[0].iter().all(|r| r.va < 0x1000_0000));
        // nothing below it: the chain alone is a clump larger than the report
        let c = cut_report(&[chain(1500, 0)], 700);
        assert!(c.truncated);
        // a report big enough for the whole chain takes it whole
        let c = cut_report(&[chain(300, 4)], 5000);
        assert!(!c.is_cut && c.entries[0].len() == 604);
        // chains of every small length at every small capacity: the spec and the closure agree
        for len in [1u64, 2, 3, 7, 40] {
            for iso in [0u64, 3] {
                let s = chain(len, iso);
                for cap in 1..(2 * s.len()) {
                    let c = cut_report(std::slice::from_ref(&s), cap);
                    if c.truncated {
                        assert!(
                            iso == 0 || cap < iso as usize,
                            "len {len} iso {iso} cap {cap}"
                        );
                        continue;
                    }
                    if !c.is_cut {
                        continue;
                    }
                    let rep = &c.entries[0];
                    assert!(rep.len() <= cap);
                    for u in s.iter().filter(|r| !rep.contains(r)) {
                        assert!(rep.iter().all(|w| !overlaps(u, w)));
                    }
                }
            }
        }
    }

    /// Hostile arguments: a capacity of zero, of one, and of everything; nothing staged; runs at the top
    /// of the address space (the end of a run saturates, it never wraps).
    #[test]
    fn cut_report_hostile_arguments() {
        let one = |va: u64| run(va, 0x20_0000, PAGE, SYS);
        // capacity 0: nothing can be reported, the legacy refusal
        let c = cut_report(&[vec![one(0x1000), one(0x3000)]], 0);
        assert!(c.truncated && !c.is_cut && c.entries == vec![Vec::new()]);
        // capacity 1: the lowest run
        let c = cut_report(&[vec![one(0x1000), one(0x3000)]], 1);
        assert_eq!(c.entries[0], vec![one(0x1000)]);
        // nothing staged, any capacity: whole, quiet
        for cap in [0, 1, usize::MAX] {
            let c = cut_report(&[Vec::new(), Vec::new()], cap);
            assert!(!c.is_cut && !c.truncated && c.cut == vec![false, false]);
            assert_eq!(cut_report(&[], cap), CutReport::default());
        }
        // an unbounded capacity: whole
        let big: Vec<KfMapRun> = (0..50u64).map(|i| one(0x1000 * (2 * i + 1))).collect();
        let c = cut_report(std::slice::from_ref(&big), usize::MAX);
        assert!(!c.is_cut && c.entries[0] == big);
        // the top of the space: a run whose end is u64::MAX saturates; a run at u64::MAX - 0x1000 too
        let top = vec![
            run(u64::MAX - 0x2000, 0, 0x1000, SYS),
            run(u64::MAX - 0x1000, 0, 0x1000, SYS),
            run(u64::MAX - 0x1000, 0, u64::MAX, SYS),
        ];
        let c = cut_report(&[top], 1);
        assert!(c.is_cut || c.truncated, "no panic, and a decision");
        // the same run staged twice (a hostile diff with a duplicate): one clump
        let c = cut_report(&[vec![one(0x1000), one(0x1000), one(0x1000)]], 2);
        assert!(
            c.truncated,
            "three runs on one page are a clump larger than two"
        );
    }

    /// ★ Draining a big diff through capacity-bounded windows, each applied and committed before the
    /// next walk, ends where the whole diff ends — every step makes progress, and the diff is empty
    /// only at the end. (Closed under overlap + committed per run ⇒ the rest stays in the next diff.)
    #[test]
    fn draining_a_diff_through_windows_reaches_the_uncut_state() {
        let mut x = 0xD1B5_4A32_D192_ED03u64;
        let (mut multi, mut refused, mut cases) = (0, 0, 0);
        for _ in 0..1500 {
            // an old state, settled, and a new walk
            let dense = x % 2;
            let mk = |x: &mut u64| -> Vec<KfMapRun> {
                random_staged(x, dense)
                    .into_iter()
                    .filter(|r| r.op == KFWR_OP_MAP)
                    .collect()
            };
            let old = mk(&mut x);
            let new = mk(&mut x);
            let cap = 4 + (x % 20) as usize;
            let c0 = settle(&Committed::default(), &old);
            let uncut = settle(&c0, &new);
            let mut com = c0.clone();
            let mut steps = 0;
            let drained = loop {
                let d = diff(&com, &new, usize::MAX);
                if d.runs.is_empty() {
                    break true;
                }
                let c = cut_report(std::slice::from_ref(&d.runs), cap);
                if c.truncated {
                    break false;
                }
                let rep = &c.entries[0];
                assert_eq!(c.cut[0], rep.len() < d.runs.len());
                for u in d.runs.iter().filter(|r| !rep.contains(r)) {
                    assert!(rep.iter().all(|w| !overlaps(u, w)));
                }
                let next = commit(&com, rep, &ok(rep.len()));
                assert_ne!(next, com, "every window makes progress");
                assert_sound(&next);
                com = next;
                steps += 1;
                assert!(steps < 5000);
            };
            if !drained {
                refused += 1;
                continue;
            }
            cases += 1;
            multi += usize::from(steps > 1);
            assert_eq!(
                coverage(&com.flat()),
                coverage(&uncut.flat()),
                "same coverage as the uncut apply"
            );
            assert_eq!(com, uncut, "and the same committed placements");
        }
        assert!(
            cases > 400 && multi > 150,
            "{cases} drained, {multi} in more than one window, {refused} refused"
        );
    }
}
