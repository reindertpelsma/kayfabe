//! ★★ **CANCELLATION AND THE RECLAMATION LIFECYCLE** — `l1_os_shell.md` §7 (decision 12).
//!
//! > Owner, verbatim: *"cancelling operations is really important, also test that. this
//! > genuinely must not be a bolt on. clean cleanup on gpu getting idle, restart driver,
//! > process killed, isolate can be gc collected, etc etc. no leaks, safe."*
//!
//! So cancellation is tested here as **one trigger of a conservation invariant**, never
//! as a feature: every test in this file ends by asserting that every host resource it
//! acquired was released exactly once — via [`Guarded`]'s universal teardown
//! post-condition, which runs whether or not the test remembered to ask.
//!
//! ## ★ The two measured facts everything here is built on
//!
//! Both come from the C artifact and the bench, and both were *verified against the
//! sources before this file was written* rather than taken on trust:
//!
//! 1. **RM serialises every ioctl-reachable path on a per-client `down_write`, and its
//!    waits are UNINTERRUPTIBLE**. The `down_write` is
//!    `_serverLockClientWithLockInfo(.., LOCK_ACCESS_WRITE, ..)`, byte-identical at the
//!    same line in both tags (`ogkm-580: .../resserv/src/rs_server.c:778`, `ogkm-610:`
//!    idem). The wait is `_kgspRpcRecvPoll`'s `for(;;) { … osSpinLoop(); }` with no
//!    signal check at either tag (`ogkm-610: .../gpu/gsp/kernel_gsp.c:2963-3060`,
//!    `ogkm-580: :2391-2481`; 610 adds a fatal-timeout classification the 580 loop does
//!    not have, which changes what it logs, not that it cannot be interrupted).
//!    So "cancel" can never mean "interrupt the
//!    host ioctl" — it means *deliver a break signal and find out*. The mock models the
//!    denial explicitly ([`RmRecorder::never_cancels`]), because a harness in which every
//!    cancel lands would prove a property the host does not have and would make §7.5's
//!    whole escape untestable.
//! 2. **In Mode-2 the GUEST KERNEL is the garbage collector**: killing a guest process
//!    makes it free the process's client tree with no application cooperation (measured:
//!    178 `fn=10` RM-FREE RPCs, then fn-47). So our bookkeeping need not enumerate the
//!    guest's objects — but it must not *leak* the ones **we** minted on its behalf, and
//!    that is the half no guest collector can reach.
//!
//! ## What each test pins
//!
//! | test | trigger / kill point | the claim |
//! |---|---|---|
//! | `a_cancel_mid_chain_releases_exactly_what_it_had_allocated` | T1 · `MidChain(k)` | the residue is neither empty nor total, and the unwind disposes of it |
//! | `the_unwind_of_a_cancelled_chain_is_not_itself_cancelled` | T1 · refinement 4 | observing the signal DISARMS it, or the cleanup leaks everything |
//! | `a_cancel_that_loses_the_race_commits_normally` | T1 · `AfterVerb_BeforeCommit` | the reply names host objects that now exist; discarding it is the silent leak §7.3 warns about |
//! | `a_stale_txn_cancel_lands_on_no_later_operation` | T1 · txn identity | the C's refinement 4 verbatim |
//! | `every_checked_out_worker_of_a_dying_proc_is_cancelled` | T2 | *every*, not just the first |
//! | `a_guest_process_killed_mid_ioctl_cancels_and_reclaims_per_object` | T2 · clean death | the owner's "process killed" case, end to end |
//! | `a_cancel_the_host_wait_never_breaks_still_refuses_as_staleness` | T1 · RM's real behaviour | the delivered-but-not-observed arm, and the R5 commit guard behind it |
//! | `the_wedge_escape_releases_condemns_and_stages_in_one_act` | T5 · §7.5 | bounded loud failure + a leak we can NAME |
//! | `a_deferred_reap_during_teardown_defers_rather_than_reaping_under_a_live_verb` | T3 | a deadline that passes during teardown |
//! | `discharging_a_cancel_under_a_ranked_lock_panics_naming_r1` | R1 | firing a cancel is a syscall |
//!
//! Nothing here waits on a clock: every ordering edge is a [`VerbHold`] latch or a
//! `wait_until_pending` progress edge, so "the cancel arrived while the verb was parked"
//! is a fact the harness enforces rather than a race it hopes for.
//!
//! **No `/dev/kvm`, no OS.** Every test in this file is mock-only and runs on a machine
//! with no KVM device and no GPU.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::rmgraph::ClientKey;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Stale};
use kayfabe_isolate::{CancelReason, IsolateId, WorkerId};
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbHold, VerbKind,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_rt::executor::{Effect, Executor};
use kayfabe_rt::inbox::{CoreEvent, inbox};
use kayfabe_tests::{Guarded, ResidueClaim, Scenario, identical_handles};
use kayfabe_vmm::CoreEventKind;

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

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

/// ★★ **Release the parked verb on EVERY path out of a `thread::scope` body, including a
/// panicking one** — the defect class `reactor_os.rs`'s `StopOnDrop` closed, in the file
/// that had twelve instances of it.
///
/// Every test here parks a real thread inside a [`VerbHold`] and lets it out with an
/// explicit `held.release()` placed *after* the assertions that make the test worth
/// having. `VerbHold` has no `Drop`, so a failing assertion unwinds straight past that
/// statement and `thread::scope` then joins, forever, a thread nothing will ever wake.
/// Measured on this box: 3 threads in `futex_do_wait`, 205 s and climbing, and libtest
/// printed **nothing at all** — not the assertion message, not a failure, because a
/// captured panic is flushed only when the test ends. The failure that a hang hides is
/// precisely the cancellation bug this file exists to find.
///
/// Armed as the **first statement inside the closure**, so the release path is identical
/// whether the body returns or panics.
///
/// ## ★ Why this `Drop` is R1-safe (`l1_concurrency.md` §3.3)
///
/// A `Drop` is a call site, so it owes the same argument as any other. It calls exactly
/// one thing, [`VerbHold::release`], which takes the hold's own `std::sync::Mutex`, sets
/// a bool, and does a `notify_all`. That mutex carries **no rank** — it is not a
/// `kayfabe_rt::lock::RankedMutex` and never enters the `lockwitness` — so it cannot be
/// the second acquisition R3 forbids, and `release` performs no blocking call and no
/// syscall door that would call `assert_lock_free`. It also never waits on the woken
/// thread: it notifies and returns.
///
/// The ordering argument for a panic that unwinds while some `MutexGuard` is live: any
/// such guard is a **later** creation than this one — a `rec.lock()` temporary inside an
/// assertion's condition, or the `RankedMutex` guard in
/// `discharging_a_cancel_under_a_ranked_lock_panics_naming_r1` — and Rust drops
/// temporaries first and locals in reverse declaration order, so every one of them is
/// released *before* this guard runs. This `Drop` therefore always executes with zero
/// ranked locks held, which is exactly what R1 asks for.
///
/// Releasing on the success path too is deliberate and inert: `RmRecorder::hold` arms a
/// **one-shot** hold that `MockRmBackend::gate` removes from the recorder's list the
/// first time a verb matches it, so a hold that has already been consumed can never be
/// entered again and a second `release` is a flag store nobody reads. That is what makes
/// the guard correct even for `the_wedge_escape_releases_condemns_and_stages_in_one_act`,
/// where the hold is by design *never* released on the success path: there the parked
/// verb has already left through §7.5's abandon by the time the guard runs, and on the
/// failure path the release is the only thing that lets the abandoned thread be joined.
#[must_use]
struct ReleaseOnDrop<'a>(&'a VerbHold);

impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const VAS0: HObject = HObject(0x5c00_0010);

/// The warm-up VA — published before every script, so the proc's host VAS is already
/// materialized and a held chain's `MidChain(k)` state is the one the test NAMES rather
/// than an incidental first touch.
const VA_WARM: GpuVa = GpuVa(0x2_0000_0000);
/// The VA the cancelled publication targets.
const VA: GpuVa = GpuVa(0x2_0010_0000);
/// A second, disjoint VA for a sibling thread of the same proc.
const VA2: GpuVa = GpuVa(0x2_0020_0000);
/// The VA used after a cancellation, to prove the proc still serves.
const VA_AFTER: GpuVa = GpuVa(0x2_0030_0000);

/// One guest proc on GPU0, warmed up, wrapped in the teardown guard.
fn world(pool: usize, mode: LockMode) -> (Guarded<Arc<SharedDevice>>, ProcId, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::with_pool_size(pool);
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    let device = Guarded::new(
        "cancellation::world",
        Arc::new(SharedDevice::new(gpu, mode)),
        recorder.clone(),
    );
    device
        .publish_backing(GPU, PDB, VA_WARM, 0x1000)
        .expect("the warm-up publish materializes the host VAS");
    (device, pid, recorder)
}

/// Arm a one-shot hold on `verb` of `(pid's isolate, GPU, worker)`.
fn hold(rec: &SharedRecorder, pid: ProcId, worker: u32, verb: VerbKind) -> Arc<VerbHold> {
    rec.lock().expect("recorder").hold(HoldSpec::exact(
        IsolateId::new(pid.0, GPU),
        WorkerId(worker),
        verb,
    ))
}

/// Every host `free` this isolate has issued.
fn frees(rec: &SharedRecorder, pid: ProcId) -> usize {
    rec.lock()
        .expect("recorder")
        .verbs_of(IsolateId::new(pid.0, GPU))
        .iter()
        .filter(|v| matches!(v, RmVerb::Free { .. }))
        .count()
}

/// `(cancels fired at a sink, of which delivered, of which OBSERVED by a verb)`.
///
/// ★ The three are separate on purpose and asserting only the last is how this whole
/// area gets a vacuous test: a run in which nothing was ever *fired* also observes
/// nothing, and "observed == 0" would pass for a completely broken cancel seam. Every
/// assertion below names all three, or names why one of them is the point.
fn cancel_census(rec: &SharedRecorder) -> (usize, usize, usize) {
    let r = rec.lock().expect("recorder");
    (
        r.cancels_delivered.len(),
        r.cancels_delivered.iter().filter(|c| c.delivered).count(),
        r.cancels_observed.len(),
    )
}

/// Every host object outstanding across every isolate (the ledger's leak set).
fn outstanding(rec: &SharedRecorder) -> usize {
    rec.lock().expect("recorder").ledger().leaked_count()
}

/// The three corruption classes, which no disposition can excuse.
fn corruption(rec: &SharedRecorder) -> (usize, usize, usize) {
    let l = rec.lock().expect("recorder").ledger();
    (
        l.double_free.len(),
        l.free_of_unknown.len(),
        l.unmap_of_unknown.len(),
    )
}

/// How many host objects+mappings the whole device has STAGED for release, live procs
/// and corpses alike — the "scheduled, not leaked" set §12.35 distinguishes on.
fn staged_total(device: &SharedDevice) -> usize {
    let live: usize = device
        .live_pids()
        .into_iter()
        .filter_map(|p| device.with_proc(p, Proc::pending_release_len))
        .sum();
    live + device.with_retired(|c| c.iter().map(Proc::pending_release_len).sum::<usize>())
}

/// The staged host **objects** themselves, across live procs and corpses.
///
/// ★ A count is not enough for the wedge test: the proc's whole warm-up state is staged
/// by the same teardown, so "4 things are queued" is equally true of a run that staged
/// the warm-up and LOST the wedged chain's intermediate — which is the exact bug that
/// test exists to catch. Naming the handle is the only assertion that distinguishes them.
fn staged_objects(device: &SharedDevice) -> BTreeSet<kayfabe_isolate::HostHandle> {
    let mut out = BTreeSet::new();
    let mut absorb = |p: &Proc| {
        for (_gpu, o) in p.staged_releases() {
            out.extend(o.free.iter().copied());
        }
    };
    for pid in device.live_pids() {
        device.with_proc(pid, &mut absorb);
    }
    device.with_retired(|c| {
        for p in c {
            absorb(p);
        }
    });
    out
}

/// The handle of the LAST host memory object this isolate allocated — the one a
/// mid-chain script just minted, named rather than counted.
fn last_sysmem(rec: &SharedRecorder, pid: ProcId) -> kayfabe_isolate::HostHandle {
    rec.lock()
        .expect("recorder")
        .verbs_of(IsolateId::new(pid.0, GPU))
        .iter()
        .rev()
        .find_map(|v| match v {
            RmVerb::AllocSysmem { handle, .. } => Some(*handle),
            _ => None,
        })
        .expect("the script allocated one")
}

// =================================================================================
// T1 — the cancel seam itself
// =================================================================================

/// ★★ **`MidChain(k)`: the state where "release what you allocated" is neither empty nor
/// total** (`l1_os_shell.md` §7.8's KillPoint matrix — *"the one that finds real bugs"*).
///
/// The publish chain is `alloc_sysmem → map_gpu_va` (the host VAS is already
/// materialized by the warm-up). The mapping verb is held, so at the moment the break
/// signal lands the chain owns **exactly one** host memory object and no mapping. Then:
///
/// - the fault names the truth — `Cancelled { reason: GuestSignal }`, not `Rm(..)` and
///   not `Stale(..)` (§7.3, §12.10 one layer over);
/// - the chain's own unwind runs **on the same live worker** and frees exactly that one
///   object — asserted as a delta, so a run that happened to free something else does
///   not pass;
/// - the address table was not mutated: MISS = FAULT for the cancelled VA;
/// - the proc keeps serving, because cancellation is a fact about one requester and
///   never about the process or the device.
#[test]
fn a_cancel_mid_chain_releases_exactly_what_it_had_allocated() {
    let _wd = watchdog("cancel_mid_chain", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = world(2, mode);
        let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
        let before = frees(&rec, pid);

        let out = thread::scope(|sc| {
            let _release = ReleaseOnDrop(&held);
            let d: &SharedDevice = &device;
            let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
            held.wait_until_pending();
            // ★ The host memory object EXISTS at this instant — the premise of the whole
            // test, asserted rather than assumed. Without it the "neither empty nor
            // total" claim would be about an empty residue.
            assert!(
                rec.lock()
                    .expect("recorder")
                    .verbs_of(IsolateId::new(pid.0, GPU))
                    .iter()
                    .any(|v| matches!(v, RmVerb::AllocSysmem { .. })),
                "({mode:?}) the chain must have allocated before it is cancelled, or \
                 MidChain names nothing"
            );
            assert!(
                device.request_cancel(pid, GPU, WorkerId(0), CancelReason::GuestSignal),
                "({mode:?}) the break signal named a live transaction"
            );
            held.release();
            t.join().expect("the publishing thread joins")
        });

        assert_eq!(
            out,
            Err(FwdFault::Cancelled {
                proc: pid,
                reason: CancelReason::GuestSignal
            }),
            "({mode:?}) a cancelled verb must name WHY — not Rm(..), not Stale(..), and \
             not a `Cancelled` with a guessed reason"
        );
        assert_eq!(
            frees(&rec, pid) - before,
            1,
            "({mode:?}) the cancelled chain must release the ONE host object it had \
             already allocated — exactly one free, on its own still-live worker"
        );
        assert_eq!(
            device.resolve(GPU, PDB, VA),
            Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
                pdb: PDB,
                va: VA
            })),
            "({mode:?}) the cancelled publication mutated nothing"
        );
        assert_eq!(
            cancel_census(&rec),
            (1, 1, 1),
            "({mode:?}) fired, delivered, observed"
        );
        assert_eq!(corruption(&rec), (0, 0, 0), "({mode:?})");
        // The proc is untouched and still serves.
        device
            .publish_backing(GPU, PDB, VA_AFTER, 0x1000)
            .expect("the proc keeps working after one of its ops is cancelled");
    }
}

/// ★★ **§7.2 refinement 4 — cancellation is armed for exactly ONE ioctl.**
///
/// > *"Once `EINTR` is observed it disarms, so the unwind's `free`s are not themselves
/// > interrupted."*
///
/// This is the sharpest bug in the area and it is invisible from the fault variant: a
/// seam that stayed armed would interrupt the unwind's own `free`, the chain's host
/// memory object would survive, and the op would *still* return exactly
/// `Cancelled { GuestSignal }`. Only the ledger sees it.
///
/// So the assertion is the **conservation** one: after a cancelled mid-chain publish,
/// the isolate's outstanding set is back to what it was before the op started, and the
/// residue staged for later release is empty — i.e. nothing was deferred, it was
/// actually freed.
#[test]
fn the_unwind_of_a_cancelled_chain_is_not_itself_cancelled() {
    let _wd = watchdog("cancel_unwind_disarm", Duration::from_secs(60));
    let (device, pid, rec) = world(2, LockMode::Sharded);
    let before_objects = outstanding(&rec);
    let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);

    let out = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&held);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();
        assert!(device.request_cancel(pid, GPU, WorkerId(0), CancelReason::Watchdog));
        held.release();
        t.join().expect("joins")
    });
    assert_eq!(
        out,
        Err(FwdFault::Cancelled {
            proc: pid,
            reason: CancelReason::Watchdog
        })
    );

    // ★ THE ASSERTION. Equality with the pre-op count, not `<= something`: a residue of
    // one is exactly what a still-armed seam leaves, and it is one object out of a
    // warm-up that already holds several — so a bound would hide it.
    assert_eq!(
        outstanding(&rec),
        before_objects,
        "★★ §7.2 refinement 4 VIOLATED: the cancelled chain's own unwind was interrupted \
         too, so the host memory object it had allocated is still outstanding. The fault \
         variant is IDENTICAL either way — only the ledger can see this"
    );
    assert_eq!(
        staged_total(&device),
        0,
        "nothing was merely SCHEDULED for release: a live worker disposed of it there \
         and then, which is what 'the worker survives a cancellation' means"
    );
    assert_eq!(cancel_census(&rec), (1, 1, 1));
    assert_eq!(corruption(&rec), (0, 0, 0));
}

/// ★★ **§7.3's fourth row — the cancel that LOSES the race.**
///
/// > *"A cancel that arrives after the verb completed must not cause the reply to be
/// > discarded, because the reply names host objects that now exist. Discarding it leaks
/// > them silently — and it is exactly the class of bug that no refusal-shaped test
/// > finds."*
///
/// The op completes first; the request is fired afterwards and finds a txn that is over.
/// Three things are asserted together, and the first is the non-vacuity control: the
/// request really was **fired** (so `delivered == false` is a fact about the stale-txn
/// rule and not about a cancel that never happened), the publication **committed**, and
/// the ledger is clean.
#[test]
fn a_cancel_that_loses_the_race_commits_normally() {
    let _wd = watchdog("cancel_loses_race", Duration::from_secs(60));
    let (device, pid, rec) = world(2, LockMode::Sharded);

    // A handle armed for a txn that is genuinely in flight…
    let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    let (handle, published) = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&held);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();
        let h = device
            .cancel_handle(pid, GPU, WorkerId(0))
            .expect("worker 0 has a transaction in flight");
        held.release();
        (h, t.join().expect("joins"))
    });
    let published = published.expect("the publication commits — the cancel had not fired yet");

    // …fired only after the verb finished and the worker was checked back in.
    let delivered = handle.request(CancelReason::ProcExit).discharge();
    assert!(
        !delivered,
        "a cancel naming a transaction that is over must be DROPPED, never re-aimed at \
         whatever the slot is doing now"
    );
    assert_eq!(
        cancel_census(&rec),
        (1, 0, 0),
        "★ non-vacuity: the request WAS fired at the sink (1), it was not delivered (0), \
         and no verb observed it (0). Asserting only the last two would pass for a seam \
         that never fires anything at all"
    );

    // ★ The reply was committed, not discarded: the binding it computed reads back.
    let (binding, off) = device
        .resolve(GPU, PDB, GpuVa(VA.0 + 0x40))
        .expect("the committed publication resolves");
    assert_eq!(
        (binding.phys, binding.host_va(), off),
        (published.gpa, Some(published.host_va), 0x40),
        "the reply named host objects that now exist; discarding it would leak them"
    );
    assert_eq!(corruption(&rec), (0, 0, 0));
}

/// ★★ **The C's refinement 4, verbatim: a request naming a stale txn must not land on an
/// unrelated LATER operation.**
///
/// > *"main thread only signals the worker if it is still on that txn_id"* — and the C
/// > needed it because without it a cancel races the completion and lands on an
/// > innocent op.
///
/// Distinct from the test above, which only proves the stale request is dropped. Here a
/// **second, unrelated publication is in flight on the same slot** when the stale request
/// fires, so a seam that ignored txn identity would cancel *that* one. The proof is that
/// the second op commits.
#[test]
fn a_stale_txn_cancel_lands_on_no_later_operation() {
    let _wd = watchdog("cancel_stale_txn", Duration::from_secs(60));
    // Pool of ONE, so the second op is guaranteed to reuse the very slot the stale
    // handle names. With a wider pool it might land elsewhere and the test would pass
    // without ever constructing the hazard.
    let (device, pid, rec) = world(1, LockMode::Sharded);

    let first = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    let handle = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&first);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        first.wait_until_pending();
        let h = device
            .cancel_handle(pid, GPU, WorkerId(0))
            .expect("worker 0 is in flight");
        first.release();
        t.join()
            .expect("joins")
            .expect("the first publication commits");
        h
    });

    // A SECOND op is now in flight on the same slot, under a fresh txn.
    let second = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    let out = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&second);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA2, 0x1000));
        second.wait_until_pending();
        assert!(
            !handle.request(CancelReason::ProcExit).discharge(),
            "★★ the stale request was DELIVERED — it is now aimed at an innocent op"
        );
        second.release();
        t.join().expect("joins")
    });
    assert!(
        out.is_ok(),
        "★★ a stale-txn cancel killed an unrelated later operation: {out:?}"
    );
    assert_eq!(
        cancel_census(&rec),
        (1, 0, 0),
        "fired once, delivered never, observed never"
    );
    assert_eq!(corruption(&rec), (0, 0, 0));
}

// =================================================================================
// T2 — the guest process goes away
// =================================================================================

/// ★★ **§7.6 T2: "for EVERY checked-out worker, `request_cancel(ProcExit)`"** — every,
/// not just the first one the retire happened to look at.
///
/// Two sibling threads of ONE proc park verbs on two pool workers at once (§7.2: N
/// workers are N independent 1-deep channels). The proc is then retired out of band.
/// Both come back cancelled, both cancels were delivered, and the counts are exact —
/// a retire that cancelled only slot 0 would leave the second thread to run to
/// completion against a dead proc and would still look green on a one-thread test.
#[test]
fn every_checked_out_worker_of_a_dying_proc_is_cancelled() {
    let _wd = watchdog("cancel_every_worker", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (mut device, pid, rec) = world(4, mode);
        // ★ §7.0 — `retire_proc` is the VIOLENT death: it stops the isolate, so neither
        // cancelled chain can free what it already allocated and the disposition of
        // record is the session's own. Both objects are STAGED (nameable), which is why
        // this declares nothing: the audit's UNACCOUNTED set is empty.
        let _ = &mut device;

        let h0 = hold(&rec, pid, 0, VerbKind::MapGpuVa);
        let out = thread::scope(|sc| {
            let _release0 = ReleaseOnDrop(&h0);
            let d: &SharedDevice = &device;
            let t0 = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
            // Staged: `checkout` hands out the first idle slot, so waiting for slot 0 to
            // be parked before arming slot 1 makes the slot each latch names a FACT.
            h0.wait_until_pending();
            let h1 = hold(&rec, pid, 1, VerbKind::MapGpuVa);
            // Armed the instant the second latch exists — before the thread that will park
            // in it is even spawned, so there is no window in which slot 1 could be parked
            // with no guard covering it. It drops FIRST (reverse declaration order), and
            // both releases are needed: leaving either thread parked wedges the join.
            let _release1 = ReleaseOnDrop(&h1);
            let t1 = sc.spawn(move || d.publish_backing(GPU, PDB, VA2, 0x1000));
            h1.wait_until_pending();
            // Both chains have a host memory object outstanding at this instant; name
            // them now, while they are still reachable through the log's tail.
            let mem: BTreeSet<_> = rec
                .lock()
                .expect("recorder")
                .verbs_of(IsolateId::new(pid.0, GPU))
                .iter()
                .filter_map(|v| match v {
                    RmVerb::AllocSysmem { handle, .. } => Some(*handle),
                    _ => None,
                })
                .collect();
            assert_eq!(
                mem.len(),
                3,
                "({mode:?}) the warm-up's backing plus BOTH parked chains' — the premise"
            );

            assert!(device.retire_proc(pid), "({mode:?}) the proc was live");
            h0.release();
            h1.release();
            (t0.join().expect("joins"), t1.join().expect("joins"), mem)
        });
        let (out, mem) = ((out.0, out.1), out.2);
        let mut it = mem.iter().copied();
        let (_warm, m0, m1) = (
            it.next().expect("warm-up"),
            it.next().expect("chain 0"),
            it.next().expect("chain 1"),
        );

        let want = Err(FwdFault::Cancelled {
            proc: pid,
            reason: CancelReason::ProcExit,
        });
        assert_eq!(
            out,
            (want, want),
            "({mode:?}) BOTH of the dying proc's in-flight verbs must be cancelled — a \
             retire that cancels only the first slot leaves the rest running against a \
             proc that no longer exists"
        );
        assert_eq!(
            cancel_census(&rec),
            (2, 2, 2),
            "({mode:?}) two fired, two delivered, two observed — exact in all three, so \
             neither an over-broadcast nor a single-slot retire can pass"
        );
        // ★ Both cancelled chains' host memory objects are STAGED on the corpse, NAMED
        // rather than counted: the same teardown also stages the proc's whole warm-up
        // state, so a count alone would be satisfied by a run that staged the warm-up and
        // lost both intermediates.
        let staged = staged_objects(&device);
        assert!(
            staged.contains(&m0) && staged.contains(&m1),
            "({mode:?}) a retired isolate refuses the disposal too, so BOTH chains' host \
             memory objects must be STAGED on the corpse — §7.0 namespace death as a \
             STATED disposition rather than a set nothing can name. staged={staged:?}, \
             wanted {m0:?} and {m1:?}"
        );
        assert_eq!(corruption(&rec), (0, 0, 0), "({mode:?})");
    }
}

/// ★★ **The owner's case, end to end: a guest process KILLED mid-ioctl.**
///
/// This is the *clean* death and it is a different path from `retire_proc`: the guest
/// kernel frees the dead process's client root with no application cooperation (measured
/// on real GA106 — 178 `fn=10` RM-FREE RPCs then fn-47), the graph re-derives, and
/// `Spine::vacate` removes the proc. Because the death is clean the **isolate stays
/// live**, so unlike the violent path the cancelled chain can and does free what it
/// allocated.
///
/// Hence the strongest possible ending: **nothing is staged and nothing is outstanding
/// that the warm-up did not leave** — full per-object reclamation on the path the guest
/// drives, with no reliance on §7.0's backstop at all.
#[test]
fn a_guest_process_killed_mid_ioctl_cancels_and_reclaims_per_object() {
    let _wd = watchdog("cancel_process_killed", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = world(2, mode);
        let handles = identical_handles(GR.0, CE.0);
        let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);

        let out = thread::scope(|sc| {
            let _release = ReleaseOnDrop(&held);
            let d: &SharedDevice = &device;
            let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
            held.wait_until_pending();
            // ---- SIGKILL, as the core sees it.
            device
                .apply(RmEvent::Free {
                    client: CLIENT,
                    handle: handles.client_root,
                })
                .expect("the guest kernel's client-root free applies");
            held.release();
            t.join().expect("joins")
        });

        assert_eq!(
            out,
            Err(FwdFault::Cancelled {
                proc: pid,
                reason: CancelReason::ProcExit
            }),
            "({mode:?}) a verb in flight when its guest process is killed must come back \
             CANCELLED — the graph-driven path cancels exactly like the out-of-band one"
        );
        assert_eq!(cancel_census(&rec), (1, 1, 1), "({mode:?})");
        // The corpse's drop (at the reap) issues whatever it staged; a CLEAN death keeps
        // its isolate alive, so there is nothing left to stage in the first place.
        // ★ The two halves of "reclaims per object", in order. First: the corpse's whole
        // host estate — the warm-up's VAS, its backing and its mapping — is NAMED on the
        // release queue rather than dropped with the `Vas` that used to hold it (§12.33's
        // exact shape). `> 0` is the non-vacuity guard on the `== 0` below: a device that
        // never allocated anything would also end with nothing outstanding.
        assert!(
            staged_total(&device) > 0,
            "({mode:?}) the vacated proc staged nothing at all — then the conservation \
             assertion below is about an empty world"
        );
        assert_eq!(device.reap_retired(), 1, "({mode:?}) the corpse reaps");
        assert_eq!(
            outstanding(&rec),
            0,
            "★★ ({mode:?}) CONSERVATION: after a guest process is killed mid-ioctl and \
             its corpse reaped, NOTHING this device ever acquired is still outstanding — \
             not the cancelled chain's intermediate, not the warm-up's backing, not the \
             host VAS"
        );
        assert_eq!(corruption(&rec), (0, 0, 0), "({mode:?})");
    }
}

// =================================================================================
// The measured RM behaviour — delivered, and NOT observed
// =================================================================================

/// ★★★ **The arm that RM's own source says is the common one** (`l1_os_shell.md` §7.9,
/// §12.26): *"the API lock is a `down_write`, the GSP RPC busy-polls with no signal
/// check, the client drain is a bare refcount spin"* — so a delivered break signal is
/// usually **not observed**, and the verb runs to completion anyway.
///
/// This is also where the R5 commit guard that `l1_mean`'s canary (a) used to prove now
/// lives. With the host wait unbreakable the verb finishes, its commit finds the proc
/// retired, and the refusal is divergent staleness naming the exact proc — the original
/// claim, on the path where it is still reachable.
///
/// The two halves must BOTH be asserted: delivered = 1 (the signal really was sent, so
/// this is not a test of a seam that does nothing) and observed = 0 (and the host
/// ignored it).
#[test]
fn a_cancel_the_host_wait_never_breaks_still_refuses_as_staleness() {
    let _wd = watchdog("cancel_uninterruptible", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = world(2, mode);
        // ★ The modelled fact: this verb's host wait does not break on a signal.
        rec.lock()
            .expect("recorder")
            .never_cancels
            .insert(VerbKind::MapGpuVa);

        let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
        let out = thread::scope(|sc| {
            let _release = ReleaseOnDrop(&held);
            let d: &SharedDevice = &device;
            let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
            held.wait_until_pending();
            assert!(device.retire_proc(pid), "({mode:?}) the proc was live");
            held.release();
            t.join().expect("joins")
        });

        assert_eq!(
            out,
            Err(FwdFault::Stale(Stale::Proc(pid))),
            "({mode:?}) when the host wait does not break, the verb RUNS TO COMPLETION \
             and its commit must refuse as divergent staleness naming the exact proc — \
             never an incidental RM error, and never a `Cancelled` the host never honoured"
        );
        assert_eq!(
            cancel_census(&rec),
            (1, 1, 0),
            "({mode:?}) ★ fired, DELIVERED, and NOT observed. Delivered==1 is the \
             non-vacuity half: without it, observed==0 would also be true of a seam that \
             never sent anything"
        );
        assert_eq!(corruption(&rec), (0, 0, 0), "({mode:?})");
        assert!(
            staged_total(&device) > 0,
            "({mode:?}) the completed chain's host objects could not be freed by a \
             stopped isolate, so they must at least be NAMED on the corpse's queue"
        );
    }
}

// =================================================================================
// T5 / §7.5 — the wedge escape
// =================================================================================

/// ★★★ **THE D-STATE ESCAPE** (`l1_os_shell.md` §7.5).
///
/// A host thread in uninterruptible sleep cannot be signalled awake. The watchdog's
/// first budget fires a cancel and it is ignored; the second declares the worker
/// **wedged**, and then — in ONE act — the slot dies, the requester is released with no
/// reply, and the component is condemned.
///
/// Five claims, and the last two are the ones that make it a *reclamation* story rather
/// than a liveness one:
///
/// 1. the requester is released at all (an unbounded silent stall becomes a bounded loud
///    failure);
/// 2. the fault names the truth — `Wedged { proc, gpu, worker }`, never `Cancelled`;
/// 3. the component is condemned, so a later op of the same client refuses `Condemned`
///    rather than being served from a half-dead isolate;
/// 4. the chain's intermediate host object is **staged**, because a wedged worker cannot
///    run its own unwind (G4's exact premise) — this is the difference between a leak we
///    can name and one nothing in the system can address;
/// 5. nothing was released twice, including by the condemnation.
#[test]
fn the_wedge_escape_releases_condemns_and_stages_in_one_act() {
    let _wd = watchdog("wedge_escape", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = world(2, mode);
        // ★ NO `ResidueClaim`. The escape stops the isolate, so none of this proc's host
        // objects can be freed per object — and yet the audit's UNACCOUNTED set is
        // EMPTY, because every one of them is on the corpse's release queue: the
        // warm-up's estate staged by the vacate, and the wedged chain's intermediate
        // staged by `stage_orphans`. "Outstanding" and "unnameable" are different
        // things, and this is the whole of §12.35's distinction.
        rec.lock()
            .expect("recorder")
            .never_cancels
            .insert(VerbKind::MapGpuVa);

        let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
        let out = thread::scope(|sc| {
            // ★ The one site where the guard is NOT a mechanical copy. This test's whole
            // point is that the hold is never released — §7.5's abandon, not a release, is
            // what frees the requester — so a guard here must not become the mechanism.
            // It does not: `declare_wedged` has already let the parked verb out through
            // `RmError::Wedged` before the closure ends, and `gate` removed this one-shot
            // hold from the recorder the moment the verb matched it, so on the success
            // path this `release` sets a flag on a latch nothing can ever enter again.
            // `t.join()` above it still proves the abandon, not the release, is what
            // returned the requester — a broken escape leaves `join` blocked with the
            // guard not yet dropped, exactly as before.
            //
            // On the FAILURE path it is the only escape there is: an assertion that fires
            // before `declare_wedged` would otherwise leave a thread parked in a latch
            // whose release statement no longer runs and whose abandon never happens, and
            // this test has no watchdog of its own beyond `_wd`'s process abort.
            let _release = ReleaseOnDrop(&held);
            let d: &SharedDevice = &device;
            let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
            held.wait_until_pending();

            // ---- Budget 1: fire the cancel. It is delivered and ignored.
            assert!(
                device.request_cancel(pid, GPU, WorkerId(0), CancelReason::Watchdog),
                "({mode:?}) the first budget's cancel named a live transaction"
            );
            assert_eq!(
                cancel_census(&rec).2,
                0,
                "({mode:?}) the first budget must NOT have released it — otherwise the \
                 escalation below is never reached and this test proves nothing"
            );

            // ---- Budget 2: declare it wedged. The requester is still parked; the hold
            // is NEVER released, which is the whole point — nothing else can free it.
            assert!(
                device.declare_wedged(pid, GPU, WorkerId(0)),
                "({mode:?}) the escape ran"
            );
            t.join()
                .expect("★★ the abandoned requester was never released")
        });

        assert_eq!(
            out,
            Err(FwdFault::Wedged {
                proc: pid,
                gpu: GPU,
                worker: WorkerId(0)
            }),
            "({mode:?}) an abandoned requester must surface WEDGED — naming the exact \
             slot — never `Cancelled` (which claims the host honoured something it did \
             not) and never an anonymous RM error"
        );
        // (3) the component is condemned, in the same act.
        assert_eq!(
            device.resolve(GPU, PDB, VA_WARM).map(|_| ()),
            Err(FwdFault::Condemned {
                anchor: ProcAnchor(ClientKey::first(CLIENT))
            }),
            "({mode:?}) the escape must condemn the component in the SAME act — \
             abandoning a reply is safe only because no future reader of that channel \
             exists, and the condemnation is what guarantees that"
        );
        // (4) the intermediate is named, not lost.
        let wedged_mem = last_sysmem(&rec, pid);
        assert!(
            staged_objects(&device).contains(&wedged_mem),
            "({mode:?}) a wedged worker cannot run its own unwind (G4), so the host \
             memory object it had allocated ({wedged_mem:?}) must be STAGED — 'in no \
             Orphans and in no core state' is exactly the shape this closes. \
             staged={:?}",
            staged_objects(&device)
        );
        // ★★ …and NO verb was even ATTEMPTED on the wedged worker. This is the assertion
        // that makes `Worker::execute`'s wedge short-circuit testable at all: a wedged
        // worker answers everything with `Wedged`, so a chain that runs its unwind anyway
        // leaves byte-identical residue to one that correctly does not — the outcome is
        // invisible and only the attempt count distinguishes them. `== 1` and not `== 0`
        // because the map verb this test parked IS the attempt that discovered the
        // abandon; anything above that is an unwind that should not have run.
        //
        // ★ It is a **pair**, and both halves are two-sided. The first element is the
        // positive control (exactly one ABANDON — a `CancelEvent` with no reason — was
        // delivered, so the world these numbers describe is one where the escape really
        // ran); the second is `1` rather than `0` precisely so that a deleted counter
        // fails it too. An `== 0` invariant is equally true of an instrument nobody
        // increments, which is the vacuity this campaign keeps finding.
        let (abandons, after) = {
            let r = rec.lock().expect("recorder");
            (
                r.cancels_delivered
                    .iter()
                    .filter(|c| c.reason.is_none() && c.delivered)
                    .count(),
                r.verbs_that_hit_an_abandoned_worker,
            )
        };
        assert_eq!(
            (abandons, after),
            (1, 1),
            "({mode:?}) exactly one abandon was delivered, and exactly ONE verb hit the \
             wedged worker — the parked map that discovered it. Anything above one is an \
             unwind that ran on a worker whose thread is inside the ioctl that wedged it \
             and whose channel is gone; anything below one means the instrument is dead"
        );
        // ★ …and it was never FREED, which is the other half: a wedged worker that
        // somehow answered a `free` would satisfy the line above only if it also staged,
        // and this says the disposition really is the session's death.
        assert!(
            !rec.lock()
                .expect("recorder")
                .verbs_of(IsolateId::new(pid.0, GPU))
                .iter()
                .any(|v| matches!(v, RmVerb::Free { obj } if *obj == wedged_mem)),
            "({mode:?}) the wedged worker issued a free — it is inside the ioctl that \
             wedged it and cannot have"
        );
        assert_eq!(corruption(&rec), (0, 0, 0), "({mode:?})");
    }
}

// =================================================================================
// T3 — a deferred deadline that passes during teardown
// =================================================================================

/// ★★ **§7.6 T3 / G3: a `DeferredReap` deadline that fires while a retired proc still
/// has a verb in flight must DEFER, not reap.**
///
/// > *"reaping too early tears the sandbox down under a live connection (a
/// > use-after-free); reaping too late leaks until the next quiesce point."*
///
/// The deadline is delivered through the real inbox and the real executor, so this is
/// the whole path and not a direct call: `Effect::Reaped(0)` while the isolate is not
/// quiesced, then `Reaped(1)` once the cancelled verb has returned its worker. Both
/// numbers are asserted — `Reaped(0)` alone would also be produced by a reap that found
/// no corpse at all, so the second is the control that proves there was one.
#[test]
fn a_deferred_reap_during_teardown_defers_rather_than_reaping_under_a_live_verb() {
    let _wd = watchdog("deferred_reap_teardown", Duration::from_secs(60));
    let (mut device, pid, rec) = world(2, LockMode::Sharded);
    // ★ §7.0 — `retire_proc` is the violent death: the isolate is stopped, so when the
    // corpse is finally reaped its staged release is refused and the whole estate is the
    // session's to free. Declared exactly, in both directions, per `ResidueClaim`.
    device.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pid.0, GPU),
            "§7.0 namespace death: the corpse was reaped with its isolate already \
             stopped by `retire_proc`, so its staged release could not be issued — the \
             warm-up's host VAS + backing + mapping and the uninterrupted chain's own \
             memory object",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 2)
        .maps(1),
    );
    rec.lock()
        .expect("recorder")
        .never_cancels
        .insert(VerbKind::MapGpuVa);
    let (tx, rx) = inbox();
    let mut ex = Executor::new(Arc::clone(&device), rx);

    let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    let out = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&held);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();
        assert!(device.retire_proc(pid), "the proc was live");

        // ---- The deadline passes DURING teardown, with the verb still in flight.
        tx.send(CoreEvent::Deferred(CoreEventKind::DeferredReap));
        assert_eq!(
            ex.drain_all(),
            vec![Effect::Reaped(0)],
            "★★ a reap that ran here would drop the isolate under a live connection a \
             foreign thread is still inside — the G3 use-after-free"
        );
        assert_eq!(
            device.retired_len(),
            1,
            "★ non-vacuity: the deferral is a DEFERRAL. `Reaped(0)` is equally true of a \
             device with no corpse at all, so the corpse has to be shown to exist"
        );
        held.release();
        t.join().expect("joins")
    });
    assert_eq!(out, Err(FwdFault::Stale(Stale::Proc(pid))));

    // ---- The same deadline, now that the isolate has quiesced.
    tx.send(CoreEvent::Deferred(CoreEventKind::DeferredReap));
    assert_eq!(
        ex.drain_all(),
        vec![Effect::Reaped(1)],
        "once the verb is back the corpse reaps — a deferred reap must be DEFERRED, \
         never lost"
    );
    assert_eq!(device.retired_len(), 0);
    assert_eq!(corruption(&rec), (0, 0, 0));
    drop(ex);
}

// =================================================================================
// R1 — firing a cancel is a syscall
// =================================================================================

/// ★ **§7.1's reason for the latch/discharge split, made a test.**
///
/// > *"firing a cancel is a syscall, and §6.2 forbids syscalls under locks."*
///
/// If `discharge` did not assert this, the two-step would be ceremony: someone would
/// eventually fire a latched request inside the critical section that produced it, and
/// nothing anywhere would say so. The `#[should_panic]` message is matched on `R1` so
/// the test cannot pass on an unrelated panic.
#[test]
#[should_panic(expected = "R1")]
fn discharging_a_cancel_under_a_ranked_lock_panics_naming_r1() {
    // Defence in depth behind `ReleaseOnDrop`: this test parks a real thread and was one
    // of the two in the file with no bound of its own at all.
    let _wd = watchdog("discharge_under_lock_r1", Duration::from_secs(60));
    let (device, pid, rec) = world(1, LockMode::Sharded);
    let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    thread::scope(|sc| {
        // ★ Declared FIRST, which is what makes the ranked-lock argument work: `guard`
        // below is a later local, so on any unwind between its acquisition and its `drop`
        // Rust releases rank 0 first and this guard's `release` then runs lock-free.
        let _release = ReleaseOnDrop(&held);
        let d: &SharedDevice = &device;
        let t = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();
        let handle = device
            .cancel_handle(pid, GPU, WorkerId(0))
            .expect("in flight");
        let req = handle.request(CancelReason::ProcExit);
        // Take a ranked lock and fire it — the thing the design forbids.
        let lock = kayfabe_rt::lock::RankedMutex::new(kayfabe_rt::lock::LockRank::Device, ());
        let guard = lock.lock();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            req.discharge();
        }));
        drop(guard);
        // Release the parked verb before re-raising, or the scope's join wedges.
        held.release();
        t.join().expect("joins").expect("commits");
        if let Err(e) = panicked {
            std::panic::resume_unwind(e);
        }
    });
}

/// A guard against this file becoming a place where a cancellation bug hides as a hang.
///
/// ★ It was itself the worst instance of exactly that: it parks a thread in a `VerbHold`
/// released only by a statement *after* the `assert!` above it, and it had no watchdog, so
/// the test whose stated job is bounding this file hung unbounded and silently. The
/// `t.elapsed()` assertion below cannot help — it is evaluated only on a run that finished.
/// The bound is now `ReleaseOnDrop` plus a real watchdog; `t.elapsed()` stays as the
/// tighter *success*-path claim it always was.
#[test]
fn cancellation_suite_is_bounded() {
    let _wd = watchdog("cancellation_suite_is_bounded", Duration::from_secs(60));
    let t = WallInstant::now();
    let (device, pid, rec) = world(1, LockMode::Sharded);
    let held = hold(&rec, pid, 0, VerbKind::MapGpuVa);
    let out = thread::scope(|sc| {
        let _release = ReleaseOnDrop(&held);
        let d: &SharedDevice = &device;
        let h = sc.spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
        held.wait_until_pending();
        assert!(device.request_cancel(pid, GPU, WorkerId(0), CancelReason::DeviceReset));
        held.release();
        h.join().expect("joins")
    });
    assert_eq!(
        out,
        Err(FwdFault::Cancelled {
            proc: pid,
            reason: CancelReason::DeviceReset
        }),
        "the fourth reason is reachable too — all four `CancelReason`s are carried \
         through end to end, not just the two the other tests use"
    );
    assert!(t.elapsed() < Duration::from_secs(30));
    let _ = (VAS0, BTreeSet::<u32>::new());
}
