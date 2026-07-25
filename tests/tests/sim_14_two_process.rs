//! SIMULATION (end-to-end, "feels like real load"): the #14 two-process shape,
//! driven through `MockArch`/`MockVmm`/`MockIsolate` — the regression test that
//! would have caught #14 **by construction** (`mode2_rust_testing_strategy.md` §4).
//!
//! Two Procs, IDENTICAL guest VAs, IDENTICAL guest RM handles, DISTINCT PDBs. The
//! assertions are the proven #14 fix (decision #14, arch §4.3.1/§4.3.3):
//!
//! 1. Each Proc gets its OWN isolate (blast-radius containment, boundary 2).
//! 2. Each Proc's `Vas` backs the identical guest VA into a DISJOINT GPA arena and a
//!    DISJOINT host VAS — the `back_sys ALREADY-MAPPED` collision cannot occur.
//! 3. Doorbell demux routes each proc's channel to ITS OWN isolate/host token
//!    (per-proc exec plane, nothing scalar/one-shot).
//! 4. A proc that only polls still gets its completion delivered off its OWN poll
//!    (the round-8 starvation is impossible).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use nvkvm_arch::ids::GpuId;
use std::time::Duration;

use nvkvm_arch::ids::{GpuVa, Pdb};
use nvkvm_completion::OsEventRef;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_fwd::{deliver_completions, handle_doorbell, poll_completions, publish_backing, resolve};
use nvkvm_mocks::{MockArch, MockIsolateFactory, MockVmm, RmVerb};
use nvkvm_tests::{Scenario, identical_handles};

/// The identical guest VA both processes use for their working set (#14 round 1:
/// working-set base 0x200200000, pushbuffers 0x2024xxxxx — the common case).
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);

/// Build a Gpu with two processes A and B, each a compute process with IDENTICAL
/// handles and DISTINCT PDBs + vChids.
fn two_process_gpu() -> (
    Gpu,
    MockVmm,
    std::sync::Arc<std::sync::Mutex<nvkvm_mocks::RmRecorder>>,
) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    // A generous window carved into 4 GiB arenas.
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    s.compute_process(
        nvkvm_arch::ids::HClient(0xAA),
        A_PDB,
        identical_handles(0x10, 0x11),
    );
    s.compute_process(
        nvkvm_arch::ids::HClient(0xBB),
        B_PDB,
        identical_handles(0x20, 0x21),
    );
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    (gpu, MockVmm::new(), recorder)
}

#[test]
fn t14_two_procs_own_isolates_and_arenas() {
    let (gpu, _vmm, _rec) = two_process_gpu();
    assert_eq!(gpu.procs.len(), 2, "one Proc per guest process");

    // Distinct isolate sessions (== ProcId), distinct GPA arenas.
    let mut sessions = Vec::new();
    let mut ranges = Vec::new();
    for p in gpu.procs.values() {
        sessions.push(p.isolates[&GpuId::ZERO].id().0);
        ranges.push(p.arenas[&GpuId::ZERO].range.clone());
    }
    sessions.sort_unstable();
    sessions.dedup();
    assert_eq!(sessions.len(), 2, "each Proc has its OWN isolate session");
    assert!(
        ranges[0].end <= ranges[1].start || ranges[1].end <= ranges[0].start,
        "the two Procs' GPA arenas are disjoint by construction"
    );
}

/// ★ THE regression test: identical guest VA in two Procs → disjoint host backing.
#[test]
fn t14_identical_va_disjoint_backing() {
    let (mut gpu, _vmm, recorder) = two_process_gpu();

    // Route by PDB (data-plane identity) to each proc, then publish the SAME guest
    // VA in each. This is exactly the collision #14 hit in the C's shared arena.
    let pid_a = *gpu
        .by_pdb
        .get(&(GpuId::ZERO, A_PDB))
        .expect("A routed by its PDB");
    let pub_a = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A publishes");

    let pid_b = *gpu
        .by_pdb
        .get(&(GpuId::ZERO, B_PDB))
        .expect("B routed by its PDB");
    let pub_b = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("B publishes — must NOT collide with A");

    // Disjoint GPA (different arenas) AND disjoint host VA (different host VASes).
    assert_ne!(pub_a.gpa, pub_b.gpa, "identical guest VA → distinct GPA");
    assert_ne!(
        pub_a.host_va, pub_b.host_va,
        "identical guest VA → distinct host VA"
    );

    // Each Vas resolves ITS OWN backing for the identical VA — no cross-talk.
    let (bind_a, off_a) = resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(SHARED_VA.0 + 0x40)).unwrap();
    let (bind_b, _off_b) = resolve(&gpu, GpuId::ZERO, B_PDB, GpuVa(SHARED_VA.0 + 0x40)).unwrap();
    assert_eq!(off_a, 0x40);
    assert_eq!(bind_a.host_va, Some(pub_a.host_va));
    assert_eq!(bind_b.host_va, Some(pub_b.host_va));
    assert_ne!(bind_a.phys, bind_b.phys);

    // The two host VASes were allocated on DIFFERENT isolates (blast-radius).
    let log = recorder.lock().unwrap();
    let vas_allocs: Vec<_> = log
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::AllocVaSpace { .. }))
        .collect();
    assert_eq!(vas_allocs.len(), 2, "one host VAS per proc");
    assert_ne!(
        vas_allocs[0].0, vas_allocs[1].0,
        "host VASes on distinct isolates"
    );
}

#[test]
fn t14_doorbell_demux_routes_to_own_isolate() {
    let (mut gpu, _vmm, recorder) = two_process_gpu();

    // Ring A's GR channel and B's GR channel via their (distinct) vChid tokens.
    let a_token = MockArch::token_for(nvkvm_arch::ids::VChid(0x10));
    let b_token = MockArch::token_for(nvkvm_arch::ids::VChid(0x20));
    let out_a = handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[]).expect("A's doorbell routes");
    let out_b = handle_doorbell(&mut gpu, GpuId::ZERO, b_token, &[]).expect("B's doorbell routes");

    assert_ne!(out_a.proc, out_b.proc, "doorbells demux to different procs");
    assert!(
        out_a.scheduled_now && out_b.scheduled_now,
        "first submission schedules each"
    );
    assert_ne!(out_a.host_token, out_b.host_token, "distinct host tokens");

    // Each channel was allocated + scheduled + rung on ITS OWN isolate session.
    let log = recorder.lock().unwrap();
    let sched: Vec<_> = log
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::Schedule { .. }))
        .collect();
    assert_eq!(
        sched.len(),
        2,
        "each proc schedules its own channel (no shared one-shot)"
    );
    assert_ne!(sched[0].0, sched[1].0, "scheduled on distinct isolates");

    // Ringing A's doorbell again does NOT re-schedule (per-proc exec state is sticky
    // per channel, not a global one-shot that starves the other proc).
    drop(log);
    let out_a2 = handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[]).expect("A rings again");
    assert!(!out_a2.scheduled_now, "already scheduled");
}

#[test]
fn t14_malformed_and_unknown_tokens_fault_loudly() {
    let (mut gpu, _vmm, _rec) = two_process_gpu();
    // A token that does not decode (hostile bytes) — MalformedToken, never a guess.
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, 0xdead_beef, &[]),
        Err(nvkvm_fwd::FwdFault::MalformedToken { .. })
    ));
    // A well-formed token for a vChid no channel registered — UnknownVchid MISS=FAULT.
    let ghost = MockArch::token_for(nvkvm_arch::ids::VChid(0xfff));
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, ghost, &[]),
        Err(nvkvm_fwd::FwdFault::UnknownVchid { .. })
    ));
}

/// ★ The round-8 starvation, reproduced at the full-Gpu level: proc B goes quiet,
/// proc A only polls — A's completion must still be delivered off A's OWN poll.
#[test]
fn t14_polling_proc_is_not_starved() {
    let (mut gpu, mut vmm, _rec) = two_process_gpu();
    let pid_a = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let pid_b = *gpu.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();

    // B completes work; its completion is observed, posted, drained, acked.
    gpu.procs
        .get_mut(&pid_b)
        .unwrap()
        .completion
        .observe(OsEventRef(0xB0))
        .unwrap();
    let batch = deliver_completions(&mut gpu, &mut vmm, GpuId::ZERO).expect("B's completion posts");
    assert_eq!(batch.events, vec![OsEventRef(0xB0)]);
    gpu.completions_drained(GpuId::ZERO);
    gpu.procs
        .get_mut(&pid_b)
        .unwrap()
        .completion
        .ack(OsEventRef(0xB0));

    // A's completion is observed and posted, but the guest drains the batch WITHOUT
    // A's waiter waking (the lost-wakeup window), and B rings no more doorbells.
    gpu.procs
        .get_mut(&pid_a)
        .unwrap()
        .completion
        .observe(OsEventRef(0xA0))
        .unwrap();
    let batch = deliver_completions(&mut gpu, &mut vmm, GpuId::ZERO).expect("A's completion posts");
    assert_eq!(batch.events, vec![OsEventRef(0xA0)]);
    gpu.completions_drained(GpuId::ZERO);

    // In the C, delivery is now dead (doorbell-driven + any_completed-gated). Here,
    // A polls (MC_SERVICE_INTERRUPTS-shaped) and its un-acked completion is re-posted
    // off A's OWN poll. Advance the virtual clock to prove no real timer is involved.
    let _ = vmm.advance(Duration::from_micros(10));
    let irqs_before = vmm.irqs.len();
    let repost =
        poll_completions(&mut gpu, &mut vmm, GpuId::ZERO, pid_a).expect("poll re-delivers A");
    assert_eq!(
        repost.events,
        vec![OsEventRef(0xA0)],
        "A's own poll re-posts A's event"
    );
    assert_eq!(
        vmm.irqs.len(),
        irqs_before + 1,
        "SWGEN0 edge raised for the re-post"
    );

    // Drain + ack: now A has nothing outstanding and a further poll posts nothing.
    gpu.completions_drained(GpuId::ZERO);
    gpu.procs
        .get_mut(&pid_a)
        .unwrap()
        .completion
        .ack(OsEventRef(0xA0));
    assert!(
        poll_completions(&mut gpu, &mut vmm, GpuId::ZERO, pid_a).is_none(),
        "no over-posting"
    );
}
