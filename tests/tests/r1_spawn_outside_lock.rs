//! ★★★ **R1's spawn deferral: an isolate is SPAWNED with zero ranked locks held, and
//! INSTALLED under a re-validation** (`l1_concurrency.md` §3.3, R1 + R5).
//!
//! ## The defect this file is the regression test for, MEASURED
//!
//! `docs/reference/bench_evidence/f0b7efa_run_basereal_qemu.log` — a live boot of the
//! shipped archive built with `host-isolates` and `KAYFABE_ISOLATES=real`, aborting on the
//! guest's **first register write** that reached a `GSP_RM_ALLOC`:
//!
//! ```text
//! R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): spawning a sandboxed
//! child process while holding rank(s) [0] …
//!   19: kayfabe_shim_regs_write   20: nvkvm_trap_write
//! ```
//!
//! `Spine::apply` is a spine op — it runs under the device WRITE lock — and E0b made it
//! the one site that materializes an isolate. Materializing one is `clone` into six
//! namespaces, `execveat` of a sealed memfd, a blocking hello handshake and, under
//! `KAYFABE_ISOLATES=real`, a chain of real host RM ioctls. R1 admits no exception for
//! that, and the assert at `kayfabe_linux_raw::ChildSpec::spawn` said so — on the real
//! plane, on a bench, once. **Nothing in the suite could see it**, because the suite runs
//! on mocks and a mock spawn blocks on nothing.
//!
//! ## ⊘ So the first thing this file pins is the MECHANISM, not the fix
//!
//! `kayfabe_isolate::IsolateBox::new` now asserts lock-freedom, exactly as that type's
//! `Drop` already did (§12.16 G3b closed the death half and left the birth half open).
//! `IsolateBox` is the only way an isolate enters core state, whichever plane made it, so
//! the rule is one mechanism rather than three implementations that can drift — and the
//! whole mock-driven suite becomes a live guard for it. `spine_apply_*` below is the
//! measurement that it *would* have fired: it drives the exact path the boot took and
//! asserts the spawn does not happen there.
//!
//! ## What "correct" is, in three properties
//!
//! 1. **DECIDE under the lock, SPAWN without it.** `Spine::apply` latches
//!    (`Spine::take_pending_spawns` + `Proc::wants_isolate`); the shell spawns after its
//!    guards fall.
//! 2. **RE-VALIDATE on the way back in (R5).** The world moves in the gap: the proc can
//!    retire, and a sibling can materialize the same pair first. `Spine::install_isolate`
//!    refuses both and hands the sandbox back to be dropped lock-free — §12.9's
//!    compare-and-swap, with the loser releasing its duplicate rather than being refused.
//! 3. **The gap is a NAMED state, and it is DEFER rather than FAULT.** A verb routed to a
//!    proc whose isolate is still being spawned gets `FwdFault::IsolatePending`, which
//!    `SharedDevice::verb_op` resolves by materializing and re-planning. Reporting it as
//!    `FwdFault::NoTarget` ("an internal inconsistency") would make a legal concurrent
//!    state a guest-visible fault — §12.9's "worse bug".
//!
//! ⊘ **What this file cannot settle.** Every isolate here is a `MockIsolate`; nothing
//! forks, so "the spawn really was expensive" is not measured here and cannot be. What is
//! measured is *where in the lock discipline it happens*, which is the whole defect. The
//! live boot is the acceptance (`only_live_boots_are_proof`).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped PDB/handle literals

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::{ProcId, rmgraph::RmEvent};
use kayfabe_fwd::FwdFault;
use kayfabe_isolate::{Isolate, IsolateBox, IsolateFactory, IsolateId};
use kayfabe_mocks::{MockArch, MockIsolateFactory, watchdog};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_rt::lock::LockRank;
use kayfabe_tests::{Scenario, identical_handles};

const MEM_HANDLE: HObject = HObject(0x5c00_0002);

fn pdb_of(i: usize) -> Pdb {
    Pdb(0x3400_0000 + (i as u64) * 0x1_0000)
}
fn client_of(i: usize) -> HClient {
    HClient(0xc1d0_0100 + i as u32)
}
fn gr_vchid(i: usize) -> VChid {
    VChid(0x100 + i as u16)
}
fn ce_vchid(i: usize) -> VChid {
    VChid(0x200 + i as u16)
}

fn gpa() -> GpaSpace {
    GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000)
}

/// One guest compute process's whole event stream, in order.
fn process_events(i: usize) -> Vec<RmEvent> {
    let mut s = Scenario::new();
    s.compute_process(
        client_of(i),
        pdb_of(i),
        identical_handles(gr_vchid(i).0, ce_vchid(i).0),
    );
    s.memory(
        client_of(i),
        HObject(0x5c00_0001),
        MEM_HANDLE,
        0x9_0000_0000 + (i as u64) * 0x1000_0000,
    );
    s.events
}

// =====================================================================================
// 1. The MECHANISM — `IsolateBox::new` is the door R1 is asserted at on the birth side
// =====================================================================================

/// ★★★ Taking ownership of a freshly spawned isolate while a ranked lock is held panics,
/// **naming R1**, on every plane — including the mock plane the whole suite runs on.
///
/// This is the assert whose absence let the defect ship: the real one
/// (`kayfabe_linux_raw::ChildSpec::spawn`) exists and is correct and is only reachable on
/// a box with a GPU.
///
/// ## ★★★ THE INSTRUMENT WAS THE BUG FIRST — and the bite harness is what said so
///
/// The first version of this test was six lines and a `#[should_panic]`, and it was
/// **vacuous**. `[measured]`, bite **B5** (remove both asserts from `IsolateBox::new`,
/// `cargo test --workspace --no-fail-fast` at rev `e726844` on the RTX 3060 bench): **zero
/// tests red.** The reason is the half of the lifecycle that was already correct — with
/// the birth assert gone, `new` returns, the value falls at the end of the test **with the
/// rank still held**, and `IsolateBox::drop`'s own §12.16-G3b assert fires the identical
/// R1 sentence. `#[should_panic(expected = "R1 no-blocking-under-lock")]` was satisfied by
/// the wrong door.
///
/// So this version does three things the short one could not:
///
/// 1. **releases the rank before the value falls**, so `Drop` cannot supply the panic;
/// 2. asserts on the panic's own text that it is the **birth** door
///    (*"materializing an isolate"*) and not the death door (*"dropping an isolate"*);
/// 3. **fails loudly if nothing panicked at all**, instead of relying on an attribute
///    that any panic satisfies.
///
/// `catch_unwind` rather than `#[should_panic]` for reason (1): the witness is a
/// thread-local, and under `--test-threads=1` libtest runs tests on the **main** thread,
/// so an unwind that leaves rank 0 set poisons every test after it in the binary. The
/// cleanup below runs on both paths.
#[test]
fn materializing_an_isolate_under_a_ranked_lock_panics_naming_r1() {
    let (factory, _rec) = MockIsolateFactory::new();
    let rank = LockRank::Device as u8;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Simulate the device write guard the boot died under.
        kayfabe_util::lockwitness::note_acquired(rank);
        let made = IsolateBox::new(factory.spawn(IsolateId::new(1, GpuId::ZERO)));
        // Only reachable if the BIRTH assert did not fire. Drop the rank first — see the
        // docs: otherwise the death assert answers for it.
        kayfabe_util::lockwitness::note_released(rank);
        drop(made);
    }));
    // Whatever happened, this thread holds nothing afterwards.
    kayfabe_util::lockwitness::note_released(rank);

    let err = outcome.expect_err(
        "materializing an isolate with rank 0 held must panic — the birth-side R1 assert          did not fire at all",
    );
    let text = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("<a panic payload that is not a string>");
    assert!(
        text.contains("R1 no-blocking-under-lock violation"),
        "it must panic naming R1: {text}"
    );
    assert!(
        text.contains("materializing an isolate"),
        "★ and it must be the BIRTH door. `dropping an isolate …` here means the assert          under test is absent and `IsolateBox::drop` answered instead — the exact way          this test used to pass for the wrong reason: {text}"
    );
}

/// ⊘ **The control, and it is not ceremony.** An assert that fires unconditionally is not
/// a guard, it is a brick: the test above would pass just as well against
/// `assert!(false)`. The discriminating arm is
/// `r1_spawn_outside_lock.rs::materializing_an_isolate_with_no_lock_held_is_fine` — this
/// one — and it is what turns the pair from an assertion into a check.
#[test]
fn materializing_an_isolate_with_no_lock_held_is_fine() {
    let (factory, _rec) = MockIsolateFactory::new();
    assert_eq!(kayfabe_util::lockwitness::held_mask(), 0);
    let iso = IsolateBox::new(factory.spawn(IsolateId::new(1, GpuId::ZERO)));
    assert_eq!(iso.id(), IsolateId::new(1, GpuId::ZERO));
}

// =====================================================================================
// 2. DECIDE under the lock, SPAWN outside it
// =====================================================================================

/// ★★★ **The exact path the boot died on**: `Spine::apply` — a spine op, i.e. under the
/// device write lock in the L1 shell — decides that an isolate is wanted and **spawns
/// nothing**.
///
/// Asserted three ways, because the counter is written by the code under test: the
/// device's own census, the pending latch, and the **factory's** witness, which is a
/// different object and cannot be made to agree by a counter that stopped counting.
#[test]
fn a_spine_apply_decides_an_isolate_and_spawns_nothing() {
    let (factory, _rec) = MockIsolateFactory::new();
    let witness = Arc::new(WitnessFactory::new(factory));
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(WitnessHandle(Arc::clone(&witness))),
        gpa(),
    )
    .expect("device realizes");

    let ev = process_events(0).into_iter().next().expect("events");
    // ⊘ `Spine::apply`, NOT `Gpu::apply`: the composed entry point drains, and draining is
    // exactly what the shell cannot do while it holds the lock. This is the half that runs
    // under rank 0.
    gpu.spine
        .apply(&mut gpu.system, &mut gpu.procs, ev)
        .expect("the first event applies");

    assert_eq!(
        witness.calls(),
        0,
        "the locked half of an apply must not reach the factory at all"
    );
    assert_eq!(
        gpu.isolate_census().materialized,
        0,
        "and nothing is installed yet"
    );
    assert!(
        gpu.system.wants_isolate(GpuId::ZERO),
        "what it DID do is decide: the guest kernel's isolate is pending"
    );
    let pending = gpu.spine.take_pending_spawns();
    assert!(!pending.is_empty(), "and the work is latched for the shell");
    drop(pending);
}

/// The composed single-threaded device holds **no** lock, so it materializes in place and
/// its observable behaviour is exactly what it was before the fix: when `Gpu::apply`
/// returns, every isolate the event called for exists.
#[test]
fn the_composed_single_threaded_apply_materializes_before_it_returns() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");

    let ev = process_events(0).into_iter().next().expect("events");
    gpu.apply(ev).expect("the first event applies");

    assert_eq!(gpu.isolate_census().materialized, 1);
    assert!(gpu.system.isolates.contains_key(&GpuId::ZERO));
    assert!(
        !gpu.system.wants_isolate(GpuId::ZERO),
        "and the pending marker is cleared by the install, not left to rot"
    );
}

/// And the sharded shell — the configuration the archive ships and the one that aborted —
/// materializes too, after its guard drops.
#[test]
fn the_sharded_shell_materializes_after_its_guard_drops() {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    let device = SharedDevice::new(gpu, LockMode::Sharded);

    assert_eq!(
        device.isolate_census().materialized,
        0,
        "realize spawns nothing (E0b)"
    );
    for ev in process_events(0) {
        device.apply(ev).expect("the scenario applies");
    }
    let c = device.isolate_census();
    assert_eq!(
        c.materialized, 2,
        "the guest kernel's isolate and the one guest process's, both materialized by \
         guest events"
    );
    assert_eq!(c.live, 2);
}

// =====================================================================================
// 3. R5 — re-validation on the way back in
// =====================================================================================

/// ★★★ **Divergent staleness**: the proc retired while its sandbox was being spawned. The
/// install refuses and hands the sandbox back — it is **not** installed into a corpse, and
/// it is **not** dropped under the lock.
#[test]
fn an_isolate_spawned_for_a_proc_that_retired_in_the_gap_is_refused() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    for ev in process_events(0) {
        gpu.spine
            .apply(&mut gpu.system, &mut gpu.procs, ev)
            .expect("applies");
    }
    let pid = *gpu.procs.keys().next().expect("one guest proc");
    assert!(
        gpu.procs[&pid].wants_isolate(GpuId::ZERO),
        "its isolate is decided and unspawned"
    );

    // The gap: everything below happens with no lock held, and the world moves in it.
    let iso = IsolateBox::new(
        gpu.spine
            .isolate_factory()
            .spawn(IsolateId::new(pid.0, GpuId::ZERO)),
    );
    let cancels = gpu.procs.get_mut(&pid).expect("live").vacate();
    cancels.discharge_all();

    let before = gpu.isolate_census().materialized;
    let back = gpu.spine.install_isolate(
        gpu.procs.get_mut(&pid).expect("still present"),
        GpuId::ZERO,
        iso,
    );
    assert!(
        back.is_err(),
        "a sandbox spawned for a proc that left the live set is surplus (R5, divergent)"
    );
    assert_eq!(
        gpu.isolate_census().materialized,
        before,
        "and nothing is counted for it"
    );
    drop(back.err());
}

/// ★★★ **Converging staleness** (§12.9's compare-and-swap): two materializations of the
/// same `(proc, gpu)` race, and **exactly one installs**. The loser's sandbox is surplus.
///
/// ⊘ The loser is not *refused* — nothing it was doing has failed. It simply re-plans
/// against the winner's isolate, which is the resolution §12.9 prescribes and the reason
/// the alternative (gating the second thread) was not built.
#[test]
fn only_one_of_two_racing_materializations_installs() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    let ev = process_events(0).into_iter().next().expect("events");
    gpu.spine
        .apply(&mut gpu.system, &mut gpu.procs, ev)
        .expect("applies");

    let id = IsolateId::new(Gpu::SYSTEM_PROC.0, GpuId::ZERO);
    let a = IsolateBox::new(gpu.spine.isolate_factory().spawn(id));
    let b = IsolateBox::new(gpu.spine.isolate_factory().spawn(id));

    assert!(
        gpu.spine
            .install_isolate(&mut gpu.system, GpuId::ZERO, a)
            .is_ok(),
        "the winner installs"
    );
    let loser = gpu
        .spine
        .install_isolate(&mut gpu.system, GpuId::ZERO, b)
        .err();
    assert!(loser.is_some(), "the loser's sandbox comes back as surplus");
    assert_eq!(
        gpu.isolate_census().materialized,
        1,
        "and the census counts INSTALLS, not decisions — two spawns, one isolate"
    );
    drop(loser);
}

/// Deciding twice for the same pair is idempotent: one latch entry, one install.
#[test]
fn deciding_twice_for_one_pair_materializes_once() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    for ev in process_events(0) {
        gpu.apply(ev).expect("applies");
    }
    let first = gpu.isolate_census().materialized;
    // Every subsequent event re-derives the same procs and re-runs step 3b.
    for ev in process_events(1) {
        gpu.apply(ev).expect("applies");
    }
    let c = gpu.isolate_census();
    assert_eq!(first, 2, "kernel + the first guest process");
    assert_eq!(
        c.materialized, 3,
        "the second guest process adds exactly one, and nothing re-spawns"
    );
    assert_eq!(c.live, 3);
}

// =====================================================================================
// 4. The gap is a NAMED state, and it DEFERS
// =====================================================================================

/// ★★★ The two absences are told apart. `NoTarget` means *"an internal inconsistency"* and
/// must keep meaning that; the deferral gets its own name.
#[test]
fn a_deferred_isolate_is_isolate_pending_and_an_unasked_for_one_is_no_target() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    let ev = process_events(0).into_iter().next().expect("events");
    gpu.spine
        .apply(&mut gpu.system, &mut gpu.procs, ev)
        .expect("applies");

    // GpuId::ZERO was decided by the apply above and is not installed.
    let pending = kayfabe_fwd::checkout_and_drain(&mut gpu.system, GpuId::ZERO);
    assert!(
        matches!(
            pending,
            Err(FwdFault::IsolatePending {
                proc: Gpu::SYSTEM_PROC,
                gpu: GpuId::ZERO
            })
        ),
        "a decided-but-unspawned isolate is a DEFER, not an inconsistency: {pending:?}"
    );

    // GpuId(1) was never asked for at all.
    let absent = kayfabe_fwd::checkout_and_drain(&mut gpu.system, GpuId(1));
    assert!(
        matches!(absent, Err(FwdFault::NoTarget { .. })),
        "and a target nothing ever wanted is still the inconsistency it always was: \
         {absent:?}"
    );

    // A retired proc answers with its own, earlier refusal — never "pending", which would
    // send a caller off to spawn a sandbox for a process that is gone.
    gpu.system.vacate().discharge_all();
    assert!(!gpu.system.wants_isolate(GpuId::ZERO));
    assert!(matches!(
        kayfabe_fwd::checkout_and_drain(&mut gpu.system, GpuId::ZERO),
        Err(FwdFault::RetiredProc(_))
    ));
}

/// ★★★ **A verb that lands in the gap RESOLVES it** rather than surfacing it: the thread
/// materializes the pending isolate itself, lock-free, and re-enters the plan.
///
/// The device is built with its procs derived through `Spine::apply` and **never drained**,
/// which is precisely the state a second vCPU thread sees between another thread's write
/// guard dropping and its install landing.
#[test]
fn a_verb_that_lands_in_the_gap_materializes_the_isolate_and_succeeds() {
    let _wd = watchdog(
        "a_verb_that_lands_in_the_gap_materializes_the_isolate_and_succeeds",
        Duration::from_secs(60),
    );
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa()).expect("device realizes");
    for ev in process_events(0) {
        gpu.spine
            .apply(&mut gpu.system, &mut gpu.procs, ev)
            .expect("applies");
    }
    let pending = gpu.spine.take_pending_spawns();
    assert_eq!(
        pending.len(),
        2,
        "the kernel's isolate and the guest process's, both undrained"
    );
    drop(pending);
    let pid = *gpu.procs.keys().next().expect("one guest proc");
    assert!(gpu.procs[&pid].wants_isolate(GpuId::ZERO));

    // ★ #177 — the guest schedules before it rings.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);

    let device = SharedDevice::new(gpu, LockMode::Sharded);
    assert_eq!(
        device.isolate_census().materialized,
        0,
        "nothing has been spawned"
    );

    device
        .doorbell(GpuId::ZERO, MockArch::token_for(gr_vchid(0)), &[])
        .expect("the doorbell resolves the deferral rather than refusing it");

    assert_eq!(
        device.isolate_census().materialized,
        1,
        "exactly the one the verb needed — a verb materializes what it routes to, never \
         the whole latch"
    );
}

/// ★★★ **Two threads racing the same deferral, on the shell.** Both find
/// `IsolatePending`, both spawn, and exactly one install lands. Both verbs succeed.
///
/// ★ **That both really spawned is asserted, not hoped for**: `SpawnGate::arrivals` is the
/// factory's own count, read at the end of
/// `r1_spawn_outside_lock.rs::two_threads_racing_one_deferral_spawn_twice_and_install_once`.
/// The gate is also what makes the race deterministic — the first spawn of the contested id
/// parks until a second arrives, so neither thread can install before both have spawned.
/// Without it this test would pass by simply never racing, which is the shape a
/// concurrency test fails in silently.
#[test]
fn two_threads_racing_one_deferral_spawn_twice_and_install_once() {
    let _wd = watchdog(
        "two_threads_racing_one_deferral_spawn_twice_and_install_once",
        Duration::from_secs(60),
    );
    let (factory, rec) = MockIsolateFactory::new();
    let contested = IsolateId::new(1, GpuId::ZERO); // the first guest proc's
    let gate = Arc::new(SpawnGate::new(contested, 2));
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(GatedFactory {
            inner: factory,
            gate: Arc::clone(&gate),
        }),
        gpa(),
    )
    .expect("device realizes");
    for ev in process_events(0) {
        gpu.spine
            .apply(&mut gpu.system, &mut gpu.procs, ev)
            .expect("applies");
    }
    assert_eq!(
        *gpu.procs.keys().next().expect("one guest proc"),
        ProcId(1),
        "the contested id names the proc this test believes it does"
    );
    drop(gpu.spine.take_pending_spawns());

    // ★ #177 — the guest schedules before it rings.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);

    let device = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));
    let token = MockArch::token_for(gr_vchid(0));
    let hands: Vec<_> = (0..2)
        .map(|_| {
            let d = Arc::clone(&device);
            thread::spawn(move || d.doorbell(GpuId::ZERO, token, &[]))
        })
        .collect();
    for h in hands {
        h.join().expect("no thread panicked").expect("doorbell");
    }

    assert_eq!(
        gate.arrivals(),
        2,
        "★ the race is LIVE: both threads really did spawn the contested isolate"
    );
    assert_eq!(
        device.isolate_census().materialized,
        1,
        "and exactly one of the two was installed (R5, converging)"
    );
    // The mock records one RM client lock per isolate the factory built, keyed by id —
    // a second object's view of the same fact.
    assert!(
        rec.lock()
            .expect("recorder")
            .client_locks
            .contains_key(&contested),
        "the contested isolate was really built"
    );
}

// =====================================================================================
// Test doubles
// =====================================================================================

/// Counts factory calls without answering anything itself — the "different object" the
/// census is checked against.
struct WitnessFactory {
    inner: MockIsolateFactory,
    calls: AtomicUsize,
}

impl WitnessFactory {
    fn new(inner: MockIsolateFactory) -> Self {
        WitnessFactory {
            inner,
            calls: AtomicUsize::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

/// `Gpu::new` takes the factory by value; this hands it an `Arc`-shared view so the test
/// can still read the witness afterwards.
struct WitnessHandle(Arc<WitnessFactory>);

impl IsolateFactory for WitnessHandle {
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.inner.spawn(id)
    }
}

/// A rendezvous inside `spawn`, for exactly one [`IsolateId`]: the first `n - 1` arrivals
/// park until the `n`th shows up. It makes "two threads spawned the same isolate
/// concurrently" a fact the test arranges rather than one it hopes for.
struct SpawnGate {
    id: IsolateId,
    n: usize,
    arrived: Mutex<usize>,
    cv: Condvar,
}

impl SpawnGate {
    fn new(id: IsolateId, n: usize) -> Self {
        SpawnGate {
            id,
            n,
            arrived: Mutex::new(0),
            cv: Condvar::new(),
        }
    }
    fn arrivals(&self) -> usize {
        *self.arrived.lock().expect("gate")
    }
    fn pass(&self, id: IsolateId) {
        if id != self.id {
            return;
        }
        let mut a = self.arrived.lock().expect("gate");
        *a += 1;
        self.cv.notify_all();
        while *a < self.n {
            a = self.cv.wait(a).expect("gate");
        }
    }
}

struct GatedFactory {
    inner: MockIsolateFactory,
    gate: Arc<SpawnGate>,
}

impl IsolateFactory for GatedFactory {
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate> {
        self.gate.pass(id);
        self.inner.spawn(id)
    }
}
