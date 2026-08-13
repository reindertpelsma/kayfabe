//! # ★★★ E6 — **THE JOIN**: a guest's copy-engine request becomes a host verb
//! (`docs/design/execution_plane_increments.md`, the E6 row).
//!
//! > **E6** — the join: guest CE copy → `plan_doorbell` → `Worker::execute` →
//! > `HostRmBackend::ce_copy_outcome`.
//! > *acceptance that could fail*: `CeEvidence::copied()` — R17's predicate, driven by the
//! > guest.
//! > *control*: the same boot with `KAYFABE_ISOLATES` unset must **not** produce it.
//!
//! # ⊘ What this file is, and what it is NOT
//!
//! `CeEvidence::copied()` reads **device memory** through an independent mapping. It needs
//! a GPU, so it lives in [`e6_hw_join.rs`](../e6_hw_join.rs), gated on one being present.
//! **This file is the other half**: the composition itself, GPU-free, driven through the
//! mocks — *did the guest's own operands, unaltered and in submission order, reach
//! `RmBackend::ce_copy` in the ringing channel's own host VAS?* A hardware run cannot
//! answer that (a copy that lands proves the bytes moved, not that they were the bytes the
//! guest named), and this file cannot answer the hardware's question. Neither substitutes
//! for the other and both are cited in the E6 evidence log.
//!
//! # ★★★ THE CONTROL, and it is E6's own row read literally
//!
//! *"the same boot with `KAYFABE_ISOLATES` unset must not produce it"* — the shipped
//! archive's default plane is [`kayfabe_isolate::StillbornIsolates`]. So the control is
//! **the same guest ring, the same core, an isolate plane that can never serve a verb**,
//! and the bar is that it produces no copy *and says so by name*.
//!
//! ★ Writing that control is what found the defect
//! [`a_permanently_dead_isolate_is_REFUSED_and_does_not_park_forever`] pins: it did not
//! refuse, it **hung**. `kayfabe_isolate::Isolate::checkout` answers `None` both for a
//! saturated pool and for an isolate that will never serve again, `kayfabe_fwd::checkout`
//! passed both up as `Ok(None)`, and `SharedDevice::verb_op` treats that as backpressure —
//! park on the pool gate and re-enter. A stillborn isolate has pool width **zero**, so the
//! generation never moves and the vCPU thread parks forever. It was unreachable until
//! E6 because nothing had ever routed as far as a checkout.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals
#![allow(non_snake_case)] // one test name shouts REFUSED, because that is the finding

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::FwdFault;
use kayfabe_isolate::{CeExecutor, CeSource, HostHandle, StillbornIsolates};
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, RmVerb, SharedRecorder,
    mock_classes as mc,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{
    Guarded, Scenario, bind_ring, bind_ring_dev, identical_handles, script_ring_via,
};

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xE6);
const PDB0: Pdb = Pdb(0x4001_0000);

/// The guest's two operands. Both are **published** below, so both partition to
/// `Representability::HostBacked` and the plan chooses a real engine for them.
const SRC_VA: GpuVa = GpuVa(0x2_0000_0000);
const DST_VA: GpuVa = GpuVa(0x2_0010_0000);
const COPY_LEN: u64 = 0x1000;

/// Where the ring's method words are scripted into guest RAM.
const RING_GPA: u64 = 0x5000_0000;

/// A VA nothing ever publishes — the operand that must be forwarded as `Untracked`
/// rather than guessed into a binding. Deliberately far from both operands above.
const NEVER_PUBLISHED: GpuVa = GpuVa(0x7_0000_0000);

// =====================================================================================
// Scaffolding
// =====================================================================================

/// One compute proc with one channel on one declared VAS.
fn device() -> (Guarded<Gpu>, MockVmm, SharedRecorder, ProcId, ChanId) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB0, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");
    (
        Guarded::new("e6_join", gpu, rec.clone()),
        MockVmm::new(),
        rec,
        pid,
        cid,
    )
}

/// ★ The **control's** device: the same guest, the same core, and the isolate plane the
/// shipped archive installs when `KAYFABE_ISOLATES` is unset — one that can never serve a
/// verb. Not `Guarded`, because there is no recorder: nothing was ever acquired to leak.
fn stillborn_device() -> (Gpu, MockVmm, ProcId, ChanId) {
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(StillbornIsolates::new(
            "no forwarding plane in this archive (KAYFABE_ISOLATES unset)",
        )),
        gpa,
    )
    .expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB0, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");
    (gpu, MockVmm::new(), pid, cid)
}

/// Publish `va` in the proc's own VAS, so the operand at it is `HostBacked` and the
/// `Vas` acquires a host VAS. Returns the host memory object.
fn publish(gpu: &mut Gpu, pid: ProcId, va: GpuVa) -> HostHandle {
    let proc = gpu.procs.get_mut(&pid).expect("live");
    kayfabe_fwd::publish_backing(proc, GPU, PDB0, va, COPY_LEN)
        .expect("the range publishes")
        .memory
}

/// The guest's ring: bind the CE class on a subchannel, then one `LAUNCH_DMA` naming
/// **both** operands virtually — the shape a real `AMPERE_DMA_COPY_B` copy has.
fn ce_ring(vmm: &mut MockVmm, dst: GpuVa, src: GpuVa, len: u64) -> Vec<u8> {
    script_ring_via(
        vmm,
        RING_GPA,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma_full(
                dst.0,
                true,
                src,
                true,
                len,
                kayfabe_arch::CeWork::Copy,
            ),
        ],
    )
}

/// Every `ce_copy` the backend was asked to perform, in order.
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

/// The host VAS every `ce_copy` named — as a set, so "they all went to one" is a fact
/// the assertion states rather than one it assumes by reading the first.
fn copy_vases(rec: &SharedRecorder) -> Vec<HostHandle> {
    let mut v: Vec<HostHandle> = rec
        .lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_iso, r)| match r {
            RmVerb::CeCopy { vas, .. } => Some(*vas),
            _ => None,
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

// =====================================================================================
// ★★★ THE ACCEPTANCE
// =====================================================================================

/// ★★★ **The guest's own CE operands reach `RmBackend::ce_copy`, unaltered, in the
/// ringing channel's own host VAS.**
///
/// The chain, end to end, every link a production function:
/// `submit_ring` → `parse_pushbuffer` (`read_pushbuffer` translating the GPFIFO entry's VA
/// through the channel's address table → `apply_pushbuffer` → `decode_run` accumulating
/// the CE method runs → `partition_ce`) → `plan_ce` → `Worker::execute` →
/// `RmBackend::ce_copy`.
///
/// ★ The **non-vacuity** arm is first and it is not decoration: with nothing submitted the
/// backend has been asked for no copy at all, so every number below is this submission's
/// doing and not the fixture's.
#[test]
fn a_guests_ce_copy_reaches_the_backend_with_the_guests_own_operands() {
    let (mut gpu, mut vmm, rec, pid, cid) = device();
    let _src = publish(&mut gpu, pid, SRC_VA);
    let _dst = publish(&mut gpu, pid, DST_VA);

    assert!(
        copies(&rec).is_empty(),
        "★ non-vacuity: publishing must not itself move a byte"
    );

    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);

    let (parsed, forwarded) =
        kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &ring).expect("the join runs");

    // ---- what the CORE decided
    assert_eq!(
        parsed.ce_spans.len(),
        1,
        "one contiguous request over two published ranges is one sub-copy"
    );
    assert_eq!((forwarded.host_ce, forwarded.ours), (1, 0));
    assert!(forwarded.reached_hardware());
    assert_eq!((forwarded.proc, forwarded.chan), (pid, cid));

    // ---- what the BACKEND was asked to do, which is the acceptance
    let seen = copies(&rec);
    assert_eq!(seen.len(), 1, "exactly one host copy, {seen:?}");
    assert_eq!(seen[0].dst, DST_VA.0, "★ the guest's OWN destination");
    assert_eq!(
        seen[0].src,
        CeSource::Address(SRC_VA.0),
        "★ the guest's OWN source"
    );
    assert_eq!(seen[0].len, COPY_LEN, "★ the guest's OWN length");
    assert_eq!(seen[0].by, CeExecutor::HostCe, "★ on a REAL engine");

    // ---- and in the ringing channel's own host VAS, which is #14's boundary
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB0)]
        .host_vas
        .expect("publishing materialized it");
    assert_eq!(
        copy_vases(&rec),
        vec![host_vas],
        "★ one host VAS, and it is the channel's own — an address is only meaningful \
         inside the address space it is issued against"
    );
}

/// ★★ **The operands are carried, not re-derived** — a second copy at a different offset
/// and a different length produces a different instruction, so nothing above can be a
/// constant that happens to match.
#[test]
fn a_second_copy_with_different_operands_produces_a_different_instruction() {
    let (mut gpu, mut vmm, rec, pid, cid) = device();
    publish(&mut gpu, pid, SRC_VA);
    publish(&mut gpu, pid, DST_VA);

    let a = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &a);
    kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &a).expect("the first join runs");

    let half = COPY_LEN / 2;
    let b = ce_ring(
        &mut vmm,
        GpuVa(DST_VA.0 + half),
        GpuVa(SRC_VA.0 + half),
        half,
    );
    bind_ring(&mut gpu, pid, cid, &b);
    kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &b).expect("the second join runs");

    let seen = copies(&rec);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(
        (seen[0].dst, seen[0].len),
        (DST_VA.0, COPY_LEN),
        "the first instruction is unchanged by the second"
    );
    assert_eq!(
        (seen[1].dst, seen[1].src, seen[1].len),
        (DST_VA.0 + half, CeSource::Address(SRC_VA.0 + half), half),
        "★ the second names ITS operands"
    );
}

/// ★★ **Submission ORDER is preserved through the join.** A copy engine's within-request
/// ordering is what the guest's own semaphore release depends on, so `ce_spans` is a
/// sequence and the join must not turn it into a set.
#[test]
fn the_submission_order_of_a_multi_copy_ring_survives_the_join() {
    let (mut gpu, mut vmm, rec, pid, cid) = device();
    publish(&mut gpu, pid, SRC_VA);
    publish(&mut gpu, pid, DST_VA);

    // Three copies whose destinations are deliberately NOT ascending, so "in order" and
    // "sorted" are different answers and only one of them passes.
    let quarter = COPY_LEN / 4;
    let order = [2u64, 0, 1];
    let methods: Vec<(u32, Vec<u32>)> = core::iter::once(MockPushbuffer::set_object(mc::DMA_COPY))
        .chain(order.iter().map(|&i| {
            MockPushbuffer::ce_launch_dma_full(
                DST_VA.0 + i * quarter,
                true,
                GpuVa(SRC_VA.0 + i * quarter),
                true,
                quarter,
                kayfabe_arch::CeWork::Copy,
            )
        }))
        .collect();
    let ring = script_ring_via(&mut vmm, RING_GPA, &methods);
    bind_ring(&mut gpu, pid, cid, &ring);

    let (_, forwarded) =
        kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &ring).expect("the join runs");
    assert_eq!((forwarded.host_ce, forwarded.ours), (3, 0));

    assert_eq!(
        copies(&rec).iter().map(|s| s.dst).collect::<Vec<_>>(),
        order
            .iter()
            .map(|&i| DST_VA.0 + i * quarter)
            .collect::<Vec<_>>(),
        "★ the guest's order, not a sorted one"
    );
}

// =====================================================================================
// ★★★ THE CONTROL — no forwarding plane, no copy, and a NAME
// =====================================================================================

/// ★★★ **E6's control, read literally: with no isolate plane the identical guest ring
/// produces NO COPY AT ALL — and it is refused by name.**
///
/// The shipped archive's default plane is `StillbornIsolates`, so *"the same boot with
/// `KAYFABE_ISOLATES` unset"* is this device. Nothing can be published on it (the same
/// isolate is what mints host memory), so the channel's `Vas` never acquires a host
/// address space and the refusal lands at `plan_ce`'s [`FwdFault::NoHostVas`] — **before**
/// a host verb exists, which is where a refusal belongs.
///
/// ⊘ **The parse still succeeds, and that is what makes this a control rather than a
/// nothing-happened.** The address plane is not what is missing: the guest's ring is read,
/// translated and decoded into a real request, and only then is there something to refuse.
/// A control that failed earlier would be measuring the fixture.
#[test]
fn the_control_with_no_isolate_plane_produces_no_copy_and_refuses_by_name() {
    let (mut gpu, mut vmm, pid, cid) = stillborn_device();
    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);

    let parsed =
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("it PARSES");
    assert_eq!(
        parsed.ce_spans.len(),
        1,
        "★ non-vacuity: there IS a request, so the refusal below is about the PLANE"
    );

    let proc = gpu.procs.get_mut(&pid).expect("live");
    assert_eq!(
        kayfabe_fwd::exec_ce(proc, cid, &parsed.ce_spans),
        Err(FwdFault::NoHostVas {
            chan: cid,
            pdb: PDB0
        }),
        "★ no plane ⇒ no host address space ⇒ nowhere to point an engine, said by name"
    );
}

/// ★★★ **THE FINDING: a submission that DOES reach the worker pool on a dead plane is
/// REFUSED, and does not park forever.**
///
/// The control above refuses upstream of the pool. The **doorbell** does not:
/// `kayfabe_fwd::plan_doorbell` reads `Vas::host_vas` as an `Option` and lets the verb
/// chain allocate one, so it reaches `Staged::check_out` with a channel routed and an
/// isolate that can never serve. That is the exact configuration E6 makes reachable for
/// the first time — the shipped default plane, a routed channel, a guest ring.
///
/// ⊘ **The timeout is the assertion, not a safety net.** Asserting *"it returns an error"*
/// is satisfied by a call that never returns at all, because a hanging test is reported by
/// the harness as a hang rather than as this assertion failing. So the call runs on its own
/// thread and this one waits with a bound; a timeout is a **FAIL** carrying a sentence that
/// says what it means. The bound is generous (10 s) because the failure it guards against
/// is *unbounded*, not slow.
#[test]
fn a_permanently_dead_isolate_is_REFUSED_and_does_not_park_forever() {
    let (mut gpu, _vmm, pid, cid) = stillborn_device();
    // ★ #177 — the guest schedules before it rings; this test's subject is what happens
    // AFTER that (a dead isolate plane), not the scheduling gate itself.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    let vchid = gpu.procs[&pid].channels[&cid].vchid;
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    let (tx, rx) = mpsc::channel();
    let d = Arc::clone(&dev);
    std::thread::spawn(move || {
        let _ = tx.send(d.doorbell(None, GPU, MockArch::token_for(vchid), &[], None));
    });
    let got = rx.recv_timeout(Duration::from_secs(10)).expect(
        "★★★ IT PARKED. `Isolate::checkout` answers `None` for a saturated pool AND for \
         an isolate that will never serve again; treating the second as backpressure \
         parks a vCPU thread on a pool-gate generation that can never move. This is the \
         E6 finding — see FwdFault::IsolateRetired.",
    );
    assert_eq!(
        got.map(|o| o.chan),
        Err(FwdFault::IsolateRetired {
            proc: pid,
            gpu: GPU
        }),
        "★ refused BY NAME, and the name says WHICH of the two `None`s it was"
    );
}

/// ★★ The same fact one layer down, at the seam that decides it: `kayfabe_fwd::checkout`
/// must **fault** on an isolate that can never serve and **defer** on one that is merely
/// busy. Two near neighbours over one call — `testing_doctrine.md` §2 rule 3.
#[test]
fn checkout_separates_a_dead_isolate_from_a_busy_one() {
    // ---- dead: pool width zero, retired at birth ⇒ FAULT
    {
        let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
        let mut gpu = Gpu::new(
            Box::new(MockArch::new()),
            Box::new(StillbornIsolates::new("no plane")),
            gpa,
        )
        .expect("realizes");
        let mut s = Scenario::new();
        s.compute_process(CLIENT, PDB0, identical_handles(0x20, 0x21));
        for ev in s.events {
            gpu.apply(ev).expect("applies");
        }
        let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("routed");
        let proc = gpu.procs.get_mut(&pid).expect("live");
        assert_eq!(
            kayfabe_fwd::checkout(proc, GPU).map(|w| w.is_some()),
            Err(FwdFault::IsolateRetired {
                proc: pid,
                gpu: GPU
            })
        );
    }
    // ---- busy: a live pool with every worker checked out ⇒ DEFER (`Ok(None)`)
    {
        let (mut gpu, _vmm, _rec, pid, _cid) = device();
        let proc = gpu.procs.get_mut(&pid).expect("live");
        let mut held = Vec::new();
        while let Ok(Some(w)) = kayfabe_fwd::checkout(proc, GPU) {
            held.push(w);
        }
        assert!(!held.is_empty(), "★ the pool really had workers to exhaust");
        assert_eq!(
            kayfabe_fwd::checkout(proc, GPU).map(|w| w.is_some()),
            Ok(false),
            "★ a busy pool is a fact that WILL change — deferring is correct here, and \
             conflating it with the arm above is what made the arm above hang"
        );
        for w in held {
            kayfabe_fwd::checkin(proc, GPU, w);
        }
    }
}

// =====================================================================================
// The refusals `plan_ce` owns — each is a plane saying something different
// =====================================================================================

/// ★★ **A channel whose VAS was never host-published refuses BEFORE any host verb.**
///
/// Not a materialization: an empty host VAS would let the chain proceed and point a real
/// engine at addresses that resolve to nothing in it.
#[test]
fn an_unpublished_vas_refuses_by_name_and_issues_no_verb() {
    let (mut gpu, mut vmm, rec, pid, cid) = device();
    // ⊘ Nothing is published, so `Vas::host_vas` is `None` — and the operands are
    // `Untracked`, which the partition still forwards. So the request exists and it is
    // the ADDRESS SPACE that is missing, which is the refusal being asserted.
    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);

    assert_eq!(
        kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &ring).map(|(_, f)| f),
        Err(FwdFault::NoHostVas {
            chan: cid,
            pdb: PDB0
        })
    );
    assert!(
        copies(&rec).is_empty(),
        "★ refused BEFORE any host verb — nothing to orphan and nothing moved"
    );
}

/// ★★ **A `ChanId` the routed proc does not hold is named as such**, and is not folded
/// into the doorbell's device-wide routing miss.
#[test]
fn an_unknown_channel_is_named_rather_than_folded_into_the_routing_miss() {
    let (mut gpu, _vmm, _rec, pid, cid) = device();
    let ghost = ChanId(cid.0 + 999);
    let proc = gpu.procs.get_mut(&pid).expect("live");
    assert_eq!(
        kayfabe_fwd::exec_ce(proc, ghost, &[]),
        Err(FwdFault::UnknownChannel {
            proc: pid,
            chan: ghost
        })
    );
}

/// ★★ **An empty partition issues no verb and costs no worker.** A ring that carried no
/// copy is the ordinary case, and it must not consume a pool slot.
#[test]
fn an_empty_partition_issues_no_verb_and_takes_no_worker() {
    let (mut gpu, _vmm, rec, pid, cid) = device();
    let idle_before = {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        proc.isolates[&GPU].idle_workers()
    };
    let proc = gpu.procs.get_mut(&pid).expect("live");
    let out = kayfabe_fwd::exec_ce(proc, cid, &[]).expect("an empty request is legal");
    assert_eq!((out.host_ce, out.ours), (0, 0));
    assert!(!out.reached_hardware());
    assert!(copies(&rec).is_empty());
    assert_eq!(
        proc.isolates[&GPU].idle_workers(),
        idle_before,
        "★ no worker was checked out for a request that names no address"
    );
}

/// ★★ **A copy whose operands were never published still goes to the isolate — as
/// `Untracked`, never guessed into a binding.** Stated here because it is the arm a
/// reader will assume is a fault: the address plane's law is about *resolving*, and this
/// range is not being resolved. The safety net is the #14 ring gate, not a fault here.
#[test]
fn an_unpublished_operand_is_forwarded_untracked_and_never_guessed() {
    let (mut gpu, mut vmm, rec, pid, cid) = device();
    publish(&mut gpu, pid, DST_VA); // so the Vas HAS a host VAS

    let ring = ce_ring(&mut vmm, DST_VA, NEVER_PUBLISHED, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);
    let (parsed, forwarded) =
        kayfabe_fwd::submit_ring(&mut gpu, &mut vmm, pid, cid, &ring).expect("it forwards");

    assert_eq!(
        parsed.ce_spans[0].src_kind,
        Some(kayfabe_fwd::Representability::Untracked),
        "★ the never-published source is UNTRACKED — not resolved to some other range"
    );
    assert_eq!((forwarded.host_ce, forwarded.ours), (1, 0));
    assert_eq!(
        copies(&rec)[0].src,
        CeSource::Address(NEVER_PUBLISHED.0),
        "★ and the address that goes out is the guest's own number, unchanged"
    );
}

// =====================================================================================
// R5 — the commit re-validates, because the bytes have already moved by then
// =====================================================================================

/// ★★ **A channel torn down between the plan and the commit refuses**, so the result is
/// never filed against whatever inherits the slot.
#[test]
fn a_channel_torn_down_in_the_gap_refuses_at_the_commit() {
    let (mut gpu, mut vmm, _rec, pid, cid) = device();
    publish(&mut gpu, pid, SRC_VA);
    publish(&mut gpu, pid, DST_VA);
    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);
    let parsed =
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    // The plan is made against a world in which the channel exists…
    let proc = gpu.procs.get_mut(&pid).expect("live");
    let planned = kayfabe_fwd::plan_ce(proc, cid, &parsed.ce_spans).expect("plans");
    // …and the commit meets one in which it does not.
    proc.channels.remove(&cid);
    let refused = kayfabe_fwd::commit_ce(
        proc,
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::CeSplit {
            host_ce: 1,
            ours: 0,
        }),
    );
    assert_eq!(
        refused.map(|f| f.host_ce).map_err(|r| (r.fault, r.retry)),
        Err((FwdFault::Stale(kayfabe_fwd::Stale::Channel(cid)), false)),
        "★ and NEVER retryable: re-running a copy that already executed performs it twice"
    );
}

/// ★ …and the non-vacuity half: the same commit against an **unchanged** world reports
/// what ran. A guard that refused unconditionally would pass the test above.
#[test]
fn the_same_commit_against_an_unchanged_world_reports_what_ran() {
    let (mut gpu, mut vmm, _rec, pid, cid) = device();
    publish(&mut gpu, pid, SRC_VA);
    publish(&mut gpu, pid, DST_VA);
    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring(&mut gpu, pid, cid, &ring);
    let parsed =
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    let proc = gpu.procs.get_mut(&pid).expect("live");
    let planned = kayfabe_fwd::plan_ce(proc, cid, &parsed.ce_spans).expect("plans");
    let out = kayfabe_fwd::commit_ce(
        proc,
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::CeSplit {
            host_ce: 1,
            ours: 0,
        }),
    )
    .expect("nothing moved");
    assert_eq!((out.host_ce, out.ours, out.chan), (1, 0, cid));
}

// =====================================================================================
// The L1 shell drives the same join through its ranked locks
// =====================================================================================

/// ★★ **`SharedDevice::submit_ring` is the same join, with each half taking and releasing
/// its own locks.** Asserted separately because the shell is where R1 is enforced: if the
/// execute phase were run under a ranked lock, `Worker::execute`'s own assert would panic
/// here and nowhere else.
#[test]
fn the_locked_shell_runs_the_join_and_holds_no_lock_across_the_host_verb() {
    let (gpu, mut vmm, rec, pid, cid) = device();
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));

    dev.publish_backing(GPU, PDB0, SRC_VA, COPY_LEN)
        .expect("src publishes");
    dev.publish_backing(GPU, PDB0, DST_VA, COPY_LEN)
        .expect("dst publishes");

    let ring = ce_ring(&mut vmm, DST_VA, SRC_VA, COPY_LEN);
    bind_ring_dev(&dev, pid, cid, &ring);

    let (parsed, forwarded) = dev
        .submit_ring(&mut vmm, pid, cid, &ring)
        .expect("the shell runs the join");
    assert_eq!(parsed.ce_spans.len(), 1);
    assert_eq!((forwarded.host_ce, forwarded.ours), (1, 0));

    let seen = copies(&rec);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].dst, seen[0].src, seen[0].len, seen[0].by),
        (
            DST_VA.0,
            CeSource::Address(SRC_VA.0),
            COPY_LEN,
            CeExecutor::HostCe
        )
    );
}
