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
//!    wakes, never signals), and `wakes` is **bounded above** by that number *plus the
//!    loop's own control wakes* (a level-triggered spin blows through it without bound).
//!    ★ That second term is not decoration. The loop's readiness set always holds the
//!    control notifier alongside the armed sources, so in any harness that calls
//!    `shutdown()` while the loop is running a wake has **two** causes and a ceiling of `K`
//!    is a race — measured red on an unmodified tree twice, `l1_mean.rs` 2026-07-27 and
//!    test 6 below 2026-08-01. Both are structural; neither reads a clock.
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
use std::sync::{Arc, Condvar, Mutex};
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

/// ★★ **The watchdog**: cleanup that survives an unwind.
///
/// Every assertion in the scope below runs while two real threads are parked on this
/// test — the reactor inside a real `epoll_wait`, the executor on the [`Parker`]'s
/// condvar — and **only** `handle.shutdown()` and `parker.stop()` can ever wake them. A
/// panicking assert unwinds straight past both, and `thread::scope`'s join-all then waits
/// forever on threads nobody will ever tell to stop: the panic message is real, but the
/// process never exits, so libtest never prints a result and cargo never returns. A slow
/// box thereby converts a loud, readable failure into an indefinite hang — and a hang gets
/// re-run, not read. (Measured, not theorised: before this guard existed, an induced
/// `assert_eq!(1, 2)` here left cargo and the test binary alive indefinitely with a
/// *completely empty* captured log, and three such binaries were found wedged on this box
/// for 23 hours.)
///
/// So the shutdown moves into a `Drop`, and the path is then identical whether the body
/// returns or panics — the same lesson, in the same shape, as `l1_mean.rs`'s `Latches` and
/// `OpenOnDrop` drop guards.
///
/// ## ★ Why this `Drop` is safe under R1 — *a `Drop` is a call site*
///
/// Whatever a `Drop` does is subject to the same rules as ordinary code, and both calls
/// here are R1-guarded: [`ReactorHandle::shutdown`](kayfabe_shell::ReactorHandle::shutdown)
/// and [`Parker::stop`] are entered through the witness and refuse a caller holding a
/// ranked lock. This guard satisfies that on **both** exits:
///
/// - **Normal return** — it is a local of the scope closure, so it drops after the closure's
///   last statement. The test holds no core lock between statements: `with_proc` and
///   `stats()` take and release theirs inside the call.
/// - **Unwind** — unwinding drops in reverse creation order, and this guard is created
///   *before* every assertion below. So any lock guard live at the panic point (the sink's
///   `MutexGuard`, `with_proc`'s internal one) is a later creation and is dropped strictly
///   first. The witness therefore reads zero by the time this `Drop` runs.
///
/// Neither call blocks, either: `shutdown` is an atomic store plus a non-blocking counter
/// write, and `stop` takes only the parker's own uncontended state mutex to set a flag and
/// notify. And the `Result` is **dropped, not unwrapped** — a panic inside a `Drop` that is
/// itself running on an unwind aborts the process and destroys the very message this guard
/// exists to let through.
struct StopOnDrop<'a> {
    handle: &'a kayfabe_shell::ReactorHandle,
    parker: &'a Parker,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        // Both are idempotent, so running on the success path after an explicit shutdown
        // is a no-op rather than a second meaning.
        let _ = self.handle.shutdown();
        self.parker.stop();
    }
}

/// ★ The executor thread's sink — and the main thread's **rendezvous** with it.
///
/// This replaces a `while … { yield_now() }` spin over the core's completion depth. That
/// spin was not merely inelegant: it burned a whole core inside the very window in which
/// the reactor and executor threads had to make progress, so the instrument perturbed what
/// it measured, and it did so *worst* on the small boxes where there is no slack to absorb
/// it. Blocking on the real condition costs nothing and cannot starve anybody.
///
/// The condition is the end of the chain, so it is strictly stronger than the depth poll it
/// replaces: an [`Effect`] is only handed to `on_effect` **after** `signal_source` has
/// already applied the completion into the owning proc's queue under the core's own locks.
#[derive(Default)]
struct EffectSink {
    seen: Mutex<Vec<Effect>>,
    arrived: Condvar,
}

impl EffectSink {
    /// Record one effect and wake the waiter. Called on the executor thread, which the
    /// `run_until_stopped` contract guarantees holds no lock here.
    ///
    /// Poisoning is tolerated rather than re-panicked: if the main thread died holding this
    /// lock it is already reporting a failure, and a second panic on the executor thread
    /// would only bury it under `thread::scope`'s propagation.
    fn push(&self, e: Effect) {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(e);
        self.arrived.notify_all();
    }

    /// Block until at least `want` effects have arrived; return how many actually did.
    ///
    /// `hang_guard` is a **last-resort backstop, never the synchronisation mechanism** —
    /// the condvar is. It exists only so a genuinely broken chain reports instead of
    /// hanging, and the count it returns is what lets the caller say *what did not arrive*.
    fn wait_for(&self, want: usize, hang_guard: Duration) -> usize {
        let g = self.seen.lock().expect("sink");
        let (g, _timed_out) = self
            .arrived
            .wait_timeout_while(g, hang_guard, |v| v.len() < want)
            .expect("sink");
        g.len()
    }

    /// Everything recorded so far.
    fn snapshot(&self) -> Vec<Effect> {
        self.seen.lock().expect("sink").clone()
    }
}

/// ★★ **T2-OS** (§10's M2-d gate): *"a real counter fires and a real IRQ descriptor is
/// observed end to end; the wake-count assert."*
///
/// Everything here is real and nothing is polled by the test: a producer thread writes real
/// counter descriptors, the reactor thread blocks in a real `epoll_wait`, the executor
/// thread is parked on the [`ExecutorWaker`] and is woken by the reactor, and the
/// completions land in the core under its own ranked locks. The test's own synchronisation
/// is structural too — an [`EffectSink`] condvar signalled by the executor thread, and then
/// joining the threads. Nothing here polls, sleeps or yields.
///
/// The wake-count gate rides along and is *stronger* here than in test 1, because the
/// producer runs concurrently with the loop: coalescing is now the kernel's choice rather
/// than the test's, and `signals_pushed == K` has to hold across every choice it makes.
#[test]
fn os_a_real_counter_signal_reaches_the_core_through_real_reactor_and_executor_threads() {
    kayfabe_linux_raw::require_kvm!(
        "os_a_real_counter_signal_reaches_the_core_through_real_reactor_and_executor_threads"
    );
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
        /// The backstop on the rendezvous below — **not** how the test waits. Generous,
        /// because it costs exactly nothing when the chain works: the wait is a condvar,
        /// so this deadline is only ever reached by a chain that is genuinely broken.
        const HANG_GUARD: Duration = Duration::from_secs(30);
        let seen = Arc::new(EffectSink::default());

        thread::scope(|sc| {
            // ★ Armed BEFORE the first assertion, so an unwind from any of them still
            // stops both threads and lets `thread::scope` join. See `StopOnDrop`.
            let _stop = StopOnDrop {
                handle: &handle,
                parker: &parker,
            };

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
                exec.run_until_stopped(&pk, Duration::from_secs(60), |e| sink.push(e));
            });

            // ---- the producer: a relay writing a real counter, K times.
            for _ in 0..K {
                counter.signal().expect("relay write");
            }

            // ---- ★ THE RENDEZVOUS. The main thread now *blocks* on the far end of the
            // chain until the executor thread has run all K events — a real condition on a
            // real condvar, woken by the thread that does the work, so this thread consumes
            // no CPU and starves nobody. (It used to spin on `yield_now`, which on a
            // 4-core box competes for a core with the two threads it is waiting on.)
            let arrived = seen.wait_for(K as usize, HANG_GUARD);
            assert_eq!(
                arrived,
                K as usize,
                "({mode:?}) ★ THE CHAIN DID NOT COMPLETE. {arrived} of {K} effects reached \
                 the executor thread within {HANG_GUARD:?}; the missing {} never traversed \
                 counter → epoll → inbox → ExecutorWaker → executor. reactor stats={:?}; \
                 proc {:?}'s completion queue holds {:?}; parker seq={} stopping={}. (This \
                 deadline is a backstop on a condvar wait, NOT how the test synchronises — \
                 if it fired, something is genuinely wedged, not merely slow.)",
                K as usize - arrived,
                handle.stats(),
                pids[1],
                device.with_proc(pids[1], |p| p.completion.outstanding_len()),
                parker.sequence(),
                parker.is_stopping(),
            );
            // ★ …and the same end-state the old spin only ever waited for: the completions
            // really are in the OWNING proc's queue, asserted as an equality rather than
            // implied by a loop condition.
            assert_eq!(
                device.with_proc(pids[1], |p| p.completion.outstanding_len()),
                Some(K as usize),
                "({mode:?}) ★ all K completions sit in proc {:?}'s own queue, put there by \
                 the core's own dispatch",
                pids[1],
            );

            handle.shutdown().expect("shutdown");
            let fault = t_reactor.join().expect("the reactor thread joins");
            assert_eq!(fault, Ok(()), "({mode:?}) the loop must exit cleanly");
            parker.stop();
            t_exec.join().expect("the executor thread joins");
        });

        // ---- ★ the F1 gate, with the kernel choosing the batching.
        let s = handle.stats();
        assert_eq!(
            s.signals_pushed, K,
            "({mode:?}) ★ exactly K signals, however the kernel chose to coalesce them: {s:?}"
        );
        // ★★ THE CEILING INCLUDES THE LOOP'S OWN CONTROL WAKE — and it did not until
        // 2026-08-01 (#147), which is #73's defect surviving one assertion to the left of
        // where #73 looked.
        //
        // `ReactorStats::wakes` has exactly ONE increment site — `Reactor::run_with`, on
        // every wait that came back with at least one ready token — and this run's readiness
        // set holds exactly TWO descriptors: the counter armed above, and the loop's own
        // control notifier, which `Reactor::new` watches under `CONTROL_TOKEN` before the
        // loop ever starts. So a wake has **two** causes, not one, and `handle.shutdown()`
        // below the rendezvous writes the second of them (`stop.store(Release)`, *then*
        // `control.signal()`). When that write lands in a wait of its own instead of
        // coalescing into one that also carried a counter edge, `wakes` is `K + 1` and a
        // ceiling of `K` is red. Measured on an unmodified tree under a full-suite run,
        // 2026-08-01: `wakes: 201, ready_reports: 201, signals_pushed: 200,
        // control_reports: 1` — the excess being that one control report, exactly.
        // `ReactorStats::wakes`'s own rustdoc had said so all along: *"bounded above by the
        // number of signals sent plus the control wakes"*.
        //
        // ★★ AND IT IS NOT A RARE SCHEDULE, IT IS A REGIME — which is worth more than the
        // 1.15% rate it was first written down as. The producer above writes its K counter
        // units in a tight loop, so on a fast box the kernel coalesces them into a handful
        // of readiness reports and `wakes` lands nowhere near K. Put **50 µs between the
        // writes** and the loop drains between every one of them instead: `wakes` becomes
        // `K + 1` in 58 of 60 samples, and the pre-#147 assertion at this line fails with
        // `ReactorStats { wakes: 201, idle_waits: 0, ready_reports: 201, signals_pushed:
        // 200, control_reports: 1, … }` — the observed line, reconstructed. So the trigger
        // is not luck, it is *anything that spaces the producer's writes out by more than
        // the loop's drain latency*, which under a full workspace suite is an ordinary
        // preemption. The bound had to be wrong, not rare.
        //
        // ★ It is the same off-by-one `l1_mean.rs`'s reactor gate was fixed for on
        // 2026-07-27 (`RSIGNALS + 1` → `signals_pushed + MAX_CONTROL_WAKES`), and the same
        // mechanism #73 removed `control_reports >= 1` from THIS test for on 2026-07-30.
        // Those two are opposite polarities of one coin: the shutdown edge either reaches
        // the loop through the control descriptor — this ceiling's `+ 1` — or it does not,
        // which is what `control_reports >= 1` raced on. #73 corrected the polarity it was
        // looking at and left the other one standing, in the same `assert!` block.
        //
        // ★ The slack is a LITERAL and not `s.control_reports`, for `l1_mean.rs`'s reason: a
        // ceiling whose slack is read out of the very snapshot it is bounding is asserted
        // against itself, and the runaway it exists to catch would raise it without limit.
        // The slack is pinned separately instead, immediately below.
        const MAX_CONTROL_WAKES: u64 = 1;
        // ☆ THE PIN, first — because a ceiling of `K + 1` is only honest while the thing it
        // makes room for cannot happen twice. Exactly one write to the control descriptor is
        // reachable before the loop returns: the `shutdown()` above. `ReactorHandle::wake`
        // is called nowhere in this harness, and `StopOnDrop`'s second `shutdown()` runs
        // after `t_reactor` has already been joined. `handle_token` DRAINS what it reads, so
        // anything above one is the control source left readable and re-reporting — the F1
        // spin, on the loop's own descriptor.
        //
        // ⊘ This is NOT #73's `control_reports >= 1` turned around. That one asserted the
        // shutdown edge *arrived*, which is a scheduling order and therefore a clock. This
        // one says it cannot arrive *twice*, which no scheduling order can change: #73's
        // whole race lives inside {0, 1}, and both are green here.
        assert!(
            s.control_reports <= MAX_CONTROL_WAKES,
            "({mode:?}) ★ at most the ONE control wake this run can perform — the \
             teardown's `shutdown()`. Above it means the control descriptor was not drained \
             and re-reported: the F1 spin, on the loop's own descriptor. {s:?}"
        );
        assert!(
            s.wakes >= 1 && s.wakes <= K + MAX_CONTROL_WAKES,
            "({mode:?}) ★ the F1 quantity: {} wakes for {K} signals plus at most \
             {MAX_CONTROL_WAKES} control wake. `>= 1` is the non-vacuity half (a loop that \
             never woke satisfies every ceiling); the upper half is what an undrained \
             level-triggered source blows through without limit. The signal term is pinned \
             as an EQUALITY directly above, the control term is a literal and is pinned too, \
             so this is the fixed ceiling {} and not an arithmetic identity. {s:?}",
            s.wakes,
            K + MAX_CONTROL_WAKES,
        );
        // ★ …and the same ceiling one level closer to the kernel, which is the form test 1
        // asserts. `wakes <= ready_reports` holds BY CONSTRUCTION — a wake is a wait that
        // carried at least one report — so this is the clause that says the ceiling above is
        // about the loop's own ARITHMETIC rather than about how the kernel chose to batch.
        //
        // ★ Which of the three clauses catches what, MEASURED on 2026-08-01 (38-core box)
        // rather than reasoned, because the answer was not the expected one:
        //   * the loop re-signalling its own control descriptor once per batch — the
        //     "removal barrier" over-applied — is caught by the PIN, `control_reports: 268`;
        //   * `wakes.fetch_add(2)` (every wait counted twice) is caught HERE, at
        //     `wakes: 24, ready_reports: 12`, and slips straight past the ceiling above;
        //   * the ceiling above is reached by an instrument that CONFLATES the two
        //     quantities the F1 gate keeps apart — a wake counted per signal *pushed* as
        //     well as per wait. That one fires 10 times in 10, at `wakes: 232`, and it is
        //     deterministic for the reason the ceiling is a ceiling: `signals_pushed` is
        //     pinned at K directly above, so `K + wakes` clears `K + 1` on every schedule.
        //   * `wakes.fetch_add(16)` reaches it only 7 times in 20 (the other 13 land on the
        //     clause here) — recorded because a probabilistic biter is not a bite.
        // The reason the ×2 form cannot reach a ceiling of 201 is worth writing down: on
        // that box K = 200 signals coalesced into **2 to 43** wakes (n = 1160 samples over
        // five core pinnings, from one core to all of them, `wakes` never once above 43), so
        // the ceiling carries 5× to 100× of slack there. `ready_reports.fetch_add(n + 1)` is
        // caught by nothing here for the same reason, and that is recorded rather than
        // hidden. The regime the flake came from is the opposite one — the loop draining
        // between every producer write, `ready_reports` equal to K — and it is the regime
        // in which these ceilings are tight. ⊘ It is NOT reached by loading the box: 400
        // runs at 40-way concurrency drove `wakes` DOWN (median 9), because starving the
        // loop coalesces harder. It is reached by spacing the PRODUCER out, which is why
        // the 50 µs reconstruction above is the honest repro and "run it 180 more times"
        // was not.
        //
        // ⊘ Stated rather than implied: `ready_reports <= K + MAX_CONTROL_WAKES` is a
        // CONSEQUENCE of `signals_pushed == K` and `undrained_reports == 0` below it, not an
        // independent fact — every non-control report either drained a unit or is counted as
        // undrained. The F1 teeth are therefore in `undrained_reports == 0` and in test 2's
        // `UndrainableSource` refusal; these three lines are the arithmetic frame that makes
        // the widening above provable instead of merely convenient.
        assert!(
            s.wakes <= s.ready_reports && s.ready_reports <= K + MAX_CONTROL_WAKES,
            "({mode:?}) each wake carries at least one report, and the {K} counter units \
             plus at most {MAX_CONTROL_WAKES} control edge are the only things in this \
             loop's readiness set that can report: {s:?}"
        );
        assert_eq!(
            (s.undrained_reports, s.stale_reports),
            (0, 0),
            "({mode:?}) every report was drained and named a live source: {s:?}"
        );
        // ★★ What this test can and cannot say about the shutdown edge.
        //
        // It used to assert `control_reports >= 1` here — *"the shutdown really travelled
        // through the control source"* — and that failed on unmodified master at about
        // 1.15%. It is not a race in the product: `ReactorHandle::shutdown` publishes the
        // stop through **two** independent channels on purpose (`stop.store(Release)` then
        // `control.signal()`), and the loop honours whichever it reaches first. Which one
        // that is, is a scheduling order — a clock.
        //
        // The window is wide and was measured: the loop pushes its last `SourceSignal` onto
        // the inbox BEFORE it calls `waker.wake()`, so an executor thread that is still
        // awake can consume it, finish the chain and release the main thread's rendezvous
        // while the loop is still inside `wake()`. `shutdown()` then lands before the loop
        // reaches its post-batch `stop` check, the loop returns `Ok` from there, and the
        // control descriptor is simply never polled. Inserting a 5 ms sleep in front of
        // `waker.wake()` turns that 1.15% into 9 failures in 10.
        //
        // ⊘ Removing it must not remove the coverage, and it does not: proving the control
        // descriptor really carries an edge into the loop moved to
        // `os_the_control_source_is_the_loops_only_way_to_learn_of_a_shutdown_it_is_waiting_on`,
        // which drives it single-threaded with nothing else armed, so it is an EQUALITY with
        // no order in it at all. That matters more than it looks: production `Reactor::run`
        // waits with `PollTimeout::Blocking`, where the `stop` flag alone is never observed
        // and a dead control source hangs the loop forever. The 50 ms backstop above is what
        // hides that here — which is exactly why the property belongs in a test that does
        // not have one.
        //
        // ★ #147, 2026-08-01 — this paragraph used to end *"and NOT replaced here by a
        // `control_reports <= 1` consolation bound"*, on the measured ground that poisoning
        // `handle_token` to leave the control source readable does not make it fire, because
        // the loop exits on the very next line once `stop` is set. That measurement still
        // holds, and 6b is still the only place THAT mutant bites. What it got wrong is the
        // conclusion. `control_reports <= 1` is not a consolation here: it is the pin under
        // the `+ MAX_CONTROL_WAKES` slack the ceiling above now needs, and without it that
        // slack would be a literal justified by nothing in the test. It also bites a
        // different mutant — a loop that re-signals its own control descriptor once per
        // batch, the "removal barrier" over-applied — which was watched fail at
        // `control_reports: 94`. It is asserted above.
        //
        // ★ What DOES survive here is the concern the old assertion's own message named —
        // "otherwise the loop exited on its wait budget" — as a bound that cannot race.
        assert!(
            s.wakes + s.idle_waits < 100_000,
            "({mode:?}) ★ the loop returned because it was told to stop, NOT because it \
             spent all 100_000 of its waits: {s:?}"
        );

        // ---- ★ the end-state, in the core.
        let effects = seen.snapshot();
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
// 6b. THE CONTROL SOURCE, WITH NO ORDER IN IT
// =====================================================================================

/// ★★ The loop's control descriptor really carries an edge, and it is the **only** thing
/// that can end a wait — asserted as an equality, single-threaded, with nothing armed.
///
/// This is the half test 6 cannot state. `ReactorHandle::shutdown` publishes the stop
/// through two independent channels — `stop.store(Release)` and `control.signal()` — and a
/// loop running concurrently honours whichever it reaches first, so *which* channel carried
/// it is a scheduling order. Test 6 used to assert the descriptor won that race and failed
/// about 1.15% of the time saying so.
///
/// Here nothing races: the signal is written **before** the wait begins, on this thread, and
/// nothing else is armed, so the only descriptor in the readiness set is the control one.
/// `control_reports` is therefore an exact count and `signals_pushed` an exact zero.
///
/// ★ Why the property is worth a test of its own rather than a bound inside test 6:
/// production `Reactor::run` waits with [`PollTimeout::Blocking`], where the `stop` flag
/// alone is **never** observed — a loop whose control source did not deliver would sit in
/// `epoll_wait` forever on shutdown. Test 6's 50 ms poll timeout hides exactly that, which
/// is why the wait below is 30 s: long enough that only a control source that genuinely does
/// not deliver can reach it, and it then FAILS by name instead of hanging.
#[test]
fn os_the_control_source_is_the_loops_only_way_to_learn_of_a_shutdown_it_is_waiting_on() {
    /// Long enough that reaching it means the descriptor did not deliver at all, short
    /// enough that the suite reports rather than hangs. Never the mechanism: when the
    /// control source works the wait below returns immediately, because the write already
    /// happened.
    const NO_DELIVERY: PollTimeout = PollTimeout::Millis(30_000);

    let reg = registrar(64);
    let (tx, _rx) = inbox();
    let parker = Arc::new(Parker::new());
    let (mut reactor, handle) =
        Reactor::new(reg, tx, parker as Arc<dyn ExecutorWaker>).expect("the reactor builds");

    // ---- ★ THE ZERO DIRECTION FIRST. Nothing has been signalled, so one wait must come
    // back empty. Without this the counts below could be a control source that reports
    // whether or not anybody wrote it, which would make every assertion here vacuous.
    reactor
        .run_with(PollTimeout::Immediate, 1)
        .expect("no fault");
    assert_eq!(
        (
            handle.stats().control_reports,
            handle.stats().wakes,
            handle.stats().idle_waits
        ),
        (0, 0, 1),
        "★ NON-VACUITY: an unsignalled control source must report NOTHING. stats={:?}",
        handle.stats()
    );

    // ---- ★ ONE wake, written before the wait, and therefore exactly one report.
    handle
        .wake()
        .expect("the control descriptor accepts a write");
    reactor.run_with(NO_DELIVERY, 1).expect("no fault");
    let s = handle.stats();
    assert_eq!(
        (s.control_reports, s.wakes, s.idle_waits, s.signals_pushed),
        (1, 1, 1, 0),
        "★ one write to the control descriptor ended one wait and produced exactly one \
         control report and NO signals — the wake is a barrier, not work. Reaching this with \
         idle_waits=2 means the {NO_DELIVERY:?} backstop expired, i.e. the write never became \
         readable to the loop's readiness set at all. stats={s:?}"
    );
    assert_eq!(
        (s.undrained_reports, s.stale_reports, s.retired_reports),
        (0, 0, 0),
        "the control token resolves through its own arm, never through the registrar: {s:?}"
    );

    // ---- ★ AND THE SHUTDOWN EDGE ITSELF, over the same descriptor. `shutdown` sets the
    // flag too, but the loop is INSIDE the wait when it is called here, so the flag alone
    // cannot end it — this is the production shape (`run` blocks) that test 6's poll timeout
    // conceals.
    handle.shutdown().expect("shutdown");
    reactor
        .run_with(NO_DELIVERY, 1)
        .expect("the loop exits cleanly");
    let s = handle.stats();
    assert_eq!(
        (s.control_reports, s.wakes, s.idle_waits),
        (2, 2, 1),
        "★ the shutdown TRAVELLED: it ended a wait that nothing else could have ended, and \
         it was drained rather than left readable. stats={s:?}"
    );

    // ---- ★ …and it was drained, so a further wait finds nothing. A control source left
    // readable is the level-triggered re-report `handle_token` drains to refuse — the F1
    // spin, on the loop's own descriptor. This is the ONLY assertion in the tree that can
    // see it: test 6 exits on `stop` immediately after its single control report, so a
    // bound there never gets a second wait to fire in (measured, not assumed).
    reactor
        .run_with(PollTimeout::Immediate, 1)
        .expect("no fault");
    let s = handle.stats();
    assert_eq!(
        (s.control_reports, s.idle_waits),
        (2, 2),
        "★ the control descriptor was DRAINED: the next wait came back empty instead of \
         reporting the same edge again. stats={s:?}"
    );
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
