//! ★ **Teardown and reclamation** — the four L1-M1 completeness gaps
//! (`l1_concurrency.md` §12.16, gaps G1 / G3 / G3b / G4).
//!
//! The audit that produced §12.16 found the core could *enter* every teardown state
//! and *leave* almost none of them: a published backing's host memory object existed
//! nowhere in core state, the reap dropped isolates under a rank-0 lock with no assert
//! to notice, it never checked that dropping them was safe, and a cancelled verb was
//! indistinguishable from a host failure. This file is the empirical half of the fix.
//!
//! The bar, per the owner's framing: *"clean cleanup on gpu getting idle, restart
//! driver, process killed, isolate can be gc collected, etc etc. no leaks, safe."* So
//! these tests assert reclamation **happens**, not that the API now permits it — and
//! the general form of "no leaks" is [`kayfabe_mocks::HostLedger`], an acquire/release
//! ledger replayed from the mock's own verb log. A test that only checked a `Binding`'s
//! shape would pass without proving anything.
//!
//! What each test pins:
//!
//! - **`g1_a_published_backing_names_the_exact_host_object_that_allocated_it`** — the
//!   `HostBacking` identity *is* the handle the backend minted.
//! - **`g1_a_published_backing_can_actually_be_freed_at_teardown`** — drive a real
//!   reclaim off core state alone and assert the `free` verb for that exact handle
//!   reaches the backend. This is the one that would have failed before G1: the handle
//!   was unrecoverable, so no reclaim could name it.
//! - **`g1_a_full_process_lifecycle_leaves_the_host_ledger_balanced`** — publish, ring,
//!   forward an engine object, then tear the process down: every host object acquired
//!   is released exactly once, every mapping unmapped exactly once.
//! - **`g3b_dropping_an_isolate_under_a_lock_panics_naming_r1`** (+ its success
//!   polarity) — the drop-side R1 door, the §12.6 shape closed one layer over.
//! - **`g3b_the_reap_drops_its_procs_with_zero_locks_held`** — the shell path: reaping
//!   through `SharedDevice` (which holds the device *write* lock to do it) must not
//!   trip the assert above, because the drop happens after the guard falls.
//! - **`g3_the_reap_defers_a_proc_whose_isolate_is_not_quiesced`** — a checked-out
//!   worker keeps its proc on the retired list instead of tearing the sandbox down
//!   under it; returning the worker makes the very next reap take it.
//! - **`g3_in_flight_is_asked_for_not_derived_from_idle_workers`** — the `Slot::Dead`
//!   trap: a lost worker must not wedge quiescence forever.
//! - **`g3_a_worker_whose_proc_retired_in_the_gap_still_reaches_its_slot`** — the
//!   hazard G3's own check creates, closed; found by the §8.4 mean test.
//! - **`g4_a_cancelled_verb_surfaces_cancelled_not_an_rm_failure`** — §12.10's
//!   wrong-reason conflation, one layer over. Asserts the EXACT variant.
//! - **`g4_a_mid_chain_failure_enumerates_the_orphans_it_could_not_free`** and
//!   **`g4_a_failing_release_reports_its_residue_instead_of_swallowing_it`** — the
//!   record that used to be a `let _ =`.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::rmgraph::ClientKey;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Orphans, Stale};
use kayfabe_isolate::{
    CancelReason, HostHandle, IsolateBox, IsolateFactory, IsolateId, RmError, VerbPlan, VerbReply,
    Worker, WorkerId,
};
use kayfabe_mocks::{HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbKind};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, ResidueClaim, Scenario, identical_handles};

/// Bounded termination for every test in this file — see [`kayfabe_mocks::watchdog`].
///
/// ★ This used to be a hand-copied local definition, one of **ten** identical ones, and
/// every one of them announced its abort with `eprintln!` — which libtest's inherited
/// output capture buffers and `abort()` then discards, so a wedged test reported a bare
/// `SIGABRT` and nothing else — measured 2026-07-31 with a standalone `cargo test` probe
/// whose watchdog thread wrote the same text twice, once via `eprintln!` (nothing reached
/// the log) and once to a real fd 2 (all of it did). The shared one writes its diagnostic,
/// including every thread's kernel wait channel, to a real descriptor;
/// `kayfabe_mocks::watchdog::the_diagnostic_reaches_a_real_descriptor` is the test that
/// fails if that stops being true.
use kayfabe_mocks::watchdog;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);
const VA: GpuVa = GpuVa(0x2_0020_0000);
const VA2: GpuVa = GpuVa(0x2_0030_0000);

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// One guest compute process on GPU0, plus the shared verb recorder.
fn one_proc_gpu() -> (Guarded<Gpu>, ProcId, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    // #177: `plan_doorbell` now refuses a channel the guest never scheduled via
    // `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`; declare every channel scheduled so tests here
    // that ring a doorbell reach their actual subject instead of `NotScheduled`.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (
        Guarded::new("teardown_reclaim", gpu, recorder.clone()),
        pid,
        recorder,
    )
}

/// The handle every `AllocSysmem` in the log minted, in order.
fn sysmem_handles(rec: &SharedRecorder) -> Vec<HostHandle> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::AllocSysmem { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect()
}

/// Every handle the log records a successful `Free` of.
fn freed_handles(rec: &SharedRecorder) -> Vec<HostHandle> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::Free { obj } => Some(*obj),
            _ => None,
        })
        .collect()
}

/// ★ **The reclaim a teardown must be able to perform, written using ONLY core state.**
///
/// This is deliberately test-side: building the production reclamation *policy* is
/// L1-M2's (§12.16 "what remains"). What the tests need — and what G1 is about — is the
/// proof that the policy is *expressible*, i.e. that every host object a proc owns is
/// nameable from the proc alone. So this walks the proc exactly as a reclaimer would:
/// every `Vas`'s bindings (unmap + free the backing, G1's whole point), the `Vas`'s own
/// host VAS, and every channel's host channel + engine objects.
///
/// Order is RM's, not ours: unmaps first, then objects. RM auto-unmaps a resource's
/// inter-mappings inside `clientFreeResource_IMPL` before `objDelete` (`ogkm-580:
/// src/nvidia/src/libraries/resserv/src/rs_client.c:830-849`, and `ogkm-610:` is
/// byte-identical at the same lines), so the ordering does not
/// protect RM — it keeps OUR mirror of the mapping honest.
fn reclaim_plan(proc: &Proc, gpu: GpuId) -> Orphans {
    let mut o = Orphans::default();
    for ((vgpu, _pdb), vas) in &proc.vases {
        if *vgpu != gpu {
            continue;
        }
        let Some(host_vas) = vas.host_vas else {
            continue;
        };
        for (_va, _len, b) in vas.table.iter() {
            // ★ G1: BOTH halves come out of the binding. Before the fix `b` carried
            // only the VA, so this loop could unmap and never free.
            if let Some(h) = b.host() {
                o.unmap.push((host_vas, h.host_va()));
                // ★ §8.2: the free is per-OBJECT, the unmap per-binding. This mirrors
                // `kayfabe_fwd::unpublish_backing`, so it must mirror its extent check
                // too — a hand-rolled reclaim that frees an arena once per slice is the
                // double free the production path was just taught to avoid.
                if h.frees_object() {
                    o.free.push(h.memory());
                }
            }
        }
        o.free.push(host_vas);
    }
    for c in proc.channels.values() {
        if c.gpu != gpu {
            continue;
        }
        o.free.extend(c.host_engine_objects.values().copied());
        o.free.extend(c.host_channel);
    }
    o
}

/// Run `orphans`' release chain on a worker checked out of `proc`'s `gpu` isolate.
fn run_release(proc: &mut Proc, gpu: GpuId, orphans: &Orphans) {
    let mut w = proc
        .isolate_mut(gpu)
        .expect("materialized isolate")
        .checkout()
        .expect("a free worker");
    let outcome = w.execute(&orphans.release_plan(), &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"));
    assert_eq!(
        outcome,
        Ok(VerbReply::Released),
        "the reclaim chain must dispose of everything it named"
    );
    proc.isolate_mut(gpu).expect("isolate").checkin(w);
}

// =================================================================================
// G1 — a published backing's host memory handle is recoverable, and freeable
// =================================================================================

/// ★ **G1's core fact.** After a successful publish the binding names the *exact*
/// `HostHandle` the backend minted for it — not a VA it happens to be mapped at.
///
/// Before the fix `commit_publish` received `VerbReply::Published { memory, .. }` and
/// stored only `host_va`; `memory` survived nowhere. This asserts identity, not shape:
/// the handle in the binding is the handle in the verb log.
#[test]
fn g1_a_published_backing_names_the_exact_host_object_that_allocated_it() {
    let (mut gpu, pid, rec) = one_proc_gpu();
    let out =
        kayfabe_fwd::publish_backing(gpu.procs.get_mut(&pid).expect("proc"), GPU, PDB, VA, 0x1000)
            .expect("publish");

    let minted = sysmem_handles(&rec);
    assert_eq!(minted.len(), 1, "exactly one sysmem object was allocated");

    let (binding, _off) = kayfabe_fwd::resolve(&gpu, GPU, PDB, VA).expect("resolves");
    let backing = binding.host().expect("the range is host-published");
    assert_eq!(
        (backing.memory(), backing.host_va()),
        (minted[0], out.host_va),
        "the binding must name the allocated host object AND its placement"
    );
    assert_eq!(binding.host_memory(), Some(minted[0]));
}

/// ★ **G1's consequence, and the assertion the coordinator asked for by name:** the
/// object can actually be *freed* at teardown, driven from core state alone.
///
/// The reclaim walks the proc (see [`reclaim_plan`]) and runs the release chain; the
/// backend must record `Free` for the exact handle the publish minted. Before G1 the
/// handle was unrecoverable, so a reclaim could unmap the range and never free the
/// memory behind it — the majority of allocated host bytes.
#[test]
fn g1_a_published_backing_can_actually_be_freed_at_teardown() {
    let (mut gpu, pid, rec) = one_proc_gpu();
    // ★ §12.35 — DECLARED RESIDUE (dangling). This test hand-rolls a reclaim chain
    // (`reclaim_plan` + `run_release`) straight onto a worker, which is its entire point:
    // it proves the host objects G1 made addressable really CAN be freed. It deliberately
    // does not touch core state, so afterwards the `Vas`es and `Channel`s still name
    // handles the ledger has already released — a bypass, declared as one.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pid.0, GPU),
            "harness bypass: the release chain is run directly on a worker to prove the \
             backings are freeable, with core state deliberately left naming them",
        )
        .dangling(3, 2),
    );
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("publish");
    kayfabe_fwd::publish_backing(proc, GPU, PDB, VA2, 0x2000).expect("publish 2");
    let minted = sysmem_handles(&rec);
    assert_eq!(minted.len(), 2);

    // Guest process dies → reclaim everything it owns on this target.
    let plan = reclaim_plan(proc, GPU);
    assert!(
        plan.free.iter().any(|h| *h == minted[0]) && plan.free.iter().any(|h| *h == minted[1]),
        "the reclaim plan must NAME both backings: {plan:?}"
    );
    run_release(proc, GPU, &plan);

    let freed = freed_handles(&rec);
    for h in &minted {
        assert!(
            freed.contains(h),
            "the sysmem object {h:?} was never freed; freed = {freed:?}"
        );
    }
    // …and the mappings went with them.
    let led = rec.lock().expect("recorder").ledger();
    assert!(
        led.leaked_maps
            .values()
            .all(std::collections::BTreeSet::is_empty),
        "every published mapping must be unmapped: {:?}",
        led.leaked_maps
    );
}

/// ★ **The general invariant, over a whole process lifecycle:** acquire/release
/// balances. Publish twice, ring a doorbell (host VAS + channel + schedule), forward an
/// engine object, then tear the process down — and the ledger must show every host
/// object released exactly once and every mapping unmapped exactly once.
///
/// This is the assertion that generalises G1–G4 and that L1-M2's reclamation design is
/// meant to keep true: it catches the leak nobody thought to name, which a hand-picked
/// list of expected verbs cannot.
#[test]
fn g1_a_full_process_lifecycle_leaves_the_host_ledger_balanced() {
    let (mut gpu, pid, rec) = one_proc_gpu();
    // ★ §12.35 — DECLARED RESIDUE (dangling). This test hand-rolls a reclaim chain
    // (`reclaim_plan` + `run_release`) straight onto a worker, which is its entire point:
    // it proves the host objects G1 made addressable really CAN be freed. It deliberately
    // does not touch core state, so afterwards the `Vas`es and `Channel`s still name
    // handles the ledger has already released — a bypass, declared as one.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pid.0, GPU),
            "harness bypass: the teardown is a hand-rolled release chain run directly on \
             a worker, with core state left standing — the point being that the objects \
             are addressable, not that the core reclaimed them",
        )
        .dangling(5, 2),
    );

    {
        let proc = gpu.procs.get_mut(&pid).expect("proc");
        kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("publish");
        kayfabe_fwd::publish_backing(proc, GPU, PDB, VA2, 0x1000).expect("publish 2");
    }
    kayfabe_fwd::handle_doorbell(&mut gpu, GPU, MockArch::token_for(GR), &[VA]).expect("ring");
    kayfabe_fwd::forward_engine_object(
        &mut gpu,
        GPU,
        GR,
        kayfabe_mocks::mock_classes::COMPUTE,
        &[],
    )
    .expect("engine object forwards");

    let led = rec.lock().expect("recorder").ledger();
    assert!(
        led.leaked_count() > 0,
        "precondition: the workload really did allocate host objects"
    );

    // ---- Teardown: reclaim everything the proc owns, then let it go. ----
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    let plan = reclaim_plan(proc, GPU);
    run_release(proc, GPU, &plan);

    let led = rec.lock().expect("recorder").ledger();
    assert!(
        led.is_balanced(),
        "the host ledger must balance after a full teardown: {led:?}"
    );
}

// =================================================================================
// G3b — the reap hands back its corpses; dropping an isolate under a lock is loud
// =================================================================================

/// ★ **The drop-side R1 door.** A real `Isolate::drop` is `waitpid` + namespace
/// teardown — a blocking syscall the compiler runs at a point no call site names.
/// `Worker::execute`'s assert guards verbs and cannot see it, which is §12.6's shape
/// ("an assert guarding a wrapper rather than the thing") one layer over.
///
/// The witness is driven directly (as `kayfabe_util::lockwitness`'s own tests do) so
/// the assertion is about the drop, not about any particular lock type.
#[test]
#[should_panic(expected = "R1 no-blocking-under-lock violation")]
fn g3b_dropping_an_isolate_under_a_lock_panics_naming_r1() {
    let (factory, _rec) = MockIsolateFactory::new();
    let iso = IsolateBox::new(factory.spawn(IsolateId::new(1, GPU)));
    kayfabe_util::lockwitness::note_acquired(0); // the device lock, as the reap held it
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(iso)));
    kayfabe_util::lockwitness::note_released(0); // keep the harness thread clean
    std::panic::resume_unwind(r.expect_err("dropping an isolate under a lock must panic"));
}

/// The success polarity: with zero ranked locks held, the same drop is silent.
#[test]
fn g3b_dropping_an_isolate_with_no_lock_held_is_fine() {
    let (factory, _rec) = MockIsolateFactory::new();
    assert_eq!(kayfabe_rt::lock::held_depth(), 0);
    drop(IsolateBox::new(factory.spawn(IsolateId::new(1, GPU))));
}

/// A `Gpu` with one live compute proc, **unguarded** — the two tests below drop the
/// device on purpose (one of them mid-unwind, leaving the ledger deliberately
/// unbalanced), which is exactly what [`Guarded`]'s own drop-time audit exists to
/// forbid. Same construction as [`one_proc_gpu`] otherwise.
fn raw_proc_gpu() -> (Gpu, ProcId, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (gpu, pid, recorder)
}

/// Publish one backing, then stage this proc's whole reclaim on its `pending_release`
/// queue — the state a `Proc` drop is supposed to discharge. Returns the handles the
/// discharge must free, sorted.
fn stage_a_full_reclaim(gpu: &mut Gpu, pid: ProcId) -> Vec<HostHandle> {
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("publish");
    let plan = reclaim_plan(proc, GPU);
    let mut expected = plan.free.clone();
    expected.sort();
    assert!(
        !expected.is_empty(),
        "the staged reclaim must actually name host objects, or neither polarity below \
         is measuring anything"
    );
    proc.stage_release(GPU, plan);
    expected
}

/// ★★★ **Precondition 3 of `Proc`'s drop: a drop that runs DURING AN UNWIND issues no
/// host verb at all.**
///
/// `Proc::drop` discharges the `pending_release` queue by checking a worker out of the
/// proc's isolate and executing a release chain — a *blocking host call*, and on a real
/// isolate one that can fail and panic. Running it while a panic is already unwinding is
/// how a `Drop` turns a test failure into a **process abort**, and it is why the guard
/// is `panicking() || queue-empty` rather than either term alone.
///
/// ★ Both polarities, because either term alone satisfies half of it and a single-sided
/// test would pass with the disjunction turned into a conjunction:
///
/// - **A (the control).** An ordinary drop with a non-empty queue MUST issue the release
///   — otherwise this test could "pass" by measuring a path that never issues anything,
///   and §12.33's whole point ("removed before cleaned" must not be expressible) would be
///   unproven.
/// - **B (the property).** The identical drop, with a panic in flight, must issue
///   NOTHING. Not "fewer verbs", not "best effort" — zero.
#[test]
fn a_proc_drop_discharges_its_staged_release_but_never_during_an_unwind() {
    // ---- A: the control. Non-empty queue, no panic in flight → the release is issued.
    let (mut gpu, pid, rec) = raw_proc_gpu();
    let expected = stage_a_full_reclaim(&mut gpu, pid);
    assert_eq!(
        freed_handles(&rec),
        Vec::new(),
        "staging is pure bookkeeping — nothing may be freed until the drop"
    );
    drop(gpu);
    let mut freed = freed_handles(&rec);
    freed.sort();
    assert_eq!(
        freed, expected,
        "an ordinary Proc drop discharges EXACTLY its staged release queue",
    );

    // ---- B: the property. Identical state, dropped inside an unwind → nothing at all.
    let (mut gpu, pid, rec) = raw_proc_gpu();
    let expected = stage_a_full_reclaim(&mut gpu, pid);
    assert!(!expected.is_empty());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // the panic below is the test's instrument
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _dropped_mid_unwind = gpu;
        panic!("a teardown-time panic, with a full reclaim still staged");
    }));
    std::panic::set_hook(hook);
    assert!(
        caught.is_err(),
        "the harness's own panic must have unwound through the drop"
    );
    assert_eq!(
        freed_handles(&rec),
        Vec::new(),
        "★★ a Drop running during an unwind must issue ZERO host verbs — the staged \
         queue's disposition falls back to §7.0 namespace death, which is a different \
         disposition, not a failure",
    );
}

/// ★ **The shell path.** `SharedDevice::reap_retired` takes the device **write** lock
/// to run the reap, and reaping a proc drops its isolates. Those two facts used to
/// coexist silently; now the first must finish before the second starts, or the assert
/// above fires. This test is green only because `Spine::reap_retired` returns the procs
/// and the shell drops them after the guard falls.
#[test]
fn g3b_the_reap_drops_its_procs_with_zero_locks_held() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, pid, _rec) = one_proc_gpu();
        let device = gpu.map(|g| SharedDevice::new(g, mode));
        device
            .publish_backing(GPU, PDB, VA, 0x1000)
            .expect("({mode:?}) publish");

        // Guest teardown: the client root goes away, so `refresh` retires the proc.
        device
            .apply(RmEvent::Free {
                client: CLIENT,
                handle: identical_handles(GR.0, CE.0).client_root,
            })
            .expect("teardown applies");
        assert_eq!(
            device.retired_len(),
            1,
            "({mode:?}) retired, not yet reaped"
        );

        // The drop happens INSIDE this call, after the write guard is released.
        assert_eq!(device.reap_retired(), 1, "({mode:?}) reaped exactly once");
        assert_eq!(device.retired_len(), 0, "({mode:?}) and the list is empty");
        assert_eq!(device.reap_retired(), 0, "({mode:?}) and never again");
        assert_eq!(
            kayfabe_rt::lock::held_depth(),
            0,
            "({mode:?}) the test thread leaked no guard"
        );
        let _ = pid;
    }
}

// =================================================================================
// G3 — quiesce is checked, not trusted
// =================================================================================

/// ★★ **The reap must not tear a sandbox down under a live connection.**
///
/// `SharedDevice::verb_op` checks a worker out, releases every lock, and runs the chain
/// on a foreign thread's stack. The executor may legally run the reap in that gap. With
/// an unconditional reap that is a use-after-free: the isolate is dropped while a
/// `Box<dyn RmBackend>` into it is live, and the op's own orphan disposal then runs
/// against a sandbox that is gone.
///
/// So the reap **checks** [`Proc::is_quiesced`] and puts a non-quiesced proc back. This
/// drives the exact shape by hand: worker out → retire → reap defers → worker back →
/// reap takes it. Nothing is lost; the arena recycles one quiesce point later.
#[test]
fn g3_the_reap_defers_a_proc_whose_isolate_is_not_quiesced() {
    let (mut gpu, pid, _rec) = one_proc_gpu();
    kayfabe_fwd::publish_backing(gpu.procs.get_mut(&pid).expect("proc"), GPU, PDB, VA, 0x1000)
        .expect("publish");

    // A verb is in flight: its worker is checked OUT, exactly as `verb_op`'s gap has it.
    let worker: Worker = gpu
        .procs
        .get_mut(&pid)
        .expect("proc")
        .isolate_mut(GPU)
        .expect("isolate")
        .checkout()
        .expect("a free worker");
    assert!(
        !gpu.procs[&pid].is_quiesced(),
        "a checked-out worker means NOT quiesced"
    );

    // The guest tears the process down in the meantime.
    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: identical_handles(GR.0, CE.0).client_root,
    })
    .expect("teardown applies");
    assert_eq!(gpu.retired_len(), 1);

    // ★ The reap runs and takes NOTHING — the sandbox stays up under its live worker.
    let first = gpu.reap_retired();
    assert_eq!(
        (first.len(), first.deferred()),
        (0, 1),
        "a non-quiesced proc must be DEFERRED, not dropped"
    );
    drop(first);
    assert_eq!(gpu.retired_len(), 1, "and it is still on the retired list");

    // The verb lands; the worker goes back to the (retired) proc's pool.
    assert!(
        gpu.spine.checkin_retired(pid, GPU, worker),
        "a retired proc still accepts returns"
    );

    // ★ Now it reaps, and the arena recycles.
    let second = gpu.reap_retired();
    assert_eq!(
        (second.len(), second.deferred()),
        (1, 0),
        "once quiesced, the very next reap takes it"
    );
    drop(second);
    assert_eq!(gpu.retired_len(), 0);
}

/// ★ **The `Slot::Dead` trap.** In-flight must be *asked for*, never derived as
/// `pool_size() - idle_workers()`: a slot that died out of band is neither idle nor in
/// flight and can never become either (§7.3, "no resurrect"). Deriving it by
/// subtraction would report a lost worker as a live round trip forever — an isolate
/// that never quiesces, a proc that never reaps, an arena that never recycles.
#[test]
fn g3_in_flight_is_asked_for_not_derived_from_idle_workers() {
    let (factory, _rec) = MockIsolateFactory::with_pool_size(2);
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    assert!(iso.is_quiesced(), "a fresh pool is quiesced");

    let w = iso.checkout().expect("worker 0");
    assert_eq!(iso.in_flight(), 1);
    assert!(!iso.is_quiesced());
    iso.checkin(w);
    assert!(iso.is_quiesced());

    // A worker dies out of band. Its slot is permanently dead.
    assert!(iso.worker_died(WorkerId(0)));
    assert_eq!(iso.idle_workers(), 1, "one idle, one dead");
    assert_eq!(iso.pool_size(), 2);
    assert_eq!(
        iso.in_flight(),
        0,
        "a DEAD slot is not in flight — the subtraction pool_size - idle would say 1"
    );
    assert!(
        iso.is_quiesced(),
        "an isolate that lost a worker must still be able to quiesce, or it never reaps"
    );
}

/// ★★ **The hazard G3's own check creates, closed — and it is the real path, not a
/// hand-driven one.**
///
/// A verb executes with every lock released, so its proc can be retired *in that gap*;
/// the ordinary return path then misses on the live map. Dropping the handle there used
/// to be merely untidy — with the quiesce check it wedges the isolate at "checked out"
/// forever, so the proc is deferred at every quiesce point for the life of the device
/// and its GPA arena never recycles (#80). A leak is not an acceptable price for
/// closing a use-after-free.
///
/// Found by the §8.4 mean test, which reaped 1 of 2 the moment the check landed. Driven
/// here exactly as it happens: a verb held pending in the backend, an out-of-band
/// retire while it is held, then release.
#[test]
fn g3_a_worker_whose_proc_retired_in_the_gap_still_reaches_its_slot() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let _wd = watchdog("worker_returns_after_retire", Duration::from_secs(60));
        let (gpu, pid, rec) = one_proc_gpu();
        let mut device = gpu.map(|g| Arc::new(SharedDevice::new(g, mode)));
        // ★ §12.35 — DECLARED RESIDUE: the proc is retired VIOLENTLY mid-verb, so its
        // isolate is stopped and the host VAS its parked publish had already materialized
        // cannot be released per object. §7.0 namespace death is the disposition.
        device.declare_residue(
            ResidueClaim::on(
                IsolateId::new(pid.0, GPU),
                "an out-of-band `retire_proc` fires while a publish is parked in its \
                 sysmem alloc; the isolate is stopped, so the host VAS it had already \
                 materialized is left to the session's death (§7.0)",
            )
            .objects(kayfabe_mocks::VerbKind::AllocVaSpace, 1),
        );

        // ★★ M2-e — this verb's host wait is declared UNINTERRUPTIBLE, which is what RM's
        // own source says is the usual case (`l1_os_shell.md` §7.9, §12.26: the API lock
        // is a `down_write`, the GSP RPC busy-polls with no signal check). Without it the
        // retire below now CANCELS the parked verb, its thread returns the worker
        // immediately, and the "reap defers because a worker is still checked out"
        // assertion becomes a race against that thread — which it lost. The G3 interlock
        // this test exists for is about the worker reaching its slot, not about
        // cancellation, so the right fix is to keep the worker genuinely checked out
        // across the retire rather than to weaken the assertion.
        rec.lock()
            .expect("recorder")
            .never_cancels
            .insert(VerbKind::AllocSysmem);

        // Hold this proc's sysmem alloc pending: the worker is checked OUT and the
        // thread is inside the backend with ZERO locks held (R1) — `verb_op`'s gap.
        let held = rec.lock().expect("recorder").hold(HoldSpec::on_isolate(
            IsolateId::new(pid.0, GPU),
            VerbKind::AllocSysmem,
        ));

        let d = Arc::clone(&device);
        let t = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();

        // The world moves under it: an out-of-band retire (the §7.3 worker-death
        // teardown route) takes the proc out of the live set entirely.
        assert!(device.retire_proc(pid), "({mode:?}) retired out of band");
        assert_eq!(device.retired_len(), 1);
        assert_eq!(
            device.reap_retired(),
            0,
            "({mode:?}) deferred — a worker of this isolate is still checked out"
        );

        held.release();
        // EXACT variant, never `is_err()` (§12.10's lesson): the commit re-validated
        // and found its proc gone, which is divergent staleness — not an RM failure,
        // not a cancellation. (★★ M2-e: reachable here precisely BECAUSE the wait is
        // declared uninterruptible above. The cancelled flavour of this same script is
        // `cancellation.rs::every_checked_out_worker_of_a_dying_proc_is_cancelled`.)
        assert_eq!(
            t.join().expect("the publishing thread joins"),
            Err(FwdFault::Stale(Stale::Proc(pid))),
            "({mode:?}) the op must refuse with the fault that names what happened"
        );

        // ★ The worker went back to the RETIRED proc's pool, so the isolate quiesces
        // and the very next reap takes it. Without the fallback this stays 0 forever.
        assert_eq!(
            device.reap_retired(),
            1,
            "({mode:?}) a returned worker must let a retired isolate quiesce"
        );
        assert_eq!(device.retired_len(), 0);
    }
}

// =================================================================================
// G4 — cancellation vocabulary, and a mid-chain failure's orphans
// =================================================================================

/// ★★ **§12.10's wrong-reason conflation, one layer over.**
///
/// A cancelled verb (`RmError::Interrupted`, the §5.4 handshake's reply) used to arrive
/// as `RmError::Other(n)` and the failure-path re-validation resolved it to
/// `FwdFault::Rm(e)` **whenever the proc was still live** — which is the *normal*
/// cancellation case: a guest thread dies, its process does not. The op then reported
/// "the host refused" about a host that did exactly what it was asked.
///
/// Asserts the EXACT variant, per §12.10's own lesson about canaries that pass for the
/// wrong reason.
#[test]
fn g4_a_cancelled_verb_surfaces_cancelled_not_an_rm_failure() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, pid, rec) = one_proc_gpu();
        let device = gpu.map(|g| SharedDevice::new(g, mode));

        rec.lock().expect("recorder").fail_next = Some(RmError::Interrupted);
        let out = device.publish_backing(GPU, PDB, VA, 0x1000);
        assert_eq!(
            out,
            Err(FwdFault::Cancelled {
                proc: pid,
                // `fail_next` injects the error straight into the backend without ever
                // going through the §7.2 cancel seam, so nothing OBSERVED a reason. The
                // fallback is `GuestSignal` — §5.4's founding case — never a guess and
                // never a silent re-type as a host failure.
                reason: CancelReason::GuestSignal,
            }),
            "({mode:?}) a cancelled verb is CANCELLED — not Rm(..), not Stale(..)"
        );

        // The proc is untouched and still serves: cancellation is a fact about one
        // requester, never about the process or the device.
        assert!(
            device.publish_backing(GPU, PDB, VA, 0x1000).is_ok(),
            "({mode:?}) the proc keeps working after one of its ops is cancelled"
        );

        // …and a genuine host failure still reads as one, so the new variant did not
        // simply swallow the old distinction.
        rec.lock().expect("recorder").fail_next = Some(RmError::NoMemory);
        assert_eq!(
            device.publish_backing(GPU, PDB, VA2, 0x1000),
            Err(FwdFault::Rm {
                err: RmError::NoMemory,
                on: None
            }),
            "({mode:?}) a real host refusal is still Rm(..)"
        );
    }
}

/// ★ **The record that used to be a `let _ =`.** `Worker::execute` promised
/// all-or-nothing by unwinding internally, and the unwind's own `free`s were discarded.
/// So a chain that failed mid-way *and* could not clean up left host objects in no
/// `Orphans`, in no core state, enumerable from nothing.
///
/// Scripted with a standing per-kind failure (the map fails, and so does every `free`
/// the unwind then attempts) because that is the only shape in which the residue is
/// non-empty — a failing teardown, which is exactly the case the vocabulary is for.
#[test]
fn g4_a_mid_chain_failure_enumerates_the_orphans_it_could_not_free() {
    let (factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut w = iso.checkout().expect("fresh pool");

    {
        let mut r = rec.lock().expect("recorder");
        r.fail_kinds.insert(VerbKind::MapGpuVa, RmError::NoMemory);
        r.fail_kinds
            .insert(VerbKind::Free, RmError::Other(0xdead_beef));
    }
    let failure = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: VA,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect_err("the map fails, so the chain fails");

    // The CAUSE is the original error, never the cleanup's.
    assert_eq!(
        failure.err,
        RmError::NoMemory,
        "the surfaced cause must be the chain's failure, not the unwind's"
    );
    // The RESIDUE names exactly what the unwind could not dispose of, newest first:
    // the sysmem object, then the host VAS the chain allocated for it.
    let minted: Vec<HostHandle> = rec
        .lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::AllocVaSpace { handle } => Some(*handle),
            RmVerb::AllocSysmem { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect();
    assert_eq!(minted.len(), 2, "the chain allocated a host VAS and sysmem");
    assert_eq!(
        failure.orphans.free,
        vec![minted[1], minted[0]],
        "every object the unwind could not free must be enumerable from the failure"
    );
    assert!(failure.orphans.unmap.is_empty(), "the map never succeeded");

    // Ledger view of the same fact: those two objects are outstanding, and they are
    // outstanding *namefully* — a reclaimer has the handles.
    let led = rec.lock().expect("recorder").ledger();
    assert_eq!(
        led.leaked_on(IsolateId::new(1, GPU)),
        minted.iter().copied().collect(),
        "the ledger and the VerbFailure must agree about what still exists"
    );
}

/// ★ The same discipline on the disposal path itself: `VerbPlan::Release` is still
/// best-effort (a refusal must not abort the rest of the disposal) but it is no longer
/// **silent** — it reports the residue, and `kayfabe_fwd::dispose_on` hands that residue
/// to the caller instead of `let _ =`-ing it.
#[test]
fn g4_a_failing_release_reports_its_residue_instead_of_swallowing_it() {
    let (factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut w = iso.checkout().expect("fresh pool");

    let reply = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: VA,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect("the chain runs");
    let VerbReply::Published {
        host_vas: Some(vas),
        memory,
        host_va,
    } = reply
    else {
        panic!("wrong reply: {reply:?}")
    };

    // A healthy release disposes of everything and reports nothing outstanding.
    let orphans = Orphans {
        unmap: vec![(vas, host_va)],
        free: vec![memory],
        guest_ram: Vec::new(),
    };
    assert!(
        kayfabe_fwd::dispose_on(&mut w, orphans).is_empty(),
        "a release that succeeds leaves no residue"
    );
    assert!(
        rec.lock()
            .expect("recorder")
            .ledger()
            .leaked_on(IsolateId::new(1, GPU))
            == [vas].into_iter().collect(),
        "only the host VAS is left (the test did not free it)"
    );

    // Now make the free fail: the residue must NAME the object, not vanish.
    rec.lock()
        .expect("recorder")
        .fail_kinds
        .insert(VerbKind::Free, RmError::Other(7));
    let residue = kayfabe_fwd::dispose_on(
        &mut w,
        Orphans {
            unmap: vec![],
            free: vec![vas],
            guest_ram: Vec::new(),
        },
    );
    assert_eq!(
        residue,
        Orphans {
            unmap: vec![],
            free: vec![vas],
            guest_ram: Vec::new(),
        },
        "a release that could not dispose of an object must hand it back"
    );

    // And the raw port surface says the same thing with the cause attached.
    let failure = w
        .execute(&VerbPlan::Release {
            unmap: vec![],
            free: vec![vas],
            guest_ram: Vec::new(),
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect_err("the free fails");
    assert_eq!(failure.err, RmError::Other(7));
    assert_eq!(failure.orphans.free, vec![vas]);
}

/// A guard against the *other* direction of G4: the new variant must not make every
/// failure look like a cancellation. `verb_fault` maps `Interrupted` and only
/// `Interrupted`.
#[test]
fn g4_verb_fault_maps_only_interrupted_to_cancelled() {
    let p = ProcId(7);
    // ★ §16.105 — the attempted-on handle is carried on EVERY arm's input, so this test
    // also witnesses that the `Interrupted` arm DROPS it (a cancellation is a fact about
    // the requester, and naming a host object there would misattribute the cause).
    let attempted = HostHandle::new(IsolateId::new(0, GPU), 0xcafe_0031);
    assert_eq!(
        kayfabe_fwd::verb_fault(
            p,
            RmError::Interrupted,
            Some(CancelReason::ProcExit),
            Some(attempted)
        ),
        FwdFault::Cancelled {
            proc: p,
            reason: CancelReason::ProcExit
        }
    );
    // …and with nothing observed it still names a reason rather than guessing.
    assert_eq!(
        kayfabe_fwd::verb_fault(p, RmError::Interrupted, None, None),
        FwdFault::Cancelled {
            proc: p,
            reason: CancelReason::GuestSignal
        }
    );
    for e in [
        RmError::NoMemory,
        RmError::InsufficientPermissions,
        RmError::BadHandle(HostHandle::new(IsolateId::new(0, GPU), 1)),
        RmError::Other(3),
    ] {
        assert_eq!(
            kayfabe_fwd::verb_fault(p, e, Some(CancelReason::ProcExit), None),
            FwdFault::Rm { err: e, on: None },
            "{e:?}"
        );
        // ★ …and when the worker DID name what it was issued against, that identity
        // reaches the fault unchanged. Without this arm the parameter could be ignored
        // and every assertion above would still pass.
        assert_eq!(
            kayfabe_fwd::verb_fault(p, e, Some(CancelReason::ProcExit), Some(attempted)),
            FwdFault::Rm {
                err: e,
                on: Some(attempted)
            },
            "{e:?}"
        );
    }
}

/// Sanity: the whole file terminates promptly (bounded-termination rule).
#[test]
fn teardown_suite_is_bounded() {
    let t = std::time::Instant::now();
    let (mut gpu, pid, _rec) = one_proc_gpu();
    kayfabe_fwd::publish_backing(gpu.procs.get_mut(&pid).expect("proc"), GPU, PDB, VA, 0x1000)
        .expect("publish");
    assert!(t.elapsed() < Duration::from_secs(10));
    let _ = ProcAnchor(ClientKey::first(CLIENT));
}

// =================================================================================
// ★ G7 — the reap routes every arena home, and SAYS SO when it cannot
// (`l1_concurrency.md` §12.19)
// =================================================================================

/// ★ **The positive polarity: a multi-target proc's arenas all reach their own window.**
///
/// The reap used to look each arena's window up by the `BTreeMap` key it was filed
/// under, which is a key the caller could get wrong; it now routes by the arena's own
/// `owner()` stamp, so there is no key to get wrong. This pins the consequence: a proc
/// spanning two GPUs is reaped, nothing is orphaned, and **both** ranges are back in
/// **their own** windows — proved by re-carving each and getting that exact range back
/// (a free list is LIFO, so the recycled range is the one just released).
#[test]
fn g7_the_reap_routes_each_arena_home_and_orphans_nothing() {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x40_0000_0000, 0x10_0000_0000);
    // ★ G9 (§12.21): realized with two physical GPUs — the entitlement.
    const GPU1: GpuId = GpuId(1);
    let mut gpu = Gpu::realize(
        Box::new(MockArch::new()),
        Box::new(factory),
        gpa,
        &[GpuId::ZERO, GPU1],
    )
    .expect("realizes");
    // ★ §12.27 — ONE client namespace has exactly ONE root, so the two subgraphs below
    // (the same client spanning GPU0 and GPU1) must name the SAME client-root handle;
    // the second `compute_process_on_gpu` then re-sends it idempotently. Naming a second
    // root handle here is a `DuplicateClientRoot`, which is what a real `hClient` — the
    // handle of its own root object — makes impossible.
    let root = identical_handles(GR.0, CE.0).client_root;

    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), Some(0));
    s.compute_process_on_gpu(
        CLIENT,
        Pdb(PDB.0 + 1),
        kayfabe_tests::ProcessHandles {
            client_root: root,
            device: HObject(0x5d00_0001),
            vaspace: HObject(0x5d00_0010),
            tsg: HObject(0x5d00_0012),
            gr_channel: HObject(0x5d00_0019),
            gr_vchid: VChid(GR.0 + 1),
            ce_channel: HObject(0x5d00_001a),
            ce_vchid: VChid(CE.0 + 1),
        },
        Some(1),
    );
    for ev in s.events {
        gpu.apply(ev).expect("a proc spanning GPU0 and GPU1");
    }
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    assert_eq!(gpu.procs[&pid].arenas.len(), 2, "one arena per target");
    let ranges: Vec<(GpuId, std::ops::Range<u64>)> = gpu.procs[&pid]
        .arenas
        .iter()
        .map(|(g, a)| (*g, a.range.clone()))
        .collect();
    for (g, a) in &gpu.procs[&pid].arenas {
        assert_eq!(
            a.owner(),
            *g,
            "an arena is stamped with the window that carved it"
        );
    }

    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: root,
    })
    .expect("teardown applies");
    let reaped = gpu.reap_retired();
    assert_eq!((reaped.len(), reaped.deferred()), (1, 0));
    assert_eq!(
        reaped.orphaned(),
        &[] as &[(GpuId, std::ops::Range<u64>)],
        "every arena found its own window"
    );
    drop(reaped);

    // Each window got ITS range back — recycled, not leaked, and not swapped.
    for (g, range) in ranges {
        let recycled = gpu
            .spine
            .targets
            .get_mut(&g)
            .expect("target")
            .gpa
            .carve()
            .expect("carves");
        assert_eq!(
            (recycled.range.clone(), recycled.owner()),
            (range, g),
            "target {g:?} must recycle its OWN released arena"
        );
    }
    let _ = GPU1;
}

/// ★ **The negative polarity: an arena the reap cannot route home is REPORTED.**
///
/// `reap_retired`'s release was `if let Some(t) = self.targets.get_mut(&gpu) { … }` — the
/// `else` arm **silently dropped the arena**, permanently losing that GPA range from the
/// device's guest-physical space with nothing said anywhere. Targets are never removed
/// today, so the arm is unreachable; that is precisely the argument that lets a silent
/// drop survive a review, and it is why the arm now records on `Reclaimed::orphaned()`
/// instead of swallowing. The condition is driven directly (the target map is public
/// state) because the point is what happens *when* it holds, not whether the core can
/// currently reach it.
#[test]
fn g7_an_arena_the_reap_cannot_route_home_is_reported_not_dropped() {
    let (mut gpu, pid, _rec) = one_proc_gpu();
    let range = gpu.procs[&pid].arenas[&GPU].range.clone();

    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: identical_handles(GR.0, CE.0).client_root,
    })
    .expect("teardown applies");

    // The arena's window is gone by the time the reap runs.
    gpu.spine
        .targets
        .remove(&GPU)
        .expect("GPU0's target existed");

    let reaped = gpu.reap_retired();
    assert_eq!(reaped.len(), 1, "the proc is still reaped");
    assert_eq!(
        reaped.orphaned(),
        &[(GPU, range)],
        "the range it could not return must be NAMED, not silently dropped"
    );
    drop(reaped);
}

// =================================================================================
// ★ G6 — the per-process arena was bump-only (`l1_concurrency.md` §12.20)
// =================================================================================

/// One guest compute process on GPU0 with a **deliberately small** arena, plus the
/// recorder. 512 KiB of GPA per proc: big enough for real work, small enough that a
/// bump-only allocator dies within a few dozen map/unmap cycles.
fn one_proc_small_arena() -> (Guarded<Gpu>, ProcId, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1_0010_0000, 0x0008_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (
        Guarded::new("teardown_reclaim", gpu, recorder.clone()),
        pid,
        recorder,
    )
}

/// ★ **The headline: a long-lived process that maps and unmaps forever keeps working.**
///
/// 512 KiB of arena, 4096 publish/reclaim cycles of 4 KiB — 16 MiB, 32× the arena. With
/// `alloc` and no `free` the 128th cycle took a permanent `FwdFault::Arena` and the
/// process was finished: "clean cleanup when the GPU goes idle" was impossible for
/// exactly the long-running process this project exists for, which is the C's #80 leak
/// (`teardown_hardening_done`) reproduced at intra-proc granularity after being fixed at
/// window and proc granularity.
///
/// The host ledger is checked at the end too, because a GPA free list that let host
/// memory leak instead would be trading one unbounded resource for another — the two
/// halves travel together in `unpublish_backing`'s returned `Orphans` for that reason.
#[test]
fn g6_a_long_lived_process_that_maps_and_unmaps_never_exhausts_its_arena() {
    let (mut gpu, pid, rec) = one_proc_small_arena();
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    let arena_len = proc.arenas[&GPU].range.end - proc.arenas[&GPU].range.start;

    let mut total = 0u64;
    for i in 0..4096u64 {
        // Rotate the VA so this is a real map/unmap stream, not one range reused.
        let va = GpuVa(0x2_0000_0000 + (i % 64) * 0x1_0000);
        kayfabe_fwd::publish_backing(proc, GPU, PDB, va, 0x1000)
            .unwrap_or_else(|e| panic!("cycle {i} could not publish: {e:?}"));
        let orphans = kayfabe_fwd::unpublish_backing(proc, GPU, PDB, va)
            .unwrap_or_else(|e| panic!("cycle {i} could not reclaim: {e:?}"));
        assert_eq!(
            orphans.free.len(),
            1,
            "the host memory travels with the GPA"
        );
        run_release(proc, GPU, &orphans);
        total += 0x1000;
    }
    assert!(
        total > arena_len * 8,
        "the test must actually exceed the arena many times over ({total:#x} vs {arena_len:#x})"
    );
    assert_eq!(
        gpu.procs[&pid].arenas[&GPU].live_bytes(),
        0,
        "every GPA byte came back"
    );
    // The host side balances too, to exactly ONE outstanding object: the `Vas`'s own
    // host VAS, which is allocated once and lives as long as the Vas does. Every one of
    // the 4096 backings and every one of their mappings is gone.
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB)]
        .host_vas
        .expect("the Vas materialized its host VAS");
    let led = rec.lock().expect("recorder").ledger();
    assert_eq!(
        led.leaked.values().flatten().copied().collect::<Vec<_>>(),
        vec![host_vas],
        "the ONLY outstanding host object is the Vas's own host VAS: {led:?}"
    );
    assert!(
        led.leaked_maps
            .values()
            .all(std::collections::BTreeSet::is_empty),
        "every published mapping was unmapped: {led:?}"
    );
    assert!(led.double_free.is_empty() && led.free_of_unknown.is_empty());
}

/// ★ **The owner's exact ask: call free on an object that is missing, and see if
/// anything races.** Nothing does — the refusal happens before the table is touched.
///
/// Three flavours of "gone", each asserting the EXACT fault:
/// 1. a VA that was never published here;
/// 2. a VA that was published and already reclaimed (the double free, at the API level —
///    at the token level it does not compile, see `GpaArena::free`'s doctest);
/// 3. a VA whose whole `Vas` died **through the real graph path** (the guest freed the
///    VASpace), so the backing is genuinely gone rather than merely forgotten.
///
/// After all three the arena must be *intact*: still able to serve, and never handing the
/// same range to two live publications.
#[test]
fn g6_reclaiming_a_backing_that_is_gone_is_loud_and_leaves_the_arena_intact() {
    let (mut gpu, pid, _rec) = one_proc_small_arena();
    let proc = gpu.procs.get_mut(&pid).expect("proc");

    // 1. Never published.
    assert_eq!(
        kayfabe_fwd::unpublish_backing(proc, GPU, PDB, VA),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
            pdb: PDB,
            va: VA
        })),
        "an unpublished VA owes the arena nothing"
    );

    // 2. Published, reclaimed, reclaimed again.
    let first = kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("publish");
    let live_after_publish = proc.arenas[&GPU].live_bytes();
    assert_eq!(live_after_publish, 0x1000);
    let reclaimed = kayfabe_fwd::unpublish_backing(proc, GPU, PDB, VA).expect("first reclaim");
    run_release(proc, GPU, &reclaimed);
    assert_eq!(proc.arenas[&GPU].live_bytes(), 0);
    assert_eq!(
        kayfabe_fwd::unpublish_backing(proc, GPU, PDB, VA),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
            pdb: PDB,
            va: VA
        })),
        "the second reclaim must be refused, not silently accepted"
    );
    assert_eq!(
        proc.arenas[&GPU].live_bytes(),
        0,
        "and it must not have corrupted the arena's accounting"
    );

    // The arena still works, and never issues one range to two live publications.
    let a = kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("republish");
    let b = kayfabe_fwd::publish_backing(proc, GPU, PDB, VA2, 0x1000).expect("publish 2");
    assert_eq!(
        a.gpa, first.gpa,
        "the reclaimed range is reused, deterministically"
    );
    assert_ne!(a.gpa, b.gpa, "two LIVE publications never share a GPA");
    assert_eq!(proc.arenas[&GPU].live_bytes(), 0x2000);

    // 3. The backing goes genuinely away through the graph: the guest frees the VASpace.
    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: identical_handles(GR.0, CE.0).vaspace,
    })
    .expect("the guest frees its VASpace");
    let proc = gpu.procs.get_mut(&pid).expect("proc still lives");
    assert_eq!(
        kayfabe_fwd::unpublish_backing(proc, GPU, PDB, VA),
        Err(FwdFault::UnknownPdb { gpu: GPU, pdb: PDB }),
        "a Vas the guest destroyed is a loud miss, not a free into a stale arena"
    );
}

/// ★ **The invariant Stage 2's safety argument rests on, and it had never been pinned:**
/// no live binding anywhere in the device points into an arena that is not its own
/// proc's — so nothing can be pointing into an arena that was released.
///
/// The reason is structural rather than enforced: `commit_publish` allocates from
/// `proc.arenas[gpu]` and only ever binds into a `Vas` of that same `Proc`, and a
/// cross-process reference cannot arise because a `DUP_OBJECT` between two clients makes
/// them **one** `Proc` (one arena, one blast radius). The test states both halves: a
/// dup-joined pair publishes from ONE arena, two unjoined procs publish from disjoint
/// ones, and after one is reaped and its range recycled to a *new* proc, the survivor's
/// bindings are still entirely inside its own arena.
#[test]
fn g6_no_live_binding_ever_points_outside_its_own_procs_arena() {
    /// Every published binding must lie inside its own proc's arena for that GPU.
    fn sweep(gpu: &Gpu) {
        for (pid, p) in &gpu.procs {
            for ((g, _pdb), vas) in &p.vases {
                for (va, len, b) in vas.table.iter() {
                    let Some(_) = b.host() else { continue }; // RPC bindings are not arena GPAs
                    let arena = p
                        .arenas
                        .get(g)
                        .unwrap_or_else(|| panic!("{pid:?} published on {g:?} with no arena"));
                    assert!(
                        b.phys() >= arena.range.start && b.phys() + len <= arena.range.end,
                        "{pid:?} VA {va:#x} is backed at GPA {:#x}, OUTSIDE its own arena {:?}",
                        b.phys(),
                        arena.range,
                    );
                }
            }
        }
    }

    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x5_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    const CB: HClient = HClient(0xB0);
    const PDB_B: Pdb = Pdb(0x3500_0000);
    const UVM: HClient = HClient(0xC0);
    const UVM_PDB: Pdb = Pdb(0x3600_0000);

    let ha = identical_handles(GR.0, CE.0);
    let mut s = Scenario::new();
    let a_vas = s.compute_process_on_gpu(CLIENT, PDB, ha, None);
    // ★ The dup arrives BEFORE either side touches its data plane (the early-arm
    // discipline, L9), so A and the peer client are ONE proc with ONE arena. ★ §12.27:
    // this must be a USER↔USER dup — genuine sharing is what merges. A dup into the
    // guest kernel's UVM session is a reference and would leave two procs.
    s.peer_dup(
        UVM,
        HObject(UVM.0),
        HObject(0x6c00_0001),
        HObject(0x6c00_0010),
        UVM_PDB,
        HObject(0x6c00_0099),
        a_vas,
    );
    s.compute_process_on_gpu(
        CB,
        PDB_B,
        kayfabe_tests::ProcessHandles {
            client_root: HObject(CB.0),
            device: HObject(0x5e00_0001),
            vaspace: HObject(0x5e00_0010),
            tsg: HObject(0x5e00_0012),
            gr_channel: HObject(0x5e00_0019),
            gr_vchid: VChid(GR.0 + 1),
            ce_channel: HObject(0x5e00_001a),
            ce_vchid: VChid(CE.0 + 1),
        },
        None,
    );
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    // The dup absorbed the UVM client's own (untouched) proc; reap that corpse now so
    // the reap below is unambiguously about A.
    drop(gpu.reap_retired());
    let pa = gpu.spine.by_pdb[&(GPU, PDB)];
    let pb = gpu.spine.by_pdb[&(GPU, PDB_B)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU, UVM_PDB)],
        pa,
        "a dup makes the two clients ONE proc — the reason cross-proc refs cannot exist"
    );

    // Both of A's Vases publish, and B publishes.
    {
        let a = gpu.procs.get_mut(&pa).expect("A");
        kayfabe_fwd::publish_backing(a, GPU, PDB, VA, 0x1000).expect("A compute publishes");
        kayfabe_fwd::publish_backing(a, GPU, UVM_PDB, VA, 0x1000).expect("A's UVM Vas too");
        assert_eq!(a.arenas.len(), 1, "…out of ONE arena");
    }
    let b_range = gpu.procs[&pb].arenas[&GPU].range.clone();
    kayfabe_fwd::publish_backing(gpu.procs.get_mut(&pb).expect("B"), GPU, PDB_B, VA, 0x1000)
        .expect("B publishes");
    sweep(&gpu);
    let a_range = gpu.procs[&pa].arenas[&GPU].range.clone();
    assert!(
        a_range.end <= b_range.start || b_range.end <= a_range.start,
        "two unjoined procs hold disjoint arenas"
    );

    // A dies; its arena goes back to the window and is recycled to a NEW proc.
    //
    // ★★★ UPDATED by `l1_concurrency.md` §12.39 finding 3 — **freeing A's client root
    // SPLITS the component**, and the split is now handled instead of collapsed.
    //
    // The peer is a separate live process that merely holds a `DUP_OBJECT` alias into A's
    // namespace. Once A's root is freed that namespace has no live root, so the edge is
    // no longer a grouping edge (§12.27: grouping needs positive evidence about a LIVE
    // declaration at both ends), and the one component becomes two: A's orphaned VASpace,
    // which RM's refcount keeps alive under the peer's alias, and the peer's own. Before
    // the split was handled BOTH halves matched this one proc, `sync_proc_to_boundary`
    // ran twice on it and the second call overwrote the first — so one half lost its
    // clients, vases and channels silently and `plan.vanishing` came out empty. This test
    // read `1` for that collapse.
    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: ha.client_root,
    })
    .expect("A's client root is freed");
    let peer = gpu.spine.by_pdb[&(GPU, UVM_PDB)];
    assert_ne!(
        peer, pa,
        "★★ the two halves of the split must be two `Proc`s — sharing one means sharing \
         one isolate, one GPA arena and one host VAS between a dead namespace and a live \
         process"
    );
    assert_eq!(
        gpu.spine.by_pdb[&(GPU, PDB)],
        pa,
        "★★ and the keeper half kept its OWN routing: the orphaned VASpace's plane must \
         not vanish from `by_pdb` because the other half was synced over it"
    );
    let peer_range = gpu.procs[&peer].arenas[&GPU].range.clone();
    assert!(
        peer_range.end <= a_range.start || a_range.end <= peer_range.start,
        "the split's two halves hold DISJOINT arenas: {peer_range:?} vs {a_range:?}"
    );

    gpu.apply(RmEvent::Free {
        client: UVM,
        handle: HObject(UVM.0),
    })
    .expect("and its UVM client with it");
    let reaped = gpu.reap_retired();
    assert_eq!(
        reaped.len(),
        2,
        "★ both halves of the split are reaped — each through the ordinary staged-death \
         path, neither dropped on the floor"
    );
    assert!(reaped.orphaned().is_empty());
    drop(reaped);

    const CC: HClient = HClient(0xD0);
    const PDB_C: Pdb = Pdb(0x3700_0000);
    let mut s2 = Scenario::new();
    s2.compute_process_on_gpu(
        CC,
        PDB_C,
        kayfabe_tests::ProcessHandles {
            client_root: HObject(CC.0),
            device: HObject(0x5f00_0001),
            vaspace: HObject(0x5f00_0010),
            tsg: HObject(0x5f00_0012),
            gr_channel: HObject(0x5f00_0019),
            gr_vchid: VChid(GR.0 + 2),
            ce_channel: HObject(0x5f00_001a),
            ce_vchid: VChid(CE.0 + 2),
        },
        None,
    );
    for ev in s2.events {
        gpu.apply(ev).expect("a new process arrives");
    }
    let pc = gpu.spine.by_pdb[&(GPU, PDB_C)];
    assert!(
        [&a_range, &peer_range].contains(&&gpu.procs[&pc].arenas[&GPU].range),
        "the new proc recycled one of the dead component's ranges (#80): got {:?}, \
         released {a_range:?} and {peer_range:?}",
        gpu.procs[&pc].arenas[&GPU].range,
    );
    kayfabe_fwd::publish_backing(gpu.procs.get_mut(&pc).expect("C"), GPU, PDB_C, VA, 0x1000)
        .expect("C publishes into the recycled range");

    // ★ The property: nothing anywhere points outside its own arena — in particular B,
    // the survivor, has nothing inside the range C now owns.
    sweep(&gpu);
    for vas in gpu.procs[&pb].vases.values() {
        for (_va, len, b) in vas.table.iter() {
            if b.host().is_some() {
                for released in [&a_range, &peer_range] {
                    assert!(
                        b.phys() + len <= released.start || b.phys() >= released.end,
                        "the survivor holds a binding inside the RELEASED arena {released:?}"
                    );
                }
            }
        }
    }
}
