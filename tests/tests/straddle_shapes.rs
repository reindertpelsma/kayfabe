//! ★★★★★ **WHY 255 BINDINGS STRADDLE** — `w277`, the rung that pays `w265`'s unpaid residue.
//!
//! `[measured, w276b_on, `traces/boots/w276/run_w276b_on_qemu.log.gz`]` every one of the
//! whole-VAS sweep's 255 refusals was `StraddlesLiveBinding`, on **every one of 88
//! doorbells**, and the refusal carried a **virtual address and nothing else**:
//!
//! ```text
//! refusals=255 by_kind={"StraddlesLiveBinding": 255}
//! refused_vas=[0x203e90000,…,0x203f00000,0x203fb0000,…,0x203fbf000] ⚠⚠ CAPPED at 24 of 255
//! ```
//!
//! A histogram with one bucket. That number is equally consistent with **four** causes that
//! want opposite fixes — a page-size mismatch, an extent mismatch, a stale binding, and two
//! populate sources contradicting each other — and the payload could not tell them apart.
//! `a_count_cannot_see_a_substitution`, one plane over.
//!
//! # The two things this file pins, and why they are in one file
//!
//! 1. ★★★ **The refusal now carries BOTH shapes**, and classifies the pair on two independent
//!    axes: [`StraddleShape`] (the geometry) and [`StraddleAgreement`] (**do the two shapes
//!    describe the same byte?**). The second is the one that turns a count into a finding: a
//!    straddle whose shapes *agree* is one fact at two granularities and indicts nobody; a
//!    straddle that *contradicts* is two sources disagreeing about what backs a VA, which is
//!    the class `w222`'s bad join came from.
//! 2. ★★★★★ **THE THIRD OUTCOME EXISTS AND IS NOW NAMED.** `w276` found the faulting VA was in
//!    *no* bound list and *no* refused list. There was a third exit all along: the settlement's
//!    desired set is a map keyed on the leaf's **virtual address alone**, so two leaves at one
//!    VA with different sizes collided and the loser was dropped with **no counter, no refusal
//!    and no log line**. That is `w270`'s disease — *the key was the VA, not the extent* — one
//!    layer up.
//!
//! # ⊘ What this file does NOT claim
//!
//! It does not decide **which** of two overlapping guest declarations should win. RM's own
//! walker guarantees the situation cannot arise in a settled tree — `_mmuWalkResolveSubLevelConflicts`
//! **invalidates the other sub-table over the VA range** on every map, unmap and sparsify
//! (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk.c:476-488, :1066-1092`) — and the source
//! nowhere states what hardware does if a VA *is* valid in both. ⇒ a collision is an **anomaly
//! to report**, not a tie to break, and inventing a precedence rule here would be the silent
//! overwrite again with a comment on it.
//!
//! ⊘ Mock-level and GPU-free. These bound the logic; what the real guest's tables actually
//! collide on is a boot measurement and is not in here.

use std::collections::BTreeMap;

use kayfabe_arch::ids::{GpuId, GpuVa, Pdb};
use kayfabe_arch::{Aperture, Arch, GmmuFmt, PageSize};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_mmu::reach::ReachShadow;
use kayfabe_mmu::walker::{
    DecodedLeaf, FbRead, PopulateRefusal, PtPage, Straddle, StraddleAgreement, StraddleShape,
    decode_page, populate,
};
use kayfabe_mmu::{AddressTable, Binding, HostBacking};
use kayfabe_mocks::{MOCK_DUAL_LEVEL, MockArch, MockGmmuFmt};

const A_PDB: Pdb = Pdb(0x1001_0000);
const ROOT: u64 = A_PDB.0;
const PD_L1: u64 = 0x1002_0000;
const PD_L2: u64 = 0x1003_0000;
const PD_DUAL: u64 = 0x1004_0000;
const PT_SMALL: u64 = 0x1005_0000;
const PT_BIG: u64 = 0x1007_0000;

// =====================================================================================
// Scaffolding — the same shapes `reachability.rs` uses, so a reader comparing the two is
// comparing two behaviours and not two fixtures.
// =====================================================================================

#[derive(Default)]
struct Fb {
    pages: BTreeMap<u64, Vec<u8>>,
}

impl Fb {
    fn put(&mut self, phys: u64, bytes: Vec<u8>) {
        self.pages.insert(phys, bytes);
    }
}

impl FbRead for Fb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        match self.pages.get(&phys) {
            Some(v) if v.len() >= buf.len() => {
                buf.copy_from_slice(&v[..buf.len()]);
                true
            }
            _ => false,
        }
    }
}

fn page_at(fmt: &dyn GmmuFmt, level: u8, set: &[(usize, u128)]) -> Vec<u8> {
    let g = fmt.level_shift(level).expect("a level the regime has");
    let width = usize::from(fmt.entry_size(level));
    let mut v = vec![0u8; g.entries as usize * width];
    for &(i, e) in set {
        let at = i * width;
        v[at..at + width].copy_from_slice(&e.to_le_bytes()[..width]);
    }
    v
}

fn at(phys: u64, level: u8, vabase: u64) -> PtPage {
    PtPage {
        phys,
        aperture: Aperture::Vidmem,
        level,
        vabase,
    }
}

fn observe(shadow: &mut ReachShadow, fmt: &dyn GmmuFmt, fb: &mut Fb, page: PtPage) {
    let d = decode_page(fmt, fb, page).expect("the synthetic source serves this page");
    shadow
        .observe(page, &d)
        .expect("within the shadow's bounds");
}

/// The small-page leaf table's level; its sibling is the big-page one.
const SMALL_LEVEL: u8 = MOCK_DUAL_LEVEL + 1;
const BIG_LEVEL: u8 = MOCK_DUAL_LEVEL + 2;

fn decoded(va: u64, phys: u64, size: u64, level: u8) -> DecodedLeaf {
    DecodedLeaf {
        va: GpuVa(va),
        phys,
        aperture: Aperture::Vidmem,
        size: PageSize(size),
        read_only: false,
        level,
    }
}

/// A binding with no host object — a declaration, which is what a decode produces.
fn declared(phys: u64) -> Binding {
    Binding::declared_by_guest(phys, Aperture::Vidmem).expect("vidmem is kind 2")
}

fn straddle_of(refusals: &[PopulateRefusal]) -> (GpuVa, Straddle) {
    match refusals {
        [PopulateRefusal::StraddlesLiveBinding { va, straddle }] => (*va, *straddle),
        other => panic!("expected exactly one straddle refusal, got {other:?}"),
    }
}

// =====================================================================================
// 1. THE GEOMETRY — all four shapes, each reached by a leaf a real regime can produce
// =====================================================================================

/// ★★★ **The four straddle geometries are distinguishable**, and each is reached by
/// populating a leaf over a live binding rather than by constructing a [`Straddle`] by hand.
///
/// ⊘ Constructing the value directly would test the classifier against itself. Every row here
/// goes through [`populate`], so the fields the classifier reads are the fields the bind site
/// actually fills — the two cannot drift.
#[test]
fn every_straddle_geometry_is_reached_through_populate_and_named() {
    let arch = MockArch::new();
    let fmt = arch.mmu();

    // 4 KiB leaf, same base, over a live 64 KiB binding: the coarse page went down first.
    // ⇒ SameStartShorter.
    // 4 KiB leaf one page in: the interior of a finer tiling. ⇒ InsideLarger.
    // 64 KiB leaf over a live 4 KiB binding at the same base. ⇒ SameStartLonger.
    // 64 KiB leaf starting in the last 4 KiB of a live 64 KiB binding. ⇒ CrossesEnd.
    let cases: [(u64, u64, u64, u64, StraddleShape); 4] = [
        (
            0x10_0000,
            0x1_0000,
            0x10_0000,
            0x1000,
            StraddleShape::SameStartShorter,
        ),
        (
            0x10_0000,
            0x1_0000,
            0x10_1000,
            0x1000,
            StraddleShape::InsideLarger,
        ),
        (
            0x10_0000,
            0x1000,
            0x10_0000,
            0x1_0000,
            StraddleShape::SameStartLonger,
        ),
        (
            0x10_0000,
            0x1_0000,
            0x10_f000,
            0x1_0000,
            StraddleShape::CrossesEnd,
        ),
    ];

    for (live_va, live_len, leaf_va, leaf_len, want) in cases {
        let mut t = AddressTable::default();
        t.bind(A_PDB, GpuVa(live_va), live_len, declared(0xF000_0000))
            .expect("a free range");
        let out = populate(
            fmt,
            &mut t,
            A_PDB,
            &[decoded(leaf_va, 0xF000_0000, leaf_len, SMALL_LEVEL)],
        );
        assert_eq!(out.bound, 0, "a straddling leaf never binds");
        let (va, s) = straddle_of(&out.refusals);
        assert_eq!(va.0, leaf_va);
        assert_eq!(
            s.shape(va),
            want,
            "leaf 0x{leaf_va:x}+0x{leaf_len:x} over live 0x{live_va:x}+0x{live_len:x}"
        );
        // ★ The payload is the whole reason the variant changed: both extents, verbatim.
        assert_eq!(
            (s.size.0, s.live_start, s.live_len),
            (leaf_len, live_va, live_len)
        );
    }
}

/// ★★★★★ **AGREEMENT IS COMPUTED AT THE LEAF'S OWN VA, NOT BETWEEN THE TWO BASES.**
///
/// The interior page of a perfectly consistent 4 KiB tiling over a 64 KiB binding has a
/// `phys` that differs from the binding's `phys` by exactly its offset. Comparing the two
/// base addresses would report `Contradicts` for **every** such row — a false alarm on the
/// single most common shape — which is `w270`'s mistake inverted: keyed on the base, not on
/// the extent.
#[test]
fn a_consistent_finer_tiling_agrees_and_an_inconsistent_one_contradicts() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    const LIVE_VA: u64 = 0x10_0000;
    const LIVE_PHYS: u64 = 0xF000_0000;

    // Consistent: the 4 KiB leaf three pages in names the byte the 64 KiB binding already
    // resolves that VA to.
    let mut t = AddressTable::default();
    t.bind(A_PDB, GpuVa(LIVE_VA), 0x1_0000, declared(LIVE_PHYS))
        .expect("free");
    let out = populate(
        fmt,
        &mut t,
        A_PDB,
        &[decoded(
            LIVE_VA + 0x3000,
            LIVE_PHYS + 0x3000,
            0x1000,
            SMALL_LEVEL,
        )],
    );
    let (va, s) = straddle_of(&out.refusals);
    assert_eq!(
        s.agreement(va),
        StraddleAgreement::SameMemory,
        "a granularity restatement of one fact indicts neither source"
    );

    // Inconsistent: same VA, a different page. THIS is the row a reader must act on.
    let mut t = AddressTable::default();
    t.bind(A_PDB, GpuVa(LIVE_VA), 0x1_0000, declared(LIVE_PHYS))
        .expect("free");
    let out = populate(
        fmt,
        &mut t,
        A_PDB,
        &[decoded(
            LIVE_VA + 0x3000,
            LIVE_PHYS + 0x9000,
            0x1000,
            SMALL_LEVEL,
        )],
    );
    let (va, s) = straddle_of(&out.refusals);
    assert_eq!(
        s.agreement(va),
        StraddleAgreement::Contradicts,
        "two sources naming different memory for one VA is the finding"
    );

    // And an aperture move contradicts even when the arithmetic lines up — the address
    // table's answer includes which aperture it is in.
    let mut t = AddressTable::default();
    t.bind(A_PDB, GpuVa(LIVE_VA), 0x1_0000, declared(LIVE_PHYS))
        .expect("free");
    let mut leaf = decoded(LIVE_VA + 0x3000, LIVE_PHYS + 0x3000, 0x1000, SMALL_LEVEL);
    leaf.aperture = Aperture::SysmemCoherent;
    let out = populate(fmt, &mut t, A_PDB, &[leaf]);
    let (va, s) = straddle_of(&out.refusals);
    assert_eq!(s.agreement(va), StraddleAgreement::Contradicts);
}

/// ★★ **A straddle over a HOST-PUBLISHED binding is marked as such**, because the two states
/// want different work: an unpublished straddle could in principle be reconciled by unbinding
/// the old shape, and a published one cannot — that is
/// [`PopulateRefusal::RepointsPublished`]'s whole argument, and a refusal that cannot say
/// which it is sends its reader to the wrong plane.
#[test]
fn the_refusal_says_whether_the_live_binding_is_host_published() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    for published in [false, true] {
        let mut t = AddressTable::default();
        let b = if published {
            // ⊘ Sysmem: a `Vidmem` aperture with a host object is ruling 3's forbidden
            // state. The subject here is only whether the refusal REPORTS publication.
            Binding::real_gpu_memory(
                0x3000_0000,
                Aperture::SysmemCoherent,
                HostBacking::whole(
                    HostHandle::new(IsolateId::new(1, GpuId::ZERO), 7),
                    0x10_0000,
                    kayfabe_mmu::BackingBytes::SoleBacking,
                ),
            )
            .expect("host memory at the guest's own VA is kind 3")
        } else {
            declared(0xF000_0000)
        };
        t.bind(A_PDB, GpuVa(0x10_0000), 0x1_0000, b)
            .expect("a free range");
        let out = populate(
            fmt,
            &mut t,
            A_PDB,
            &[decoded(0x10_1000, 0xF000_1000, 0x1000, SMALL_LEVEL)],
        );
        let (_, s) = straddle_of(&out.refusals);
        assert_eq!(
            s.live_published, published,
            "the refusal must carry whether unbinding is even available"
        );
    }
}

/// ★ **The leaf's LEVEL travels with the refusal**, which is what names the *producer*.
/// On GA10x a straddle between level 4 (`PT_BIG`, 64 KiB) and level 5 (`PT_SMALL`, 4 KiB)
/// names the two halves of one 16-byte dual PD0 entry; a straddle whose leaf came from PD0
/// itself is a large-page split. A size alone cannot say which, because two producers can
/// emit the same size.
#[test]
fn the_refusal_carries_the_level_the_leaf_was_decoded_at() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut t = AddressTable::default();
    t.bind(A_PDB, GpuVa(0x10_0000), 0x1000, declared(0xF000_0000))
        .expect("free");
    let out = populate(
        fmt,
        &mut t,
        A_PDB,
        &[decoded(0x10_0000, 0xF000_0000, 0x1_0000, BIG_LEVEL)],
    );
    let (_, s) = straddle_of(&out.refusals);
    assert_eq!(s.level, BIG_LEVEL, "provenance, not just extent");
}

// =====================================================================================
// 2. THE THIRD OUTCOME — a leaf that is neither bound nor refused
// =====================================================================================

/// ★★★★★ **A DUAL PDE WHOSE TWO HALVES BOTH DESCRIBE ONE VA PRODUCES A LEAF THAT REACHES
/// NEITHER `bound` NOR `refusals` — and the settlement now says so by name.**
///
/// This is the whole of `w276`'s unexplained third state, built from the shape that produces
/// it in the field: one 16-byte dual slot naming a small-page table **and** a big-page table,
/// with a valid entry for the same virtual address in both. The desired set is keyed on the
/// VA, so one of the two is displaced before the address table ever sees it.
///
/// The assertions are deliberately three:
/// - the collision is **recorded** (it used to be invisible);
/// - it names **both** leaves — level, size, physical address **and the page each came from**,
///   because *"which producer described this VA twice"* is the only actionable half;
/// - the survivor is still proposed, so recording the loss did not cost the pass a bind.
///
/// ⊘ It does **not** assert which of the two wins. See this file's header: RM guarantees the
/// state cannot arise in a settled tree and the source never says what hardware would do, so
/// there is nothing here to be right about — only something to report.
#[test]
fn a_dual_pde_describing_one_va_twice_is_recorded_and_not_silently_dropped() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = ReachShadow::new(ROOT);

    // The dual slot: low half → small-page table, high half → big-page table.
    fb.put(
        ROOT,
        page_at(fmt, 0, &[(0, MockGmmuFmt::encode_pde(PD_L1, false, false))]),
    );
    fb.put(
        PD_L1,
        page_at(fmt, 1, &[(0, MockGmmuFmt::encode_pde(PD_L2, false, false))]),
    );
    fb.put(
        PD_L2,
        page_at(
            fmt,
            2,
            &[(0, MockGmmuFmt::encode_pde(PD_DUAL, false, false))],
        ),
    );
    fb.put(
        PD_DUAL,
        page_at(
            fmt,
            MOCK_DUAL_LEVEL,
            &[(0, MockGmmuFmt::encode_dual_pde(PT_SMALL, PT_BIG))],
        ),
    );
    // Slot 0 of each leaf table describes virtual address 0 — the same address, two sizes,
    // and (deliberately) two DIFFERENT physical pages, so a reader can tell which survived.
    fb.put(
        PT_SMALL,
        page_at(
            fmt,
            SMALL_LEVEL,
            &[(0, MockGmmuFmt::encode_leaf(0xA000_0000, false))],
        ),
    );
    fb.put(
        PT_BIG,
        page_at(
            fmt,
            BIG_LEVEL,
            &[(0, MockGmmuFmt::encode_leaf(0xB000_0000, false))],
        ),
    );

    for (phys, level) in [
        (ROOT, 0),
        (PD_L1, 1),
        (PD_L2, 2),
        (PD_DUAL, MOCK_DUAL_LEVEL),
        (PT_SMALL, SMALL_LEVEL),
        (PT_BIG, BIG_LEVEL),
    ] {
        s.witness(phys);
        observe(&mut s, fmt, &mut fb, at(phys, level, 0));
    }

    let settled = s.settle(fmt);

    assert_eq!(
        settled.shape_collisions.len(),
        1,
        "one VA described twice ⇒ exactly one recorded collision (was: silence)"
    );
    let c = settled.shape_collisions[0];
    assert_eq!(c.va, GpuVa(0), "the address both halves claim");
    let mut sides = [c.kept, c.dropped];
    sides.sort_by_key(|l| l.size.0);
    assert_eq!(
        (
            sides[0].level,
            sides[0].size.0,
            sides[0].phys,
            sides[0].from_page
        ),
        (SMALL_LEVEL, 0x1000, 0xA000_0000, PT_SMALL),
        "the small-page side, named with the page it came out of"
    );
    assert_eq!(
        (
            sides[1].level,
            sides[1].size.0,
            sides[1].phys,
            sides[1].from_page
        ),
        (BIG_LEVEL, 0x1_0000, 0xB000_0000, PT_BIG),
        "the big-page side, named with the page it came out of"
    );
    assert_eq!(settled.duplicate_leaves, 0, "the two are not identical");

    // ★ And exactly ONE of them is proposed — the loss is recorded, not repaired, and the
    // pass is not left proposing both (which would be an overlap the table would refuse).
    let at_zero: Vec<&DecodedLeaf> = settled.binds.iter().filter(|l| l.va.0 == 0).collect();
    assert_eq!(at_zero.len(), 1, "one desired leaf per address, as before");
    assert_eq!(
        (at_zero[0].level, at_zero[0].size.0, at_zero[0].phys),
        (c.kept.level, c.kept.size.0, c.kept.phys),
        "the survivor the collision names IS the one that was proposed"
    );
}

/// ★★ **A byte-identical leaf seen twice is NOT a collision.** Two pages describing the same
/// mapping the same way is one fact observed twice; folding it into `shape_collisions` would
/// make a benign duplicate read as a contradiction, and the histogram this rung exists to
/// produce would be noise. Counted separately, and the collision list stays empty.
#[test]
fn an_identical_leaf_seen_twice_counts_as_a_duplicate_and_not_a_collision() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = ReachShadow::new(ROOT);

    fb.put(
        ROOT,
        page_at(fmt, 0, &[(0, MockGmmuFmt::encode_pde(PD_L1, false, false))]),
    );
    fb.put(
        PD_L1,
        page_at(fmt, 1, &[(0, MockGmmuFmt::encode_pde(PD_L2, false, false))]),
    );
    fb.put(
        PD_L2,
        page_at(
            fmt,
            2,
            &[(0, MockGmmuFmt::encode_pde(PD_DUAL, false, false))],
        ),
    );
    fb.put(
        PD_DUAL,
        page_at(
            fmt,
            MOCK_DUAL_LEVEL,
            &[(0, MockGmmuFmt::encode_dual_pde(PT_SMALL, PT_BIG))],
        ),
    );
    // ⊘ Both tables are read at the SMALL level here, so the two decodes are byte-identical
    // facts about one address — the case the duplicate counter exists for.
    let bytes = page_at(
        fmt,
        SMALL_LEVEL,
        &[(0, MockGmmuFmt::encode_leaf(0xA000_0000, false))],
    );
    fb.put(PT_SMALL, bytes.clone());
    fb.put(PT_BIG, bytes);

    for (phys, level) in [
        (ROOT, 0),
        (PD_L1, 1),
        (PD_L2, 2),
        (PD_DUAL, MOCK_DUAL_LEVEL),
        (PT_SMALL, SMALL_LEVEL),
        (PT_BIG, SMALL_LEVEL),
    ] {
        s.witness(phys);
        observe(&mut s, fmt, &mut fb, at(phys, level, 0));
    }

    let settled = s.settle(fmt);
    assert_eq!(
        settled.shape_collisions,
        vec![],
        "identical is not conflicting"
    );
    assert_eq!(
        settled.duplicate_leaves, 1,
        "and it is still counted, never silent"
    );
}

/// ★ **A settlement with nothing overlapping records nothing** — the negative control for
/// both counters. Without it, *"we saw no collisions"* is indistinguishable from an
/// instrument that cannot report one, which is the failure mode this campaign has paid for
/// about twenty times.
#[test]
fn distinct_addresses_produce_no_collisions_and_no_duplicates() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = ReachShadow::new(ROOT);

    fb.put(
        ROOT,
        page_at(fmt, 0, &[(0, MockGmmuFmt::encode_pde(PD_L1, false, false))]),
    );
    fb.put(
        PD_L1,
        page_at(fmt, 1, &[(0, MockGmmuFmt::encode_pde(PD_L2, false, false))]),
    );
    fb.put(
        PD_L2,
        page_at(
            fmt,
            2,
            &[(0, MockGmmuFmt::encode_pde(PD_DUAL, false, false))],
        ),
    );
    fb.put(
        PD_DUAL,
        page_at(
            fmt,
            MOCK_DUAL_LEVEL,
            &[(0, MockGmmuFmt::encode_pde(PT_SMALL, false, false))],
        ),
    );
    fb.put(
        PT_SMALL,
        page_at(
            fmt,
            SMALL_LEVEL,
            &[
                (0, MockGmmuFmt::encode_leaf(0xA000_0000, false)),
                (1, MockGmmuFmt::encode_leaf(0xA000_1000, false)),
            ],
        ),
    );
    for (phys, level) in [
        (ROOT, 0),
        (PD_L1, 1),
        (PD_L2, 2),
        (PD_DUAL, MOCK_DUAL_LEVEL),
        (PT_SMALL, SMALL_LEVEL),
    ] {
        s.witness(phys);
        observe(&mut s, fmt, &mut fb, at(phys, level, 0));
    }

    let settled = s.settle(fmt);
    assert_eq!(
        settled.binds.len(),
        2,
        "the known-positive: the pass DOES bind here"
    );
    assert_eq!(settled.shape_collisions, vec![]);
    assert_eq!(settled.duplicate_leaves, 0);
}
