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
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Orphans, Stale};
use kayfabe_isolate::{
    HostHandle, IsolateBox, IsolateFactory, IsolateId, RmError, VerbPlan, VerbReply, Worker,
    WorkerId,
};
use kayfabe_mocks::{HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbKind};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Scenario, identical_handles};

/// Abort the process loudly if the guard is not dropped within `limit` — the suite's
/// bounded-termination rule (`concurrency_stress.rs`), so a regression that wedges a
/// teardown fails fast instead of eating the CI timeout.
/// `KAYFABE_STRESS_WATCHDOG_SECS` overrides it (TSan runs 10-20x slower).
#[must_use]
fn watchdog(test: &'static str, limit: Duration) -> WatchdogGuard {
    let limit = match std::env::var("KAYFABE_STRESS_WATCHDOG_SECS") {
        Ok(s) => Duration::from_secs(s.parse().expect("KAYFABE_STRESS_WATCHDOG_SECS: seconds")),
        Err(_) => limit,
    };
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    thread::spawn(move || {
        let deadline = WallInstant::now() + limit;
        while WallInstant::now() < deadline {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if !flag.load(Ordering::Relaxed) {
            eprintln!("WATCHDOG: {test} still running after {limit:?} — aborting the process");
            std::process::abort();
        }
    });
    WatchdogGuard(done)
}

struct WatchdogGuard(Arc<AtomicBool>);
impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

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
fn one_proc_gpu() -> (Gpu, ProcId, SharedRecorder) {
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
/// inter-mappings inside `clientFreeResource_IMPL` before `objDelete` (`ogkm:
/// src/nvidia/src/libraries/resserv/src/rs_client.c:830-849`), so the ordering does not
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
            if let Some(h) = b.host {
                o.unmap.push((host_vas, h.host_va));
                o.free.push(h.memory);
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
    let outcome = w.execute(&orphans.release_plan());
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
    let backing = binding.host.expect("the range is host-published");
    assert_eq!(
        (backing.memory, backing.host_va),
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
    let (mut factory, _rec) = MockIsolateFactory::new();
    let iso = IsolateBox::new(factory.spawn(IsolateId(1), GPU));
    kayfabe_util::lockwitness::note_acquired(0); // the device lock, as the reap held it
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(iso)));
    kayfabe_util::lockwitness::note_released(0); // keep the harness thread clean
    std::panic::resume_unwind(r.expect_err("dropping an isolate under a lock must panic"));
}

/// The success polarity: with zero ranked locks held, the same drop is silent.
#[test]
fn g3b_dropping_an_isolate_with_no_lock_held_is_fine() {
    let (mut factory, _rec) = MockIsolateFactory::new();
    assert_eq!(kayfabe_rt::lock::held_depth(), 0);
    drop(IsolateBox::new(factory.spawn(IsolateId(1), GPU)));
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
        let device = SharedDevice::new(gpu, mode);
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
    let (mut factory, _rec) = MockIsolateFactory::with_pool_size(2);
    let mut iso = factory.spawn(IsolateId(1), GPU);
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
        let device = Arc::new(SharedDevice::new(gpu, mode));

        // Hold this proc's sysmem alloc pending: the worker is checked OUT and the
        // thread is inside the backend with ZERO locks held (R1) — `verb_op`'s gap.
        let held = rec.lock().expect("recorder").hold(HoldSpec::on_isolate(
            IsolateId(pid.0),
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
        // not a cancellation.
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
        let device = SharedDevice::new(gpu, mode);

        rec.lock().expect("recorder").fail_next = Some(RmError::Interrupted);
        let out = device.publish_backing(GPU, PDB, VA, 0x1000);
        assert_eq!(
            out,
            Err(FwdFault::Cancelled { proc: pid }),
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
            Err(FwdFault::Rm(RmError::NoMemory)),
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
    let (mut factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId(1), GPU);
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
        })
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
        led.leaked_on(IsolateId(1)),
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
    let (mut factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId(1), GPU);
    let mut w = iso.checkout().expect("fresh pool");

    let reply = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        })
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
    };
    assert!(
        kayfabe_fwd::dispose_on(&mut w, orphans).is_empty(),
        "a release that succeeds leaves no residue"
    );
    assert!(
        rec.lock()
            .expect("recorder")
            .ledger()
            .leaked_on(IsolateId(1))
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
        },
    );
    assert_eq!(
        residue,
        Orphans {
            unmap: vec![],
            free: vec![vas]
        },
        "a release that could not dispose of an object must hand it back"
    );

    // And the raw port surface says the same thing with the cause attached.
    let failure = w
        .execute(&VerbPlan::Release {
            unmap: vec![],
            free: vec![vas],
        })
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
    assert_eq!(
        kayfabe_fwd::verb_fault(p, RmError::Interrupted),
        FwdFault::Cancelled { proc: p }
    );
    for e in [
        RmError::NoMemory,
        RmError::InsufficientPermissions,
        RmError::BadHandle(HostHandle(1)),
        RmError::Other(3),
    ] {
        assert_eq!(kayfabe_fwd::verb_fault(p, e), FwdFault::Rm(e), "{e:?}");
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
    let _ = ProcAnchor(CLIENT);
}
