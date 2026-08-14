//! #102 stage C3 — **the page-table decoder and the production `FbRead`**
//! (`eight_blockers_resolved.md` §12).
//!
//! §11 blocked this stage on one question: *where does page content come from?* §12
//! answered it — we perform a copy only where it is **unrepresentable** by a real copy
//! engine, i.e. where an operand lands in fabricated space, and the executor is the
//! **isolate**. The consequence is the whole of this file: every byte in the fabricated
//! aperture was written by us, so the guest's page tables are already in the isolate's
//! mapping of it, and `FbRead` is a *connection to that mapping* rather than a store.
//!
//! Four families, and the first two are the product:
//!
//! 1. ★★★ **The orphan leaf** (§12.1(i)) — a page filled *before* any PDE points at it.
//!    This is the case that killed the rejected design (a core-owned store of intercepted
//!    payloads is empty exactly here, because at fill time nothing classifies the page as
//!    a page table). It must work, and the reason it works is that the criterion is the
//!    ADDRESS: the bytes were ours from the first write.
//! 2. ★★★ **Decode is not policy** — the 512 MiB leaf decodes faithfully at the walker
//!    and is dropped by policy at the binding site. **Both halves are asserted**, and
//!    separately: conflating them is #13's round-4 silent drop.
//! 3. **MISS = FAULT** — a page the aperture does not cover is a loud fault, never zeros,
//!    and the contrast case (a page of genuine zeros, inside the aperture) proves the two
//!    are actually distinguished rather than accidentally equal.
//! 4. **The pass** — plan / execute / commit through the real shell, with R5's
//!    re-validation and the R1 shape (the blocking phase is between the two locked ones).
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

#![allow(clippy::unusual_byte_groupings)]

use std::collections::BTreeMap;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_arch::{Aperture, Arch, GmmuFmt, LevelShift, PageSize};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{
    IsolateFb, PT_DECODE_BUDGET, commit_pt_decode, plan_pt_decode, publish_backing, run_pt_decode,
};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, HostHandle, IsolateFactory, IsolateId, RmError, VerbPlan,
    VerbReply, Worker,
};
use kayfabe_mmu::walker::{
    DecodedLeaf, DropReason, LeafDisposition, PopulateRefusal, PtPage, WalkFault, decode_page,
    decode_subtree, leaf_disposition, populate,
};
use kayfabe_mmu::{AddressTable, Binding};
use kayfabe_mocks::{
    MOCK_DUAL_LEVEL, MOCK_LEVELS, MOCK_PAGE_SIZES, MockArch, MockGmmuFmt, MockIsolateFactory,
    SharedRecorder,
};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;

// The fabricated aperture this suite models the isolate mapping: 64 MiB at 256 MiB.
// Deliberately small and low — nothing here needs 47 address bits, and a GPA that did
// would not survive a 46-bit host.
const FAB_BASE: u64 = 0x1000_0000;
const FAB_LEN: u64 = 0x0400_0000;

/// The root page directory. A PDB **is** its own root page, which is why level 0 needs
/// no metadata anywhere.
const A_PDB: Pdb = Pdb(0x1001_0000);
const ROOT: u64 = A_PDB.0;
/// A second address space on the SAME proc — the thing that makes "re-resolve by
/// `(gpu, pdb)`" a different statement from "use whatever this proc has".
const B_PDB: Pdb = Pdb(0x1008_0000);
const PD_L1: u64 = 0x1002_0000;
const PD_L2: u64 = 0x1003_0000;
const PD_DUAL: u64 = 0x1004_0000;
const PT_SMALL: u64 = 0x1005_0000;
const PT_SMALL_B: u64 = 0x1006_0000;
const PT_BIG: u64 = 0x1007_0000;
/// Outside the declared aperture — the address that must produce `MISS = FAULT`.
const OUTSIDE: u64 = 0x2000_0000;

/// A staging area inside the aperture that page content is copied *from*, so the write
/// half of the story is a real `CeExecutor::Ours` sub-copy rather than a test poking a
/// map.
const STAGE: u64 = 0x1300_0000;

// =====================================================================================
// Scaffolding
// =====================================================================================

/// One table page's bytes: `entries` slots of `width` bytes, all zero except `set`.
fn image(width: usize, entries: usize, set: &[(usize, u128)]) -> Vec<u8> {
    let mut v = vec![0u8; entries * width];
    for &(i, e) in set {
        let at = i * width;
        v[at..at + width].copy_from_slice(&e.to_le_bytes()[..width]);
    }
    v
}

/// A page at `level`, filled from `set`.
fn page_at(fmt: &dyn GmmuFmt, level: u8, set: &[(usize, u128)]) -> Vec<u8> {
    let g = fmt.level_shift(level).expect("a level the regime has");
    image(usize::from(fmt.entry_size(level)), g.entries as usize, set)
}

/// A checked-out worker on a standalone mock isolate, with its aperture declared.
fn aperture_worker() -> (MockIsolateFactory, SharedRecorder) {
    let (factory, rec) = MockIsolateFactory::new();
    rec.lock().expect("recorder").fb_declare(FAB_BASE, FAB_LEN);
    (factory, rec)
}

/// Allocate a host VAS on `worker` (the mock's `Publish` chain is the only minting path).
fn fresh_host_vas(worker: &mut Worker) -> HostHandle {
    match worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: GpuVa(0x4000_0000),
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect("a host VAS")
    {
        VerbReply::Published { host_vas, .. } => host_vas.expect("freshly allocated"),
        other => panic!("unexpected reply {other:?}"),
    }
}

/// ★★★ Put `bytes` at fabricated address `phys` **the way production does**: stage them,
/// then have the ISOLATE perform a `CeExecutor::Ours` sub-copy into fabricated space.
///
/// This is not ceremony. The claim §12.2 makes is that the read side and the write side
/// are the *same* memory — the content is in the isolate's mapping of the aperture
/// *because we are the ones who put it there*. Seeding the model store directly would
/// test the decoder against a fixture and prove nothing about that identity.
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
                guest_release: None,
            }],
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect("an unrepresentable copy is ours to perform");
}

/// A leaf entry pointing at `phys`, in vidmem.
fn leaf(phys: u64) -> u128 {
    MockGmmuFmt::encode_leaf(phys, false)
}
/// A directory entry pointing at the small-page sub-table `next`.
fn pde(next: u64) -> u128 {
    MockGmmuFmt::encode_pde(next, false, false)
}
/// A directory entry pointing at the **big**-page sub-table `next` — the edge whose
/// child level is not `level + 1`.
fn pde_big(next: u64) -> u128 {
    MockGmmuFmt::encode_pde(next, false, true)
}

/// The level at which a leaf maps the regime's largest page — the whole-framebuffer
/// alias's level. Derived from the geometry, never written down as a number.
fn alias_level(fmt: &dyn GmmuFmt) -> u8 {
    let largest = *fmt
        .page_sizes()
        .iter()
        .max()
        .expect("a regime maps something");
    (0..8u8)
        .find(|&l| {
            fmt.level_shift(l)
                .is_some_and(|g| 1u64.checked_shl(u32::from(g.shift)) == Some(largest.0))
        })
        .expect("the largest page size is some level's stride")
}

/// The level of the small-page leaf table.
fn small_leaf_level() -> u8 {
    MOCK_DUAL_LEVEL + 1
}

/// Run `f` against the guarded device. [`Guarded`] derefs, so this is only sugar — but
/// it keeps the borrow of the guard confined to the call, which matters because the
/// execute phase between two of these must hold **nothing**.
fn with_gpu<R>(gpu: &mut Guarded<Gpu>, f: impl FnOnce(&mut Gpu) -> R) -> R {
    f(&mut *gpu)
}

/// The single proc this file's fixtures create.
fn only_proc(gpu: &mut Gpu) -> &mut kayfabe_core::gpu::Proc {
    gpu.procs.values_mut().next().expect("one proc")
}

// =====================================================================================
// 0. The geometry the decoder walks with — the C's own table
// =====================================================================================

/// ★★ The level table is the real VER2 geometry — the C's strides
/// (`C: nvkvm_gpu_emul.c:8706-8708`) with the ROOT's entry count taken from the driver —
/// and **two** of its six rows have a count that is not `page / entry_size`.
///
/// That last clause is the one worth pinning. `entries` being a count rather than a
/// derived quantity is what stops the decoder reading 3 840 bytes past a big-page table,
/// and 4 064 bytes past the root.
///
/// ★★★ **Row 0 used to say 512 and that was the C's error, not the driver's.**
/// `virtAddrBitHi = 48` / `virtAddrBitLo = 47` is two VA bits ⇒ **4** entries
/// (`ogkm-580:`/`ogkm-610:
/// src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:59-60`, same lines at
/// both tags), which is what `kayfabe_chips::Ga10xGmmu` reports and what
/// `gmmu_fmt_oracle.rs` differentials against the driver's own compiled table. The C's
/// `{ 47, 512 }` is exactly `page_bytes / entry_size`. So this test's own premise had a
/// hole in it: with a 512-entry root, the root was the one directory at which a decoder
/// that DERIVED the count was indistinguishable from one that read it.
#[test]
fn the_level_table_is_the_regimes_and_entry_counts_are_not_derived_from_a_page_size() {
    let arch = MockArch::new();
    let fmt = arch.mmu();

    assert_eq!(
        (0..6u8).map(|l| fmt.level_shift(l)).collect::<Vec<_>>(),
        MOCK_LEVELS.iter().copied().map(Some).collect::<Vec<_>>(),
        "the geometry the walker asks for is the table"
    );
    assert_eq!(
        MOCK_LEVELS.map(|g| (g.shift, g.entries)),
        [
            (47, 4),
            (38, 512),
            (29, 512),
            (21, 256),
            (12, 512),
            (16, 32)
        ],
        "strides: C: nvkvm_gpu_emul.c:8706-8708; root count: \
         ogkm-580:/ogkm-610: kern_gmmu_fmt_gp10x.c:59-60"
    );
    // An un-enumerated level is `None`, not a stride the walker invents.
    assert_eq!(fmt.level_shift(6), None);
    assert_eq!(fmt.level_shift(u8::MAX), None);

    // The three directories BELOW the root each fill exactly one 4 KiB page…
    for l in 1..=MOCK_DUAL_LEVEL {
        let g = fmt.level_shift(l).expect("a directory level");
        assert_eq!(
            u64::from(g.entries) * u64::from(fmt.entry_size(l)),
            0x1000,
            "level {l} does not fill a page"
        );
    }
    // …and the dual directory is the one with 16-byte slots.
    assert_eq!(fmt.entry_size(MOCK_DUAL_LEVEL), 16);
    assert_eq!(fmt.entry_size(small_leaf_level()), 8);

    // ★ The TWO rows whose count is not the page's capacity. A decoder that derived either
    // would read 4 064 bytes past the root and 3 840 past the big-page table.
    for (level, entries, shift) in [(0u8, 4u32, 47u8), (5, 32, 16)] {
        let g = fmt.level_shift(level).expect("a level the regime has");
        assert_eq!((g.entries, g.shift), (entries, shift));
        assert_ne!(
            u64::from(g.entries) * u64::from(fmt.entry_size(level)),
            0x1000,
            "level {level}: if this ever equals a page, the loop above stops meaning \
             anything"
        );
    }
}

// =====================================================================================
// 1. ★★★ THE ORPHAN LEAF — §12.1(i), the case that killed the rejected design
// =====================================================================================

/// ★★★ **A page filled BEFORE any PDE points at it is decoded correctly when the link
/// arrives** — and the reason it is, is that the content came from the aperture rather
/// than from a store of intercepted payloads.
///
/// The sequence is the guest's own, from the C's own words (`C: :8681-8690`): *"the guest
/// fills a leaf PT page and links it under the root a SEPARATE push later"*. Between the
/// two, nothing in the system knows the page is a page table. §11.2(i) is the objection
/// this answers: a payload-keyed store is **empty exactly here**, because at fill time
/// `classify_ce` sees an unclassified page and the write is counted as data.
///
/// ★ Note what the test never does: it never tells anything that `PT_SMALL` is a page
/// table before the link. The only reason the bytes survive is that the destination was
/// *fabricated*, so the copy was ours to perform and ours to read back.
#[test]
fn a_leaf_page_filled_before_any_pde_points_at_it_still_decodes_when_the_link_arrives() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    // ── STEP 1: fill the leaf table. Nothing points at it. Nothing knows what it is. ──
    let leaves = page_at(
        fmt,
        small_leaf_level(),
        &[(0, leaf(0x8000_0000)), (3, leaf(0x8100_0000))],
    );
    write_fabricated(&mut worker, &rec, vas, PT_SMALL, &leaves);

    // Decoding the parent NOW finds nothing, because the parent is still empty — the
    // orphan is genuinely unreachable at this point, which is the premise.
    let empty_parent = page_at(fmt, MOCK_DUAL_LEVEL, &[]);
    write_fabricated(&mut worker, &rec, vas, PD_DUAL, &empty_parent);
    {
        let mut fb = IsolateFb::new(&mut worker);
        let d = decode_page(
            fmt,
            &mut fb,
            PtPage {
                phys: PD_DUAL,
                aperture: Aperture::Vidmem,
                level: MOCK_DUAL_LEVEL,
                vabase: 0,
            },
        )
        .expect("the parent reads");
        assert_eq!((d.children.len(), d.leaves.len()), (0, 0));
        assert_eq!(d.invalid, 256, "an empty directory is 256 empty slots");
    }

    // ── STEP 2: the SEPARATE later push links it. Now the descent reaches it. ──
    let linked = page_at(fmt, MOCK_DUAL_LEVEL, &[(2, pde(PT_SMALL))]);
    write_fabricated(&mut worker, &rec, vas, PD_DUAL, &linked);

    let mut fb = IsolateFb::new(&mut worker);
    let sub = decode_subtree(
        fmt,
        &mut fb,
        PtPage {
            phys: PD_DUAL,
            aperture: Aperture::Vidmem,
            level: MOCK_DUAL_LEVEL,
            vabase: 0,
        },
        PT_DECODE_BUDGET,
    )
    .expect("within budget");

    let dual_shift = fmt.level_shift(MOCK_DUAL_LEVEL).expect("dual").shift;
    let small = fmt.level_shift(small_leaf_level()).expect("small");
    let base = 2u64 << dual_shift;
    assert_eq!(
        sub.leaves,
        vec![
            DecodedLeaf {
                va: GpuVa(base),
                phys: 0x8000_0000,
                aperture: Aperture::Vidmem,
                size: PageSize(1 << small.shift),
                read_only: false,
                level: small_leaf_level(),
            },
            DecodedLeaf {
                va: GpuVa(base | (3 << small.shift)),
                phys: 0x8100_0000,
                aperture: Aperture::Vidmem,
                size: PageSize(1 << small.shift),
                read_only: false,
                level: small_leaf_level(),
            },
        ],
        "the bytes written before the link are exactly what comes back"
    );
    assert_eq!(sub.faults, vec![], "nothing faulted");
    assert_eq!(
        fb.misses(),
        0,
        "both pages are inside the declared aperture"
    );
    assert_eq!(fb.transport_error(), None);
    assert!(fb.reads() >= 2, "the source was actually consulted");
}

// =====================================================================================
// 2. ★★★ THE 512 MiB LEAF — decoded at the walker, dropped by policy. BOTH halves.
// =====================================================================================

/// ★★★ **The whole-framebuffer alias decodes faithfully.** The walker resolves it exactly
/// as `walk_pdb_root` does (`C: :4949`) — with the right physical address, the right
/// aperture and the regime's largest page size — because the alternative is what #13 *was*:
/// *"chan_execute silently DROPPED every such PT write"*.
///
/// This is deliberately a separate test from the drop. If one test asserted "the walker
/// produces nothing for it" the two halves would be indistinguishable, which is the exact
/// conflation §11.7 forbids.
#[test]
fn the_whole_fb_alias_leaf_is_resolved_by_the_walker_with_its_real_size_and_address() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    let lvl = alias_level(fmt);
    let g = fmt.level_shift(lvl).expect("the alias level");
    let img = page_at(fmt, lvl, &[(1, leaf(0x4000_0000))]);
    write_fabricated(&mut worker, &rec, vas, PD_L2, &img);

    let mut fb = IsolateFb::new(&mut worker);
    let d = decode_page(
        fmt,
        &mut fb,
        PtPage {
            phys: PD_L2,
            aperture: Aperture::Vidmem,
            level: lvl,
            vabase: 0,
        },
    )
    .expect("the alias page reads");

    assert_eq!(
        d.leaves,
        vec![DecodedLeaf {
            va: GpuVa(1 << g.shift),
            phys: 0x4000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(1 << g.shift),
            read_only: false,
            level: lvl,
        }],
        "RESOLVED, not skipped — this is the half #13 was missing"
    );
    assert_eq!(
        d.leaves[0].size,
        *MOCK_PAGE_SIZES.iter().max().expect("largest"),
        "and it maps the regime's largest page"
    );
    assert_eq!(d.children, vec![], "an alias leaf is not a sub-table");
}

/// ★★★ **…and the binding site drops it, by name.** The reason is about a *producer* —
/// the copy-engine utility's whole-framebuffer identity alias — which is why the
/// disposition carries [`DropReason::WholeFbIdentityAlias`] and not "too big".
///
/// The contrast half is what makes the assertion mean something: a leaf one level deeper,
/// produced the same way, **binds**. A policy that dropped every large leaf, or every leaf
/// at a directory level, would pass the first half of this test and fail the second.
#[test]
fn the_alias_leaf_is_dropped_by_policy_at_the_binding_site_and_a_smaller_leaf_is_not() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let lvl = alias_level(fmt);
    let g = fmt.level_shift(lvl).expect("the alias level");
    let deeper = lvl + 1;
    let gd = fmt.level_shift(deeper).expect("one level deeper");

    let alias = DecodedLeaf {
        va: GpuVa(1 << g.shift),
        phys: 0x4000_0000,
        aperture: Aperture::Vidmem,
        size: PageSize(1 << g.shift),
        read_only: false,
        level: lvl,
    };
    let ordinary = DecodedLeaf {
        va: GpuVa(2 << gd.shift),
        phys: 0x5000_0000,
        aperture: Aperture::Vidmem,
        size: PageSize(1 << gd.shift),
        read_only: false,
        level: deeper,
    };

    assert_eq!(
        leaf_disposition(fmt, &alias),
        LeafDisposition::Drop(DropReason::WholeFbIdentityAlias),
    );
    assert_eq!(
        leaf_disposition(fmt, &ordinary),
        LeafDisposition::Bind,
        "the drop is about ONE producer, not about big leaves"
    );

    let mut t = AddressTable::new();
    let out = populate(fmt, &mut t, A_PDB, &[alias, ordinary]);
    assert_eq!(
        out.dropped,
        vec![(alias.va, DropReason::WholeFbIdentityAlias)]
    );
    assert_eq!((out.bound, out.unchanged, out.repointed), (1, 0, 0));
    assert_eq!(out.refusals, vec![]);

    // The table is the proof: the alias is NOT in it, the ordinary leaf IS.
    assert_eq!(
        t.binding_at(alias.va),
        None,
        "an alias of the whole heap is not a working set"
    );
    assert_eq!(
        t.binding_at(ordinary.va),
        Some((
            ordinary.va.0,
            1 << gd.shift,
            Binding::declared_by_guest(0x5000_0000, Aperture::Vidmem)
                .expect("the fixture declares a kind the guest can declare")
        )),
        "and a decode DECLARES — it never claims a host publication it did not make"
    );
}

// =====================================================================================
// 3. ★★★ MISS = FAULT — and the contrast that proves it is not accidental
// =====================================================================================

/// ★★★ **A page the aperture does not cover is a LOUD fault**, carrying the address and
/// the level, and it binds nothing.
///
/// The contrast is the load-bearing half: the *same* decode against a page of genuine
/// zeros **inside** the aperture succeeds and reports an empty page. Those two are
/// opposite facts, and the C cannot tell them apart at all — its `nvkvm_pt_rd64` returns
/// `0` for an unreadable page (`C: :4891-4904`), which decodes as an invalid entry. A
/// source that answered a miss with zeros would make an entire address space read as
/// empty and every mapping in it silently vanish.
#[test]
fn a_page_outside_the_aperture_faults_loudly_and_a_page_of_zeros_inside_it_does_not() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    // An explicitly zeroed page INSIDE the aperture.
    let zeros = page_at(fmt, small_leaf_level(), &[]);
    write_fabricated(&mut worker, &rec, vas, PT_SMALL, &zeros);

    let mut fb = IsolateFb::new(&mut worker);
    let inside = decode_page(
        fmt,
        &mut fb,
        PtPage {
            phys: PT_SMALL,
            aperture: Aperture::Vidmem,
            level: small_leaf_level(),
            vabase: 0,
        },
    )
    .expect("a zero page is a page that maps nothing — not a fault");
    assert_eq!(
        (inside.leaves.len(), inside.children.len(), inside.invalid),
        (0, 0, 512)
    );

    // The same read one aperture away.
    let outside = decode_page(
        fmt,
        &mut fb,
        PtPage {
            phys: OUTSIDE,
            aperture: Aperture::Vidmem,
            level: small_leaf_level(),
            vabase: 0,
        },
    );
    assert_eq!(
        outside,
        Err(WalkFault::Unbacked {
            phys: OUTSIDE,
            level: small_leaf_level(),
        }),
        "forwarded, never guessed into a capture"
    );
    assert_eq!(fb.misses(), 1);
    assert_eq!(
        fb.transport_error(),
        None,
        "a guest naming an address we do not map is not OUR failure"
    );
}

/// ★★ **A fault on one branch does not take another branch's leaves with it.**
///
/// The descent straddles the boundary: one sub-table is inside the fabricated aperture,
/// the other is not. Discarding the whole result would be #13's silent drop with a
/// different cause; absorbing the fault would be the C's zeros. Neither.
#[test]
fn a_descent_straddling_the_aperture_keeps_the_readable_branch_and_reports_the_other() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    // Slot 1 → a readable table; slot 5 → one outside the aperture.
    let parent = page_at(
        fmt,
        MOCK_DUAL_LEVEL,
        &[(1, pde(PT_SMALL)), (5, pde(OUTSIDE))],
    );
    write_fabricated(&mut worker, &rec, vas, PD_DUAL, &parent);
    let child = page_at(fmt, small_leaf_level(), &[(7, leaf(0x9000_0000))]);
    write_fabricated(&mut worker, &rec, vas, PT_SMALL, &child);

    let mut fb = IsolateFb::new(&mut worker);
    let sub = decode_subtree(
        fmt,
        &mut fb,
        PtPage {
            phys: PD_DUAL,
            aperture: Aperture::Vidmem,
            level: MOCK_DUAL_LEVEL,
            vabase: 0,
        },
        PT_DECODE_BUDGET,
    )
    .expect("within budget");

    let dual_shift = fmt.level_shift(MOCK_DUAL_LEVEL).expect("dual").shift;
    let small = fmt.level_shift(small_leaf_level()).expect("small");
    assert_eq!(
        sub.leaves,
        vec![DecodedLeaf {
            va: GpuVa((1 << dual_shift) | (7 << small.shift)),
            phys: 0x9000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(1 << small.shift),
            read_only: false,
            level: small_leaf_level(),
        }],
        "the readable branch survives"
    );
    assert_eq!(
        sub.faults,
        vec![WalkFault::Unbacked {
            phys: OUTSIDE,
            level: small_leaf_level(),
        }],
        "and the unreadable one is reported, not absorbed"
    );
    assert_eq!(
        sub.visited.iter().map(|p| p.phys).collect::<Vec<_>>(),
        vec![PD_DUAL, PT_SMALL],
        "a page that could not be read was not 'visited'"
    );
}

// =====================================================================================
// 4. A page whose CONTENT CHANGES between two decodes
// =====================================================================================

/// ★★ **The second decode sees the second content** — because the source is memory, read
/// afresh, and not a snapshot taken at capture time.
///
/// The re-point is what a guest remapping a buffer produces, and it is the shape #13 was
/// about. The assertion is on the table: after the second decode the VA resolves to the
/// **new** physical page, and `repointed` says so by name.
#[test]
fn a_page_rewritten_between_two_decodes_repoints_the_binding_rather_than_restating_it() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    let small = fmt.level_shift(small_leaf_level()).expect("small");
    let page = PtPage {
        phys: PT_SMALL,
        aperture: Aperture::Vidmem,
        level: small_leaf_level(),
        vabase: 0,
    };
    let va = GpuVa(4 << small.shift);
    let mut t = AddressTable::new();

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(4, leaf(0xA000_0000))]),
    );
    let reads_before;
    {
        let mut fb = IsolateFb::new(&mut worker);
        let d = decode_page(fmt, &mut fb, page).expect("reads");
        let out = populate(fmt, &mut t, A_PDB, &d.leaves);
        assert_eq!((out.bound, out.repointed, out.unchanged), (1, 0, 0));
        reads_before = fb.reads();
    }
    assert_eq!(
        t.binding_at(va).map(|(_, _, b)| b.phys()),
        Some(0xA000_0000)
    );

    // Re-decoding UNCHANGED content must not churn the table — otherwise every pass
    // would unbind and rebind the whole address space.
    {
        let mut fb = IsolateFb::new(&mut worker);
        let d = decode_page(fmt, &mut fb, page).expect("reads");
        let out = populate(fmt, &mut t, A_PDB, &d.leaves);
        assert_eq!((out.bound, out.repointed, out.unchanged), (0, 0, 1));
    }

    // The guest remaps: same VA, different physical page.
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(4, leaf(0xB000_0000))]),
    );
    let mut fb = IsolateFb::new(&mut worker);
    let d = decode_page(fmt, &mut fb, page).expect("reads");
    let out = populate(fmt, &mut t, A_PDB, &d.leaves);
    assert_eq!((out.bound, out.repointed, out.unchanged), (0, 1, 0));
    assert_eq!(
        t.binding_at(va).map(|(_, _, b)| b.phys()),
        Some(0xB000_0000),
        "the table follows the guest's own page table"
    );
    assert!(
        fb.reads() >= reads_before,
        "each decode re-reads: a cached image could not have seen the change"
    );
}

// =====================================================================================
// 5. Straddling FABRICATED and REPRESENTABLE space, in the address plane
// =====================================================================================

/// ★★★ **A decode over a range that is partly host-published.**
///
/// Three leaves, three answers, and the middle one is the safety property:
/// - a VA with no binding → **bound** (declared, `host: None`);
/// - a VA already **host-published** at the same physical page → **unchanged**, and the
///   `HostBacking` **survives**. A decode declares `(phys, aperture)`; it says nothing
///   about publication, and comparing the whole `Binding` would make every re-decode of a
///   published range refuse itself;
/// - a VA host-published whose page table now names a *different* page → a **loud
///   refusal**, because re-binding here would drop a live host object on the floor while
///   its mapping stayed live. Unpublishing needs the forwarding plane, not the table.
#[test]
fn a_decode_over_published_and_unpublished_space_declares_preserves_and_refuses_by_name() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let small = fmt.level_shift(small_leaf_level()).expect("small");
    let size = 1u64 << small.shift;

    // A real publication, made through the real path, so the `HostBacking` is genuine.
    let (mut gpu, _factory, _rec) = pass_fixture();
    let kept = GpuVa(0x3_0000_0000);
    let moved = GpuVa(0x3_0000_1000);
    let fresh = GpuVa(0x3_0000_2000);
    with_gpu(&mut gpu, |g| {
        publish_backing(only_proc(g), GPU, A_PDB, kept, size).expect("publishes");
        publish_backing(only_proc(g), GPU, A_PDB, moved, size).expect("publishes");
    });

    // ★ The published binding's OWN `(phys, aperture)` — read back rather than assumed.
    // A decode restates what the guest's page table says; the fixture has to say the same
    // thing for "unchanged" to be the case under test rather than an accident.
    let (kept_decl, moved_backing) = with_gpu(&mut gpu, |g| {
        let t = &only_proc(g).vases[&(GPU, A_PDB)].table;
        let k = t.binding_at(kept).expect("published").2;
        (
            (k.phys(), k.aperture()),
            t.binding_at(moved)
                .expect("published")
                .2
                .host()
                .expect("published"),
        )
    });

    let mk = |va: GpuVa, phys: u64, aperture: Aperture| DecodedLeaf {
        va,
        phys,
        aperture,
        size: PageSize(size),
        read_only: false,
        level: small_leaf_level(),
    };

    let out = with_gpu(&mut gpu, |g| {
        let t = &mut only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .table;
        populate(
            fmt,
            t,
            A_PDB,
            &[
                mk(kept, kept_decl.0, kept_decl.1),
                mk(moved, 0xDEAD_0000, Aperture::Vidmem),
                mk(fresh, 0xC000_0000, Aperture::Vidmem),
            ],
        )
    });

    assert_eq!((out.bound, out.unchanged, out.repointed), (1, 1, 0));
    assert_eq!(
        out.refusals,
        vec![PopulateRefusal::RepointsPublished {
            va: moved,
            phys: 0xDEAD_0000,
        }],
        "a live host publication is never silently orphaned"
    );

    with_gpu(&mut gpu, |g| {
        let t = &only_proc(g).vases[&(GPU, A_PDB)].table;
        assert!(
            t.binding_at(kept).expect("still there").2.host().is_some(),
            "an unchanged declaration must not strip the publication"
        );
        assert_eq!(
            t.binding_at(moved).expect("still there").2.host(),
            Some(moved_backing),
            "and a refused re-point must leave it exactly as it was"
        );
        assert_eq!(
            t.binding_at(fresh)
                .map(|(_, _, b)| (b.phys(), b.host().is_some())),
            Some((0xC000_0000, false)),
            "a fresh declaration is a declaration, not a publication"
        );
    });
}

// =====================================================================================
// 6. Format-driven edges the walker must not paper over
// =====================================================================================

/// ★★ **A leaf claiming a size the regime does not enumerate is a LOUD fault** (#13's
/// corollary L3). Not clamped to the nearest real size, not skipped.
#[test]
fn a_leaf_whose_size_the_regime_does_not_enumerate_is_a_loud_fault() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    // The root's stride is 2^47, which is not a page size any regime here maps.
    let g0 = fmt.level_shift(0).expect("the root level");
    assert!(
        !MOCK_PAGE_SIZES.contains(&PageSize(1 << g0.shift)),
        "the premise: the root's stride is NOT an enumerated leaf size"
    );
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        ROOT,
        &page_at(fmt, 0, &[(1, leaf(0x7000_0000))]),
    );

    let mut fb = IsolateFb::new(&mut worker);
    assert_eq!(
        decode_page(
            fmt,
            &mut fb,
            PtPage {
                phys: ROOT,
                aperture: Aperture::Vidmem,
                level: 0,
                vabase: 0,
            },
        ),
        Err(WalkFault::UnknownLeafSize {
            va: 1 << g0.shift,
            size: PageSize(1 << g0.shift),
        }),
    );
}

/// ★★ **The child's level comes from the FORMAT, not from `level + 1`.**
///
/// The measured regime's deepest directory names two different tables — a small-page one
/// and a big-page one — with different strides and different entry counts. A walker that
/// incremented would read a 32-entry table as a 512-entry one, i.e. 3 840 bytes past its
/// end, and would place every leaf in it at the wrong virtual address.
#[test]
fn the_child_level_is_the_formats_answer_and_is_not_always_one_deeper() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_DUAL,
        &page_at(
            fmt,
            MOCK_DUAL_LEVEL,
            &[(1, pde(PT_SMALL_B)), (2, pde_big(PT_BIG))],
        ),
    );
    let dual_shift = fmt.level_shift(MOCK_DUAL_LEVEL).expect("dual").shift;

    let d = {
        let mut fb = IsolateFb::new(&mut worker);
        decode_page(
            fmt,
            &mut fb,
            PtPage {
                phys: PD_DUAL,
                aperture: Aperture::Vidmem,
                level: MOCK_DUAL_LEVEL,
                vabase: 0,
            },
        )
        .expect("reads")
    };
    assert_eq!(
        d.children,
        vec![
            PtPage {
                phys: PT_SMALL_B,
                aperture: Aperture::Vidmem,
                level: MOCK_DUAL_LEVEL + 1,
                vabase: 1 << dual_shift,
            },
            PtPage {
                phys: PT_BIG,
                aperture: Aperture::Vidmem,
                level: 5,
                vabase: 2 << dual_shift,
            },
        ],
        "one slot's child is one deeper, the other's is two — the format decides"
    );

    // …and the big table is then read at ITS OWN width: 32 entries, 256 bytes.
    let big = fmt.level_shift(5).expect("big");
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_BIG,
        &page_at(fmt, 5, &[(31, leaf(0xE000_0000))]),
    );
    let mut fb = IsolateFb::new(&mut worker);
    let leaves = decode_page(fmt, &mut fb, d.children[1])
        .expect("reads")
        .leaves;
    assert_eq!(
        leaves,
        vec![DecodedLeaf {
            va: GpuVa((2 << dual_shift) | (31 << big.shift)),
            phys: 0xE000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(1 << big.shift),
            read_only: false,
            level: 5,
        }],
    );

    // ★★ …and it asked the source for **256 bytes**, not 4 096. This is the assertion
    // that makes `entries` being a count rather than `page / entry_size` observable at
    // all: the decode loop only ever visits 32 slots either way, so an over-sized READ is
    // invisible in the leaves. It is not invisible in production — the extra 3 840 bytes
    // may fall outside the mapped aperture, which turns a perfectly good table into a
    // spurious `MISS = FAULT`.
    let big_reads: Vec<u64> = rec
        .lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            kayfabe_mocks::RmVerb::FbRead { phys, len, .. } if *phys == PT_BIG => Some(*len),
            _ => None,
        })
        .collect();
    assert_eq!(
        big_reads,
        vec![u64::from(big.entries) * u64::from(fmt.entry_size(5))],
        "the big-page table is read at ITS width, not at a page's"
    );
    assert_eq!(big_reads, vec![256]);
}

/// ★ **A format whose child level does not descend cannot make the walker loop.**
///
/// The depth bound is not a guard against the *guest* — the measured format's every edge
/// goes strictly deeper, so a guest can build a page directory that points at itself and
/// the walker runs out of levels before it runs out of patience. It is a guard against an
/// **adapter**: `child_level` is the format's answer (that is the whole point of the
/// field), and a format that answers "the same level" — a transcription slip, a regime
/// with a genuinely self-referential encoding — would otherwise be an infinite descent
/// inside a lock-free phase, i.e. a hang with no fault and no log line.
///
/// So the fixture is a format that does exactly that. A guard that nothing in the tree can
/// trip is not evidence, and this is the shape that trips it.
#[test]
fn a_format_whose_child_level_never_descends_is_stopped_by_the_depth_bound() {
    /// Every slot at every level points one page along, at the SAME level, forever.
    #[derive(Debug)]
    struct NeverDescends;
    impl GmmuFmt for NeverDescends {
        fn version(&self) -> kayfabe_arch::GmmuVersion {
            kayfabe_arch::GmmuVersion::Ver2
        }
        fn page_sizes(&self) -> &[PageSize] {
            &MOCK_PAGE_SIZES
        }
        fn entry_size(&self, _level: u8) -> u8 {
            8
        }
        fn levels(&self) -> u8 {
            2
        }
        fn level_shift(&self, level: u8) -> Option<LevelShift> {
            (level < 2).then_some(LevelShift {
                shift: 12,
                entries: 1,
            })
        }
        fn decode_entry(&self, level: u8, _raw: u128) -> kayfabe_arch::PteDecode {
            kayfabe_arch::PteDecode::Pde {
                edge: kayfabe_arch::PdeEdge {
                    next: PD_L1,
                    aperture: Aperture::Vidmem,
                    child_level: level,
                },
                also: None,
            }
        }
    }

    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_fabricated(&mut worker, &rec, vas, PD_L1, &[0u8; 8]);

    let fmt = NeverDescends;
    let mut fb = IsolateFb::new(&mut worker);
    let sub = decode_subtree(
        &fmt,
        &mut fb,
        PtPage {
            phys: PD_L1,
            aperture: Aperture::Vidmem,
            level: 0,
            vabase: 0,
        },
        PT_DECODE_BUDGET,
    )
    .expect("it terminates at all, which is the first claim");
    assert_eq!(
        sub.faults,
        vec![WalkFault::TooDeep { phys: PD_L1 }],
        "and it says why it stopped"
    );
    // ★★ A LITERAL, not `usize::from(MAX_WALK_DEPTH)`. Measured: with the derived form,
    // changing the constant to 17 failed nothing — a bound checked against itself is a
    // bound that moves silently, which is the same defect the gate-runner floor exists for.
    assert_eq!(sub.visited.len(), 16, "exactly the bound, then a refusal");
    assert_eq!(
        usize::from(kayfabe_mmu::walker::MAX_WALK_DEPTH),
        16,
        "and the constant is the number written above — change both, deliberately"
    );
}

/// ★ **A guest-built cycle in the MEASURED format terminates too** — by running out of
/// levels rather than out of depth, which is the honest description of what happens and
/// is worth pinning because it is not the same guard.
#[test]
fn a_page_directory_that_points_at_itself_runs_out_of_levels_and_says_so() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_L1,
        &page_at(fmt, 1, &[(0, pde(PD_L1))]),
    );

    let mut fb = IsolateFb::new(&mut worker);
    let sub = decode_subtree(
        fmt,
        &mut fb,
        PtPage {
            phys: PD_L1,
            aperture: Aperture::Vidmem,
            level: 1,
            vabase: 0,
        },
        PT_DECODE_BUDGET,
    )
    .expect("it terminates");
    assert_eq!(
        sub.faults,
        vec![WalkFault::NoSuchLevel { level: 6 }],
        "an un-enumerated level is a fault, never a stride the walker invents"
    );
}

/// ★★ **Exhausting the budget is loud**, not a partial capture presented as a whole one.
#[test]
fn a_descent_that_outruns_its_budget_refuses_rather_than_returning_what_it_got() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(0, leaf(0x8000_0000))]),
    );
    let page = PtPage {
        phys: PT_SMALL,
        aperture: Aperture::Vidmem,
        level: small_leaf_level(),
        vabase: 0,
    };
    let mut fb = IsolateFb::new(&mut worker);
    // 512 entries; 511 is one short.
    assert_eq!(
        decode_subtree(fmt, &mut fb, page, 511),
        Err(WalkFault::BudgetExhausted)
    );
    assert_eq!(
        decode_subtree(fmt, &mut fb, page, 512).map(|d| d.leaves.len()),
        Ok(1),
        "and exactly enough is enough"
    );
}

// =====================================================================================
// 7. The production `FbRead` — transport failure is NOT a guest fault
// =====================================================================================

/// ★★★ **A broken connection and a guest naming an unmapped page are different facts.**
///
/// Both make a read fail and both make the walker fault. If the pass could not tell them
/// apart, a broken socket would send someone to debug a guest's page tables. `IsolateFb`
/// keeps the first transport error and the walk fault separately, and this asserts both
/// halves in one run.
#[test]
fn a_transport_failure_is_reported_as_ours_and_not_as_a_page_the_guest_got_wrong() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(0, leaf(0x8000_0000))]),
    );

    // Standing failure on the read verb only — the aperture is intact, we are not.
    rec.lock()
        .expect("recorder")
        .fail_kinds
        .insert(kayfabe_mocks::VerbKind::FbRead, RmError::NoMemory);

    let mut fb = IsolateFb::new(&mut worker);
    let page = PtPage {
        phys: PT_SMALL,
        aperture: Aperture::Vidmem,
        level: small_leaf_level(),
        vabase: 0,
    };
    assert_eq!(
        decode_page(fmt, &mut fb, page),
        Err(WalkFault::Unbacked {
            phys: PT_SMALL,
            level: small_leaf_level(),
        }),
        "the walker's answer is the same — it has no way to know why"
    );
    assert_eq!(
        fb.transport_error(),
        Some(RmError::NoMemory),
        "but the SOURCE knows, and keeps it"
    );
    assert_eq!(
        fb.misses(),
        0,
        "and it is not counted as the aperture declining to cover the page"
    );
}

// =====================================================================================
// 8. The pass — plan / execute / commit, through the shell
// =====================================================================================

/// A device with one compute proc, plus a standalone isolate to read the aperture through.
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
        Guarded::new("pt_decode::pass", gpu, rec),
        fb_factory,
        fb_rec,
    )
}

/// The same, plus a **second** address space on the same proc — so "resolve by
/// `(gpu, pdb)`" has something it can get wrong.
fn pass_fixture_two_vases() -> (Guarded<Gpu>, MockIsolateFactory, SharedRecorder) {
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, h);
    // A second VASpace under the same device, with its own page-directory base.
    const SECOND_VAS: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5c00_0bbb);
    s.push(RmEvent::Alloc {
        client: HClient(0xAA),
        parent: h.device,
        handle: SECOND_VAS,
        class: kayfabe_mocks::mock_classes::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: HClient(0xAA),
        vaspace: SECOND_VAS,
        pdb: B_PDB,
    });
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let (fb_factory, fb_rec) = aperture_worker();
    (
        Guarded::new("pt_decode::pass2", gpu, rec),
        fb_factory,
        fb_rec,
    )
}

/// ★★★ **The whole pass, over the guest's own build order**: the leaf table is written
/// while nothing points at it (so the plan **defers** it — correctly, because its bytes
/// are already ours), and the next pass, after the link, binds its leaves.
///
/// This is §12.1(i) at the level of the pass rather than the decoder, and it is the
/// property that distinguishes this design from the rejected one end to end.
#[test]
fn the_pass_defers_an_unlinked_page_and_binds_it_once_the_link_is_witnessed() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = pass_fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    let small = fmt.level_shift(small_leaf_level()).expect("small");

    // The guest fills the leaf table. We witness the write (the latch) but nothing links
    // it yet, so its level is unknown.
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(9, leaf(0xF000_0000))]),
    );
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PT_SMALL);
    });

    // PASS 1 — plan defers it, and nothing is bound.
    let plan1 = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(plan1.tasks, vec![], "no page here has a known level yet");
    assert_eq!(
        plan1.deferred,
        vec![(GPU, A_PDB, PT_SMALL)],
        "the orphan is DEFERRED — its bytes are already ours, so this costs nothing"
    );

    // The guest links it under the root, and we witness THAT write.
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        ROOT,
        &page_at(fmt, 0, &[(0, pde(PD_L1))]),
    );
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_L1,
        &page_at(fmt, 1, &[(0, pde(PD_L2))]),
    );
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_L2,
        &page_at(fmt, 2, &[(0, pde(PD_DUAL))]),
    );
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_DUAL,
        &page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(PT_SMALL))]),
    );
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(ROOT);
    });

    // PASS 2 — the root's level is a DECLARED fact, so the descent runs and reaches the
    // orphan through the link.
    let plan2 = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(plan2.deferred, vec![]);
    assert_eq!(
        plan2.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![ROOT]
    );
    assert_eq!(plan2.tasks[0].page.level, 0);

    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan2.tasks, PT_DECODE_BUDGET)
    };
    let out = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results));
    assert!(out.is_clean(), "{out:?}");
    assert_eq!((out.bound, out.repointed, out.unchanged), (1, 0, 0));
    assert_eq!(
        out.meta_learned, 4,
        "the four pages below the root were learned forward; the root is DECLARED"
    );

    with_gpu(&mut gpu, |g| {
        let t = &only_proc(g).vases[&(GPU, A_PDB)].table;
        assert_eq!(
            t.binding_at(GpuVa(9 << small.shift))
                .map(|(_, _, b)| b.phys()),
            Some(0xF000_0000),
            "the orphan's mapping is in the table"
        );
    });

    // ★ And the metadata is now enough to decode the orphan DIRECTLY — which is what the
    // #13 fix actually needs, since a root walk cannot reach a page written after it.
    with_gpu(&mut gpu, |g| {
        let p = only_proc(g);
        p.vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PT_SMALL);
        let plan3 = plan_pt_decode(p);
        assert_eq!(plan3.deferred, vec![]);
        assert_eq!(
            plan3.tasks.iter().map(|t| t.page).collect::<Vec<_>>(),
            vec![PtPage {
                phys: PT_SMALL,
                aperture: Aperture::Vidmem,
                level: small_leaf_level(),
                vabase: 0,
            }],
            "the level chain is forward-populated, never reverse-derived"
        );
    });
}

/// ★★ **R5 — a `Vas` that vanished while the lock was released is skipped, not re-homed.**
///
/// The execute phase runs with no lock held, which is R1's whole point; the price is that
/// the commit must re-resolve everything it is about to touch. Re-attaching a decoded page
/// to whatever inherited the id is the C's never-pruned-table aliasing class.
///
/// ★★ **The proc has TWO address spaces, and that is the whole test.** With one, "re-resolve
/// by `(gpu, pdb)`" and "take whatever address space this proc still has" are the same
/// answer, and a commit that re-homed a decode onto a survivor would pass. Measured: with
/// a single `Vas`, poisoning the keyed lookup into `vases.values_mut().next()` failed
/// nothing.
#[test]
fn a_vas_that_disappeared_during_the_lock_free_phase_is_skipped_and_not_re_homed() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = pass_fixture_two_vases();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        ROOT,
        &page_at(fmt, 0, &[(0, pde(PD_L1))]),
    );
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(ROOT);
    });
    let plan = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(plan.tasks.len(), 1);

    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    // …and while that ran, the address space went away.
    with_gpu(&mut gpu, |g| {
        only_proc(g).vases.remove(&(GPU, A_PDB));
    });
    let out = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results));
    assert_eq!(out.vas_gone, 1);
    assert_eq!((out.bound, out.repointed, out.unchanged), (0, 0, 0));
    assert_eq!(
        out.meta_learned, 0,
        "and nothing was learned onto a survivor"
    );
    // ★ The survivor is the assertion. A decode belonging to a dead address space must
    // not land in a live one — that is the aliasing class, not a bookkeeping detail.
    with_gpu(&mut gpu, |g| {
        let b = &only_proc(g).vases[&(GPU, B_PDB)];
        assert_eq!(b.table.iter().count(), 0, "the surviving Vas is untouched");
        assert!(
            b.pt_meta.is_empty(),
            "…and learned nothing that was not its own"
        );
    });
}

/// ★ The dirty set is **drained** by the plan: a page decoded once is not decoded again
/// until it is written again. Otherwise every pass would re-walk the whole address space
/// and the second write would be indistinguishable from the first.
#[test]
fn planning_consumes_the_dirty_set_so_a_second_pass_has_nothing_to_do() {
    let (mut gpu, _factory, _rec) = pass_fixture();
    with_gpu(&mut gpu, |g| {
        let v = g
            .procs
            .values_mut()
            .next()
            .expect("one proc")
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas");
        v.pt_pages.insert(ROOT);
        v.pt_pages.insert(PT_SMALL);
    });
    let first = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(first.tasks.len(), 1);
    assert_eq!(first.deferred.len(), 1);
    let second = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(second, kayfabe_fwd::PtDecodePlan::default());
}

// =====================================================================================
// 9. The seam itself
// =====================================================================================

/// ★ The production source holds **no bytes**: two `IsolateFb`s built over the same worker
/// see the same memory, because the memory is the isolate's and not theirs.
#[test]
fn the_production_source_is_a_connection_and_not_a_cache() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    let page = PtPage {
        phys: PT_SMALL,
        aperture: Aperture::Vidmem,
        level: small_leaf_level(),
        vabase: 0,
    };

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(1, leaf(0x1_0000))]),
    );
    let first = {
        let mut fb = IsolateFb::new(&mut worker);
        let d = decode_page(fmt, &mut fb, page).expect("reads");
        assert_eq!((fb.reads(), fb.misses()), (1, 0));
        d.leaves
    };
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(1, leaf(0x2_0000))]),
    );
    let second = {
        let mut fb = IsolateFb::new(&mut worker);
        decode_page(fmt, &mut fb, page).expect("reads").leaves
    };
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!((first[0].phys, second[0].phys), (0x1_0000, 0x2_0000));
}

/// ★ A `LevelShift` is plain data the walker can be handed; asserted so the seam's shape
/// stays a value rather than becoming a method call chain.
#[test]
fn a_level_shift_is_a_value() {
    let g = LevelShift {
        shift: 29,
        entries: 512,
    };
    assert_eq!((g.shift, g.entries), (29, 512));
    let mut seen: BTreeMap<u8, u32> = BTreeMap::new();
    for l in MOCK_LEVELS {
        seen.insert(l.shift, l.entries);
    }
    assert_eq!(seen.len(), 6, "no two levels share a stride");
}

// =====================================================================================
// 10. Through the real shell — the lock discipline, in BOTH configurations
// =====================================================================================

/// ★★★ **The pass runs end to end through `SharedDevice`, and the phase that blocks runs
/// with no lock held.**
///
/// R1 is not asserted here by inspection: `Worker::fb_read` runs
/// `kayfabe_util::lockwitness::assert_lock_free` on every single read, which is the same
/// assertion every host verb runs. So a shell that decoded under the device read lock, or
/// under the owner's proc lock, **panics on the first page** rather than passing quietly.
/// This test's green is therefore a statement about the locking, not only about the
/// leaves.
///
/// Both [`kayfabe_rt::LockMode`]s, because the two take different locks for the same
/// operation and R1 has to hold in each.
#[test]
fn the_pass_runs_through_the_shell_in_both_lock_modes_with_the_blocking_phase_unlocked() {
    use kayfabe_rt::device::{LockMode, SharedDevice};

    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let arch = MockArch::new();
        let fmt = arch.mmu();
        let (gpu, factory, rec) = pass_fixture();
        let mut iso = factory.spawn(IsolateId::new(1, GPU));
        let mut worker = iso.checkout().expect("fresh pool");
        let vas = fresh_host_vas(&mut worker);
        let small = fmt.level_shift(small_leaf_level()).expect("small");

        // A complete chain, written the way the guest writes it: through copies WE
        // performed, because every one of these addresses is fabricated.
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

        let pid = gpu.procs.keys().copied().next().expect("one proc");
        let device = gpu.map(|g| SharedDevice::new(g, mode));
        device
            .with_proc_mut(pid, |p| {
                let v = p.vases.get_mut(&(GPU, A_PDB)).expect("the vas");
                // ★★ EVERY page the guest wrote enters the dirty set, and that is not
                // bookkeeping — it is the model. `reachability_on_transition.md` §2.2:
                // a leaf binds only if the guest was SEEN to write its page, so a
                // fixture that writes five pages and declares one is modelling four
                // unwitnessed writes, and the pass is then right to bind nothing. The
                // sibling test below writes the same chain and declares only the leaf's
                // page, and asserts exactly that.
                for (phys, _) in &chain {
                    v.pt_pages.insert(*phys);
                }
            })
            .expect("the proc is live");

        let out = device
            .decode_pt_writes(pid, fmt, &mut worker)
            .expect("the proc is live");
        assert!(out.is_clean(), "{mode:?}: {out:?}");
        assert_eq!(
            (out.bound, out.repointed, out.unchanged),
            (1, 0, 0),
            "{mode:?}"
        );
        assert_eq!(out.transport, None, "{mode:?}");
        assert_eq!(out.meta_learned, 4, "{mode:?}");
        // ★★★ E8 — the PUBLISH phase RAN, through the real shell entry point. Asserted
        // here rather than only in the join test because this is the only place
        // `SharedDevice::decode_pt_writes` is driven: a `Spine::publish_pt_pages` that
        // works but is never called would leave the join test green via its own explicit
        // call and the live path still broken.
        assert_eq!(
            (
                out.learned_pages.len(),
                out.pages_published,
                out.pages_publish_refused
            ),
            (4, 4, 0),
            "{mode:?}: every page the decode learned reached the device-global index — \
             `learned` is the rank-1 half, `published` the rank-0 half, and they agree \
             only because the address space survived the gap between them (R5)"
        );
        for (_, _, page) in &out.learned_pages {
            assert_eq!(
                device.pt_page_owner(GPU, *page),
                Some((pid, A_PDB)),
                "{mode:?}: …and the index ANSWERS for it, which is the whole point — \
                 the next guest CE write into this page is classified as a page-table \
                 write instead of forwarded as ordinary data"
            );
        }
        assert_eq!(
            (out.unwitnessed, out.unreachable),
            (0, 0),
            "{mode:?}: every page in the chain was witnessed and every one is linked"
        );

        device.with_proc(pid, |p| {
            assert_eq!(
                p.vases[&(GPU, A_PDB)]
                    .table
                    .binding_at(GpuVa(2 << small.shift))
                    .map(|(_, _, b)| (b.phys(), b.host().is_some())),
                Some((0x7700_0000, false)),
                "{mode:?}: declared by the decode, not published by it"
            );
        });

        // A second pass has nothing to do — the dirty set was consumed.
        let again = device
            .decode_pt_writes(pid, fmt, &mut worker)
            .expect("still live");
        assert_eq!(again, kayfabe_fwd::PtDecodeOutcome::default(), "{mode:?}");
    }
}

/// ★★ **The metadata bound is a LOUD refusal, and it is reachable.**
///
/// `Vas::pt_meta` grows from what the guest's own tables point at, so its size is
/// guest-influenced and needs a bound (boundary-1). A bound nothing can reach is not a
/// bound — so this test fills it to the cap and then commits one more page, and asserts
/// the refusal is **counted and surfaced** rather than absorbed.
#[test]
fn the_page_table_metadata_bound_is_reported_when_it_is_reached() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = pass_fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    write_fabricated(
        &mut worker,
        &rec,
        vas,
        ROOT,
        &page_at(fmt, 0, &[(0, pde(PD_L1))]),
    );
    write_fabricated(&mut worker, &rec, vas, PD_L1, &page_at(fmt, 1, &[]));

    with_gpu(&mut gpu, |g| {
        let v = only_proc(g).vases.get_mut(&(GPU, A_PDB)).expect("the vas");
        v.pt_pages.insert(ROOT);
        // Fill the map to its cap with pages that are not in this decode's chain.
        for k in 0..kayfabe_fwd::MAX_PT_META as u64 {
            v.pt_meta.insert(
                0x8000_0000 + k * 0x1000,
                PtPage {
                    phys: 0x8000_0000 + k * 0x1000,
                    aperture: Aperture::Vidmem,
                    level: 1,
                    vabase: 0,
                },
            );
        }
    });

    let plan = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    let out = with_gpu(&mut gpu, |g| commit_pt_decode(fmt, only_proc(g), &results));

    assert_eq!(out.meta_refused, 1, "PD_L1's level could not be remembered");
    assert_eq!(out.meta_learned, 0);
    assert!(!out.is_clean(), "a refusal is never a clean pass");
}

// =====================================================================================

/// ★★★ **A page-table page that is NOT in video memory is REFUSED BY NAME, not read.**
///
/// The byte source here is the isolate's mapping of the *fabricated aperture* — framebuffer
/// bytes and nothing else. So a table page whose aperture is system memory names a **GPA**,
/// and reading it through this source interprets that GPA as a **GPGA**: whatever framebuffer
/// byte happens to sit at the same number. The walk would succeed, decode structured garbage
/// and bind it — `#170`/`#171`'s species (*"a GPA-typed field holding a VA is the whole
/// bug"*) one level down, and silent.
///
/// ⚠ **Found 2026-08-06: `PtPage` and `PdeEdge` have carried an `aperture` field since they
/// existed and NO read site consulted it.** The value was propagated faithfully all the way
/// into `Binding` and never once used to decide whether the read was legal.
///
/// ★ **The test is built so that aperture is the ONLY difference.** Real, decodable bytes are
/// written at `PD_L2` first, so a walker that ignores aperture *succeeds* and returns a leaf.
/// Both halves are asserted below: vidmem reads it, sysmem refuses it, same address, same
/// bytes. Without that pairing the refusal could be passing because the page was unreadable.
///
/// ⊘ This is **not** a bug fix and must not be described as one: RM allocates page directories
/// from FBMEM (`ogkm-580: kern_bus_gm107.c:4050`, as `bar2.rs` cites), so no walk on today's
/// boot path meets a sysmem table page. It converts a latent *silently wrong* into a loud
/// *not implemented* — serving UVM's sysmem-resident page tables needs a second byte source,
/// which is a feature, not a wider read here.
#[test]
fn a_page_table_page_outside_vidmem_is_refused_rather_than_read_as_framebuffer() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (factory, rec) = aperture_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    let lvl = alias_level(fmt);
    let img = page_at(fmt, lvl, &[(1, leaf(0x4000_0000))]);
    write_fabricated(&mut worker, &rec, vas, PD_L2, &img);

    // CONTROL: identical page, identical bytes, aperture = vidmem. This MUST read, or the
    // refusal below proves nothing about aperture.
    let mut fb = IsolateFb::new(&mut worker);
    let ok = decode_page(
        fmt,
        &mut fb,
        PtPage {
            phys: PD_L2,
            aperture: Aperture::Vidmem,
            level: lvl,
            vabase: 0,
        },
    )
    .expect("the control must read — otherwise this test is vacuous");
    assert!(
        !ok.leaves.is_empty(),
        "the control must actually decode a leaf, or 'sysmem refuses' is not a contrast"
    );

    // THE PROPERTY: same address, same bytes, only the aperture differs.
    for foreign in [
        Aperture::SysmemCoherent,
        Aperture::SysmemNonCoherent,
        Aperture::Peer,
    ] {
        let refused = decode_page(
            fmt,
            &mut fb,
            PtPage {
                phys: PD_L2,
                aperture: foreign,
                level: lvl,
                vabase: 0,
            },
        );
        assert_eq!(
            refused,
            Err(WalkFault::ForeignAperture {
                phys: PD_L2,
                level: lvl,
                aperture: foreign,
            }),
            "★ a {foreign:?} table page was read through the FABRICATED APERTURE — its GPA was \
             interpreted as a GPGA and whatever framebuffer byte sits at that number was \
             decoded as page-table entries"
        );
    }
}
