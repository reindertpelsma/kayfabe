//! ★★★ The plan reactor, driven against **real child isolates** — and the measured
//! statement of the shape it replaces.
//!
//! Every test here spawns genuine child processes over real socketpairs, exactly as
//! `real_isolate.rs` does. The RM behind them is the loopback fixture, which is not a model
//! of RM and is not used as one: what is under test is the *transport and the lanes*.
//!
//! ## ★ The instrument for the central claim, and why it is an EDGE and not a duration
//!
//! The claim is *"one caller thread can have more than one host verb open at a time"*. A
//! duration cannot establish it — "both finished quickly" is also true of two verbs that ran
//! back to back. The fixture's `--park` knob plus its **park witness** can: the child writes
//! a byte immediately before its blocking `read`, so `HostIsolate::wait_for_park` returns only
//! when that verb is genuinely parked. Two isolates parked **at the same time**, from one
//! thread that is not blocked in either, is the claim, observed rather than timed.
//!
//! ⊘ And the negative control is the shape of the API itself: with `Worker::execute` the same
//! thread cannot even *reach* the second submission, because it is inside the first `read`.
//! `the_blocking_shape_can_only_have_one_verb_open_per_thread` states that as a measurement
//! of the current code rather than as an assertion about it.

use kayfabe_arch::ids::{GpuId, GpuVa};
use kayfabe_isolate::{CancelReason, Isolate as _, IsolateId, VerbPlan, WorkerId};
use kayfabe_isolate_host::loopback::ParkVerb;
use kayfabe_isolate_host::planreactor::{
    LanePolicy, PlanDone, PlanReactor, RejectReason, abandoned_completions_total,
};
use kayfabe_isolate_host::{HostIsolateFactory, RmMode};
use std::time::{Duration, Instant};

/// The guest VA every publish here maps at. Immaterial to what these tests are about, but it
/// must be present — `#102` made placement an argument.
const AT: GpuVa = GpuVa(0x2_0020_0000);

/// A sampling interval inside a loop whose exit condition is an edge. **Never a deadline.**
const TICK: Duration = Duration::from_millis(25);

/// A diagnostic ceiling, not a synchronisation device: it exists so a broken fixture FAILS
/// instead of hanging CI. Shortening it must not change whether a green test is green.
const CEILING: Duration = Duration::from_secs(30);

fn factory(park: ParkVerb) -> HostIsolateFactory {
    HostIsolateFactory::new(RmMode::Loopback).with_park(park)
}

fn iso(proc: u32) -> IsolateId {
    IsolateId::new(proc, GpuId(0))
}

fn publish() -> VerbPlan {
    VerbPlan::Publish {
        host_vas: None,
        len: 0x1000,
        at: AT,
    }
}

/// Collect until `want` completions have arrived, re-arming the cancels each tick for the
/// tests that need a parked verb released. Returns them in arrival order.
fn collect(r: &PlanReactor, want: usize, mut each_tick: impl FnMut()) -> Vec<PlanDone> {
    let mut out = Vec::new();
    let deadline = Instant::now() + CEILING;
    while out.len() < want {
        assert!(
            Instant::now() < deadline,
            "★ only {} of {want} completions arrived within {CEILING:?}. This bound never \
             decides a healthy run — {}",
            out.len(),
            r.census()
        );
        out.extend(r.wait(TICK));
        each_tick();
    }
    out
}

// =====================================================================================
// 1. THE MEASURED CURRENT SHAPE
// =====================================================================================

/// ★★★★★ **The shape this module replaces, measured rather than described.**
///
/// `ProxyRmBackend::call_inner` writes the request frame and then parks the calling thread in
/// `read_frame` on that worker's socket until the child answers. This times the round trip on
/// *this* box with the parent's own bracket (`isolate::ipc_totals`, thread-local, monotonic —
/// read twice and subtract), and states the structural consequence: while the thread is in
/// there, it is doing nothing else at all.
///
/// ⊘ The absolute number is a property of this machine and the loopback fixture, not of
/// hardware. The hardware numbers are in `real_isolate.rs`'s module docs (transport ≈ 29 µs,
/// RM ioctl ≈ 132 µs per round trip, `[measured w321]`); this exists so the *shape* is
/// checkable in CI and so the count is pinned: `VerbPlan::Publish` is exactly three round
/// trips, and a change that makes it four fails here.
#[test]
fn the_blocking_shape_can_only_have_one_verb_open_per_thread() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(60));
    let mut w = a.checkout().expect("a worker");

    const ROUNDS: u64 = 64;
    let (calls0, us0) = kayfabe_isolate_host::isolate::ipc_totals();
    let t0 = Instant::now();
    for _ in 0..ROUNDS {
        let off = kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb");
        w.execute(&publish(), &off).expect("the fixture serves it");
    }
    let wall = t0.elapsed();
    let (calls1, us1) = kayfabe_isolate_host::isolate::ipc_totals();
    let calls = calls1 - calls0;
    let ipc_us = us1 - us0;

    // ★ THE COUNT IS THE CHECK THAT THE TWO BRACKETS DESCRIBE THE SAME THING (w321's rule):
    // `Publish` is alloc_vaspace -> alloc_sysmem -> map_gpu_va, three `Request`s, three round
    // trips. Read from the source before it was measured.
    assert_eq!(
        calls,
        ROUNDS * 3,
        "VerbPlan::Publish is three round trips; it now issues {} per plan",
        calls as f64 / ROUNDS as f64
    );
    // Everything the thread spent is inside the blocking IPC, to within the loop's own cost.
    let share = 100.0 * ipc_us as f64 / wall.as_micros() as f64;
    eprintln!(
        "PLANREACTOR-BASELINE round_trips={calls} ipc_us={ipc_us} \
         mean_round_trip_us={:.1} wall_us={} ipc_share={share:.1}% \
         ⊘ loopback fixture on this box; hardware is 29us transport + 132us ioctl [w321]",
        ipc_us as f64 / calls as f64,
        wall.as_micros(),
    );
    assert!(
        ipc_us > 0,
        "⊘ the parent's IPC bracket read ZERO over {calls} round trips — that is an \
         unmeasured instrument, not a fast one"
    );
    assert!(
        share > 50.0,
        "the blocking round trips should dominate the caller's wall clock; got {share:.1}%"
    );
    a.checkin(w);
}

// =====================================================================================
// 2. THE CLAIM
// =====================================================================================

/// ★★★★★ **THE CLAIM, AS AN EDGE: one caller thread, TWO isolates, both parked inside a host
/// verb at the same instant.**
///
/// This is exactly the owner's *"TWO raw clients with the mean test in parallel"* reduced to
/// the property it needs. With `Worker::execute` the test thread would be inside isolate A's
/// `read` and could not submit B's plan at all; here it submits both, observes both park, and
/// is itself blocked in neither.
#[test]
fn one_thread_holds_two_isolates_parked_at_once() {
    // ★ TWO factories are not needed here (both isolates park on purpose), but the park is a
    // property of the CHILD, so this is spelled out: one factory, park armed, both children
    // park.
    let f = factory(ParkVerb::Sysmem);
    let mut a = f.spawn_host(iso(61));
    let mut b = f.spawn_host(iso(62));
    let r = PlanReactor::new().expect("notify descriptor");

    let wa = a.checkout().expect("a's worker");
    let wb = b.checkout().expect("b's worker");
    r.submit(wa, publish(), 0xA).expect("a accepted");
    r.submit(wb, publish(), 0xB).expect("b accepted");

    // THE EDGE. Neither call returns until that isolate's verb is genuinely parked, and this
    // thread reaches the second one only because it is not blocked in the first.
    a.wait_for_park(CEILING).expect("a parked");
    b.wait_for_park(CEILING).expect("b parked");

    let mid = r.stats();
    assert_eq!(
        mid.peak_in_flight,
        2,
        "two verbs were observed parked simultaneously, so the reactor must have counted \
         two in flight — {}",
        r.census()
    );
    assert_eq!(mid.in_flight, 2);
    assert_eq!(mid.lanes_spawned, 2, "one lane per isolate");
    assert_eq!(mid.completed, 0, "both are still parked");
    eprintln!("PLANREACTOR-CLAIM {}", r.census());

    // Release both the only way a parked host call can be released: a break signal each.
    let rearm = || {
        for h in [a.cancel_handle(WorkerId(0)), b.cancel_handle(WorkerId(0))]
            .into_iter()
            .flatten()
        {
            let _ = h.request(CancelReason::GuestSignal).discharge();
        }
    };
    rearm();
    let done = collect(&r, 2, rearm);

    let mut seen = Vec::new();
    for d in done {
        let failure = d
            .outcome
            .as_ref()
            .expect_err("a cancelled verb does not succeed");
        assert_eq!(failure.err, kayfabe_isolate::RmError::Interrupted);
        seen.push((d.cookie, d.isolate));
        // ★ The worker comes home. Dropping it here would wedge the slot.
        match d.isolate {
            i if i == iso(61) => a.checkin(d.worker),
            i if i == iso(62) => b.checkin(d.worker),
            other => panic!("a completion named an isolate nobody submitted to: {other:?}"),
        }
    }
    seen.sort_by_key(|(c, _)| *c);
    assert_eq!(
        seen.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        vec![0xA, 0xB],
        "both cookies come back, and they name their own isolates"
    );

    let end = r.stats();
    assert_eq!(end.completed, 2);
    assert_eq!(end.in_flight, 0);
    assert!(
        r.shutdown().is_empty(),
        "shutdown after a full drain owes nothing"
    );
}

// =====================================================================================
// 3. ORDERING
// =====================================================================================

/// ★ [`LanePolicy::PerIsolate`] keeps **submission order within one isolate**. That is the
/// ordering guarantee the single-funnel caller had, kept for the axis where nothing has
/// established that reordering is safe.
#[test]
fn plans_on_one_isolate_complete_in_submission_order() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(63));
    let r = PlanReactor::new().expect("notify descriptor");

    let pool = a.pool_size();
    assert!(pool >= 2, "this test needs a pool, got {pool}");
    let mut workers = Vec::new();
    for _ in 0..pool {
        workers.push(a.checkout().expect("an idle worker"));
    }
    for (i, w) in workers.into_iter().enumerate() {
        r.submit(w, publish(), i as u64).expect("accepted");
    }
    let done = collect(&r, pool, || {});
    assert_eq!(
        done.iter().map(|d| d.cookie).collect::<Vec<_>>(),
        (0..pool as u64).collect::<Vec<_>>(),
        "one lane per isolate means one order, and it is the submission order"
    );
    assert_eq!(r.stats().lanes_spawned, 1, "PerIsolate: one lane");
    for d in done {
        assert!(d.outcome.is_ok(), "{:?}", d.outcome);
        a.checkin(d.worker);
    }
    assert!(r.shutdown().is_empty());
}

/// ★ [`LanePolicy::PerWorker`] gives one lane per pool slot — more in flight, **and it
/// reorders within an isolate**. Exercised so the branch is taken and its cost is visible; it
/// is not the default and the module docs say why.
#[test]
fn the_per_worker_policy_gives_one_lane_per_pool_slot() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(64));
    let r = PlanReactor::with_policy(LanePolicy::PerWorker, 16).expect("notify descriptor");
    assert_eq!(r.policy(), LanePolicy::PerWorker);

    let pool = a.pool_size();
    let mut workers = Vec::new();
    for _ in 0..pool {
        workers.push(a.checkout().expect("an idle worker"));
    }
    for (i, w) in workers.into_iter().enumerate() {
        r.submit(w, publish(), i as u64).expect("accepted");
    }
    let done = collect(&r, pool, || {});
    assert_eq!(
        r.stats().lanes_spawned as usize,
        pool,
        "PerWorker: one lane per slot — {}",
        r.census()
    );
    let mut cookies: Vec<u64> = done.iter().map(|d| d.cookie).collect();
    cookies.sort_unstable();
    assert_eq!(cookies, (0..pool as u64).collect::<Vec<_>>());
    for d in done {
        a.checkin(d.worker);
    }
    assert!(r.shutdown().is_empty());
}

// =====================================================================================
// 4. REFUSALS, READINESS, AND THE R1 GATE
// =====================================================================================

/// ★ The lane cap is a **named refusal that hands the worker back**, never a silent inline
/// fallback and never a worker eaten by an error.
#[test]
fn the_lane_cap_refuses_by_name_and_returns_the_worker() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(65));
    let mut b = f.spawn_host(iso(66));
    let r = PlanReactor::with_policy(LanePolicy::PerIsolate, 1).expect("notify descriptor");

    let wa = a.checkout().expect("a's worker");
    r.submit(wa, publish(), 1).expect("the first lane fits");

    let wb = b.checkout().expect("b's worker");
    let rejected = r
        .submit(wb, publish(), 2)
        .expect_err("the second isolate needs a second lane, and the cap is 1");
    assert_eq!(rejected.why, RejectReason::LaneCapExceeded { cap: 1 });
    // ★ The worker is untouched and goes straight home.
    b.checkin(rejected.worker);
    assert_eq!(b.idle_workers(), b.pool_size(), "nothing was consumed");
    assert_eq!(r.stats().rejected, 1);

    for d in collect(&r, 1, || {}) {
        a.checkin(d.worker);
    }
    assert!(r.shutdown().is_empty());
}

/// ★ After [`PlanReactor::shutdown`] the reactor refuses by name rather than silently
/// dropping work.
#[test]
fn a_stopped_reactor_refuses_and_returns_the_worker() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(67));
    let r = PlanReactor::new().expect("notify descriptor");
    assert!(r.shutdown().is_empty());

    let w = a.checkout().expect("a worker");
    let rejected = r.submit(w, publish(), 1).expect_err("stopped");
    assert_eq!(rejected.why, RejectReason::Stopped);
    a.checkin(rejected.worker);
    assert_eq!(r.stats().rejected, 1);
}

/// ★★★ The readiness descriptor is a **real armed source**: a caller's own epoll set reports
/// it when a completion lands, and [`PlanReactor::drain`] clears it. Level-triggered, so a
/// reactor that skipped the drain would spin — which is what the second poll checks.
#[test]
fn the_readiness_descriptor_fires_on_a_completion_and_clears_on_the_drain() {
    use kayfabe_linux_raw::{PollTimeout, Poller, ReadyTokens};

    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(68));
    let r = PlanReactor::new().expect("notify descriptor");
    let poller = Poller::create().expect("epoll");
    poller.watch(r.readiness(), 0x7).expect("watch");

    let mut ready = ReadyTokens::new();
    let n = poller
        .wait(&mut ready, PollTimeout::Millis(0))
        .expect("poll");
    assert_eq!(n, 0, "⊘ nothing has completed yet, so nothing may be ready");

    let w = a.checkout().expect("a worker");
    r.submit(w, publish(), 0x33).expect("accepted");

    let deadline = Instant::now() + CEILING;
    loop {
        assert!(Instant::now() < deadline, "the readiness never fired");
        let n = poller
            .wait(&mut ready, PollTimeout::Millis(25))
            .expect("poll");
        if n > 0 {
            break;
        }
    }
    assert_eq!(ready.iter().collect::<Vec<_>>(), vec![0x7]);

    let done = r.drain();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].cookie, 0x33);
    for d in done {
        a.checkin(d.worker);
    }

    let n = poller
        .wait(&mut ready, PollTimeout::Millis(0))
        .expect("poll");
    assert_eq!(
        n, 0,
        "the drain must clear the counter, or a level-triggered caller spins"
    );
    poller.unwatch(r.readiness()).expect("unwatch");
    assert!(r.shutdown().is_empty());
}

/// ★ R1 is asserted at this door too. A submission hands a worker to a thread that is about
/// to issue a host verb; doing that with a ranked lock held is the same violation the verb
/// itself refuses, one call earlier.
#[test]
fn submitting_under_a_ranked_lock_panics_naming_r1() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(69));
    let r = PlanReactor::new().expect("notify descriptor");
    let w = a.checkout().expect("a worker");

    kayfabe_util::lockwitness::note_acquired(1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = r.submit(w, publish(), 1);
    }));
    kayfabe_util::lockwitness::note_released(1);

    let payload = caught.expect_err("R1 must fire");
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(msg.contains("R1"), "the panic must name R1, got: {msg}");
}

/// ⚠ **The leak has a number.** A reactor dropped with completions still queued loses those
/// workers and wedges their pool slots; this takes that branch on purpose and reads the
/// counter, so the silent version cannot come back.
#[test]
fn dropping_a_reactor_with_uncollected_completions_counts_the_leak() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(70));
    let before = abandoned_completions_total();
    {
        let r = PlanReactor::new().expect("notify descriptor");
        let w = a.checkout().expect("a worker");
        r.submit(w, publish(), 1).expect("accepted");
        // Wait for the completion to be QUEUED (not drained), then drop without collecting.
        let deadline = Instant::now() + CEILING;
        while r.stats().completed == 0 {
            assert!(Instant::now() < deadline, "the plan never completed");
            std::thread::yield_now();
        }
    }
    assert_eq!(
        abandoned_completions_total(),
        before + 1,
        "the abandoned completion must be counted, not vanish"
    );
    // ⊘ And the slot really is wedged, which is the fact the counter stands for.
    assert_eq!(
        a.idle_workers(),
        a.pool_size() - 1,
        "the abandoned worker never came back — that is what `abandoned` means"
    );
}

/// ★★★★★ **ONE ISOLATE ACCEPTS TWO VERBS AT ONCE — AND RM's CLIENT LOCK, NOT THE TRANSPORT,
/// IS WHAT SERIALISES THEM.**
///
/// Owner, 2026-09-13: *"one isolate must be able to allow multiple things in flight by
/// construction with threads."*
///
/// # ⊘⊘⊘ I FIRST WROTE THIS TEST TO ASSERT TWO PARKS, AND IT HUNG. THE HANG IS THE FINDING.
///
/// The transport genuinely allows it: `proto.rs` rules *"concurrency comes from channel COUNT,
/// never from multiplexing one channel"*, so each of the four pool workers owns its own
/// `UnixStream` and each channel is 1-deep — four sockets, four possible in-flight verbs, no
/// demux and no pending list. That half of the claim is real.
///
/// ⚠ **But the CHILD serialises them, and on purpose.** `loopback.rs`'s `verb()` takes
/// `self.shared.client.lock()` and holds it **across** the park — its own comment: *"The lock is
/// held across the park, which is the RM semantic being modelled."* So a second verb on one
/// isolate blocks on that lock **before** it can announce a park, and a test waiting for two park
/// witnesses waits forever.
///
/// ★ That is not a fixture artefact. It is the fixture being faithful to what was measured on
/// real hardware: RM holds the device-global API lock in WRITE across the GSP RPC, which is why
/// `[measured R12, real GA106, 800 alloc+free pairs]` 1 worker, 1 isolate × 4 workers and
/// 4 isolates × 1 worker all came in at **1.00x**.
///
/// ⇒ So the property worth pinning is the one that is actually ours: **the reactor accepts and
/// holds two verbs open on ONE isolate** — two lanes, two in flight, the submitting thread
/// blocked in neither. What happens beyond that is RM's lock, not our transport, and no amount
/// of epoll changes it.
///
/// ⊘ This also settles a claim I had wrong: **we do NOT need pv's `txn_id` for multi-inflight.**
/// pv reaches the same place from the other side — *"a dedicated reader thread that multiplexes
/// IOCTL responses by `txn_id` onto per-caller condvars"* (`nvkvm_isolate.c:1-8`) — one socket
/// demuxed versus four sockets undemuxed. Note pv serialises too, deliberately: its non-IOCTL
/// commands go through `sync_cmd_lock`, *"a real one-at-a-time gate"*.
#[test]
fn one_isolate_accepts_two_verbs_and_rm_s_client_lock_serialises_them() {
    let f = factory(ParkVerb::Sysmem);
    let mut a = f.spawn_host(iso(71));
    // ⊘ PerWorker, not the PerIsolate default: a lane PER POOL SLOT is what lets one isolate's
    // slots be occupied concurrently. Under PerIsolate the second plan would queue behind the
    // first on that isolate's single lane — correct for ordering, wrong policy for this claim.
    let r = PlanReactor::with_policy(
        LanePolicy::PerWorker,
        kayfabe_isolate_host::planreactor::DEFAULT_LANE_CAP,
    )
    .expect("notify descriptor");

    let w0 = a.checkout().expect("slot 0");
    let w1 = a.checkout().expect("slot 1 — a second worker on the SAME isolate");
    assert_ne!(
        w0.id(),
        w1.id(),
        "checkout handed the same pool slot twice; this test would prove nothing"
    );
    r.submit(w0, publish(), 0x10).expect("first accepted");
    r.submit(w1, publish(), 0x11).expect("second accepted");

    // ★ ONE park witness, not two. The first verb reaches the parked call and announces; the
    // second is inside the child, blocked on the client lock, which is exactly the modelled RM
    // semantic. ⚠ Waiting for a second byte here is what hung — kept as a comment because the
    // next author will be tempted by it.
    a.wait_for_park(CEILING).expect("the first verb parked");

    let mid = r.stats();
    assert_eq!(
        mid.lanes_spawned, 2,
        "PerWorker must give one lane per pool slot, not one per isolate — {}",
        r.census()
    );
    assert_eq!(
        mid.in_flight, 2,
        "both verbs are open on ONE isolate: one parked in the host call, one inside the child          waiting on RM's client lock. The submitting thread is blocked in neither — with          `Worker::execute` it could not have submitted the second at all — {}",
        r.census()
    );
    assert_eq!(mid.completed, 0, "neither has returned");
    eprintln!("PLANREACTOR-ONE-ISOLATE {}", r.census());

    // Release both: one break signal per OUTSTANDING worker slot, not one per isolate.
    let rearm = || {
        for id in [WorkerId(0), WorkerId(1)] {
            if let Some(h) = a.cancel_handle(id) {
                let _ = h.request(CancelReason::GuestSignal).discharge();
            }
        }
    };
    rearm();
    let done = collect(&r, 2, rearm);
    for d in done {
        let failure = d
            .outcome
            .as_ref()
            .expect_err("a cancelled verb does not succeed");
        assert_eq!(failure.err, kayfabe_isolate::RmError::Interrupted);
    }
}
