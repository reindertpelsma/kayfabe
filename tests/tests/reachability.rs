//! ★★★ **Reachability-on-transition** — `resume_from_fault.md` §7 step 4, model in
//! `docs/design/reachability_on_transition.md`.
//!
//! The property under test, in two sentences: *a leaf binds if and only if it is reachable
//! from the page-directory root and the guest was seen to write its page*, and *the
//! transitions are a diff over page-table CONTENT rather than a pattern match over stores*.
//!
//! # Why the file is organised by HOLE and not by function
//!
//! `resume_from_fault.md` §6 put the owner's trap-on-transition instinct to seven named ways
//! a mapping becomes reachable or unreachable **without the entry passing through the state
//! an edge-watcher waits for**. Five of them are closed here, one is closed at page
//! granularity, and one is only half closed — and the value of the file is that each section
//! is answerable against its own hole, including the sections that answer *"not this part"*.
//!
//! ★ Every test here is **mock-level**. No hardware run was available; the C artifact remains
//! the only implementation a real driver has accepted, and it does not implement this model.

use std::collections::BTreeMap;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_arch::{Aperture, Arch, GmmuFmt, PageSize};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{IsolateFb, PT_DECODE_BUDGET, commit_pt_decode, plan_pt_decode, run_pt_decode};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, HostHandle, IsolateFactory, IsolateId, VerbPlan, VerbReply,
    Worker,
};
use kayfabe_mmu::reach::{MAX_SHADOW_PAGES, ReachFault, ReachShadow, apply_settlement};
use kayfabe_mmu::walker::{
    DecodedLeaf, FbRead, PageDecode, PopulateRefusal, PtPage, decode_page, decode_subtree,
};
use kayfabe_mmu::{AddressTable, Binding, HostBacking};
use kayfabe_mocks::{MOCK_DUAL_LEVEL, MockArch, MockGmmuFmt, MockIsolateFactory, SharedRecorder};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;

const FAB_BASE: u64 = 0x1000_0000;
const FAB_LEN: u64 = 0x0400_0000;
const STAGE: u64 = 0x1300_0000;

const A_PDB: Pdb = Pdb(0x1001_0000);
const ROOT: u64 = A_PDB.0;
const PD_L1: u64 = 0x1002_0000;
const PD_L2: u64 = 0x1003_0000;
const PD_DUAL: u64 = 0x1004_0000;
const PT_SMALL: u64 = 0x1005_0000;
const PT_BIG: u64 = 0x1007_0000;
const PT_ORPHAN: u64 = 0x1009_0000;

// =====================================================================================
// Scaffolding
// =====================================================================================

/// A synthetic page-table byte source. The pass-level tests below drive the *production*
/// source through the isolate; the shadow-level ones do not need to, because what they are
/// about is which decoded slots bind, not where the bytes came from.
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

/// One table page's bytes: `entries` slots of `width` bytes, all zero except `set`.
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

fn leaf(phys: u64) -> u128 {
    MockGmmuFmt::encode_leaf(phys, false)
}
fn pde(next: u64) -> u128 {
    MockGmmuFmt::encode_pde(next, false, false)
}

/// The level of the small-page leaf table.
fn small_leaf_level() -> u8 {
    MOCK_DUAL_LEVEL + 1
}

fn at(phys: u64, level: u8, vabase: u64) -> PtPage {
    PtPage {
        phys,
        aperture: Aperture::Vidmem,
        level,
        vabase,
    }
}

/// Decode `page` out of `fb` and hand the result to `shadow`. The two halves of one
/// observation, kept together because a decode nobody observed is a decode that changed
/// nothing.
fn observe(shadow: &mut ReachShadow, fmt: &dyn GmmuFmt, fb: &mut Fb, page: PtPage) {
    let d = decode_page(fmt, fb, page).expect("the synthetic source serves this page");
    shadow
        .observe(page, &d)
        .expect("within the shadow's bounds");
}

/// The four-page directory chain from the root down to `leafpage`, put into `fb` and
/// observed. `witness` says whether the guest is modelled as having been seen to write them.
fn link_chain(
    shadow: &mut ReachShadow,
    fmt: &dyn GmmuFmt,
    fb: &mut Fb,
    leafpage: u64,
    witness: bool,
) {
    fb.put(ROOT, page_at(fmt, 0, &[(0, pde(PD_L1))]));
    fb.put(PD_L1, page_at(fmt, 1, &[(0, pde(PD_L2))]));
    fb.put(PD_L2, page_at(fmt, 2, &[(0, pde(PD_DUAL))]));
    fb.put(
        PD_DUAL,
        page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(leafpage))]),
    );
    for (phys, level) in [
        (ROOT, 0),
        (PD_L1, 1),
        (PD_L2, 2),
        (PD_DUAL, MOCK_DUAL_LEVEL),
    ] {
        if witness {
            shadow.witness(phys);
        }
        observe(shadow, fmt, fb, at(phys, level, 0));
    }
}

/// A shadow rooted where a `Vas` for [`A_PDB`] would root one.
fn shadow() -> ReachShadow {
    ReachShadow::new(A_PDB.0 & !0xfff)
}

/// The virtual address slot `i` of the small-page leaf table describes.
fn small_va(fmt: &dyn GmmuFmt, i: u64) -> GpuVa {
    GpuVa(i << fmt.level_shift(small_leaf_level()).expect("small").shift)
}

// =====================================================================================
// HOLE 1 — validity is not reachability
// =====================================================================================

/// ★★★ **A leaf is written valid long before anything points at it, and it must not bind
/// until the link is published.**
///
/// This is `resume_from_fault.md` §6 hole 1 at its sharpest. UVM writes *"entries bottom
/// up, so that they are valid once they're inserted into the tree"* (`ogkm-580:
/// kernel-open/nvidia-uvm/uvm_mmu.c:771-782`), so the leaf's own invalid→valid edge is not
/// the moment the mapping becomes reachable — the **parent's** publication is. An
/// edge-watching design binds at the first store and is wrong for the whole window between.
///
/// The two halves are asserted separately on purpose: *nothing bound* is weak evidence on
/// its own (a design that never binds passes it), so the same leaf is then linked and the
/// bind is required to appear.
#[test]
fn a_leaf_written_valid_before_its_parent_binds_only_when_the_link_is_published() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();

    // The guest fills the leaf table. We witness the write; nothing points at it yet.
    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(9, leaf(0xF000_0000))]),
    );
    s.witness(PT_SMALL);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));

    let first = s.settle(fmt);
    assert_eq!(first.binds, vec![], "valid is not reachable");
    assert_eq!(
        first.unreachable, 1,
        "the leaf is witnessed and waiting for its link — counted, not lost"
    );
    assert_eq!(first.unwitnessed, 0);

    // The guest links it, bottom-up, exactly as UVM does.
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);

    let second = s.settle(fmt);
    assert_eq!(
        second
            .binds
            .iter()
            .map(|l| (l.va, l.phys))
            .collect::<Vec<_>>(),
        vec![(small_va(fmt, 9), 0xF000_0000)],
        "the PARENT's publication is the reachability edge, and it is what binds"
    );
    assert_eq!((second.unreachable, second.unwitnessed), (0, 0));
}

// =====================================================================================
// HOLE 2 — enumerating means walking, and a walk can read residue
// =====================================================================================

/// ★★★ **Allocator residue can make a page reachable and can never bind a mapping out of
/// it.**
///
/// `mmuWalkReserveEntries(..., bInvalidate = NV_FALSE)` leaves a level reachable with
/// **uninitialised backing store** (`ogkm-580:
/// src/nvidia/src/libraries/mmu/mmu_walk_reserve.c:57-63`, `:85`), so a walker descending
/// into it reads whatever the allocator left there and decodes it as page-table entries.
/// §6.1's resolution is the witness rule — *walk to enumerate candidates, bind only entries
/// we also witnessed being written* — and this is that rule with the residue in front of it.
///
/// ★ The test is written so that the residue page is **fully reachable and full of
/// well-formed leaves**. A design that gated on "does it decode" rather than on "did we see
/// it written" passes every other test in this file and fails this one.
#[test]
fn residue_can_make_a_page_reachable_but_never_binds_a_leaf_out_of_it() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();

    // A page nobody wrote under our observation, holding three plausible leaves.
    fb.put(
        PT_SMALL,
        page_at(
            fmt,
            small_leaf_level(),
            &[
                (1, leaf(0xAAAA_0000)),
                (2, leaf(0xBBBB_0000)),
                (3, leaf(0xCCCC_0000)),
            ],
        ),
    );
    // …linked into the tree by directories that WERE witnessed, so reachability is not in
    // question and the witness gate is the only thing standing between it and the table.
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));

    let out = s.settle(fmt);
    assert_eq!(out.binds, vec![], "reachable-but-unwitnessed stays a MISS");
    assert_eq!(
        out.unwitnessed, 3,
        "and it is COUNTED — a non-zero here is the gate working, and the input to \
         `resume_from_fault.md` §7 step 5"
    );

    // And it really is only the witness that is missing: declare it and the same three bind.
    s.witness(PT_SMALL);
    let after = s.settle(fmt);
    assert_eq!(after.binds.len(), 3);
    assert_eq!(after.unwitnessed, 0);
}

// =====================================================================================
// HOLE 3 — teardown crosses no leaf edge at all
// =====================================================================================

/// ★★★ **A directory clear retires the whole sub-tree — and does NOT retire an orphan.**
///
/// `_mmuWalkPdeRelease` clears the parent entry first and frees the sub-level backing store
/// second, **with no TLB invalidate between the two** (`ogkm-580:
/// src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552`). Hundreds of leaves are unmapped by
/// one store and **none of them is ever written invalid** — the memory is simply recycled.
/// A design watching leaf entries misses the entire unmapping.
///
/// ★★ The second half is the one that is easy to get wrong in the direction that rebuilds
/// `#13`. Retirement is keyed on *was reachable and is not* — a page that fell out of the
/// tree. A page that was **never** reachable is the guest's own build order and must be
/// **kept**, because its link is coming. Both halves are in this test, and the second one is
/// exercised by linking the orphan afterwards and requiring it to bind.
#[test]
fn a_pde_clear_retires_the_whole_subtree_and_an_orphan_is_not_retired_with_it() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();
    let mut table = AddressTable::new();

    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(4, leaf(0x4000_0000))]),
    );
    fb.put(
        PT_ORPHAN,
        page_at(fmt, small_leaf_level(), &[(5, leaf(0x5000_0000))]),
    );
    s.witness(PT_SMALL);
    s.witness(PT_ORPHAN);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    // The orphan hangs at a different virtual base, so its leaf cannot collide with the
    // linked table's when it eventually binds.
    let orphan_base = 1u64 << fmt.level_shift(MOCK_DUAL_LEVEL).expect("dual").shift;
    observe(
        &mut s,
        fmt,
        &mut fb,
        at(PT_ORPHAN, small_leaf_level(), orphan_base),
    );
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);

    let up = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &up);
    let mapped = small_va(fmt, 4);
    assert_eq!(
        table.binding_at(mapped).map(|(_, _, b)| b.phys()),
        Some(0x4000_0000)
    );

    // ── The teardown: the PARENT is cleared. Nothing under it is written invalid.
    fb.put(PD_DUAL, page_at(fmt, MOCK_DUAL_LEVEL, &[]));
    s.witness(PD_DUAL);
    observe(&mut s, fmt, &mut fb, at(PD_DUAL, MOCK_DUAL_LEVEL, 0));

    let down = s.settle(fmt);
    assert_eq!(
        down.retired,
        vec![PT_SMALL],
        "the sub-table fell out of the tree and is retired — its bytes are about to be \
         something else entirely"
    );
    assert!(
        !down.retired.contains(&PT_ORPHAN),
        "the orphan was NEVER reachable; retiring it would delete the very page the \
         build-order case exists to keep"
    );
    assert_eq!(down.unbinds, vec![mapped]);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &down);
    assert_eq!(
        table.binding_at(mapped),
        None,
        "one directory store unmapped it, and the table followed"
    );

    // ── And the orphan survived: link it and it binds, with no re-observation.
    fb.put(
        PD_DUAL,
        page_at(fmt, MOCK_DUAL_LEVEL, &[(1, pde(PT_ORPHAN))]),
    );
    observe(&mut s, fmt, &mut fb, at(PD_DUAL, MOCK_DUAL_LEVEL, 0));
    let relink = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &relink);
    assert_eq!(
        table
            .binding_at(GpuVa(
                orphan_base | (5 << fmt.level_shift(small_leaf_level()).expect("small").shift)
            ))
            .map(|(_, _, b)| b.phys()),
        Some(0x5000_0000),
        "the orphan was still in the shadow, which is the whole point of not retiring it"
    );
}

// =====================================================================================
// HOLE 4 — valid→valid: the remap, and the protection change
// =====================================================================================

/// ★★ **A re-map that never passes through invalid still fires**, because the diff compares
/// what the slot *says* rather than what a store *did*.
///
/// RM drives `update_type = PTE_DOWNGRADE` for a re-map (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2602-2606`) and
/// neither of the owner's two proposed edges fires on it.
///
/// ⚠ This is a property of the comparison and not a claim about hardware: no downgrade
/// appears in any of the C oracle's captures — all 786 invalidates across the three are
/// upgrades (`resume_from_fault.md` §4.2, scan of 2026-08-01) — so the transport by which
/// one would reach this port has not been observed.
#[test]
fn a_remap_that_never_passes_through_invalid_still_fires() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();
    let mut table = AddressTable::new();

    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(4, leaf(0xA000_0000))]),
    );
    s.witness(PT_SMALL);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    let first = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &first);

    // Valid → valid. The entry is never invalid at any point.
    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(4, leaf(0xB000_0000))]),
    );
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    let second = s.settle(fmt);
    assert_eq!(second.unbinds, vec![small_va(fmt, 4)]);
    assert_eq!(second.binds.len(), 1);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &second);
    assert_eq!(
        table.binding_at(small_va(fmt, 4)).map(|(_, _, b)| b.phys()),
        Some(0xB000_0000),
        "the table follows the guest's own page table across a valid→valid change"
    );
}

/// ★★ **A change to the read-only bit ALONE is reported, and is never silently
/// `unchanged`.**
///
/// `kayfabe_mmu::Binding` carries no rights, so this port cannot *model* the change. The
/// honest state is therefore *seen, named, counted, not modelled* — and the failure this
/// pins is the tempting one: comparing `(phys, aperture)` and calling it unchanged, which
/// makes a mapping-rights change indistinguishable from nothing happening.
///
/// ⊘ It is a fidelity gap and not an isolation one: the rights are the guest's own
/// declarations about its own address space, so failing to tighten them cannot reach
/// another tenant.
///
/// The decode is built by hand here because the mock format's leaves are always writable —
/// which is exactly why the field needs a test of its own rather than riding on a decode.
#[test]
fn a_protection_only_change_is_reported_and_never_silently_unchanged() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();

    let mut table = AddressTable::new();
    let page = at(PT_SMALL, small_leaf_level(), 0);
    let mk = |read_only: bool| PageDecode {
        children: vec![],
        leaves: vec![DecodedLeaf {
            va: small_va(fmt, 4),
            phys: 0xA000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(1 << fmt.level_shift(small_leaf_level()).expect("small").shift),
            read_only,
            level: small_leaf_level(),
        }],
        sparse: vec![],
        invalid: 0,
    };

    s.witness(PT_SMALL);
    s.observe(page, &mk(false)).expect("in bounds");
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    let first = s.settle(fmt);
    assert_eq!(first.binds.len(), 1);
    assert_eq!(first.protection_changes, vec![]);
    // ★ `settle` PROPOSES and `apply_settlement` CONFIRMS: a bind the table refused must be
    // proposed again next pass, so the shadow only records what the table agreed to. Skipping
    // the apply here would make the second settle re-propose the same bind and see no change
    // at all — which is a real property of the split, and worth stating rather than
    // discovering.
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &first);

    // Same address, same aperture, same extent — only the rights moved.
    s.observe(page, &mk(true)).expect("in bounds");
    let second = s.settle(fmt);
    assert_eq!(
        second.protection_changes,
        vec![small_va(fmt, 4)],
        "a rights change is a mapping change, and it is named"
    );
    assert_eq!(
        (second.binds.len(), second.unbinds.len()),
        (0, 0),
        "…and the table is not churned for a fact it cannot carry"
    );

    // Idempotent: re-observing the same content reports nothing a second time.
    s.observe(page, &mk(true)).expect("in bounds");
    let third = s.settle(fmt);
    assert_eq!(third.protection_changes, vec![]);
}

// =====================================================================================
// HOLE 5 — a page-directory rebind changes everything with zero entry writes
// =====================================================================================

/// ★★ **A shadow whose root is not the address space's page-directory base is a loud
/// refusal.**
///
/// A `SET_PAGE_DIRECTORY` rebind re-points an entire address space with **zero** entry
/// writes — the C snoops it for exactly that reason (`C: src/qemu/nvkvm_gpu_emul.c:2736-2790`).
/// The structural defence here is inherited: a `Vas` is keyed by `(GpuId, Pdb)`, so a rebind
/// mints a different key and therefore a different shadow.
///
/// This audit exists anyway, and is not redundant, for the reason
/// `AddressTable::audit_identity` gives about the identity law: the constructor proves it
/// about every shadow *it* built; the audit proves it about the shadow the commit is
/// *holding*. Those differ the moment anything reaches a `Vas` another way.
#[test]
fn a_shadow_whose_root_is_not_the_vas_s_pdb_is_a_loud_refusal() {
    assert_eq!(shadow().audit_root(A_PDB), Ok(()));
    assert_eq!(
        ReachShadow::new(0xdead_0000).audit_root(A_PDB),
        Err(ReachFault::RootMismatch {
            pdb: A_PDB,
            root: 0xdead_0000
        })
    );
    // The root is the PDB's PAGE, so the low bits of a PDB do not make it a different tree.
    assert_eq!(
        ReachShadow::new(A_PDB.0 & !0xfff).audit_root(Pdb(A_PDB.0 | 0x40)),
        Ok(())
    );
}

// =====================================================================================
// HOLE 6 — three states, not two
// =====================================================================================

/// ★★★ **Sparse is a third state, and each of the three transitions differs.**
///
/// Sparse has its own fill templates in the walker RM ships (`MMU_WALK_FILL_SPARSE`,
/// `ogkm-580: src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:904-935`), and
/// `gmmu_publication_discipline.md` §7 rule 4 is explicit that conflating it with valid and
/// conflating it with invalid are **different** bugs. So both conflations are pinned, in
/// opposite directions:
///
/// - fold sparse into a **leaf** and the valid→sparse row binds a mapping the guest declared
///   backing-free — caught by the table being empty afterwards;
/// - fold sparse into **invalid** and the declaration disappears — caught by
///   `ReachShadow::sparse_at` and by the reported count.
#[test]
fn sparse_is_a_third_state_and_the_three_transitions_differ() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();
    let mut table = AddressTable::new();

    // Slot 4 valid, slot 7 invalid.
    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(4, leaf(0xA000_0000))]),
    );
    s.witness(PT_SMALL);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    let first = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &first);
    assert!(table.binding_at(small_va(fmt, 4)).is_some());
    assert_eq!(first.sparse, 0);
    assert!(!s.sparse_at(small_va(fmt, 4)));

    // ── valid → sparse (an UNMAP) and invalid → sparse (NOTHING), in one write.
    fb.put(
        PT_SMALL,
        page_at(
            fmt,
            small_leaf_level(),
            &[
                (4, MockGmmuFmt::encode_sparse()),
                (7, MockGmmuFmt::encode_sparse()),
            ],
        ),
    );
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    let second = s.settle(fmt);
    assert_eq!(
        second.unbinds,
        vec![small_va(fmt, 4)],
        "valid→sparse is an unmap; invalid→sparse is not a second one"
    );
    assert_eq!(second.binds, vec![], "and sparse is not a mapping");
    assert_eq!(second.sparse, 2, "both declarations are seen");
    assert!(s.sparse_at(small_va(fmt, 4)) && s.sparse_at(small_va(fmt, 7)));
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &second);
    assert_eq!(
        table.binding_at(small_va(fmt, 4)),
        None,
        "a sparse declaration must never leave a binding behind"
    );
    assert_eq!(
        table.binding_at(small_va(fmt, 7)),
        None,
        "…and must never create one"
    );

    // ── sparse → valid (a MAP).
    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(4, leaf(0xC000_0000))]),
    );
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    let third = s.settle(fmt);
    assert_eq!(third.binds.len(), 1);
    assert_eq!(third.sparse, 0);
    assert!(!s.sparse_at(small_va(fmt, 4)));
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &third);
    assert_eq!(
        table.binding_at(small_va(fmt, 4)).map(|(_, _, b)| b.phys()),
        Some(0xC000_0000)
    );
}

// =====================================================================================
// HOLE 7 — level granularity is not uniform
// =====================================================================================

/// ★★★ **A dual directory slot names TWO sub-tables, and both are followed.**
///
/// The deepest directory's slots are 16 bytes wide and hold two independent sub-table
/// pointers — a small-page table and a big-page table (`ogkm-580:
/// src/common/inc/swref/published/pascal/gp100/dev_mmu.h:112`, the `DUAL_PDE` width). A
/// decode that returns one and drops the other loses a whole sub-tree with no diagnostic,
/// which is `#13`'s shape one level up the tree, and the previous `PteDecode::Pde` could not
/// express the second edge at all.
///
/// ★ The two children are asserted at their **own levels** — 512 four-KiB slots and 32
/// sixty-four-KiB slots — because a design that followed the second edge but reused the
/// first's geometry would read past the end of a 32-entry table.
#[test]
fn a_dual_directory_slot_names_two_sub_tables_and_both_are_followed() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();

    fb.put(
        PD_DUAL,
        page_at(
            fmt,
            MOCK_DUAL_LEVEL,
            &[(0, MockGmmuFmt::encode_dual_pde(PT_SMALL, PT_BIG))],
        ),
    );
    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(1, leaf(0x1100_0000))]),
    );
    fb.put(PT_BIG, page_at(fmt, 5, &[(1, leaf(0x2200_0000))]));

    let d = decode_page(fmt, &mut fb, at(PD_DUAL, MOCK_DUAL_LEVEL, 0)).expect("decodes");
    assert_eq!(
        d.children
            .iter()
            .map(|c| (c.phys, c.level))
            .collect::<Vec<_>>(),
        vec![(PT_SMALL, small_leaf_level()), (PT_BIG, 5)],
        "one slot, two sub-tables, each at the level the FORMAT named"
    );

    // And the whole sub-tree really is walked: both leaves come back, at the two strides.
    let sub = decode_subtree(
        fmt,
        &mut fb,
        at(PD_DUAL, MOCK_DUAL_LEVEL, 0),
        PT_DECODE_BUDGET,
    )
    .expect("within budget");
    let small_shift = fmt.level_shift(small_leaf_level()).expect("small").shift;
    let big_shift = fmt.level_shift(5).expect("big").shift;
    let mut got: Vec<(u64, u64)> = sub.leaves.iter().map(|l| (l.va.0, l.phys)).collect();
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            (1 << small_shift, 0x1100_0000),
            (1 << big_shift, 0x2200_0000)
        ],
        "dropping the second edge loses the big-page half entirely, and silently"
    );
}

/// ★★★ **A directory slot that becomes a leaf unlinks the sub-tree it used to name.**
///
/// Holes 3 and 7 composed, and it belongs to neither on its own: **nothing goes invalid
/// anywhere**, and yet everything under that slot stops being reachable. This is the change
/// an edge-watching design cannot see at all — there is no store to the leaves and no store
/// that clears anything.
///
/// The chip fact behind it: on the generation this port targets a directory level is itself a
/// leaf level (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-53`), so "leaves are
/// PTEs" is wrong here specifically — which is `#13`.
#[test]
fn a_directory_slot_that_becomes_a_leaf_retires_the_subtree_it_used_to_name() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();
    let mut table = AddressTable::new();

    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(6, leaf(0x6000_0000))]),
    );
    s.witness(PT_SMALL);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    let up = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &up);
    let small = small_va(fmt, 6);
    assert!(table.binding_at(small).is_some());

    // The SAME slot now decodes as a leaf. No entry anywhere goes invalid.
    fb.put(
        PD_DUAL,
        page_at(fmt, MOCK_DUAL_LEVEL, &[(0, leaf(0x7000_0000))]),
    );
    observe(&mut s, fmt, &mut fb, at(PD_DUAL, MOCK_DUAL_LEVEL, 0));

    let out = s.settle(fmt);
    assert_eq!(
        out.retired,
        vec![PT_SMALL],
        "the sub-table the slot used to name is gone from the tree"
    );
    assert_eq!(out.unbinds, vec![small]);
    assert_eq!(
        out.binds
            .iter()
            .map(|l| (l.va.0, l.phys))
            .collect::<Vec<_>>(),
        vec![(0, 0x7000_0000)],
        "…and the big leaf that replaced it is the new mapping"
    );
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &out);
    assert_eq!(
        table.binding_at(small).map(|(s, _, _)| s),
        Some(0),
        "the 4 KiB mapping is gone; what covers that address now is the big leaf at 0"
    );
    assert_eq!(
        table
            .binding_at(GpuVa(0))
            .map(|(_, len, b)| (len, b.phys())),
        Some((
            1 << fmt.level_shift(MOCK_DUAL_LEVEL).expect("dual").shift,
            0x7000_0000
        ))
    );
}

// =====================================================================================
// The one unbind that must NOT be performed
// =====================================================================================

/// ★★★ **An unbind of a host-published range is refused, not performed.**
///
/// Dropping it from the table would leave the host object still allocated and still mapped
/// into that address space's host VAS with **no core state naming it** — worse than a leak,
/// because hardware would keep resolving it. It is `RepointsPublished`'s rule applied to the
/// other direction, and it gets its own variant because reading the two as one is how a
/// teardown quietly becomes a use-after-free.
#[test]
fn an_unbind_of_a_host_published_range_is_refused_not_performed() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut fb = Fb::default();
    let mut s = shadow();
    let mut table = AddressTable::new();

    fb.put(
        PT_SMALL,
        page_at(fmt, small_leaf_level(), &[(3, leaf(0x3000_0000))]),
    );
    s.witness(PT_SMALL);
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    link_chain(&mut s, fmt, &mut fb, PT_SMALL, true);
    let up = s.settle(fmt);
    apply_settlement(fmt, &mut table, &mut s, A_PDB, &up);

    // The forwarding plane publishes host memory behind that range.
    let va = small_va(fmt, 3);
    let len = 1u64 << fmt.level_shift(small_leaf_level()).expect("small").shift;
    table.unbind(va);
    table
        .bind(
            A_PDB,
            va,
            len,
            // ★ Sysmem: kind 3 IS host memory, and a `Vidmem` aperture with a host object
            // is ruling 3's forbidden state. The subject here is the UNBIND refusal, which
            // reads `host`, not the aperture.
            Binding::real_gpu_memory(
                0x3000_0000,
                Aperture::SysmemCoherent,
                HostBacking::whole(
                    HostHandle::new(IsolateId::new(1, GPU), 7),
                    va.0,
                    kayfabe_mmu::BackingBytes::SoleBacking,
                ),
            )
            .expect("host memory at the guest's own VA is kind 3"),
        )
        .expect("a published binding at its own VA");

    // The guest now tears the page down.
    fb.put(PT_SMALL, page_at(fmt, small_leaf_level(), &[]));
    observe(&mut s, fmt, &mut fb, at(PT_SMALL, small_leaf_level(), 0));
    let down = s.settle(fmt);
    assert_eq!(down.unbinds, vec![va]);

    let applied = apply_settlement(fmt, &mut table, &mut s, A_PDB, &down);
    assert_eq!(
        applied.refusals,
        vec![PopulateRefusal::UnbindsPublished { va }],
        "unpublishing needs a worker and an unmap verb, so the table says so"
    );
    assert_eq!(applied.unbound, 0);
    assert!(
        table
            .binding_at(va)
            .is_some_and(|(_, _, b)| b.host().is_some()),
        "the published binding SURVIVES — dropping it would strand a live host mapping"
    );
}

// =====================================================================================
// Bounds — everything here is guest-influenced
// =====================================================================================

/// ★ **A page over the shadow's page bound is refused WHOLE**, and the shadow is left
/// exactly as it was.
///
/// A truncated admission would make the desired set a statement about a page-table page that
/// never existed, and the next diff would unbind the half that was not admitted — a wrong
/// unmap manufactured by our own bookkeeping rather than by anything the guest did.
#[test]
fn a_page_over_the_shadow_bound_is_refused_whole_and_the_shadow_is_unchanged() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let mut s = shadow();
    let empty = PageDecode::default();

    for i in 0..MAX_SHADOW_PAGES {
        let phys = 0x4000_0000 + (i as u64) * 0x1000;
        s.observe(at(phys, small_leaf_level(), 0), &empty)
            .expect("under the bound");
    }
    assert_eq!(s.len(), MAX_SHADOW_PAGES);

    let over = 0x9000_0000;
    assert_eq!(
        s.observe(at(over, small_leaf_level(), 0), &empty),
        Err(ReachFault::TooManyPages { phys: over })
    );
    assert_eq!(s.len(), MAX_SHADOW_PAGES, "nothing was displaced");
    // A page already held is still updatable at the bound — the refusal is about GROWTH.
    assert_eq!(
        s.observe(at(0x4000_0000, small_leaf_level(), 0), &empty),
        Ok(())
    );
    assert!(!s.is_empty());
    let _ = fmt;
}

// =====================================================================================
// Through the pass — the wiring, not the model
// =====================================================================================

fn aperture_worker() -> (MockIsolateFactory, SharedRecorder) {
    let (factory, rec) = MockIsolateFactory::new();
    rec.lock().expect("recorder").fb_declare(FAB_BASE, FAB_LEN);
    (factory, rec)
}

fn fresh_host_vas(worker: &mut Worker) -> HostHandle {
    match worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: GpuVa(0x4000_0000),
        })
        .expect("a host VAS")
    {
        VerbReply::Published { host_vas, .. } => host_vas.expect("freshly allocated"),
        other => panic!("unexpected reply {other:?}"),
    }
}

fn write_fabricated(
    worker: &mut Worker,
    rec: &SharedRecorder,
    vas: HostHandle,
    phys: u64,
    bytes: &[u8],
) {
    rec.lock().expect("recorder").ce_seed(STAGE, bytes);
    worker
        .execute(&VerbPlan::CeSplit {
            vas,
            subs: vec![CeSubCopy {
                dst: phys,
                src: CeSource::Address(STAGE),
                len: bytes.len() as u64,
                by: CeExecutor::Ours,
            }],
        })
        .expect("an unrepresentable copy is ours to perform");
}

fn pass_fixture() -> (Guarded<Gpu>, MockIsolateFactory, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let (fb_factory, fb_rec) = aperture_worker();
    (
        Guarded::new("reachability::pass", gpu, rec),
        fb_factory,
        fb_rec,
    )
}

/// ★★★ **Through the whole pass: a retired page loses its LEVEL, so the next write to it is
/// deferred rather than decoded as a page table.**
///
/// This is hole 3's second half — *"or we misparse the recycled page's next contents as PTE
/// writes"* — and it is the half that lives in the pass rather than in the shadow, because
/// the level metadata is the `Vas`'s. `_mmuWalkPdeRelease` frees the sub-level's backing
/// store immediately after clearing the parent (`ogkm-580:
/// src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552`), so those bytes are about to be
/// something else entirely; continuing to call them a level-4 page table is how a recycled
/// page's contents get bound as mappings.
#[test]
fn the_pass_drops_the_level_of_a_retired_page_so_its_next_write_is_deferred() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = pass_fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    let chain = [
        (ROOT, page_at(fmt, 0, &[(0, pde(PD_L1))])),
        (PD_L1, page_at(fmt, 1, &[(0, pde(PD_L2))])),
        (PD_L2, page_at(fmt, 2, &[(0, pde(PD_DUAL))])),
        (
            PD_DUAL,
            page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(PT_SMALL))]),
        ),
        (
            PT_SMALL,
            page_at(fmt, small_leaf_level(), &[(2, leaf(0x7700_0000))]),
        ),
    ];
    for (phys, img) in &chain {
        write_fabricated(&mut worker, &rec, vas, *phys, img);
    }
    with_gpu(&mut gpu, |g| {
        let v = only_proc(g).vases.get_mut(&(GPU, A_PDB)).expect("the vas");
        for (phys, _) in &chain {
            v.pt_pages.insert(*phys);
        }
    });

    let plan = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    let out = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results));
    assert!(out.is_clean(), "{out:?}");
    assert_eq!((out.bound, out.unbound), (1, 0));
    with_gpu(&mut gpu, |g| {
        assert!(
            only_proc(g).vases[&(GPU, A_PDB)]
                .pt_meta
                .contains_key(&PT_SMALL),
            "the leaf table's level was learned forward"
        );
    });

    // ── The guest clears the parent. Nothing under it is written invalid.
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_DUAL,
        &page_at(fmt, MOCK_DUAL_LEVEL, &[]),
    );
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PD_DUAL);
    });
    let plan2 = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    let results2 = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan2.tasks, PT_DECODE_BUDGET)
    };
    let out2 = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results2));
    assert_eq!(out2.retired, vec![PT_SMALL]);
    assert_eq!(out2.unbound, 1, "one directory store, one mapping gone");

    with_gpu(&mut gpu, |g| {
        let p = only_proc(g);
        assert!(
            !p.vases[&(GPU, A_PDB)].pt_meta.contains_key(&PT_SMALL),
            "the retired page is no longer a page table TO US"
        );
        // Its bytes are recycled and the guest writes something else there.
        p.vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PT_SMALL);
        let plan3 = plan_pt_decode(p);
        assert_eq!(
            plan3.tasks,
            vec![],
            "with no level, there is nothing to decode it AS"
        );
        assert_eq!(
            plan3.deferred,
            vec![(GPU, A_PDB, PT_SMALL)],
            "…and it is DEFERRED, which is the honest answer to \"we no longer know what \
             this page is\""
        );
    });
}

/// ★★ **The commit refuses a shadow that is not this address space's, and binds nothing.**
///
/// Hole 5's audit, exercised where it actually runs. A shadow holding another tree's
/// reachability would answer for mappings that are not installed, so the commit stops at that
/// address space rather than believing it.
#[test]
fn the_pass_refuses_a_shadow_whose_root_is_not_the_address_spaces() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = pass_fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    for (phys, img) in [
        (ROOT, page_at(fmt, 0, &[(0, pde(PD_L1))])),
        (PD_L1, page_at(fmt, 1, &[(0, pde(PD_L2))])),
        (PD_L2, page_at(fmt, 2, &[(0, pde(PD_DUAL))])),
        (
            PD_DUAL,
            page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(PT_SMALL))]),
        ),
        (
            PT_SMALL,
            page_at(fmt, small_leaf_level(), &[(2, leaf(0x7700_0000))]),
        ),
    ] {
        write_fabricated(&mut worker, &rec, vas, phys, &img);
    }
    with_gpu(&mut gpu, |g| {
        let v = only_proc(g).vases.get_mut(&(GPU, A_PDB)).expect("the vas");
        for phys in [ROOT, PD_L1, PD_L2, PD_DUAL, PT_SMALL] {
            v.pt_pages.insert(phys);
        }
        // Whatever put THIS here, it is not the shadow of this address space.
        v.reach = ReachShadow::new(0xdead_0000);
    });

    let plan = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    let out = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results));

    assert_eq!(
        out.reach_faults,
        vec![ReachFault::RootMismatch {
            pdb: A_PDB,
            root: 0xdead_0000
        }]
    );
    assert!(!out.is_clean(), "a refusal is never a clean pass");
    assert_eq!((out.bound, out.unbound), (0, 0));
    with_gpu(&mut gpu, |g| {
        assert_eq!(
            only_proc(g).vases[&(GPU, A_PDB)].table.iter().count(),
            0,
            "nothing was believed"
        );
    });
}

fn with_gpu<R>(gpu: &mut Guarded<Gpu>, f: impl FnOnce(&mut Gpu) -> R) -> R {
    f(&mut *gpu)
}

fn only_proc(gpu: &mut Gpu) -> &mut kayfabe_core::gpu::Proc {
    gpu.procs.values_mut().next().expect("one proc")
}
