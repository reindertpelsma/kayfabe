//! ★★★★★ **w290 — THE PUBLICATION CENSUS, AND ITS KNOWN-POSITIVE FOR EVERY BUCKET.**
//!
//! `[measured, boot w290cup2]` `HOST-PUBLISHED host_rows=4 of 16425` in cup2's own address
//! space: we populate a shadow and never materialize it host-side, which is why hardware
//! faults `FAULT_PDE`. The publication pass that closes it is bounded by three gates it does
//! not own — `plan_back_fb_leaf` refuses on granularity, aperture and extent before any host
//! verb exists (`kayfabe-fwd/src/lib.rs:2328-2371`) — so the census that decides *which rows
//! are even askable* is the load-bearing part, not the loop that asks.
//!
//! ## ⊘ WHY EVERY BUCKET NEEDS ITS OWN KNOWN-POSITIVE
//!
//! A census reporting `candidates=0` has two readings that demand opposite work: *"the gates
//! refuse this table"* and *"the classifier never reached these rows"*. `GET_PTE_INFO` died
//! this week on exactly that ambiguity. So each bucket below is driven to a **non-zero** value
//! by a row built to land in it, and the bucket identity — every row in exactly one bucket,
//! buckets summing to `total` — is asserted as a value (`PublishCensus::buckets_sum`) rather
//! than trusted from the classifier's shape.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::promote::{CtxPromotion, PromoteDeclined, PromotedRange};
use kayfabe_mocks::{MockIsolateFactory, WireClassArch};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const A_CLIENT: HClient = HClient(0xc1d0_000a);
const A_PDB: Pdb = Pdb(0x20_0000);
const H_GR_CHANNEL: HObject = HObject(0x5c00_0019);

/// RM's fixed-placement granule, read from the one place that defines it. ⊘ Never restated:
/// a test carrying its own `0x1_0000` would pass while the gate it claims to pin moved.
const GRANULE: u64 = kayfabe_fwd::FB_LEAF_GRANULE;

fn world() -> Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    Guarded::new("publish_census::world", gpu, rec)
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

/// ★★★★★ **Each gate gets a row built to trip it, and each count is NON-ZERO.**
///
/// ⊘ The one bucket a promotion cannot produce is `already_host`: a promote bind is
/// `declared_by_guest` and carries no `HostBacking` — which is itself the w290 finding
/// (`kayfabe-core/src/promote.rs:1180-1195`) and is pinned by
/// [`a_promote_bind_is_never_host_backed`] below rather than faked here.
#[test]
fn every_gate_has_a_row_that_trips_it_and_the_buckets_sum() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));

    promote(
        &dev,
        vec![
            // ★ THE CANDIDATE: Vidmem, 64 KiB long, 64 KiB aligned.
            range(0x8000_0000, GRANULE, Aperture::Vidmem),
            // ⊘ Vidmem and 64 KiB ALIGNED but too SHORT — the dominant shape in cup2's real
            // table, and the reason run-coalescing is what the granularity gate wants.
            range(0x8010_0000, 0x1000, Aperture::Vidmem),
            // ⊘ Vidmem, a whole granule long, but MISALIGNED in VA.
            range(0x8020_1000, GRANULE, Aperture::Vidmem),
            // ⊘⊘ NOT VIDMEM — and it lands in `guest_ram`, NOT in `not_vidmem`. See the
            // assertion below: this row is the known-positive for w289 §43's finding, not
            // for the aperture gate.
            range(0x8030_0000, GRANULE, Aperture::SysmemCoherent),
        ],
    );

    let c = dev.vas_publish_census(a_pid, GpuId::ZERO, A_PDB, 16);
    assert!(c.buckets_sum(), "every row in exactly one bucket: {c:?}");
    assert_eq!(c.total, 4, "{c:?}");
    assert_eq!(c.candidates_total(), 1, "exactly the granular Vidmem row: {c:?}");
    assert_eq!(
        c.candidates,
        vec![(0x8000_0000, GRANULE, 0x1000_0000 + 0x8000_0000)],
        "★ the candidate is named BY IDENTITY — va, len and phys — not counted: {c:?}"
    );
    assert_eq!(c.candidate_bytes, GRANULE, "{c:?}");
    assert_eq!(
        c.not_granular, 2,
        "⊘ BOTH the short row and the misaligned one — two different ways to fail one gate, \
         and a census that caught only the first would read as healthy: {c:?}"
    );
    assert_eq!(
        c.not_granular_bytes,
        0x1000 + GRANULE,
        "weighted by SIZE too: a count of 4 KiB rows and a count of 64 KiB rows are not \
         comparable quantities: {c:?}"
    );
    // ★★★★★ **THE BUCKET THAT CANNOT FIRE, PINNED AS AN INVARIANT RATHER THAN LEFT DEAD.**
    //
    // The `SysmemCoherent` row above is classified `guest_ram`, not `not_vidmem` — and that
    // is w289 §43 restated by the type system: `Binding::declared_by_guest` maps every
    // non-`Vidmem` aperture a guest may declare onto a guest-RAM kind, and REFUSES
    // `Aperture::Peer` outright (`RegionKindFault::PeerHasNoKind`). So a guest-declared row
    // is `Vidmem` or it is guest RAM; there is no third option, and `not_vidmem` is
    // **unreachable from this population**.
    //
    // ⊘ The bucket is kept because `plan_back_fb_leaf`'s aperture gate (`FbLeafDisagrees`)
    // is real and a future non-guest-declared populate source could reach it. ⚠ But it is
    // asserted ZERO here so the census can never be read as *"the aperture gate is what is
    // refusing our rows"* — it is not, and a reader chasing that would be chasing a bucket
    // nothing can put anything in. **A dead bucket that nobody pins reads as a measured
    // zero.**
    assert_eq!(
        (c.guest_ram, c.not_vidmem),
        (1, 0),
        "the non-Vidmem row is GUEST RAM; `not_vidmem` is unreachable from guest-declared \
         rows and must not be mistaken for the gate that is refusing us: {c:?}"
    );
    assert_eq!((c.already_host, c.already_pinned), (0, 0), "{c:?}");
}

/// ★★★ **THE FINDING, AS A TEST.** A promote bind never carries a `HostBacking`, so a
/// completed two-phase join puts **nothing** in the host VAS.
///
/// This is why w290 refused to "complete the parked joins" as a fix for a `FAULT_PDE`: the
/// join is a population verb and the fault is a publication fault. If this assertion ever
/// flips, that reasoning is stale and the successor must be re-derived.
#[test]
fn a_promote_bind_is_never_host_backed() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(&dev, vec![range(0x8000_0000, GRANULE, Aperture::Vidmem)]);

    let c = dev.vas_publish_census(a_pid, GpuId::ZERO, A_PDB, 16);
    assert_eq!(
        (c.total, c.already_host),
        (1, 0),
        "a bound row with NO host backing — population without publication: {c:?}"
    );
    let published = dev.vas_published_ranges(a_pid, 16);
    assert!(
        published
            .iter()
            .any(|r| r.contains("host_rows=0 of 1") && r.contains("runs=0")),
        "★ HOST-PUBLISHED reports the row as ABSENT while TABLE-DESCRIBES holds it: {published:?}"
    );
    // ★★★★★ **AND THE SECOND RECORD IS PRINTED BESIDE IT**, or `host_rows=0` reads as "no
    // host mapping" when it only ever meant "none in THIS field". See
    // `vas_published_ranges`' 2026-08-13 correction: `commit_pin_guest_ram` maps guest RAM
    // into the host VAS and records it in `guest_ram_pins`, touching `Binding::host` never.
    assert!(
        published.iter().all(|r| r.contains("pins=")),
        "⊘ every row must carry the pin count too — one field is not the host VAS: {published:?}"
    );
    assert_eq!(
        dev.vas_publish_census(a_pid, GpuId::ZERO, A_PDB, 16).already_pinned,
        0,
        "no pin exists in this world, and the bucket says so rather than being absent"
    );
}

/// ⊘ **The cap truncates the LIST and never the COUNTS.** A reader inferring the qualifying
/// set from `candidates.len()` would under-report, which is the `dlen=0` mistake in list form.
#[test]
fn the_cap_truncates_the_list_and_never_the_counts() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(
        &dev,
        (0..5)
            .map(|i| range(0x8000_0000 + i * 0x10_0000, GRANULE, Aperture::Vidmem))
            .collect(),
    );

    let c = dev.vas_publish_census(a_pid, GpuId::ZERO, A_PDB, 2);
    assert_eq!(c.candidates.len(), 2, "the LIST is capped: {c:?}");
    assert_eq!(c.capped, 3, "and it SAYS how many it dropped: {c:?}");
    assert_eq!(c.candidates_total(), 5, "the COUNT is whole: {c:?}");
    assert_eq!(c.candidate_bytes, 5 * GRANULE, "so are the BYTES: {c:?}");
    assert!(c.buckets_sum(), "and the identity still holds: {c:?}");
}

/// ⊘ A `Vas` that does not exist answers an EMPTY census, never a panic and never a
/// half-filled one — and `buckets_sum()` holds trivially, so a caller cannot tell a missing
/// VAS from a full one by the identity alone. That is why the pass prints `total=` beside it.
#[test]
fn an_unknown_vas_answers_an_empty_census() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    let c = dev.vas_publish_census(a_pid, GpuId::ZERO, Pdb(0xdead_0000), 16);
    assert_eq!((c.total, c.candidates_total()), (0, 0), "{c:?}");
    assert!(c.buckets_sum(), "{c:?}");
}

// =================================================================================
// w291 (2a) — THE MERGE'S CONSTRUCTOR: one field, one truth, and it refuses by name
// =================================================================================

/// ★★★★★ A guest-RAM pin lands in `Binding::host` **without flipping the kind** — which is
/// the whole reason it is not `Binding::real_gpu_memory`.
#[test]
fn a_pinned_guest_ram_row_carries_its_host_backing_and_stays_guest_ram() {
    use kayfabe_mmu::{BackingBytes, Binding, HostBacking};
    let backing = HostBacking::whole(
        kayfabe_isolate::HostHandle::NULL,
        0x7f00_0000,
        BackingBytes::SoleBacking,
    );
    let b = Binding::pinned_guest_ram(0x4000_0000, Aperture::SysmemCoherent, backing)
        .expect("the guest's own pages, mapped through");
    assert!(
        b.host().is_some(),
        "★ THE MERGE: the pin is in the FIELD, not only in `Vas::guest_ram_pins`"
    );
    assert!(
        b.is_guest_ram(),
        "⊘⊘ AND THE KIND SURVIVES. `real_gpu_memory` would have flipped this false for every \
         pinned row, and `is_guest_ram()` gates the CE partitioner — the merge must not \
         re-route the data plane as a side effect of a bookkeeping fix"
    );
}

/// ⊘ Refused by name, both ways in — and they are opposite mistakes.
#[test]
fn the_pin_constructor_refuses_a_framebuffer_row_and_a_declared_shadow() {
    use kayfabe_mmu::{BackingBytes, Binding, HostBacking, RegionKindFault};
    let sole = HostBacking::whole(
        kayfabe_isolate::HostHandle::NULL,
        0x7f00_0000,
        BackingBytes::SoleBacking,
    );
    // A Vidmem row is the framebuffer JOIN's population — a different chain that mints
    // memory and re-points the guest's window.
    assert_eq!(
        Binding::pinned_guest_ram(0x1000, Aperture::Vidmem, sole),
        Err(RegionKindFault::NotGuestRam {
            aperture: Aperture::Vidmem
        }),
    );
    // A backing that declares a SHADOW is the two-memories state ruling 3 forbids under
    // every aperture — and these are supposed to BE the guest's pages.
    let shadow = HostBacking::whole(
        kayfabe_isolate::HostHandle::NULL,
        0x7f00_0000,
        BackingBytes::ShadowsGuestMemory,
    );
    assert_eq!(
        Binding::pinned_guest_ram(0x4000_0000, Aperture::SysmemCoherent, shadow),
        Err(RegionKindFault::NotGuestRam {
            aperture: Aperture::SysmemCoherent
        }),
    );
}

/// ★★★★★ **THE MERGE'S EQUALITY, ASSERTED — not merely printed beside itself.**
///
/// `MERGE-AGREES` compares the two records against each other. It is the check for the class
/// that made `host_rows=4 of 16425` wrong: a number that read as complete because nothing
/// stood beside it. ⊘ A world with no pins is the trivially-agreeing case and must still say
/// so, or the flag would only ever be read on the boots that already look healthy.
#[test]
fn the_merge_equality_is_reported_and_holds_with_no_pins() {
    let gpu = world();
    let a_pid = pid_of(&gpu, A_PDB);
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    promote(&dev, vec![range(0x8000_0000, GRANULE, Aperture::Vidmem)]);
    let published = dev.vas_published_ranges(a_pid, 16);
    assert!(
        published.iter().all(|r| r.contains("MERGE-AGREES=true")),
        "0 host rows >= 0 pins — the trivial case still REPORTS, it is not omitted: \
         {published:?}"
    );
}
