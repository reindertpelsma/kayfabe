//! ★★★★★ **w378 — THE COVERAGE PREDICATE, DRIVEN THROUGH A REAL DEVICE.**
//!
//! `kayfabe_util::coverage`'s own unit tests pin the interval algebra (union, subset,
//! difference, the print-cap property). **This file pins the JOIN** — that
//! [`SharedDevice::vas_coverage`] takes `declared` and `published` from the right places, and
//! in particular that `published` reads **both** records of host-side mapping state.
//!
//! ## ⊘ Why the join needs its own file
//!
//! `[measured, boot w290cup2]` `host_rows=4 of 16425` was read as *"the host VAS is empty"*
//! and was wrong, because `commit_pin_guest_ram` maps guest RAM into the host VAS and records
//! it in `Vas::guest_ram_pins`, touching `Binding::host` **never**. A coverage predicate that
//! read one field would inherit that wrong zero and report a confident residual over bytes
//! that ARE mapped — the same defect, now wearing a boolean. Perfect interval algebra over
//! the wrong sets is worse than no predicate, because it looks like an answer.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::promote::{CtxPromotion, PromoteDeclined, PromotedRange};
use kayfabe_isolate::GuestRamGrant;
use kayfabe_mocks::{MockIsolateFactory, WireClassArch};
use kayfabe_rt::device::{LockMode, SharedDevice, coverage_aggregate_line};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::Prot;

const A_CLIENT: HClient = HClient(0xc1d0_000a);
const A_PDB: Pdb = Pdb(0x20_0000);
const H_GR_CHANNEL: HObject = HObject(0x5c00_0019);

/// RM's fixed-placement granule, read from the one place that defines it.
const GRANULE: u64 = kayfabe_fwd::FB_LEAF_GRANULE;

fn world_inner(guest_ram: Option<u64>) -> Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let factory = match guest_ram {
        Some(b) => factory.with_guest_ram(b),
        None => factory,
    };
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(std::sync::Arc::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    Guarded::new("coverage_predicate::world", gpu, rec)
}

fn world() -> Guarded<Gpu> {
    world_inner(None)
}

fn pid_of(gpu: &Gpu, pdb: Pdb) -> kayfabe_core::ProcId {
    *gpu.spine
        .by_pdb
        .get(&(GpuId::ZERO, pdb))
        .expect("routed by its PDB")
}

fn range(va: u64, len: u64, aperture: Aperture) -> PromotedRange {
    PromotedRange {
        va: GpuVa(va),
        len,
        phys: 0x1000_0000 + va,
        aperture,
        buffer_id: 0,
    }
}

fn promote(dev: &SharedDevice, ranges: Vec<PromotedRange>) {
    dev.promote_ctx(&CtxPromotion {
        client: A_CLIENT,
        chan_client: A_CLIENT,
        object: H_GR_CHANNEL,
        ranges,
        halves: Vec::new(),
        declined: PromoteDeclined::default(),
    })
    .expect("the promotion binds");
}

/// ★★★★★ **THE KNOWN-NEGATIVE.** A promote bind carries no `HostBacking`, so the rows are
/// declared and unpublished — and the predicate must say `COVERED=false` with a residual
/// equal to **every declared byte**, not merely `host_rows=0`.
///
/// This is the `w376llmd` shape in miniature: `host_rows=0 of N`, which the old row reports
/// as a cardinality and this one reports as bytes-the-GPU-cannot-see.
#[test]
fn an_unpublished_table_is_not_covered_and_the_residual_is_every_declared_byte() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(
        &dev,
        vec![
            range(0x8000_0000, GRANULE, Aperture::Vidmem),
            range(0x8010_0000, GRANULE, Aperture::Vidmem),
        ],
    );

    let cov = dev.vas_coverage(a_pid);
    let c = cov
        .iter()
        .find(|c| c.pdb == A_PDB)
        .expect("the compute proc's own address space is present");
    assert!(!c.table.covered(), "nothing is published: {c:?}");
    assert!(
        !c.table.declared_is_empty(),
        "⊘ non-vacuity: a trivially-true verdict over an empty declared set would prove \
         nothing, and the boolean alone cannot tell the two apart: {c:?}"
    );
    assert_eq!(
        c.table.residual_bytes(),
        u128::from(2 * GRANULE),
        "every declared byte is residual: {c:?}"
    );
    assert_eq!(
        c.table.residual_intervals(),
        2,
        "★ two runs, because they are 0x10_0000 apart — the merge did not fuse two \
         separate holes into one: {c:?}"
    );
    assert_eq!(c.table.excess_bytes(), 0, "{c:?}");
    assert_eq!((c.host_rows, c.pins), (0, 0), "{c:?}");

    let line = c.render(8);
    assert!(line.contains("COVERED=false"), "{line}");
    assert!(line.contains("residual_bytes=131072"), "{line}");
    assert!(
        line.contains(&format!("pdb=0x{:x}", A_PDB.0)),
        "★ the verdict carries its join key — an Xid prints a PDB and a reader must be able \
         to land on the right line: {line}"
    );
    // ★★ **AND THE OTHER CLAUSE'S `true` IS LABELLED VACUOUS.** This fixture drives the
    // address table directly and never decodes a guest page table, so `GUEST-DESCRIBES` is
    // empty and its `COVERED=true` is worth nothing. `[measured, w378]` that clause printed
    // BARE before this assertion existed — a green a reader scanning a column would have
    // counted.
    assert!(
        line.contains("GUEST⊆PUBLISHED COVERED=true(TRIVIAL: GUEST-DESCRIBES is EMPTY)"),
        "⊘ an empty reach set must say its verdict is vacuous: {line}"
    );

    let agg = coverage_aggregate_line(&cov);
    assert!(agg.contains("COVERED=false"), "{agg}");
    assert!(agg.contains("residual_bytes=131072"), "{agg}");
}

/// ★★★★★ **THE KNOWN-POSITIVE, AND IT IS THE ONE THAT WOULD BE MISSED.** A guest-RAM pin
/// publishes bytes through `Vas::guest_ram_pins` and sets `Binding::host` **never**.
///
/// ⇒ A predicate reading only `Binding::host` reports `COVERED=false` over a range that IS
/// mapped in the host VAS. Both arms run the same verb; only the extent differs, and the
/// arm that binds nothing into the table is the one that catches the bug.
#[test]
fn a_guest_ram_pin_publishes_even_though_it_sets_no_binding_host() {
    const PIN_VA: u64 = 0x9000_0000;
    const FAR_VA: u64 = 0x9020_0000;
    const FILE_OFFSET: u64 = 0x4_0000;
    const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;

    // ---- ARM A: the RUN pin — spans two rows, so `bound_into_table` is FALSE and the ONLY
    // record of the mapping is `guest_ram_pins`. This is the arm a one-field predicate fails.
    let gpu = world_inner(Some(GUEST_RAM_BYTES));
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(
        &dev,
        vec![
            range(PIN_VA, GRANULE, Aperture::SysmemCoherent),
            range(FAR_VA, GRANULE, Aperture::SysmemCoherent),
        ],
    );
    let span = FAR_VA + GRANULE - PIN_VA;
    let pinned = dev
        .pin_guest_ram(
            GpuId::ZERO,
            A_PDB,
            GpuVa(PIN_VA),
            GuestRamGrant::originated_by_the_vmm(FILE_OFFSET, span, Prot::ReadWrite),
        )
        .expect("★ non-vacuity: the pin must RUN, or there is nothing in the second record");
    assert!(
        !pinned.bound_into_table,
        "★ the fixture is the documented run-pin case: a grant that is not one row's exact \
         extent binds NOTHING into the table. If this flips the arm stops measuring the join"
    );

    let cov = dev.vas_coverage(a_pid);
    let c = cov.iter().find(|c| c.pdb == A_PDB).expect("present");
    assert_eq!(
        c.host_rows, 0,
        "★★★ `Binding::host` is EMPTY — a predicate reading only this field sees nothing: \
         {c:?}"
    );
    assert_eq!(c.pins, 1, "{c:?}");
    assert_eq!(
        c.row_bytes, 0,
        "the two contributions are kept apart and this one is zero: {c:?}"
    );
    assert_eq!(c.pin_bytes, u128::from(span), "{c:?}");
    assert!(
        c.table.covered(),
        "★★★★★ COVERED — the declared rows lie inside the pinned span, and the pin IS a host \
         mapping. A `false` here is the w290 wrong zero re-derived as a boolean: {c:?}"
    );
    assert_eq!(c.table.residual_bytes(), 0, "{c:?}");
    // ⊘ The pin covers the 0x10_0000 gap between the two rows, and that over-cover is
    // reported rather than swallowed.
    assert_eq!(
        c.table.excess_bytes(),
        u128::from(span - 2 * GRANULE),
        "★ excess is VISIBLE — it is legitimate here (the grant is one run) but it must be a \
         number a reader can see: {c:?}"
    );

    // ---- ARM B: the EXACT-EXTENT pin — `bound_into_table` is TRUE, so the same bytes are in
    // BOTH records. The union must not double-count them into a wrong `published_bytes`.
    let gpu = world_inner(Some(GUEST_RAM_BYTES));
    let b_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(&dev, vec![range(PIN_VA, GRANULE, Aperture::SysmemCoherent)]);
    let pinned = dev
        .pin_guest_ram(
            GpuId::ZERO,
            A_PDB,
            GpuVa(PIN_VA),
            GuestRamGrant::originated_by_the_vmm(FILE_OFFSET, GRANULE, Prot::ReadWrite),
        )
        .expect("the exact-extent pin runs");
    assert!(pinned.bound_into_table, "the fixture is the merge case");
    let cov = dev.vas_coverage(b_pid);
    let c = cov.iter().find(|c| c.pdb == A_PDB).expect("present");
    assert_eq!((c.host_rows, c.pins), (1, 1), "{c:?}");
    assert_eq!(
        (c.row_bytes, c.pin_bytes),
        (u128::from(GRANULE), u128::from(GRANULE)),
        "⊘ the SAME bytes in both records — the contributions overlap and are still \
         reported separately: {c:?}"
    );
    assert!(c.table.covered(), "{c:?}");
    assert_eq!(
        c.table.published_bytes(),
        u128::from(GRANULE),
        "★★★ A UNION, NOT A SUM. `row_bytes + pin_bytes` is 0x20000 here and the published \
         set is 0x10000 — a predicate that added the two records would report DOUBLE the \
         bytes actually mapped: {c:?}"
    );
    assert_eq!(c.table.excess_bytes(), 0, "{c:?}");
}

/// ★★ **The aggregate over a device with no live proc is vacuously true, and SAYS SO.**
///
/// ⊘ `empty ⇒ covered` is the correct algebra and the wrong headline: a reader scanning for
/// `COVERED=true` on a boot where publication never ran must not find a green line. The
/// `vases=0` label is what stops that reading.
#[test]
fn an_aggregate_over_no_address_space_announces_that_its_true_is_vacuous() {
    let line = coverage_aggregate_line(&[]);
    assert!(line.contains("COVERED=true"), "{line}");
    assert!(line.contains("vases=0"), "{line}");
    assert!(
        line.contains("NO LIVE ADDRESS SPACE"),
        "★ the vacuous true must announce itself: {line}"
    );
    assert!(
        line.contains("GUEST⊆PUBLISHED COVERED=true declared=0B"),
        "⊘ and the guest-reach half carries its own declared total, so its zero is legible \
         rather than inferred: {line}"
    );
}

/// ★★★★★ **THE PRINT CAP DOES NOT CHANGE THE VERDICT — asserted through the DEVICE, not only
/// through the algebra.**
///
/// The unit test in `kayfabe_util::coverage` proves the property of `Coverage::render`. This
/// asserts the property survives the wiring: `vas_coverage` takes **no cap at all**, so the
/// same rows rendered at `cap=0` and `cap=4096` carry identical counts and an identical
/// boolean.
#[test]
fn the_render_cap_never_reaches_the_verdict_through_the_device() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    // 16 rows, each 64 KiB and separated by a 64 KiB gap so none of them coalesce.
    // ⊘ **16 and not more, and the bound is not arbitrary**: `CtxPromotion` refuses a
    // promotion of more than [`kayfabe_core::promote::MAX_PROMOTED_RANGES`] ranges
    // (`TooManyRanges { declared: 64, max: 16 }` when this was first written at 64), so a
    // fixture wanting hundreds of rows cannot be built from one promote. The 200-interval
    // case is proved directly on the algebra in `kayfabe_util::coverage`'s
    // `the_print_cap_truncates_the_list_and_never_the_verdict`; what this test adds is that
    // the WIRING carries the property, and 16 against a cap of 3 truncates for real.
    const CAP: usize = 3;
    let n = 16u64;
    assert_eq!(
        n as usize,
        kayfabe_core::promote::MAX_PROMOTED_RANGES,
        "⊘ the fixture's size IS the promote limit, read from the one place that defines it \
         — if that constant moves, this test's `residual_intervals=16` becomes a literal \
         nobody re-derived"
    );
    promote(
        &dev,
        (0..n)
            .map(|i| range(0x8000_0000 + i * 2 * GRANULE, GRANULE, Aperture::Vidmem))
            .collect(),
    );

    let cov = dev.vas_coverage(a_pid);
    let c = cov.iter().find(|c| c.pdb == A_PDB).expect("present");
    assert_eq!(
        c.table.residual_intervals(),
        n as usize,
        "★ non-vacuity: there really are more residual intervals than the cap: {c:?}"
    );
    assert_eq!(c.table.residual_bytes(), u128::from(n * GRANULE), "{c:?}");

    let tight = c.render(CAP);
    let loose = c.render(4096);
    for line in [&tight, &loose] {
        assert!(line.contains("COVERED=false"), "{line}");
        assert!(line.contains("residual_intervals=16"), "{line}");
        assert!(line.contains("residual_bytes=1048576"), "{line}");
    }
    assert!(
        tight.contains("showing 3 of 16") && tight.contains("PRINT-TRUNCATED"),
        "⊘ the capped line SAYS it is capped: {tight}"
    );
    assert!(
        loose.contains("showing 16 of 16") && !loose.contains("PRINT-TRUNCATED"),
        "{loose}"
    );
    assert!(
        tight.len() < loose.len(),
        "⊘ non-vacuity: the cap really did shorten the LINE, so 'identical counts' is a \
         property of two DIFFERENT strings"
    );
}
