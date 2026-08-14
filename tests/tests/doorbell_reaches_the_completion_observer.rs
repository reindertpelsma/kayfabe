//! # ★★★ Does a GUEST DOORBELL reach the one function that OBSERVES a host completion?
//!
//! `docs/design/completion_wait_architecture.md` §4(b), stated as a test rather than as a
//! reading:
//!
//! > **Exactly one function in the tree observes a real host GPU completion —
//! > `HostRmBackend::await_semaphore`, `crates/kayfabe-isolate-host/src/rm.rs:3260-3279` —
//! > and it is unreachable from any guest action in any build.**
//!
//! ⊘ **This file does NOT test `await_semaphore`.** That function is correct and is
//! already exercised (`kayfabe-rm-ladder` R17, `tests/tests/e6_hw_join.rs`); a second test
//! of it would restate a green. What is untested — and what
//! `execution_plane_increments.md` §15.5 names as the defect class it is the *third*
//! sighting of — is the **caller**:
//!
//! > *"Any future increment row whose acceptance is a predicate on a function must also
//! > state the **caller** that a guest action reaches it through."*
//!
//! So every assertion below quantifies over `SharedDevice::doorbell` — the production
//! entry point the shell's own doorbell port calls
//! (`crates/kayfabe-qemu-raw/src/shim.rs:2603`) — and asks what the *guest's* ring caused.
//!
//! # The two arms, and why the second one is not a decoration
//!
//! 1. **Reachability.** A guest doorbell on a channel whose GPFIFO ring carries one
//!    `LAUNCH_DMA` must reach `RmBackend::ce_copy`, whose body *is* the observer
//!    (`ce_copy_outcome` → `await_semaphore`, `rm.rs:3293, 3260`). Asserting on the verb
//!    the backend was asked for is the only thing that distinguishes *"the observer ran"*
//!    from *"a doorbell was rung at a host channel into which the guest's methods were
//!    never copied"* — which is what `SERVED` means today (§15.5 check 3).
//!
//! 2. ★★★ **The observer's VERDICT is load-bearing.** Reaching `ce_copy` is not enough: a
//!    wiring that calls it and ignores what it answers would be green on arm 1 and would
//!    still be the forged completion `mode2_real_forward_not_fake` forbids. So arm 2 arms
//!    the backend to fail the copy — standing in for the one negative
//!    `await_semaphore` can produce, `semaphore != payload` ⇒
//!    `RmError::Other(CE_NEVER_RETIRED)` (`rm.rs:2653-2657`) — and requires the guest's
//!    doorbell to **refuse**. A `Served` in that arm is a guest told its copy landed when
//!    the engine never released.
//!
//! # ⊘ What a green here does NOT mean
//!
//! - ⊘ **Nothing about a booted guest.** No guest ran; this is a guest's *bytes* and a
//!   guest's *declarations* driven through the production core, exactly as
//!   `tests/tests/e6_join.rs` §"what this file is". `only_live_boots_are_proof`.
//! - ⊘ **Nothing about the completion TAIL.** This forwards the guest's copy and observes
//!   it; it does not write the guest's finishPayload and raises no interrupt. That is
//!   deliberate and is the order §15.5 argues for — *"wire the ring first, complete
//!   second"* — because a completion written on a path whose work never ran is the
//!   forgery, not the fix.
//! - ⊘ **Nothing about arms (a) or (c)** of the three-way completion split
//!   (`completion_wait_architecture.md` §4). `await_semaphore` serves **(b)** only: ops
//!   forwarded to a real host engine. Arm (a) — kernel ops we emulate, where writing the
//!   semaphore ourselves is honest rather than forged — is `ceutils::run_submission`'s and
//!   is not touched here.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.
//!
//! # ⊘⊘⊘ STATUS 2026-08-14 (w296) — **RED, AND THE RED IS THIS FILE DOING ITS JOB**
//!
//! Three of the six tests here fail at `origin/master` `c42e6678`, and so do two in
//! `ring_out_of_our_own_framebuffer.rs`. **They are not stale and they must not be edited to
//! pass.** All five have one cause and it is a product decision, not a test defect.
//!
//! `8cca3502` (w287) scoped ring-content forwarding OFF passthrough channels —
//! `ring_content_is_forwardable(engine, kind)` is now
//! `route == CpuCe && kind == GuestChannelKind::Emulated` (`kayfabe-rt/src/device.rs:5202`).
//! That change is **right on its own terms**: `[measured, w283d]` a single CE doorbell rang
//! the ADOPTED channel and then decoded that same guest ring and ran `ce_copy` on a
//! different host channel — *"`ce_copy` DRIVES a channel; adoption means the GUEST drives
//! it; both cannot hold."* w287's own message states it landed **NOT BUILT OR RUN**, and
//! these five went red with it and were never adjudicated.
//!
//! ⊘ **The obvious repair — "give the fixture an `Emulated` channel" — is BLOCKED, and the
//! block is measured, not reasoned:**
//!
//! 1. `Emulated` ⟺ the channel's boundary anchor is `SYSTEM_ANCHOR`
//!    (`kayfabe-core/src/project.rs:312`) ⟺ it routes to `Gpu::SYSTEM_PROC`.
//! 2. `[green test]` `rmgraph_order_independence::the_system_proc_has_clients_and_a_vas_and_\
//!    still_no_data_plane` asserts `publish_backing(&mut gpu.system, …)` is
//!    `Err(FwdFault::SystemDataPlane)`. The same §12.26 refusal guards the other two
//!    host-backing verbs: `plan_pin_guest_ram` (`kayfabe-fwd/src/lib.rs:1687`) and
//!    `plan_back_fb_leaf` (`:2397`). Those are the **only three** sites that ever set
//!    `Binding::host` (`:1942`, `:2208`, `:2868`).
//! 3. ⇒ On an `Emulated` channel that declares a VAS, **no operand can carry a
//!    `HostBacking`**, so `Representability::HostBacked` is unreachable, so
//!    `Representability::executor()` never answers `CeExecutor::HostCe` (`:5852`) — and
//!    `HostCe` is the only executor `RmBackend::ce_copy` → `ce_copy_outcome` →
//!    `await_semaphore` sits behind.
//!
//! ⇒ ★★★ **On the kind whose ring we read, no operand may be host-backed; on the kind whose
//! operands may be host-backed, we do not read the ring.** A guest CE doorbell therefore
//! cannot reach the host-completion observer on either kind — which is precisely the
//! severance `execution_plane_increments.md` §15.5 named and this file was written to
//! detect. `[measured 2026-08-14, this bench]` all three arms below observe `copies() == []`
//! and `Ok(DoorbellOutcome { kind: Passthrough, … })`.
//!
//! ⊘ One corner is NOT covered by the argument above and is stated so it is not mistaken for
//! a way out: a channel with **no VAS at all** resolves `TableOperands::Untracked`, which
//! *does* route to `HostCe` (`:5852`). That path points a real engine at guest VAs nothing
//! translated, and is not a design anyone chose.
//!
//! ⚠ Resolving this needs an owner ruling on where ring-content forwarding belongs, and its
//! own arm and its own boot. It is out of scope for a maintenance rung.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;

use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, RmEvent};
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::FwdFault;
use kayfabe_isolate::{CeExecutor, CeSource, RmError};
use kayfabe_isolate_host::rm::CE_NEVER_RETIRED;
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, RmVerb, SharedRecorder, VerbKind,
    mock_classes as mc,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Scenario, bind_ring, bind_ring_at, notifier_gpa, pb_va, script_ring_via};
use kayfabe_vmm::Vmm;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xC0B_0000);
const PDB0: Pdb = Pdb(0x4002_0000);
const CE_VCHID: VChid = VChid(0x31);

/// The guest's two operands. Both are published, so both partition to
/// `Representability::HostBacked` and the plan chooses a real engine — which is the only
/// arm `await_semaphore` exists on.
const SRC_VA: GpuVa = GpuVa(0x2_0000_0000);
const DST_VA: GpuVa = GpuVa(0x2_0010_0000);
const COPY_LEN: u64 = 0x1000;

/// Where the guest's **method words** live in guest RAM.
const PUSH_GPA: u64 = 0x5000_0000;
/// Where the guest's **GPFIFO ring** itself lives in guest RAM. ⊘ A different page from
/// the method words: a fixture in which the ring and the pushbuffer share an address
/// cannot tell a read of one from a read of the other.
const GPFIFO_GPA: u64 = 0x5100_0000;

// =====================================================================================
// The guest
// =====================================================================================

/// One compute process with one copy-engine channel that **declares its GPFIFO ring**.
///
/// ⊘ Written out rather than taken from [`Scenario::compute_process`] for one reason:
/// that helper declares no `gp_fifo_ring`, and the ring's address is the whole subject
/// here. `AllocFacts::gp_fifo_ring` is `gpFifoOffset` @ +8 / `gpFifoEntries` @ +16 off the
/// channel's own `RM_ALLOC` — the same one alloc that declares `hVASpace`, so the ring and
/// the address space it is named in can never be attributed to different channels
/// (`kayfabe-core/src/rmgraph.rs:490-503`).
fn guest() -> (Gpu, MockVmm, SharedRecorder, ProcId, ChanId) {
    guest_with_gpfifo_binding(true)
}

/// [`guest`], with the guest's mapping of its **own GPFIFO ring** made optional.
///
/// ★★★★ **§16.70 — the arm that produces a `Served` doorbell which forwards NOTHING.**
/// `bind_gpfifo = false` leaves everything else identical: the channel still *declares*
/// `gpFifoOffset`/`gpFifoEntries`, the bytes are still in guest RAM, both operands are still
/// published, and the channel is still scheduled. The single difference is that this port's
/// forward-populated address table holds no binding for the ring's own VA — which is the
/// state a real GSP-managed channel can be in, because its ring is mapped on a transport
/// the table never sees (`mode2_address_table.md`; the shell's CPU executor descends the
/// guest's published page tables instead, `SharedDoorbell::try_ce_submission`).
fn guest_with_gpfifo_binding(bind_gpfifo: bool) -> (Gpu, MockVmm, SharedRecorder, ProcId, ChanId) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");

    let root = HObject(0xC0B_0000);
    let dev = HObject(0xC0B_0001);
    let vas = HObject(0xC0B_0010);
    let tsg = HObject(0xC0B_0012);
    let chan = HObject(0xC0B_001A);

    let mut vmm = MockVmm::new();
    let ring = ce_ring(&mut vmm);
    let entries = u32::try_from(ring.len() / 16).expect("a small ring");

    let mut s = Scenario::new();
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: vas,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: vas,
        pdb: PDB0,
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: tsg,
        class: mc::TSG,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: tsg,
        handle: chan,
        class: mc::CHANNEL_CE,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            userd_flags: MockArch::userd_flags_for(CE_VCHID),
            error_notifier: Some(ErrorNotifier::Sysmem {
                gpa: notifier_gpa(CE_VCHID),
            }),
            // ★★★ THE SUBJECT: the guest tells us where its ring is, and how big.
            gp_fifo_ring: Some(GpFifoRing {
                va: pb_va(GPFIFO_GPA).0,
                entries,
            }),
            ..Default::default()
        },
    });
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }

    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");

    // The guest maps both operands, so a real engine can be pointed at them.
    for va in [SRC_VA, DST_VA] {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        kayfabe_fwd::publish_backing(proc, GPU, PDB0, va, COPY_LEN).expect("the range publishes");
    }

    // The guest maps its pushbuffer, and its GPFIFO ring, into the channel's own VAS.
    bind_ring(&mut gpu, pid, cid, &ring);
    vmm.gpa_write(GPFIFO_GPA, &ring)
        .expect("the ring is written into guest RAM");
    if bind_gpfifo {
        bind_ring_at(
            &mut gpu,
            pid,
            cid,
            pb_va(GPFIFO_GPA),
            GPFIFO_GPA,
            ring.len() as u64,
        );
    }

    // ★ #177 — the guest schedules before it rings. This file's subject is what happens
    // after that, not the scheduling gate.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);

    (gpu, vmm, rec, pid, cid)
}

/// The guest's ring: bind the CE class on a subchannel, then one `LAUNCH_DMA` naming both
/// operands virtually — the shape a real `AMPERE_DMA_COPY_B` copy has.
fn ce_ring(vmm: &mut MockVmm) -> Vec<u8> {
    script_ring_via(
        vmm,
        PUSH_GPA,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma_full(
                DST_VA.0,
                true,
                SRC_VA,
                true,
                COPY_LEN,
                kayfabe_arch::CeWork::Copy,
            ),
        ],
    )
}

/// Every `ce_copy` the backend was asked to perform, in order. ⊘ The **verb**, not a
/// counter of our own: `measure_at_the_boundary_not_inside`.
fn copies(rec: &SharedRecorder) -> Vec<kayfabe_isolate::CeSubCopy> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_iso, v)| match v {
            RmVerb::CeCopy { sub, .. } => Some(*sub),
            _ => None,
        })
        .collect()
}

// =====================================================================================
// ★★★ THE ACCEPTANCE
// =====================================================================================

/// ★★★ **A guest doorbell reaches the host-completion observer.**
///
/// The chain every link of which is a production function:
/// `SharedDevice::doorbell` → the channel's own GPFIFO ring, read out of guest memory
/// through that channel's address table → `parse_pushbuffer` → `forward_ce` → `plan_ce` →
/// `Worker::execute`'s `VerbPlan::CeSplit` arm → `RmBackend::ce_copy` → *(on the host
/// backend)* `ce_copy_outcome` → `await_semaphore`.
///
/// ★ The **non-vacuity** arm is first: with nothing rung, the backend has been asked for
/// no copy at all, so the count below is this doorbell's doing and not the fixture's.
#[test]
fn a_guest_doorbell_reaches_the_host_completion_observer() {
    let (gpu, mut vmm, rec, _pid, _cid) = guest();
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    assert!(
        copies(&rec).is_empty(),
        "★ non-vacuity: building the guest must not itself move a byte"
    );

    let out = dev
        .doorbell(
            Some(&mut vmm),
            GPU,
            MockArch::token_for(CE_VCHID),
            &[],
            None,
        )
        .expect("the doorbell routes and is served");
    assert_eq!(out.chan, _cid);

    let seen = copies(&rec);
    assert_eq!(
        seen.len(),
        1,
        "★★★ THE SEVERANCE. A guest rang a doorbell on a channel whose ring carries one \
         LAUNCH_DMA and the only function in this tree that observes a host completion \
         (`HostRmBackend::await_semaphore`) was never asked for anything. `Served` here \
         means: we rang a doorbell on a host channel into which the guest's methods were \
         never copied. Saw {seen:?}"
    );
    assert_eq!(seen[0].dst, DST_VA.0, "★ the guest's OWN destination");
    assert_eq!(
        seen[0].src,
        CeSource::Address(SRC_VA.0),
        "★ the guest's OWN source"
    );
    assert_eq!(seen[0].len, COPY_LEN, "★ the guest's OWN length");
    assert_eq!(
        seen[0].by,
        CeExecutor::HostCe,
        "★ on a REAL engine — the only executor `await_semaphore` observes"
    );
}

/// ★★★ **A second doorbell over an UNCHANGED ring forwards NOTHING.**
///
/// A GPFIFO ring is append-and-ring: every entry the guest ever wrote is still in it after
/// the doorbell that ran it. A path that forwarded what it could read would re-issue the
/// same copy on every later doorbell — bytes moved twice, on a real engine, with no error
/// anywhere. ⊘ That is not a hypothetical: it is what this wiring does without
/// `ExecPlane::forwarded`, and it is `#13`'s `CE-DROP` inverted and equally silent.
///
/// ★ The **second** doorbell is asserted to be a `Served` that moved nothing, not a
/// refusal. A ring with no new entries is the guest saying "no more work", which is a
/// legitimate doorbell — refusing it would be a different bug.
#[test]
fn a_second_doorbell_over_an_unchanged_ring_forwards_nothing() {
    let (gpu, mut vmm, rec, _pid, _cid) = guest();
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    dev.doorbell(
        Some(&mut vmm),
        GPU,
        MockArch::token_for(CE_VCHID),
        &[],
        None,
    )
    .expect("the first doorbell is served");
    assert_eq!(
        copies(&rec).len(),
        1,
        "★ non-vacuity: the first one forwarded"
    );

    dev.doorbell(
        Some(&mut vmm),
        GPU,
        MockArch::token_for(CE_VCHID),
        &[],
        None,
    )
    .expect("a doorbell over a ring with no new entries is still served");

    let seen = copies(&rec);
    assert_eq!(
        seen.len(),
        1,
        "★★★ THE COPY RAN TWICE. The guest wrote one `LAUNCH_DMA` and rang twice; the \
         entries it already ran are still sitting in its ring, and a doorbell path with no \
         cursor re-issues every one of them on a REAL engine. {seen:?}"
    );
}

/// ★★★ **The observer's NEGATIVE verdict refuses the guest's doorbell.**
///
/// `await_semaphore` returns three facts and `ce_copy` turns exactly one of them into a
/// verdict: `semaphore != payload` after `CE_COPY_TIMEOUT` ⇒
/// `RmError::Other(CE_NEVER_RETIRED)` (`kayfabe-isolate-host/src/rm.rs:2653-2657`). This
/// arm arms the mock backend with that same error and requires the guest's doorbell to
/// carry it out as a **refusal**.
///
/// ⊘ Without this arm, a wiring that calls `ce_copy` and drops its `Result` passes the
/// test above — and that wiring is precisely the forged completion: a guest told its copy
/// landed on an engine that never released the semaphore.
#[test]
fn the_observers_negative_verdict_refuses_the_guest_doorbell() {
    let (gpu, mut vmm, rec, _pid, _cid) = guest();
    rec.lock()
        .expect("recorder")
        .fail_kinds
        .insert(VerbKind::CeCopy, RmError::Other(CE_NEVER_RETIRED));
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    let got = dev.doorbell(
        Some(&mut vmm),
        GPU,
        MockArch::token_for(CE_VCHID),
        &[],
        None,
    );

    assert!(
        got.is_err(),
        "★★★ The engine never released the semaphore and the guest was told `Served`. \
         `await_semaphore`'s whole value is that it refuses to conflate 'we were never \
         woken' with 'it never landed' — a caller that discards the verdict throws that \
         away and forges the completion. Got {got:?}"
    );
    // ★ NON-VACUITY, and it cannot be the recorder's log: `MockRmBackend::ce_copy` runs
    // its injected-failure gate *before* it records, so an armed `CeCopy` leaves no
    // `RmVerb::CeCopy` behind at all. Reading the log here would assert 0 and prove
    // nothing about which refusal fired. The error itself is the witness — it must be
    // THIS verb's error, carried out unchanged, and not some earlier gate's.
    assert_eq!(
        got.map(|o| o.chan),
        Err(FwdFault::Rm {
            err: RmError::Other(CE_NEVER_RETIRED),
            on: None
        }),
        "★ the refusal must be the OBSERVER's, by name — an upstream refusal (an unrouted \
         token, the #177 schedule gate, a dead isolate) would also make `is_err()` true \
         while the copy was never issued at all"
    );
}

/// ★★★★ **§16.70 — A `Served` DOORBELL THAT FORWARDED NOTHING SAYS SO, BY NAME.**
///
/// `[measured 2026-08-10, boot `p2_29e7c25_planereal`]` three guest doorbells were reported
/// `3 forwarded (host channel rung)` and the guest's copy-engine scrubber died on
/// `lastCompletedPayload == lastSubmittedPayload`. `execution_plane_increments.md` §16.69.5
/// records that the evidence could not separate two stories — *the GPU ran the work and we
/// lost the completion* versus *the channel was rung with nothing in it* — because
/// `forwarded` counts the doorbells the **port** rang and has never counted work.
///
/// This is the second story, constructed: everything about the guest is unchanged except
/// that this port's address table holds no binding for the ring's own VA. The doorbell is
/// still **`Ok`** (that is correct — a channel whose ring we cannot read is not a channel to
/// refuse traffic on at this rung), the backend is asked for **nothing**, and
/// [`kayfabe_fwd::read_gpfifo_ring`] names the reason `RING-VA-UNBOUND` instead of returning
/// the bare `None` that made all six of its absences one silence.
///
/// ⊘ **The naming is the subject, not the counting.** `copies(&rec).is_empty()` is also true
/// of `a_second_doorbell_over_an_unchanged_ring_forwards_nothing`, of a ring that decodes to
/// no CE span, and of four other absences — so an assertion on the count alone would pass
/// for a completely different reason and prove nothing about which one happened. That is the
/// §16.69.5 defect one layer down, and asserting on the variant is what closes it.
#[test]
fn a_served_doorbell_that_forwarded_nothing_names_the_reason() {
    let (gpu, mut vmm, rec, pid, cid) = guest_with_gpfifo_binding(false);

    // ★ THE NAMED ABSENCE, read off the production function rather than off a print.
    let look = kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, None)
        .expect("an unbound ring VA is an absence, never a fault");
    assert_eq!(
        look,
        kayfabe_fwd::RingLook::RingVaUnbound {
            va: pb_va(GPFIFO_GPA).0
        },
        "★★★ Six different facts used to answer `Ok(None)` here, and §16.69's boot could \
         not say which of them made `3 forwarded` mean nothing. An absence with no name is \
         not a measurement. Got {look:?}"
    );

    // ★ And the doorbell over it: SERVED, with nothing forwarded.
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));
    dev.doorbell(
        Some(&mut vmm),
        GPU,
        MockArch::token_for(CE_VCHID),
        &[],
        None,
    )
    .expect("a doorbell whose ring this port cannot read is still served at this rung");
    assert!(
        copies(&rec).is_empty(),
        "★ the second story, whole: the port reports the doorbell SERVED and the only \
         function that observes a host completion was asked for nothing. Saw {:?}",
        copies(&rec)
    );
}

/// ★★★ **The POSITIVE control for the arm above** — the same fixture with the ring bound
/// answers [`kayfabe_fwd::RingLook::Ring`], and the copy is forwarded.
///
/// ⊘ Without this, `RingVaUnbound` above could be the fixture never mapping anything, and
/// the test would assert a property of the harness rather than of the binding. `[refuted
/// 2026-08-10]` §16.69.1(3) is exactly this failure at a boot's scale: a `0 forwarded`
/// over-determined by two independent impossibilities, cited as evidence for one of them.
#[test]
fn the_same_fixture_with_the_ring_bound_reads_it() {
    let (gpu, mut vmm, _rec, pid, cid) = guest_with_gpfifo_binding(true);
    let look = kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, None)
        .expect("the bound ring reads");
    assert!(
        matches!(look, kayfabe_fwd::RingLook::Ring(ref b) if !b.is_empty()),
        "★ the control must reach the ring, or the negative above proves nothing. Got {}",
        look.tag()
    );
}

/// ★★★★ **§16.71 — THE TWO RING RESOLVERS NAME THE SAME OBJECT, or this test says so.**
///
/// # ⊘ The question this closes, and why a boot alone could not close it
///
/// `[measured 2026-08-10, boots `w205_227194f_ctl` and `_real`]` two ring addresses were
/// carried out of one campaign for what `execution_plane_increments.md` §16.70.6 called
/// *"one token"*: `0x120064000` from the CPU executor's path
/// (`SharedDoorbell::try_ce_submission` → [`SharedDevice::ce_channel_facts`]) and
/// `0x420064000` from the forwarding path ([`kayfabe_fwd::read_gpfifo_ring`]). §16.70.6
/// recorded two readings it could not separate — *RM placed it differently* versus
/// *the two paths are looking at different channels* — and made the separation the gate on
/// the resolver ruling.
///
/// ★ **It is separable from the source, and this test is that separation.** Both resolvers
/// end at the *same expression*: `rmgraph.node_of_resource(chan.key).facts.gp_fifo_ring`,
/// reached through the same `proc.channels[cid]`, where `cid` came from the same
/// `route_doorbell(token)` in both cases (`SharedDevice::doorbell` routes and then hands
/// `out.chan` to `forward_ring`). ⇒ For one token, on one boot, they are **incapable** of
/// naming different channels unless the graph changed between the two lock acquisitions.
/// That is a **lifetime** answer, not a value one, and it is the third cell of the rung's
/// falsifier rather than either of the first two.
///
/// ⊘ **So this test is not corroboration and must not be read as any** — two of our own
/// computations agreeing proves nothing about the guest (`measure_at_the_boundary_not_inside`).
/// What it pins is narrower and is exactly what the boot needs: the *join key* both
/// instruments now print (`CeChannelFacts::chan_key` and the `FWD-RING` line's `key=`)
/// really does identify the same object, so a boot in which the two lines carry **different**
/// keys is a real finding and not an artefact of the two fields meaning different things.
///
/// ⚠ Run against the `bind_gpfifo = false` fixture — the very shape that produced
/// `RING-VA-UNBOUND` on the bench — so the identity is asserted on the population the
/// question is about, not on a healthy channel that never walls.
#[test]
fn the_two_ring_resolvers_name_the_same_object() {
    let (gpu, mut vmm, _rec, pid, cid) = guest_with_gpfifo_binding(false);

    // ---- The FORWARDING path's identity, read exactly where `read_gpfifo_ring` reads it.
    let (fwd_client, fwd_handle, fwd_ring) = {
        let proc = &gpu.procs[&pid];
        let chan = proc
            .channels
            .get(&cid)
            .expect("the fixture's channel is live");
        let node = gpu
            .spine
            .rmgraph
            .node_of_resource(chan.key)
            .expect("and it has a graph node — `NoChannelNode` would be a different arm");
        (
            node.key.client.0,
            node.key.handle.0,
            node.facts.gp_fifo_ring.map(|r| r.va),
        )
    };

    // ★ Confirm the forwarding resolver really does wall here, so the identity above is
    // the identity of the walling channel and not of some other one.
    let look = kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, None)
        .expect("an unbound ring VA is an absence, never a fault");
    assert_eq!(
        look,
        kayfabe_fwd::RingLook::RingVaUnbound {
            va: pb_va(GPFIFO_GPA).0
        },
        "the fixture must reproduce the bench's arm, or this asserts about a channel the \
         question is not about. Got {look:?}"
    );

    // ---- The CPU executor's identity, through the whole `route_doorbell` → graph chain.
    let dev = SharedDevice::new(gpu, LockMode::Sharded);
    let facts = dev
        .ce_channel_facts(GPU, MockArch::token_for(CE_VCHID))
        .expect("the token routes");

    assert_eq!(
        (facts.chan_key.0, facts.chan_key.1, facts.ring_va),
        (fwd_client, fwd_handle, fwd_ring),
        "★★★★ §16.70.6's open question, decided at the source: the two resolvers must \
         name the SAME object and the SAME declared ring for one token, because they read \
         one field of one node reached by one route. A failure here means the two paths \
         genuinely resolve different channels and the address-table miss is a SYMPTOM, \
         not the defect — which is the rung's `DIFFERENT` arm, and it would be the finding."
    );
    assert_eq!(
        facts.ring_va,
        Some(pb_va(GPFIFO_GPA).0),
        "⊘ and both name the ring the fixture declared — otherwise the equality above \
         could hold with both sides equally wrong"
    );
}
