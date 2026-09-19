//! ★★★★★ **w746 — `SharedDevice::vaspace_handover`, ITS ENSURE PATH, AND THE THREE ASSERTS
//! THAT REPLACED ITS RE-DERIVED ROUTE (constraints 26, 29 and 30).**
//!
//! # What this file is the offline half of
//!
//! `[measured from w745's committed evidence, w746]` the ownership split refused **4619
//! times**, every one at one conjunct: `Vas::host_vas` was `None`. It was `None` because
//! every verb that mints one is a verb that has to SUCCEED first, and on the `device` arm
//! the only verb that runs is `EngineObject`, which births a channel whose ring is
//! `RingSource::Ours` — and that birth **maps its own ring through the space**, which on the
//! `scratchpad` arm is bare and cannot be mapped through. Every engine object was refused
//! (`forwarded=0 refused=10`) and its freshly-minted space unwound. ⇒ **the repair was gated
//! on the success it repairs.**
//!
//! The cut is here, because a bare address space is the one link in that chain that needs
//! nothing from the rest of it.
//!
//! # ⊘ What this file cannot say
//!
//! Nothing about hardware. The mock mints handles and records verbs; whether RM refuses a
//! channel birth over a bare space with `0x51` — which is what w745's log shows — is a fact
//! about the host driver and is measured on a bench, not here. What IS here is every
//! decision this port makes before the wire.

#![allow(clippy::too_many_lines)]

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::ProcId;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::FwdFault;
use kayfabe_mmu::Binding;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const PDB: Pdb = Pdb(0x3401_000);
const CLIENT: HClient = HClient(0xAA);

/// The leaf every test offers. ⊘ Its numbers are only ever compared against what the `Vas`
/// says; nothing here depends on them being any particular address.
const LEAF_VA: u64 = 0x2_0020_0000;
const LEAF_LEN: u64 = 0x10000;
const LEAF_PHYS: u64 = 0x15_0000;

/// The `Vas`'s own `host_vas`, read the way the boot census would.
fn host_vas_of(dev: &SharedDevice, pid: ProcId) -> Option<kayfabe_isolate::HostHandle> {
    dev.with_proc_mut(pid, |p| p.vas_by_pdb(GPU, PDB).and_then(|v| v.host_vas))
        .flatten()
}

fn leaf() -> kayfabe_rt::completion_watch::FbLeaf {
    kayfabe_rt::completion_watch::FbLeaf {
        va: LEAF_VA,
        len: LEAF_LEN,
        phys: LEAF_PHYS,
    }
}

/// One compute process with a materialized isolate, behind a `SharedDevice`.
fn one_process_device() -> (SharedDevice, ProcId) {
    let arch = std::sync::Arc::new(MockArch::new());
    let (factory, _recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB)).expect("routed");
    let dev = SharedDevice::new(gpu, LockMode::Sharded);
    // ⊘ The isolate is spawned lazily (`R1`'s deferral), so a test that skipped this would
    // measure `IsolatePending` and read it as the hand-over refusing.
    dev.materialize_pending();
    (dev, pid)
}

/// Put a row at the leaf's VA, with whatever `phys`/extent the caller wants to test.
fn bind_row(dev: &SharedDevice, pid: ProcId, va: u64, len: u64, phys: u64) {
    dev.with_proc_mut(pid, |p| {
        let vas = p.vas_by_pdb_mut(GPU, PDB).expect("the Vas");
        vas.table
            .bind(
                PDB,
                GpuVa(va),
                len,
                Binding::declared_by_guest(phys, kayfabe_arch::Aperture::Vidmem)
                    .expect("vidmem has a region kind"),
            )
            .expect("the row binds");
    })
    .expect("the proc is live");
}

// =========================================================================================
// ★★★★★ THE ENSURE PATH — the w745 blocker, and the whole reason this increment exists.
// =========================================================================================

#[test]
fn the_handover_mints_a_bare_space_when_the_vas_holds_none() {
    let (dev, pid) = one_process_device();
    assert_eq!(
        host_vas_of(&dev, pid),
        None,
        "★ NON-VACUITY: the fixture must start with NO host VAS, or the ensure path below \
         is not the path being tested"
    );

    let bare = dev
        .vaspace_handover(pid, GPU, PDB, leaf())
        .expect("★★★★★ w746 — the hand-over MINTS the space when the `Vas` holds none");

    assert_eq!(
        host_vas_of(&dev, pid),
        Some(bare.space),
        "★★★★★ the mint was not COMMITTED. A hand-over that mints and does not record is \
         one host address space leaked per publish, and the next leaf mints another — which \
         is the `Stale::Rebound` shape every other commit in this tree refuses."
    );

    // ★★★ IDEMPOTENCE, and it is the property that fails first if the commit is removed:
    // a second ask must reach the SAME space, not a second one.
    let again = dev
        .vaspace_handover(pid, GPU, PDB, leaf())
        .expect("a second hand-over answers");
    assert_eq!(
        again.space, bare.space,
        "★★★★★ the second hand-over minted a SECOND address space. Two spaces for one `Vas` \
         means the scratchpad places slices in one and the guest's channels are born in the \
         other: every mapping resolves to nothing, with no error anywhere."
    );
}

#[test]
fn the_space_handed_over_is_the_asking_procs_own_isolates() {
    // ★★★★★ **CONSTRAINT 30.** `[ogkm-580.159.04, kernel_channel.c:277-295]` privilege and
    // process identity are stamped AT CREATION and survive a `DupObject`. ⇒ the direction of
    // this hand-over is the whole of its safety: the space is created by the per-proc
    // isolate and lent UP to the scratchpad. A space the scratchpad created and lent DOWN
    // would stamp every channel born in it with the scratchpad's privilege.
    let (dev, pid) = one_process_device();
    let bare = dev
        .vaspace_handover(pid, GPU, PDB, leaf())
        .expect("the hand-over answers");
    assert!(
        bare.space
            .belongs_to(kayfabe_isolate::IsolateId::new(pid.0, GPU)),
        "★★★★★ CONSTRAINT 30 — the space handed over is {:?}, which is not this proc's own \
         isolate's. Constraint 30 is that a resource the scratchpad creates and shares must \
         be PROVEN not to carry its privilege; the proof this seam offers is that the \
         scratchpad never created it.",
        bare.space
    );
}

// =========================================================================================
// ★★★ CONSTRAINT 29 — THE ASSERTS THAT REPLACED THE RE-DERIVED ROUTE.
// =========================================================================================

#[test]
fn a_route_the_spine_disagrees_with_is_refused_by_name() {
    // ★★★ **This IS the replacement for `route_pdb` inside the hand-over** (constraint 29
    // part 2): the argument that retired the derivation is *"the caller already holds the
    // right route"*, so the assert goes RED when the caller's route and the authority's
    // disagree. ⊘ It is its own known-positive — the input below is the disagreement.
    let (dev, pid) = one_process_device();
    let impostor = ProcId(pid.0.wrapping_add(7));
    let err = dev.vaspace_handover(impostor, GPU, PDB, leaf()).expect_err(
        "★★★★★ CONSTRAINT 29 — a hand-over was performed for a proc the spine does not \
             say owns this PDB. That is a cross-address-space hand-over: the scratchpad \
             would place another guest process's slices in this one's page tables.",
    );
    match err {
        FwdFault::HandoverRouteDisagrees {
            caller,
            spine,
            pdb,
            gpu,
        } => {
            assert_eq!(caller, impostor);
            assert_eq!(spine, pid);
            assert_eq!(pdb, PDB);
            assert_eq!(gpu, GPU);
        }
        // ⊘ `RetiredProc` would also be an error, and it would be the WRONG one: it means
        // the refusal came from the proc not existing rather than from the route check, and
        // a test that accepted it would pass with the assert deleted.
        other => panic!(
            "★ the refusal must be the route's own, by name — a different refusal means this \
             test would still pass with the assert removed: got {other:?}"
        ),
    }
    assert_eq!(
        host_vas_of(&dev, pid),
        None,
        "★ …and NOTHING may have been minted on the refused path"
    );
}

#[test]
fn a_leaf_this_vas_describes_at_another_frame_is_refused() {
    let (dev, pid) = one_process_device();
    // The `Vas` says this VA is a DIFFERENT framebuffer frame.
    bind_row(&dev, pid, LEAF_VA, LEAF_LEN, LEAF_PHYS + 0x1000);
    let err = dev.vaspace_handover(pid, GPU, PDB, leaf()).expect_err(
        "★★★★★ CONSTRAINT 29 — the caller's route reached a `Vas` that describes this VA \
             as another frame, and the hand-over proceeded anyway. The scratchpad would then \
             place a slice of the one reserved object over a frame the guest is using for \
             something else.",
    );
    assert!(
        matches!(err, FwdFault::FbLeafDisagrees { .. }),
        "the refusal must name the disagreement: got {err:?}"
    );
    assert_eq!(host_vas_of(&dev, pid), None, "★ nothing was minted");
}

#[test]
fn a_leaf_this_vas_describes_at_another_extent_is_refused() {
    let (dev, pid) = one_process_device();
    bind_row(&dev, pid, LEAF_VA, LEAF_LEN * 2, LEAF_PHYS);
    let err = dev
        .vaspace_handover(pid, GPU, PDB, leaf())
        .expect_err("★★★★★ CONSTRAINT 29 — the extents disagree and the hand-over proceeded");
    assert!(
        matches!(err, FwdFault::FbLeafExtent { .. }),
        "the refusal must name the extent: got {err:?}"
    );
}

#[test]
fn a_leaf_this_vas_agrees_with_is_admitted() {
    // ⊘⊘ **THE KNOWN-NEGATIVE, and it is not optional.** The two refusals above prove the
    // gate can fire; this proves it is not firing on everything — a gate that refuses the
    // legitimate case is a gate that makes the boot green by refusing to run.
    let (dev, pid) = one_process_device();
    bind_row(&dev, pid, LEAF_VA, LEAF_LEN, LEAF_PHYS);
    dev.vaspace_handover(pid, GPU, PDB, leaf())
        .expect("★ a leaf the `Vas` describes exactly must be admitted");
}

#[test]
fn a_leaf_with_no_row_at_all_is_admitted_and_counted() {
    // ⚠ **THE HOLE, STATED RATHER THAN HIDDEN.** The ring source presents leaves off a
    // page-table walk, and the address table may have no row for one yet. Refusing absence
    // would refuse that source outright; so absence is admitted — and COUNTED, because
    // *"not found is not not-written"* and a silent pass is what assert 2 exists to close.
    let (dev, pid) = one_process_device();
    let before = SharedDevice::handover_leaf_untabled();
    dev.vaspace_handover(pid, GPU, PDB, leaf())
        .expect("an untabled leaf is admitted");
    assert!(
        SharedDevice::handover_leaf_untabled() > before,
        "★★★ the untabled-leaf counter did not move. The boot census reads it as the ONLY \
         evidence that assert 2 was reached and had nothing to compare against; a zero here \
         would read as `asked and passed` on a boot where it was never asked."
    );
}

#[test]
fn the_space_not_held_counter_is_a_defect_counter_and_starts_at_zero() {
    // ⊘ Assert 3 re-reads core state after the commit rather than trusting the branch that
    // wrote it. It cannot be provoked from outside the type (which is the point), so what
    // this pins is that a clean hand-over leaves it at zero — i.e. that the census line's
    // `space_not_held=0` is a MEASURED zero on a boot where the hand-over ran, not the
    // vacuous kind.
    let (dev, pid) = one_process_device();
    let before = SharedDevice::handover_space_not_held();
    dev.vaspace_handover(pid, GPU, PDB, leaf())
        .expect("the hand-over answers");
    assert_eq!(
        SharedDevice::handover_space_not_held(),
        before,
        "★★★★★ assert 3 FIRED on a clean hand-over: the space returned is not the one the \
         `Vas` holds"
    );
}

// =========================================================================================
// ★★★★★ w811 — THE SPACE WHOSE ROOT NEVER ARRIVES, AND IT IS THE COMMON CASE ON A GSP PART
// =========================================================================================
//
// `MAP_MEMORY_DMA` is a HAL stub on GSP-client parts, so a client-allocated VASpace is never
// declared to us: `[measured w811, thin guest]` the boot carries **zero** `SET_PAGE_DIRECTORY`
// events and **zero** refusals of one, while five CE channels run in such a space. Every
// framebuffer-leaf hand-over for them was refused `UndeclaredPdb`, so no operand ever got a
// host object, every copy was graded `CeExecutor::Ours`, and the guest's CE work never reached
// the GPU — reported to the client as *"the copy NEVER RETIRED"*.
//
// ⊘ These fixtures build that shape the way the boot does — by **omitting** the `SetPageDir`
// event rather than by poking a field — so a change that starts declaring roots some other way
// makes them non-vacuous instead of silently still passing.

/// A proc whose VASpace exists and is populated but whose root was never declared.
/// `extra_undeclared` adds further undeclared spaces, for the ambiguity test.
fn undeclared_space_device(extra_undeclared: u32) -> (SharedDevice, ProcId) {
    let arch = std::sync::Arc::new(MockArch::new());
    let (factory, _recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    // ⊘ THE WHOLE FIXTURE: drop the declaration, keep everything else byte-identical.
    let declared: usize = s
        .events
        .iter()
        .filter(|e| matches!(e, kayfabe_core::rmgraph::RmEvent::SetPageDir { .. }))
        .count();
    assert_eq!(
        declared, 1,
        "★ NON-VACUITY: the scenario must contain exactly one declaration for dropping it to \
         mean anything. If this is 0 the helper stopped declaring roots and these tests would \
         pass while testing nothing."
    );
    let events: Vec<_> = s
        .events
        .into_iter()
        .filter(|e| !matches!(e, kayfabe_core::rmgraph::RmEvent::SetPageDir { .. }))
        .collect();
    for ev in events {
        gpu.apply(ev).expect("scenario applies cleanly without the declaration");
    }
    for i in 0..extra_undeclared {
        let vas = kayfabe_arch::ids::HObject(0x9000 + i);
        gpu.apply(kayfabe_core::rmgraph::RmEvent::Alloc {
            client: CLIENT,
            parent: identical_handles(0x10, 0x11).device,
            handle: vas,
            class: kayfabe_mocks::mock_classes::VASPACE,
            facts: Default::default(),
        })
        .expect("a second undeclared VASpace allocates");
    }

    let pid = gpu
        .procs
        .keys()
        .copied()
        .find(|p| *p != kayfabe_core::gpu::Gpu::SYSTEM_PROC)
        .expect("the guest proc exists even with no root declared");
    let dev = SharedDevice::new(gpu, LockMode::Sharded);
    dev.materialize_pending();
    (dev, pid)
}

/// ★★★★★ **THE FIX.** One undeclared space ⇒ `Pdb(0)` names it unambiguously, and the
/// hand-over proceeds. ⊘ This is the row the thin guest needs green.
#[test]
fn the_one_undeclared_space_of_a_proc_is_handed_over_rather_than_refused() {
    let (dev, pid) = undeclared_space_device(0);
    let bare = dev
        .vaspace_handover(pid, GPU, Pdb(0), leaf())
        .expect(
            "★★★★★ w811: a proc with exactly ONE undeclared space is not ambiguous, and \
             `Vas::pdb`'s own doc says such a space is `nameable, routable and populatable`",
        );
    // ⊘ Same idempotence property the declared path is held to: a second ask must reach the
    // SAME space, or the scratchpad places slices in one while channels are born in another.
    let again = dev
        .vaspace_handover(pid, GPU, Pdb(0), leaf())
        .expect("a second hand-over answers");
    assert_eq!(
        again.space, bare.space,
        "★★★★★ the undeclared path minted a SECOND address space — the exact `Stale::Rebound` \
         shape the declared path refuses"
    );
}

/// ⊘⊘ **AMBIGUITY IS WHAT IS REFUSED — not the key.** Two undeclared spaces and nothing can
/// say which `Pdb(0)` means, so it refuses BY NAME rather than taking the first.
#[test]
fn two_undeclared_spaces_refuse_by_name_instead_of_picking_one() {
    let (dev, pid) = undeclared_space_device(1);
    match dev.vaspace_handover(pid, GPU, Pdb(0), leaf()) {
        Err(FwdFault::UndeclaredPdb { pid: who }) => assert_eq!(who, pid),
        other => panic!(
            "★ two undeclared spaces must refuse, never resolve to the first one found: \
             {other:?}"
        ),
    }
}

/// ⊘ And the refusal is still reachable when there is NOTHING to name: a proc whose every
/// space is declared has no undeclared one, so `Pdb(0)` names nothing. ★ The variant now means
/// *"zero or several"* rather than *"this space has no root"*, and both arms are tested.
#[test]
fn a_proc_with_no_undeclared_space_still_refuses_pdb_zero() {
    let (dev, pid) = one_process_device();
    match dev.vaspace_handover(pid, GPU, Pdb(0), leaf()) {
        Err(FwdFault::UndeclaredPdb { pid: who }) => assert_eq!(who, pid),
        other => panic!("★ `Pdb(0)` names nothing here and must refuse: {other:?}"),
    }
}
