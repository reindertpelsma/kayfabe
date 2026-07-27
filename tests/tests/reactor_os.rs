//! ★★ The completion-source reactor **over real host readiness primitives** —
//! `l1_os_shell.md` §3, stage M2-d's T2-OS gate.
//!
//! `reactor.rs` next door drives the same model with **no** OS at all (its own docs:
//! *"no threads, no reactor loop, no host readiness machinery of any kind"*), which is
//! what makes the completion flow a scripted order. This file is the other half: real
//! counter descriptors, a real readiness set, a real `epoll_wait`, real threads — and the
//! two things only they can answer.
//!
//! ## What only this file can pin
//!
//! 1. **★★ The F1 wake-count gate, as a QUANTITY** (§3.4). Not *"the loop does not
//!    busy-spin"* — an absence any hung loop satisfies vacuously — but two numbers:
//!    `signals_pushed` is an **equality** against what the producers sent (coalescing moves
//!    wakes, never signals), and `wakes` is **bounded above** by that same number (a
//!    level-triggered spin blows through it without bound). Both are structural; neither
//!    reads a clock.
//! 2. **The gate can fail.** A source the loop cannot drain drives it into a loud, bounded
//!    `ReactorFault::UndrainableSource` — asserted by variant and by streak, so the
//!    instrument is proven to have teeth rather than assumed to.
//! 3. **§3.3's designed answer**, constructed rather than raced: the doc's own cited
//!    hazard (*"a duplicated [descriptor] silently stays and keeps firing"*) is built on
//!    purpose, and the resulting report for a **retired** handle is pushed anyway and
//!    resolves to a loud `SourceFault` — never onto whatever was armed next.
//! 4. **The source cap's refusal** (§3.8), by exact variant, and contained.
//! 5. **The whole chain on real threads**: a counter fires → the reactor drains it → the
//!    inbox → the `ExecutorWaker` → the executor thread → the core's own locks → the
//!    owning proc's completion queue. Plus a real notify descriptor observed end to end.

#![allow(clippy::unusual_byte_groupings)]

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use kayfabe_arch::ids::{GpuId, HClient, Pdb};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::reactor::{SourceFault, SourceKind};
use kayfabe_core::{ProcId, rmgraph::RmEvent};
use kayfabe_isolate::WorkerId;
use kayfabe_linux_raw::{PollTimeout, Poller};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_rt::executor::{Effect, Executor, ExecutorWaker, Parker};
use kayfabe_rt::inbox::inbox;
use kayfabe_shell::{
    HostSource, MAX_UNPRODUCTIVE_STREAK, Reactor, ReactorError, ReactorFault, Registrar, Resolved,
    SourceShape,
};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const CLIENT_ROOT: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5c00_0000);

/// Two procs on two GPUs, behind the real shell. Integration, not unit: every signal below
/// lands in the same `SharedDevice` every other suite drives.
fn world(mode: LockMode) -> (Guarded<Arc<SharedDevice>>, Vec<ProcId>) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x11_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::realize(arch, Box::new(factory), gpa, &[GpuId::ZERO, GpuId(1)])
        .expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        HClient(0xAA),
        Pdb(0x1234_000),
        identical_handles(0x10, 0x11),
        None,
    );
    s.compute_process_on_gpu(
        HClient(0xBB),
        Pdb(0x5678_000),
        identical_handles(0x10, 0x11),
        Some(1),
    );
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pids = vec![
        gpu.spine.by_pdb[&(GpuId::ZERO, Pdb(0x1234_000))],
        gpu.spine.by_pdb[&(GpuId(1), Pdb(0x5678_000))],
    ];
    let guarded = Guarded::new("reactor_os::world", gpu, rec.clone())
        .map(|g| Arc::new(SharedDevice::new(g, mode)));
    (guarded, pids)
}

/// A registrar over a fresh readiness set, with the bound stated so the refusal test is
/// about the refusal.
fn registrar(bound: u64) -> Arc<Registrar> {
    let poller = Arc::new(Poller::create().expect("a readiness set"));
    Arc::new(Registrar::with_bound(poller, bound))
}

// =====================================================================================
// 1. ★★ THE F1 WAKE-COUNT GATE, AS A QUANTITY
// =====================================================================================

/// ★★ **The F1 gate** (§3.4): drive K signals across three real counter sources and assert
/// the loop's own numbers.
///
/// Two assertions, deliberately of different shapes, because they catch different bugs:
///
/// - `signals_pushed == K` — an **equality**. A counter coalesces (N writes then one read
///   returns N) and the loop pushes that many, so this number is invariant under every
///   batching the kernel may choose. A lost wakeup makes it smaller; a re-armed or
///   double-drained source makes it larger.
/// - `wakes <= K` — a **bound**. Coalescing can only push it down (here, all the way to
///   one). A level-triggered source the loop failed to drain would push it up without
///   limit, which is the F1 failure, and it would do so *on any machine* with no timing
///   dependence at all.
///
/// And the end-state is the point of the whole thing: every one of those K signals is a
/// completion sitting in the **owning** proc's queue, put there by the core's own dispatch.
#[test]
fn os_reactor_pushes_exactly_the_signals_sent_and_wakes_no_more_than_that() {
    let (device, pids) = world(LockMode::Sharded);
    let reg = registrar(64);
    let (tx, rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) = Reactor::new(
        Arc::clone(&reg),
        tx,
        Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
    )
    .expect("the reactor builds");

    // Three armed os-events: two for proc A (one per GPU target — MG-5's separate
    // completion path per target), one for proc B.
    let arms = [
        (pids[0], GpuId::ZERO, OsEventRef(0xA0)),
        (pids[0], GpuId(1), OsEventRef(0xA1)),
        (pids[1], GpuId(1), OsEventRef(0xB0)),
    ];
    let counters: Vec<_> = arms
        .iter()
        .map(|&(proc, gpu, ev)| {
            let src = device.register_source(SourceKind::OsEvent { proc, gpu, ev });
            (src, reg.arm_counter(src).expect("arms"))
        })
        .collect();

    // ---- K signals, deliberately UNEVEN and deliberately all before the first wait, so
    // the kernel coalesces every one of them into a single readiness report per source.
    const PER_SOURCE: [u64; 3] = [3, 5, 1];
    let k: u64 = PER_SOURCE.iter().sum();
    for ((_, n), count) in counters.iter().zip(PER_SOURCE) {
        for _ in 0..count {
            n.signal().expect("a relay writes the counter");
        }
    }

    reactor
        .run_with(PollTimeout::Immediate, 32)
        .expect("no fault");
    let s = handle.stats();

    // ---- ★ THE EQUALITY.
    assert_eq!(
        s.signals_pushed, k,
        "★ every signal a producer sent must become exactly one SourceSignal. A counter \
         coalesces N writes into one readiness report and the loop pushes N — so this is \
         invariant under batching, and neither a lost wakeup nor a double drain can hide \
         in it. stats={s:?}"
    );
    // ---- ★ THE BOUND. This is the half a spin blows through.
    assert!(
        s.wakes >= 1 && s.wakes <= k,
        "★ the F1 quantity: the loop woke {} times for {k} signals. `>= 1` is the \
         non-vacuity half (a loop that never woke satisfies every upper bound); `<= {k}` \
         is the half an undrained level-triggered source violates WITHOUT BOUND, \
         structurally and with no timing dependence. stats={s:?}",
        s.wakes,
    );
    assert!(
        s.wakes <= s.ready_reports && s.ready_reports <= k,
        "each wake carries at least one report, and no source may report more times than \
         it was signalled: {s:?}"
    );
    assert_eq!(
        (s.undrained_reports, s.stale_reports, s.retired_reports),
        (0, 0, 0),
        "★ every ready report was drained and named a live source — the three ways a \
         report can fail to be productive, all zero, so the equality above is not \
         concealing a compensating pair"
    );
    assert_eq!(
        s.control_reports, 0,
        "nothing woke the loop but the sources themselves"
    );

    // ---- ★ and the END-STATE: K completions, in the OWNING procs' queues.
    let mut exec = Executor::new(Arc::clone(&device), rx);
    let effects = exec.drain_all();
    assert_eq!(effects.len() as u64, k, "one effect per pushed signal");
    let mut observed = std::collections::BTreeMap::<(ProcId, GpuId, OsEventRef), u64>::new();
    for e in effects {
        match e {
            Effect::Signal(SignalOutcome::Observed { proc, gpu, ev }) => {
                *observed.entry((proc, gpu, ev)).or_default() += 1;
            }
            other => panic!("a real counter signal must observe, not {other:?}"),
        }
    }
    for (&(proc, gpu, ev), count) in arms.iter().zip(PER_SOURCE) {
        assert_eq!(
            observed.get(&(proc, gpu, ev)).copied(),
            Some(count),
            "★ {ev:?} on {gpu:?} must have been observed into {proc:?}'s OWN queue exactly \
             {count} times — the routing key is the whole tuple, and the count is the \
             coalesced counter's value"
        );
    }
    // The completions really are in the core, not merely reported by the executor.
    let outstanding = device
        .with_proc(pids[0], |p| p.completion.outstanding_len())
        .expect("proc A is live");
    assert_eq!(
        outstanding, 8,
        "proc A's two sources carried 3 + 5 completions into its own queue"
    );
    handle.shutdown().expect("shutdown");
    assert_eq!(reg.disarm_all(), 3, "every source disarms at shutdown");
    assert_eq!(
        reg.armed(),
        0,
        "★ and the CONSEQUENCE, not just the count: nothing is still armed. A `disarm_all` \
         that returned the right number without disarming anything passed the line above \
         (bite-checked), and would leave every counter descriptor open and watched across \
         a shutdown — §7.9's ordered teardown, silently not happening"
    );
    assert!(
        reg.armed_ever() >= 3,
        "★ NON-VACUITY: the table really held sources to disarm"
    );
}

// =====================================================================================
// 2. ★★ THE GATE HAS TEETH
// =====================================================================================

/// ★★ **The F1 gate firing.** A readiness primitive the loop cannot drain is exactly the
/// hazard §3.4 exists to prevent, and it is the case an absence-shaped rule cannot catch:
/// a loop spinning on it never busy-*waits* in any way a "no `sleep`, no poll" grep can
/// see — it simply never stops.
///
/// So it is built on purpose ([`SourceShape::Foreign`] — the shape §3.5 option (a) was
/// rejected for) and the refusal is asserted **by exact variant, token and streak**: the
/// loop stops after a bounded number of unproductive waits and names the offender.
#[test]
fn os_reactor_refuses_an_undrainable_level_triggered_source_by_name() {
    let (device, pids) = world(LockMode::Sharded);
    let reg = registrar(64);
    let (tx, rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) = Reactor::new(
        Arc::clone(&reg),
        tx,
        Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
    )
    .expect("the reactor builds");

    // A pipe with a byte in it: readable forever, and nothing the loop knows how to do
    // clears it.
    let (r, w) = std::io::pipe().expect("a pipe");
    std::io::Write::write_all(&mut { w }, b"x").expect("one byte, then the writer drops");
    let src = device.register_source(SourceKind::OsEvent {
        proc: pids[0],
        gpu: GpuId::ZERO,
        ev: OsEventRef(0xDEAD),
    });
    reg.arm(src, HostSource::Foreign(OwnedFd::from(r)))
        .expect("arms");

    // A budget an order of magnitude past the streak: if the gate did not fire, this
    // would run to the budget and the assertion below would say so.
    let fault = reactor
        .run_with(PollTimeout::Immediate, 10_000)
        .expect_err("an undrainable source must be refused, not tolerated");
    assert_eq!(
        fault,
        ReactorFault::UndrainableSource {
            token: src.as_token(),
            streak: MAX_UNPRODUCTIVE_STREAK,
        },
        "★ the refusal must name the offending token and stop at the BOUND — a gate that \
         fired after an unbounded number of spins would be indistinguishable from the \
         spin it exists to catch"
    );

    let s = handle.stats();
    assert_eq!(
        s.undrained_reports, MAX_UNPRODUCTIVE_STREAK,
        "★ exactly the streak's worth of unproductive reports, and not one more: the loop \
         stopped ON the bound. stats={s:?}"
    );
    assert_eq!(
        s.signals_pushed, 0,
        "an undrainable source must never manufacture a completion"
    );
    assert_eq!(
        s.wakes, MAX_UNPRODUCTIVE_STREAK,
        "★ and THIS is the F1 quantity doing its job: {} wakes for ZERO signals. The \
         equality of test 1 would read `0 == 0` here and pass; only the wake COUNT sees it",
        s.wakes
    );

    // Nothing reached the core.
    let mut exec = Executor::new(Arc::clone(&device), rx);
    assert!(
        exec.drain_all().is_empty(),
        "a refused loop must not have pushed anything"
    );
    handle.shutdown().expect("shutdown");
}

// =====================================================================================
// 3. THE SECOND SHAPE — a terminal source, which no drain can clear
// =====================================================================================

/// ★ `SourceKind::Worker`'s readiness is a **HUP**, not a count (§3.1) — and a hung-up
/// channel is readable forever with nothing to read. Under §3.4(c)'s "drain everything"
/// rule that is the spin from test 2, arriving through a source class the design itself
/// defines.
///
/// The loop therefore fires a terminal source **once** and stops watching it. Pinned three
/// ways: the fire count, the absence of any further wake, and — the end-state — the core's
/// own `WorkerDied` consequence.
#[test]
fn os_reactor_fires_a_worker_hup_exactly_once_and_stops_watching_it() {
    let (device, pids) = world(LockMode::Sharded);
    let reg = registrar(64);
    let (tx, rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) = Reactor::new(
        Arc::clone(&reg),
        tx,
        Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
    )
    .expect("the reactor builds");

    let src = device.register_source(SourceKind::Worker {
        proc: pids[0],
        gpu: GpuId::ZERO,
        worker: WorkerId(0),
    });
    assert_eq!(
        SourceShape::of(SourceKind::Worker {
            proc: pids[0],
            gpu: GpuId::ZERO,
            worker: WorkerId(0)
        }),
        SourceShape::Terminal,
        "the Worker class's shape is a property of the class, not of this test's choice"
    );
    let channel = reg.arm_channel(src).expect("arms");

    // Alive: nothing is ready.
    reactor
        .run_with(PollTimeout::Immediate, 4)
        .expect("no fault");
    assert_eq!(
        handle.stats().wakes,
        0,
        "a live worker's channel must not be ready — otherwise the 'fires once' below \
         would be about a source that was firing all along"
    );

    // ---- the worker dies.
    drop(channel);
    reactor
        .run_with(PollTimeout::Immediate, 64)
        .expect("a terminal source must not drive the loop into the F1 refusal");
    let s = handle.stats();
    assert_eq!(
        (s.terminal_fired, s.signals_pushed),
        (1, 1),
        "★ ONE fire, ONE signal — for a readiness that the kernel would otherwise report \
         on all 64 of those waits. stats={s:?}"
    );
    assert_eq!(
        s.wakes, 1,
        "★ the F1 quantity again: the loop woke exactly once for a permanently-readable \
         descriptor, because it stopped watching what it cannot clear. 64 would be the \
         spin. stats={s:?}"
    );
    assert_eq!(
        s.undrained_reports, 0,
        "the fire counted as productive work"
    );

    // ---- the end-state: the core kills the slot and retires the proc (§7.3 — never a
    // silent respawn).
    let mut exec = Executor::new(Arc::clone(&device), rx);
    let effects = exec.drain_all();
    assert_eq!(
        effects,
        vec![Effect::Signal(SignalOutcome::WorkerDied {
            proc: pids[0],
            gpu: GpuId::ZERO,
            worker: WorkerId(0),
        })],
        "a channel HUP is the worker-death consequence, by exact variant"
    );
    assert!(
        !device.live_pids().contains(&pids[0]),
        "★ and the proc really retired — a worker that died mid-verb may have left host \
         state the core cannot reason about"
    );
    assert!(
        device.live_pids().contains(&pids[1]),
        "…and the other proc is untouched"
    );
    handle.shutdown().expect("shutdown");
}

// =====================================================================================
// 4. §3.3 — a report in flight across a deregistration
// =====================================================================================

/// ★★ §3.3's two rules, and its **designed** answer — constructed rather than raced.
///
/// > *"Deregister then close, never close then deregister. Closing a descriptor removes it
/// > from the readiness set **only if it was the last reference**; a duplicated one
/// > silently stays and keeps firing."*
/// >
/// > *"A readiness report in flight across a deregistration is normal and safe… it pushes
/// > them anyway, and dispatch faults. `SourceFault` is not an error path we are
/// > tolerating — it is the **designed** answer."*
///
/// Racing a real disarm against a real batch would make the bite a coin flip. So the
/// hazard the doc names is **built**: a second registration under the same token, which is
/// exactly what a duplicated descriptor is. The disarm removes the source's own descriptor
/// and the duplicate keeps reporting — and the report lands on a handle that is now
/// retired, which must fault and must never route onto whatever was armed next.
#[test]
fn os_a_report_in_flight_across_a_disarm_faults_and_never_reroutes() {
    let (device, pids) = world(LockMode::Sharded);
    let reg = registrar(64);
    let (tx, rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) = Reactor::new(
        Arc::clone(&reg),
        tx,
        Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
    )
    .expect("the reactor builds");

    let doomed = device.register_source(SourceKind::OsEvent {
        proc: pids[0],
        gpu: GpuId::ZERO,
        ev: OsEventRef(0x11),
    });
    let counter = reg.arm_counter(doomed).expect("arms");

    // ★ The hazard, built: a second readiness registration carrying the SAME token. This
    // is what a duplicated descriptor is, and §3.3 says it stays and keeps firing.
    let (dup_r, dup_w) = std::io::pipe().expect("a pipe");
    std::io::Write::write_all(&mut { dup_w }, b"!").expect("permanently readable");
    reg.poller()
        .watch(std::os::fd::AsFd::as_fd(&dup_r), doomed.as_token())
        .expect("the duplicate is watched under the same token");

    // While it is live, both registrations resolve to the live source and the drain works.
    counter.signal().expect("signal");
    reactor
        .run_with(PollTimeout::Immediate, 2)
        .expect("no fault");
    assert_eq!(handle.stats().signals_pushed, 1);

    // ---- the guest tears the proc down, so the core deregisters the handle...
    device
        .apply(RmEvent::Free {
            client: HClient(0xAA),
            handle: CLIENT_ROOT,
        })
        .expect("teardown applies");
    // ...and the shell disarms it: DEREGISTER (the counter leaves the readiness set) THEN
    // CLOSE (the `Arc` drops).
    reg.disarm(doomed).expect("disarms");
    assert!(
        matches!(reg.resolve(doomed.as_token()), Resolved::Retired(s) if s == doomed),
        "★ a just-disarmed handle is REMEMBERED, not forgotten: that is what lets a report \
         already in flight resolve to the handle it names instead of to nothing"
    );

    // ---- the duplicate is still firing. The report resolves to a retired handle.
    reactor
        .run_with(PollTimeout::Immediate, 4)
        .expect("no fault");
    let s = handle.stats();
    assert!(
        s.retired_reports >= 1,
        "the duplicated registration must still be reporting — otherwise this test proves \
         nothing about a report in flight. stats={s:?}"
    );

    // ---- ★ and the report FAULTS. Never a route onto the retired proc, never a route
    // onto whatever was armed next.
    let mut exec = Executor::new(Arc::clone(&device), rx);
    let effects = exec.drain_all();
    assert!(
        !effects.is_empty(),
        "the retired reports were pushed, per §3.3"
    );
    for e in &effects {
        assert_eq!(
            *e,
            Effect::Signal(SignalOutcome::Fault(SourceFault { source: doomed })),
            "★ a signal on a handle whose proc retired must be a loud SourceFault naming \
             THAT handle — the C's F4 use-after-retire species, designed out by a mint \
             that never recycles"
        );
    }
    assert!(
        device
            .with_proc(pids[1], |p| p.completion.outstanding_len())
            .expect("proc B is live")
            == 0,
        "and it certainly must not have landed in the surviving proc's queue"
    );

    // A handle the registrar never knew is a different answer again — "forgotten" and
    // "never armed" must not collapse into one.
    let never = device.register_source(SourceKind::Notify);
    assert!(matches!(reg.resolve(never.as_token()), Resolved::Unknown));
    reg.poller()
        .unwatch(std::os::fd::AsFd::as_fd(&dup_r))
        .expect("clean up the deliberate duplicate");
    handle.shutdown().expect("shutdown");
}

// =====================================================================================
// 5. §3.8 — the source cap
// =====================================================================================

/// ★ §3.8's cap, **exercised** — *"an unexercised refusal path is where the C's worst
/// behaviour lived."*
///
/// One armed os-event costs a descriptor in the VMM process, so an unbounded arm loop is a
/// device-wide (and possibly VMM-wide) descriptor exhaustion — *"not a contained refusal"*.
/// The refusal is asserted by exact variant with both numbers, and containment is asserted
/// positively: the sources armed before the bound still work afterwards.
#[test]
fn os_the_source_cap_refuses_by_name_and_the_refusal_is_contained() {
    let (device, pids) = world(LockMode::Sharded);
    const BOUND: u64 = 4;
    let reg = registrar(BOUND);
    let (tx, rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) = Reactor::new(
        Arc::clone(&reg),
        tx,
        Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
    )
    .expect("the reactor builds");

    let mut armed = Vec::new();
    for i in 0..BOUND {
        let src = device.register_source(SourceKind::OsEvent {
            proc: pids[0],
            gpu: GpuId::ZERO,
            ev: OsEventRef(i),
        });
        armed.push((src, reg.arm_counter(src).expect("under the bound")));
    }
    assert_eq!(reg.armed(), BOUND);

    // ---- past the bound: refused, by exact variant and with both numbers.
    let over = device.register_source(SourceKind::OsEvent {
        proc: pids[0],
        gpu: GpuId::ZERO,
        ev: OsEventRef(0xFFFF),
    });
    assert_eq!(
        reg.arm_counter(over).unwrap_err(),
        ReactorError::SourceBudgetExhausted {
            armed: BOUND,
            bound: BOUND
        },
        "★ the arm past the cap is refused with the numbers an operator needs, not a bare \
         'it refused'"
    );
    assert_eq!(
        reg.armed(),
        BOUND,
        "a refused arm must leave the table exactly as it found it — no descriptor with no \
         owner"
    );

    // ---- ★ CONTAINMENT: the refusal cost the guest its arm and nothing else.
    for (_, n) in &armed {
        n.signal()
            .expect("every source armed before the bound still works");
    }
    reactor
        .run_with(PollTimeout::Immediate, 8)
        .expect("no fault");
    assert_eq!(
        handle.stats().signals_pushed,
        BOUND,
        "★ containment: every previously-armed source still delivers. A cap that broke the \
         sources under it would be a device-wide failure wearing a refusal's clothes"
    );

    // Re-arming the same handle is a refusal too, and a DIFFERENT one — the two must never
    // collapse, because one is a resource limit and the other is a caller bug.
    reg.disarm(armed[0].0).expect("disarms");
    let (fresh, _n) = {
        let src = armed[1].0;
        (src, ())
    };
    assert_eq!(
        reg.arm(fresh, HostSource::Counter(Arc::clone(&armed[1].1))),
        Err(ReactorError::AlreadyArmed)
    );
    assert_eq!(reg.disarm(armed[0].0), Err(ReactorError::NotArmed));

    // And the derived bound is a real function of the process's real limit.
    let derived = Registrar::new(Arc::new(Poller::create().expect("set"))).expect("derives");
    assert!(
        derived.bound() > 0 && derived.bound() <= kayfabe_shell::MAX_ARMED_SOURCES,
        "the RLIMIT_NOFILE-derived bound must be positive and under the absolute ceiling, \
         got {}",
        derived.bound()
    );
    assert!(
        derived.bound() < kayfabe_linux_raw::descriptor_budget().expect("limit"),
        "★ the reserve is real: the reactor may not spend the WHOLE descriptor budget, or \
         guest RAM windows and isolate channels have nothing left"
    );

    let mut exec = Executor::new(Arc::clone(&device), rx);
    assert_eq!(
        exec.drain_all().len(),
        usize::try_from(BOUND).expect("small")
    );
    handle.shutdown().expect("shutdown");
    reg.disarm_all();
}

// =====================================================================================
// 6. THE WHOLE CHAIN, ON REAL THREADS — the T2-OS gate
// =====================================================================================

/// ★★ **T2-OS** (§10's M2-d gate): *"a real counter fires and a real IRQ descriptor is
/// observed end to end; the wake-count assert."*
///
/// Everything here is real and nothing is polled by the test: a producer thread writes real
/// counter descriptors, the reactor thread blocks in a real `epoll_wait`, the executor
/// thread is parked on the [`ExecutorWaker`] and is woken by the reactor, and the
/// completions land in the core under its own ranked locks. The test's only synchronisation
/// is joining the threads.
///
/// The wake-count gate rides along and is *stronger* here than in test 1, because the
/// producer runs concurrently with the loop: coalescing is now the kernel's choice rather
/// than the test's, and `signals_pushed == K` has to hold across every choice it makes.
#[test]
fn os_a_real_counter_signal_reaches_the_core_through_real_reactor_and_executor_threads() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pids) = world(mode);
        let reg = registrar(64);
        let (tx, rx) = inbox();
        let parker = Arc::new(Parker::new());
        let (mut reactor, handle) = Reactor::new(
            Arc::clone(&reg),
            tx,
            Arc::clone(&parker) as Arc<dyn ExecutorWaker>,
        )
        .expect("the reactor builds");

        let src = device.register_source(SourceKind::OsEvent {
            proc: pids[1],
            gpu: GpuId(1),
            ev: OsEventRef(0x5150),
        });
        let counter = reg.arm_counter(src).expect("arms");

        // A real IRQ descriptor, on the real memory plane, observed at the end.
        let machine = kayfabe_vmm_kvm::KvmMachine::realize(kayfabe_vmm_kvm::MachineConfig {
            shareable_ram: true,
            bars: Vec::new(),
        })
        .expect("/dev/kvm must be present and permitted (a deployment fact, §9.3)");
        let mut vmm = machine.vmm();

        const K: u64 = 200;
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Effect>::new()));
        let stop_exec = Arc::new(AtomicBool::new(false));

        thread::scope(|sc| {
            // ---- the reactor thread: a real blocking wait, woken by real descriptors.
            let t_reactor = sc.spawn(move || {
                // A bounded timeout so the loop notices shutdown even if the control
                // source's edge were ever lost — a backstop, never the mechanism.
                reactor.run_with(PollTimeout::Millis(50), 100_000)
            });

            // ---- the executor thread: parked, woken by the reactor's ExecutorWaker.
            let dev = Arc::clone(&device);
            let sink = Arc::clone(&seen);
            let pk = Arc::clone(&parker);
            let t_exec = sc.spawn(move || {
                let mut exec = Executor::new(dev, rx);
                // ★ A park interval of a MINUTE, and that is the assertion: if the
                // reactor's `ExecutorWaker` did not really wake this thread, nothing here
                // would make progress before the deadline below. A short interval would
                // have turned the §3.7 seam into a poll and hidden it — which a bite-check
                // duly demonstrated (neutering `wake()` to a no-op did not bite at 50 ms).
                exec.run_until_stopped(&pk, Duration::from_secs(60), |e| {
                    sink.lock().expect("sink").push(e);
                });
            });

            // ---- the producer: a relay writing a real counter, K times.
            for _ in 0..K {
                counter.signal().expect("relay write");
            }

            // Wait for the core to have observed all K — a condition, never a sleep.
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while device
                .with_proc(pids[1], |p| p.completion.outstanding_len())
                .unwrap_or(0)
                < K as usize
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "({mode:?}) the chain wedged: {:?}",
                    handle.stats()
                );
                std::thread::yield_now();
            }

            handle.shutdown().expect("shutdown");
            let fault = t_reactor.join().expect("the reactor thread joins");
            assert_eq!(fault, Ok(()), "({mode:?}) the loop must exit cleanly");
            stop_exec.store(true, Ordering::Release);
            parker.stop();
            t_exec.join().expect("the executor thread joins");
        });

        // ---- ★ the F1 gate, with the kernel choosing the batching.
        let s = handle.stats();
        assert_eq!(
            s.signals_pushed, K,
            "({mode:?}) ★ exactly K signals, however the kernel chose to coalesce them: {s:?}"
        );
        assert!(
            s.wakes >= 1 && s.wakes <= K,
            "({mode:?}) ★ the wake count is bounded by the signal count: {s:?}"
        );
        assert_eq!(
            (s.undrained_reports, s.stale_reports),
            (0, 0),
            "({mode:?}) every report was drained and named a live source: {s:?}"
        );
        assert!(
            s.control_reports >= 1,
            "({mode:?}) ★ the shutdown really travelled through the control source — \
             otherwise the loop exited on its wait budget and this test is about a timeout"
        );

        // ---- ★ the end-state, in the core.
        let effects = seen.lock().expect("sink").clone();
        assert_eq!(
            effects.len() as u64,
            K,
            "({mode:?}) the executor thread ran every event"
        );
        assert!(
            effects.iter().all(|e| matches!(
                e,
                Effect::Signal(SignalOutcome::Observed { proc, gpu, ev })
                    if *proc == pids[1] && *gpu == GpuId(1) && *ev == OsEventRef(0x5150)
            )),
            "({mode:?}) every one routed to the owning proc and target"
        );

        // ---- ★ and a REAL interrupt descriptor, observed.
        use kayfabe_vmm::Vmm as _;
        for _ in 0..3 {
            vmm.raise_irq(kayfabe_vmm::IrqSpec::Msix(0))
                .expect("a real notify-descriptor write");
        }
        assert_eq!(
            machine.drain_irqs().expect("drain"),
            3,
            "({mode:?}) ★ T2-OS's other half: three real interrupt edges, counted by the \
             kernel and drained exactly once"
        );
        assert_eq!(
            machine.drain_irqs().expect("drain"),
            0,
            "({mode:?}) and the counter self-clears — the same drain discipline the reactor \
             depends on"
        );
        reg.disarm_all();
    }
}

// =====================================================================================
// 7. The retired ring's bound — G10's shape, one component over
// =====================================================================================

/// ★ The memory of retired handles is **bounded**, and its overflow behaviour is the one
/// that keeps a refusal contained rather than a fault silent.
///
/// §3.3 wants a report in flight across a deregistration to still resolve to its handle, so
/// the registrar remembers retired ones. Retirement is guest-driven, so remembering them
/// all is `l1_os_shell.md` §3.8's G10 shape — *"`condemned` and `retired` are unbounded"* —
/// rebuilt in the shell. The bound is therefore real, and evicting the oldest costs only
/// the **diagnosis**: an evicted late report becomes a counted stale report instead of a
/// `SourceFault`, and both mutate nothing.
///
/// Pinned in both directions, because a bound asserted only from the "still remembered"
/// side is a bound that could be infinite.
#[test]
fn os_the_memory_of_retired_handles_is_bounded_and_forgets_the_oldest_first() {
    let (device, pids) = world(LockMode::Sharded);
    let reg = registrar(u64::try_from(kayfabe_shell::MAX_REMEMBERED_RETIRED + 8).expect("small"));

    // Arm and immediately disarm more sources than the ring can remember.
    let mut order = Vec::new();
    for i in 0..(kayfabe_shell::MAX_REMEMBERED_RETIRED + 4) {
        let src = device.register_source(SourceKind::OsEvent {
            proc: pids[0],
            gpu: GpuId::ZERO,
            ev: OsEventRef(i as u64),
        });
        reg.arm_counter(src).expect("arms");
        reg.disarm(src).expect("disarms");
        order.push(src);
    }
    assert_eq!(reg.armed(), 0, "nothing is left armed");

    // The newest are remembered — a report in flight for one of them still names its handle.
    let newest = *order.last().expect("some");
    assert!(
        matches!(reg.resolve(newest.as_token()), Resolved::Retired(s) if s == newest),
        "the most recently retired handle must still be remembered"
    );
    // ★ …and the oldest have been forgotten, which is the half that proves the bound.
    let oldest = order[0];
    assert!(
        matches!(reg.resolve(oldest.as_token()), Resolved::Unknown),
        "★ the ring is BOUNDED: {} retirements past its capacity, and the first one is \
         forgotten. Unbounded memory of dead handles is exactly the growth §3.8 names",
        order.len()
    );
    // Forgetting is not the same as guessing: the forgotten handle resolves to NOTHING, so
    // a late report for it can never be re-bound to a live source.
    assert!(
        !matches!(reg.resolve(oldest.as_token()), Resolved::Live { .. }),
        "a forgotten handle must never resolve to whatever was armed next"
    );
}
