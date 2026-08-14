//! ★★★ The real isolate, driven **through the existing ports**.
//!
//! Every test here spawns a genuine child process, hands it descriptors, and drives it with
//! `kayfabe_isolate::{IsolateFactory, Isolate, Worker, VerbPlan}` — the same types the whole
//! suite has been driving against `kayfabe_mocks` since L1-M1. Nothing here reaches past the
//! port to poke the implementation.
//!
//! ## The requirement these exist to settle, and ★★ WHAT THEY DO AND DO NOT PROVE
//!
//! > *"multiple ioctls must execute IN PARALLEL WITHOUT HANGING"*
//!
//! The two concurrency tests below run against the **loopback fixture**, whose client lock
//! models RM's per-client serialisation in its *strongest* form: a sibling cannot even
//! enter. They therefore prove **the design survives that constraint** — no deadlock, no
//! lost work, a queued sibling completes when the holder is released.
//!
//! They do **not** prove the constraint has that shape on real hardware, and it does not.
//! Measured on RTX 3060 / open 580.159.04 (`kayfabe-rm-ladder --concurrency`, R12), 800
//! `alloc_vaspace` + `free` pairs:
//!
//! | configuration | wall time | speedup |
//! |---|---|---|
//! | 1 worker, sequential | 1610 ms | 1.00x (baseline) |
//! | 1 isolate, 4 workers (ONE RM client) | 1602 ms | **1.00x** |
//! | 4 isolates, 1 worker each (FOUR RM clients, four processes) | 1610 ms | **1.00x** |
//!
//! ★★★ **Neither the pool nor separate clients buys any alloc/free throughput.** The
//! bottleneck is **device-global**, not per-client: RM takes the global API lock in WRITE
//! for every alloc/free and holds it across the GSP RPC — which
//! `kayfabe_isolate::DEFAULT_POOL_WORKERS`' own docs already cite
//! (`ogkm-610:`/`ogkm-580: .../rmapi/rmapi.c:53-58`, `:535`;
//! `ogkm-610: .../rmapi/alloc_free.c:1714-1718`, `ogkm-580: :1692-1696`).
//!
//! So the guidance that parallelism "must come from multiple clients" is false for this
//! verb class, and so is the belief that the pool provides it. Both were reasonable; the
//! hardware says the answer is neither. The measurement is deliberately kept as a *program*
//! (`--concurrency`) rather than a test, because it needs a GPU and a wall clock, and a
//! timing assertion in CI is a flake with a justification.
//!
//! ## No sleeps
//!
//! Progress is asserted as an **edge**, never as a duration: a channel that must stay empty
//! while a park is held and must deliver once it is released. `recv_timeout` appears only as
//! a *sampling* interval inside a loop whose exit condition is the edge itself, so a slow
//! machine makes the test slower and never makes it lie.

use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa};
use kayfabe_isolate::{
    CancelReason, HostHandle, Isolate as _, IsolateFactory, IsolateId, RingWorkingSet, RmError,
    VerbPlan, VerbReply, Worker,
};
use kayfabe_isolate_host::loopback::ParkVerb;
use kayfabe_isolate_host::{HostIsolateFactory, RmMode};
use std::sync::mpsc;
use std::time::Duration;

/// The isolate binary this crate builds **for the host triple**. `CARGO_BIN_EXE_*` is
/// cargo's own answer, so the test cannot drift from the binary it is testing.
///
/// ★ This is NOT what a factory spawns. The factory spawns the **embedded** static musl
/// image (`build.rs`, `isolate::embedded_isolate_bytes`); this path exists only for the two
/// tests below that need a program to run *by hand* — one to check the isolate's refusal to
/// start standalone, one as the decoy that proves an environment variable can no longer
/// redirect what gets executed.
const ISOLATE: &str = env!("CARGO_BIN_EXE_kayfabe-isolate");

/// ★ #102 — the guest VA every publish here maps AT. Address identity made placement an
/// argument, so these plans carry one; the specific value is immaterial to what each test
/// is about (all of them allocate a fresh host VAS), but it must be *present*, because a
/// publish with no address is no longer expressible.
const AT: GpuVa = GpuVa(0x2_0020_0000);

/// How long one sampling tick waits before re-checking a progress edge. Never a deadline:
/// see the module docs.
const TICK: Duration = Duration::from_millis(25);

fn factory(park: ParkVerb) -> HostIsolateFactory {
    HostIsolateFactory::new(RmMode::Loopback).with_park(park)
}

fn iso(proc: u32) -> IsolateId {
    IsolateId::new(proc, GpuId(0))
}

/// The `#14` ring gate's address plane, for the one test that rings. Says "yes" to
/// everything, because what is under test here is the transport, not the gate — and saying
/// so out loud is the point: a test that quietly implemented a permissive plane would be the
/// *commission* `VerbPlan::gated_doorbell`'s docs describe.
struct EverythingPublished;
impl RingWorkingSet for EverythingPublished {
    fn is_host_published(&self, _va: GpuVa) -> bool {
        true
    }
}

// =====================================================================================
// The plain path
// =====================================================================================

#[test]
fn a_real_isolate_serves_a_verb_chain_through_the_port() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(1));
    assert!(
        !isolate.is_retired(),
        "spawn failed — the isolate image is embedded, so this is not a path problem"
    );
    assert_eq!(isolate.pool_size(), kayfabe_isolate::DEFAULT_POOL_WORKERS);
    assert_eq!(isolate.idle_workers(), isolate.pool_size());

    let mut w = isolate.checkout().expect("a worker");
    assert_eq!(isolate.in_flight(), 1);
    assert!(!isolate.is_quiesced(), "a checked-out worker is in flight");

    let reply = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect("the child served it");
    match reply {
        VerbReply::Published {
            host_vas: Some(vas),
            memory,
            host_va,
        } => {
            // ★ Every handle carries OUR namespace, stamped by the parent. The child never
            // sent one.
            assert_eq!(vas.isolate(), iso(1));
            assert_eq!(memory.isolate(), iso(1));
            assert_ne!(vas, memory);
            assert_ne!(host_va, 0);
        }
        other => panic!("expected a fresh publication, got {other:?}"),
    }

    isolate.checkin(w);
    assert_eq!(isolate.in_flight(), 0);
    assert!(isolate.is_quiesced());
}

#[test]
fn every_verb_shape_survives_the_round_trip() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(2));
    let mut w = isolate.checkout().expect("worker");

    // A doorbell chain, built through the ONE constructor that runs the #14 gate.
    let plan = VerbPlan::gated_doorbell(
        &EverythingPublished,
        &[GpuVa(0x1000)],
        None,
        None,
        EngineKind::Ce,
        true,
        // ⊘ `None`: this fixture has no guest and therefore no guest-RAM grant to mint. A
        // channel born here carries `hObjectError = 0`, which is the pre-w288 shape.
        None,
    )
    .expect("the gate passes an all-published working set");
    match w.execute(&plan, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")).expect("doorbell") {
        VerbReply::Doorbell {
            host_vas: Some(_),
            channel: Some((chan, token)),
            scheduled: true,
        } => {
            assert_eq!(chan.isolate(), iso(2));
            assert_ne!(token, 0);
        }
        other => panic!("expected a doorbell reply, got {other:?}"),
    }

    // A control, payload in and out by value.
    let obj = w
        .with_rm(&kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"), |rm| rm.alloc_vaspace())
        .expect("a control target");
    let plan = VerbPlan::Control {
        obj,
        cmd: kayfabe_isolate::ControlCmd(0x801813),
        payload: vec![0xAB; 32],
    };
    match w.execute(&plan, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")).expect("control") {
        VerbReply::Control { payload } => assert_eq!(payload, vec![0xAB; 32]),
        other => panic!("expected a control reply, got {other:?}"),
    }

    // And the disposal path.
    assert_eq!(
        w.execute(&VerbPlan::Release {
            unmap: Vec::new(),
            free: vec![obj],
            guest_ram: Vec::new(),
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")),
        Ok(VerbReply::Released)
    );
    isolate.checkin(w);
}

/// A handle the child never minted comes back as the exact variant, carrying the value as
/// **presented** — not as a transport failure.
#[test]
fn an_unknown_handle_is_a_bad_handle_and_not_a_wedge() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(3));
    let mut w = isolate.checkout().expect("worker");
    let bogus = HostHandle::new(iso(3), 0xDEAD_BEEF);
    let failure = w
        .execute(&VerbPlan::Release {
            unmap: Vec::new(),
            free: vec![bogus],
            guest_ram: Vec::new(),
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect_err("an unknown handle must be refused");
    assert_eq!(failure.err, RmError::BadHandle(bogus));
    // The disposal path reports what it could not dispose of, rather than swallowing it.
    assert_eq!(failure.orphans.free, vec![bogus]);
    isolate.checkin(w);
}

/// ★★ The foreign-handle gate, against two REAL isolates whose handles genuinely collide.
///
/// This is the case `07da582` found and that `HostHandle`'s own docs say a real host does
/// **not** catch: both children mint from the same base, so isolate 1's first handle and
/// isolate 2's first handle have the identical raw value and name different live objects.
/// Nothing downstream would fault. The gate is the only thing that refuses it.
#[test]
fn two_real_isolates_mint_colliding_values_and_the_gate_is_what_refuses_them() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn(iso(10));
    let mut b = f.spawn(iso(11));
    let mut wa = a.checkout().expect("worker a");
    let mut wb = b.checkout().expect("worker b");

    let ha = wa.with_rm(&kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"), |rm| rm.alloc_vaspace()).expect("a's vas");
    let hb = wb.with_rm(&kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"), |rm| rm.alloc_vaspace()).expect("b's vas");

    assert_eq!(
        ha.raw(),
        hb.raw(),
        "two real isolates must mint the SAME raw value — that is the hazard"
    );
    assert_ne!(
        ha, hb,
        "…and the recorded namespace is what tells them apart"
    );

    // Present a's handle on b's connection through the port. Refused before any verb runs.
    let failure = wb
        .execute(&VerbPlan::Publish {
            host_vas: Some(ha),
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .expect_err("a foreign handle must be refused");
    assert_eq!(
        failure.err,
        RmError::ForeignHandle {
            handle: ha,
            worker_isolate: iso(11),
        }
    );
    assert!(
        failure.orphans.is_empty(),
        "the gate runs first, so nothing was allocated"
    );

    a.checkin(wa);
    b.checkin(wb);
}

// =====================================================================================
// ★★★ The parallelism question
// =====================================================================================

/// What a verb thread hands back: the worker (so the caller can check it in and read
/// `cancel_observed`) and the outcome.
type VerbOutcome = (Worker, Result<VerbReply, kayfabe_isolate::VerbFailure>);

/// Run `plan` on `isolate` in its own thread, reporting the outcome on a channel.
fn spawn_verb(
    mut worker: Worker,
    plan: VerbPlan,
) -> (mpsc::Receiver<VerbOutcome>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let h = std::thread::spawn(move || {
        let r = worker.execute(&plan, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"));
        let _ = tx.send((worker, r));
    });
    (rx, h)
}

/// ★★★ **The owner's requirement, settled by measurement.**
///
/// Two isolates — two children, two RM clients — run verbs at the same time. One of them is
/// parked *forever* inside a verb; the other completes a full three-verb chain while it is
/// parked. Parallelism comes from having more clients, and it does not hang.
#[test]
fn parallelism_comes_from_isolates_and_a_parked_client_does_not_stall_its_peers() {
    // ★ TWO factories, and that is the instrument. The park is a property of the CHILD, so
    // one factory would park BOTH isolates and the peer would have nothing to prove — the
    // first version of this test did exactly that and hung, which is the honest way to
    // discover that a fixture knob is per-process.
    let parking = factory(ParkVerb::Sysmem);
    let plain = factory(ParkVerb::Nothing);
    let mut a = parking.spawn(iso(20));
    let mut b = plain.spawn(iso(21));

    let wa = a.checkout().expect("a's worker");
    let (parked_rx, ta) = spawn_verb(
        wa,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        },
    );

    // While A is parked, B completes the identical chain — repeatedly, so this is not one
    // lucky interleaving.
    let mut wb = b.checkout().expect("b's worker");
    for round in 0..8 {
        let reply = wb.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"));
        assert!(
            reply.is_ok(),
            "round {round}: a peer isolate stalled behind a parked one: {reply:?}"
        );
        assert!(
            parked_rx.try_recv().is_err(),
            "round {round}: the parked verb was supposed to still be parked — the fixture \
             is not producing the hazard"
        );
    }
    b.checkin(wb);

    // Release A the only way a parked host call can be released: a break signal.
    let handle = a
        .cancel_handle(kayfabe_isolate::WorkerId(0))
        .expect("armed");
    assert!(handle.request(CancelReason::GuestSignal).discharge());
    let (wa, result) = loop {
        match parked_rx.recv_timeout(TICK) {
            Ok(v) => break v,
            Err(_) => {
                assert!(
                    a.cancel_handle(kayfabe_isolate::WorkerId(0))
                        .expect("still armed")
                        .request(CancelReason::GuestSignal)
                        .discharge()
                );
            }
        }
    };
    let failure = result.expect_err("a cancelled verb does not succeed");
    assert_eq!(failure.err, RmError::Interrupted);
    // ★ §7.3: the fault names the TRUTH. The reason travels all the way back.
    assert_eq!(wa.cancel_observed(), Some(CancelReason::GuestSignal));
    a.checkin(wa);
    ta.join().expect("join");
}

/// ★★★ The other half: **under the strongest form of per-client serialisation, N pool
/// workers of ONE isolate do not get N verbs in flight — and nothing deadlocks.**
///
/// ★ Read the module docs before reading this as a statement about RM. The fixture's lock
/// blocks a sibling from *entering*; real RM accepts the sibling's ioctl and queues it
/// inside the kernel, so from our side both verbs really are in flight. What the hardware
/// measurement adds is that neither arrangement completes any *faster* (R12), because the
/// binding constraint on alloc/free is the device-global API lock rather than the
/// per-client one. This test's value is that the design is correct under the stricter
/// model too: the sibling is queued, not lost, and releasing the holder releases it.
///
/// Worker 0 parks. Worker 1 of the *same* isolate then issues a completely different verb —
/// `alloc_vaspace`, which does not park — and makes **no progress**, because RM's
/// serialisation is per *client* and the whole pool shares one. Meanwhile a peer isolate
/// keeps completing, which is what makes this an assertion about the client rather than
/// about the fixture being stuck.
#[test]
fn the_pool_does_not_buy_wire_concurrency_on_one_client() {
    // ★★ THE ANTECEDENT HAS TO BE ESTABLISHED, and there is no signal for it.
    //
    // The claim is conditional: *while worker 0 holds the client lock, worker 1 cannot
    // proceed*. Establishing "worker 0 holds it" is the hard part, because a parked verb by
    // definition sends nothing — so there is no edge to wait on. Worker 1's request can
    // legitimately reach the child first, complete, and that is **not** a violation.
    //
    // So the experiment is repeated, and the property is asserted only in the arm where the
    // interleaving we need actually occurred. A run the sibling wins is discarded; a run
    // where it wins EVERY time is a loud failure, which is what the attempt bound is for.
    // The first version of this test asserted unconditionally and flaked 1 run in 3 — with
    // the code correct.
    const ATTEMPTS: usize = 12;
    let plain = factory(ParkVerb::Nothing);
    let mut peer = plain.spawn(iso(31));
    let mut wp = peer.checkout().expect("peer worker");

    for _attempt in 0..ATTEMPTS {
        let parking = factory(ParkVerb::Sysmem);
        let mut a = parking.spawn(iso(30));

        let w0 = a.checkout().expect("worker 0");
        assert_eq!(w0.id(), kayfabe_isolate::WorkerId(0));
        let (parked_rx, t0) = spawn_verb(
            w0,
            VerbPlan::Publish {
                host_vas: None,
                len: 0x1000,
                at: AT,
            },
        );

        let w1 = a.checkout().expect("worker 1");
        assert_eq!(w1.id(), kayfabe_isolate::WorkerId(1));
        // ★ The sibling's plan contains NO parking verb — vaspace, channel, engine object.
        // So its silence below can only be the CLIENT lock, never its own park. Choosing
        // `Publish` here would have made the assertion unfalsifiable.
        let (sibling_rx, t1) = spawn_verb(
            w1,
            VerbPlan::EngineObject {
                host_vas: None,
                channel: None,
                engine: EngineKind::GrCompute,
                class: kayfabe_arch::ids::ClassId(0xc7c0),
                params: Vec::new(),
                // ⊘ `None`: this test is about worker parking, not about leg A2, and a
                // channel born over an adopted ring would exercise a different alloc path.
                adopt: None,
                // ⊘ `None` for the same reason, one field over: a notifier would exercise
                // the guest-RAM plane, which this fixture does not arm.
                err_notifier: None,
            },
        );

        // Sample. The peer isolate is the liveness control: if IT stops making progress the
        // fixture is simply wedged and the sibling's silence proves nothing.
        let mut sibling_won = false;
        for round in 0..8 {
            assert!(
                wp.execute(&VerbPlan::Publish {
                    host_vas: None,
                    len: 0x1000,
                    at: AT,
                }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
                .is_ok(),
                "round {round}: the peer isolate must keep running"
            );
            assert!(parked_rx.try_recv().is_err(), "round {round}: still parked");
            if !matches!(sibling_rx.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                sibling_won = true;
                break;
            }
            std::thread::sleep(TICK);
        }

        if sibling_won {
            // The sibling's request reached the child before worker 0 took the lock. Not a
            // violation; discard and retry. Dropping the isolate kills the child, which
            // releases the parked requester.
            drop(a);
            let _ = t0.join();
            let _ = t1.join();
            continue;
        }

        // ★★★ THE FINDING, asserted: eight sampling rounds in which a peer ISOLATE
        // completed a full three-verb chain every time, and a sibling WORKER of the parked
        // client completed nothing. Parallelism comes from clients, not from pool slots.
        //
        // Releasing worker 0 releases worker 1 — the sibling was queued, not lost.
        loop {
            assert!(
                a.cancel_handle(kayfabe_isolate::WorkerId(0))
                    .expect("armed")
                    .request(CancelReason::Watchdog)
                    .discharge()
            );
            if let Ok((w, r)) = parked_rx.recv_timeout(TICK) {
                assert_eq!(r.expect_err("cancelled").err, RmError::Interrupted);
                a.checkin(w);
                break;
            }
        }
        let (w1, r1) = sibling_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the sibling completes once the client lock is free");
        assert!(r1.is_ok(), "the sibling's own verb was fine: {r1:?}");
        a.checkin(w1);
        t0.join().expect("join 0");
        t1.join().expect("join 1");
        peer.checkin(wp);
        return;
    }
    panic!(
        "in {ATTEMPTS} attempts the sibling worker was NEVER observed blocked behind its \
         own client's parked verb. Either the client lock stopped being per-client, or the \
         fixture stopped parking — both are findings, neither is a flake."
    );
}

// =====================================================================================
// Cancellation and the wedge escape
// =====================================================================================

/// A cancel naming a **stale** txn is dropped: §7.3's fourth row, *"the verb finished
/// first"*. Not an error, and — critically — it must not land on the next operation.
#[test]
fn a_cancel_for_a_finished_transaction_is_dropped() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(40));

    let mut w = isolate.checkout().expect("worker");
    let stale = isolate
        .cancel_handle(kayfabe_isolate::WorkerId(0))
        .expect("armed while checked out");
    assert!(
        w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .is_ok()
    );
    isolate.checkin(w);

    assert!(
        !stale.request(CancelReason::ProcExit).discharge(),
        "a cancel for a completed txn must report that it was stale"
    );

    // …and the NEXT checkout is unharmed, which is the property the txn id exists for.
    let mut w = isolate.checkout().expect("worker again");
    assert!(
        w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .is_ok(),
        "a stale cancel landed on an innocent later operation"
    );
    assert_eq!(w.cancel_observed(), None);
    isolate.checkin(w);
}

/// ★ An isolate with **no park armed** must REFUSE the wait, not block forever.
///
/// ⊘ This is the refusal, and without a test for it the obvious wrong implementation —
/// return `Ok(())` when there is no witness — passes everything else in this file, because
/// every other caller *does* arm a park. A vacuous `Ok` would then silently restore the exact
/// defect this whole mechanism replaced: the caller proceeds without the park having happened.
#[test]
fn waiting_for_a_park_that_was_never_armed_is_refused_not_awaited() {
    let f = factory(ParkVerb::Nothing);
    let mut a = f.spawn_host(iso(51));
    let e = a
        .wait_for_park(Duration::from_secs(30))
        .expect_err("an isolate with nothing parked must refuse, not wait");
    assert!(
        e.contains("without a park armed"),
        "the refusal must say WHY nothing will ever arrive, so a caller is not left \
         wondering whether it simply lost a race: {e}"
    );
}

/// ★★ §7.5's escape, end to end against a real process: the requester is **released
/// without a reply**, and the verb it was waiting for is still parked in the child.
#[test]
fn abandon_releases_a_wedged_requester_with_wedged() {
    let f = factory(ParkVerb::Sysmem);
    // ★ `spawn_host`, not `spawn` — the concrete type, because `wait_for_park` is deliberately
    // NOT on the `Isolate` trait. The park witness is test scaffolding, and putting it on the
    // production interface would make every implementation owe an answer to a question only a
    // fixture can ask.
    let mut a = f.spawn_host(iso(50));

    let w = a.checkout().expect("worker");
    let (rx, t) = spawn_verb(
        w,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        },
    );
    // ★★★ WAIT FOR THE PARK ITSELF, not for a duration.
    //
    // This line used to be `assert!(rx.recv_timeout(TICK).is_err())` — a 25 ms bet that the
    // chain had got past `alloc_vaspace` and into the parked `alloc_sysmem`. But "no reply
    // arrived in 25 ms" is a strictly weaker fact than "the verb is parked": it is also true
    // when the chain has not started. When the bet lost, `abandon` landed first, the chain
    // unwound with ZERO intermediates, and the `orphans.free.len() == 1` assertion below
    // failed — ~0.5 % of runs, and **20/20** with the duration shortened to zero.
    //
    // The child now announces the park immediately before its blocking read, so on return
    // `alloc_vaspace` has provably completed and exactly one intermediate exists. ⊘ The old
    // form must not come back: it makes this test's subject a scheduling outcome.
    a.wait_for_park(Duration::from_secs(30))
        .expect("the chain parks in alloc_sysmem");
    assert!(
        rx.try_recv().is_err(),
        "a parked verb has not replied — if this fires, the park is not where the test thinks"
    );

    let request = a
        .abandon(kayfabe_isolate::WorkerId(0))
        .expect("a checked-out slot can be abandoned");
    assert!(request.is_abandon());
    assert!(request.discharge());
    // In the SAME act, per §7.5 — the escape is safe only because the slot is retired, so
    // no future reader of that channel exists.
    assert!(a.worker_died(kayfabe_isolate::WorkerId(0)));

    let (w, result) = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the abandon releases the requester");
    let failure = result.expect_err("an abandoned requester does not succeed");
    assert_eq!(failure.err, RmError::Wedged);
    // ★★ THE ASSERTION THIS TEST WAS WRITTEN WITH WAS WRONG, and the real isolate is what
    // corrected it. The first version asserted `orphans.is_empty()`, reasoning that the
    // parent never learned the intermediate handle. It does: `Publish` allocates the host
    // VAS **first**, that verb SUCCEEDS and its handle comes back, and only then does the
    // chain park in `alloc_sysmem`. So the wedge leaves exactly one named object behind.
    //
    // Which is §7.5/G4 working precisely as written — *"a wedged worker cannot issue a
    // `free`: it is still inside the host ioctl that wedged it. So the chain's
    // intermediates come out UNTOUCHED in `VerbFailure::orphans`… the caller must STAGE
    // them"*. The corrected assertion pins that contract instead of denying it.
    assert_eq!(
        failure.orphans.free.len(),
        1,
        "the wedge must hand back exactly the host VAS the chain had already allocated: {:?}",
        failure.orphans
    );
    assert_eq!(failure.orphans.free[0].isolate(), iso(50));
    assert!(
        failure.orphans.unmap.is_empty(),
        "nothing had been mapped yet"
    );
    // ★ And they are UNTOUCHED, not disposed of: the disposition of record is §7.0's
    // process boundary, which the drop at the end of this test exercises.
    a.checkin(w);

    // The dead slot is never resurrected, and the isolate reports itself quiesced so the
    // reap can proceed.
    assert!(a.checkout().is_some(), "the other slots still work");
    t.join().expect("join");
}

/// A retired isolate refuses **new** checkouts — backpressure, not failure.
#[test]
fn a_retired_isolate_refuses_new_checkouts() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(60));
    let w = isolate.checkout().expect("worker");
    isolate.retire();
    assert!(isolate.is_retired());
    assert!(isolate.checkout().is_none());
    // …and the worker that was already out still completes.
    let mut w = w;
    assert!(
        w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
        .is_ok()
    );
    isolate.checkin(w);
    assert!(isolate.is_quiesced());
}

/// ★ `request_cancel_all` reaches every checked-out worker — §7.6 T2's *"for every
/// checked-out worker, `request_cancel(ProcExit)`"*.
#[test]
fn retiring_a_proc_cancels_every_checked_out_worker() {
    let f = factory(ParkVerb::Sysmem);
    let mut a = f.spawn(iso(70));

    let mut parked = Vec::new();
    for _ in 0..2 {
        let w = a.checkout().expect("worker");
        parked.push(spawn_verb(
            w,
            VerbPlan::Publish {
                host_vas: None,
                len: 0x1000,
                at: AT,
            },
        ));
    }
    assert_eq!(a.checked_out().len(), 2);

    // Latch, then discharge with no lock held — the two-step §7.1 requires.
    //
    // ★ THE INSTRUMENT, corrected: the first version polled every channel with `try_recv`
    // to count how many had landed. `try_recv` **consumes** the message, so the test ate
    // its own results and then failed with `Disconnected` on the real read. A progress
    // probe that destroys the progress it is probing for is not a probe.
    for (rx, t) in parked {
        let (w, r) = loop {
            let cancels = a.request_cancel_all(CancelReason::ProcExit);
            assert_eq!(
                cancels.requests().len(),
                a.checked_out().len(),
                "every checked-out worker must be reachable (§7.6 T2)"
            );
            cancels.discharge_all();
            if let Ok(v) = rx.recv_timeout(TICK) {
                break v;
            }
        };
        assert_eq!(r.expect_err("cancelled").err, RmError::Interrupted);
        assert_eq!(w.cancel_observed(), Some(CancelReason::ProcExit));
        a.checkin(w);
        t.join().expect("join");
    }
}

// =====================================================================================
// Process hygiene
// =====================================================================================

/// ★ §7.0: the isolate process boundary IS the garbage collector. Dropping the isolate must
/// leave no zombie and no orphaned child.
#[test]
fn dropping_an_isolate_mid_verb_does_not_hang() {
    let f = factory(ParkVerb::Sysmem);
    let mut isolate = f.spawn(iso(80));
    // Put a verb in flight that will NEVER come back, then drop everything. §7.0: the
    // process boundary is the collector, and it must not need the verb's cooperation.
    let w = isolate.checkout().expect("worker");
    let (_rx, t) = spawn_verb(
        w,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        },
    );
    drop(isolate);
    // The requester is released by the child's death (its sockets closed), and the thread
    // joins. If the drop needed the parked verb to finish, this would hang forever.
    t.join()
        .expect("the requester was released by the isolate's death");
}

/// The isolate binary refuses to run without the descriptors only its parent can grant, and
/// says so rather than dying on the first read.
#[test]
fn the_isolate_binary_refuses_to_run_standalone() {
    let out = std::process::Command::new(ISOLATE)
        .output()
        .expect("the binary exists");
    assert!(!out.status.success(), "it must not pretend to work");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--workers is required"),
        "it must name what is missing, got: {stderr}"
    );
}

/// A verb issued with a ranked lock held panics naming R1 — asserted against the REAL
/// backend, so the assert is not a property of the mock.
#[test]
fn a_verb_under_a_ranked_lock_panics_naming_r1() {
    let f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(90));
    let mut w = isolate.checkout().expect("worker");
    kayfabe_util::lockwitness::note_acquired(1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: AT,
        }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"));
    }));
    kayfabe_util::lockwitness::note_released(1);
    let payload = caught.expect_err("R1 must fire");
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(msg.contains("R1"), "the panic must name R1, got: {msg}");
    isolate.checkin(w);
}

/// ★★★ The **isolate itself** — not a `/bin/sh` fixture — is born in its own user, pid,
/// mount, network, IPC and UTS namespaces.
///
/// This is the gate over the production call site. `ChildSpec::in_new_namespaces()` is one
/// line in `build_isolate`, and without this test deleting it would turn **nothing** red:
/// `kayfabe_linux_raw`'s own namespace test drives `/bin/sh`, and the sandbox prober is
/// spawned by the test that measures it, not by the factory.
///
/// ★ Measured from the **parent**, by inode identity (`readlink /proc/<pid>/ns/<kind>`).
/// Nothing inside the child can answer it: with `CLONE_NEWPID` its own `getpid()` is 1
/// whatever happens, and a number is not a namespace.
#[test]
fn the_isolate_child_is_born_in_its_own_namespaces() {
    kayfabe_linux_raw::require_user_namespace!("the_isolate_child_is_born_in_its_own_namespaces");
    let f = HostIsolateFactory::new(RmMode::Loopback);
    let isolate = f.spawn_host(iso(55));
    assert!(
        !isolate.is_retired(),
        "the isolate did not start: {:?}",
        isolate.spawn_error()
    );
    let pid = isolate.child_pid().expect("a live child has a pid");

    let link = |path: String| -> String {
        std::fs::read_link(&path)
            .unwrap_or_else(|e| panic!("readlink {path}: {e}"))
            .to_string_lossy()
            .into_owned()
    };
    for kind in ["pid", "user", "net", "ipc", "uts", "mnt"] {
        let ours = link(format!("/proc/self/ns/{kind}"));
        let theirs = link(format!("/proc/{pid}/ns/{kind}"));
        // Non-vacuity: both readings have to be real namespace identities, or "they differ"
        // would be satisfied by two different kinds of failure.
        assert!(
            ours.starts_with(&format!("{kind}:[")) && theirs.starts_with(&format!("{kind}:[")),
            "unreadable {kind} namespace: ours={ours:?} theirs={theirs:?}"
        );
        assert_ne!(
            ours, theirs,
            "the isolate shares our {kind} namespace — it was not born namespaced"
        );
    }
}

// =====================================================================================
// ★★★ The environment variable is gone — asserted by watching it be ignored
// =====================================================================================

/// ★★★ `KAYFABE_ISOLATE_BIN` can no longer redirect what gets executed.
///
/// The factory used to resolve its isolate by name: that variable if set, otherwise a
/// sibling of `current_exe()`. Its own rustdoc named the hazard — *"an isolate found on
/// `PATH` is an isolate an environment variable chose, and this process hands that binary a
/// descriptor for `/dev`"* — and then kept a narrower instance of it. The image is now
/// embedded, so there is no name to resolve at all.
///
/// ## How this is made non-vacuous
///
/// A test that merely sets a variable nothing reads asserts nothing. So it runs in two
/// halves, and the outer half **proves the decoy is a real, different, runnable program**
/// before concluding anything from the inner half:
///
/// 1. the decoy is `exec`'d directly and observed to exit **3** — it runs, and it is not an
///    isolate, so a factory that used it would produce a stillborn isolate (its hello frame
///    never arrives);
/// 2. this test binary is re-`exec`'d with `KAYFABE_ISOLATE_BIN` pointing at the decoy, and
///    the inner run drives a **full verb chain** through a spawned isolate. A verb chain can
///    only complete if the real isolate ran.
///
/// Setting the variable in-process is not available: `std::env::set_var` needs the
/// `unsafe_code` this crate forbids under edition 2024. Re-`exec`ing is also the more
/// honest instrument — the variable is set *before* the process starts, which is the only
/// way it was ever going to be set in production.
#[test]
fn an_environment_variable_can_no_longer_redirect_the_isolate() {
    const INNER: &str = "KAYFABE_ISOLATE_DECOY_INNER";

    if std::env::var_os(INNER).is_some() {
        // ── the inner run: KAYFABE_ISOLATE_BIN names the decoy, and this still works ──
        let f = factory(ParkVerb::Nothing);
        let mut isolate = f.spawn(iso(77));
        assert!(
            !isolate.is_retired(),
            "the decoy was used: the isolate did not start"
        );
        let mut w = isolate.checkout().expect("worker");
        let reply = w
            .execute(&VerbPlan::Publish {
                host_vas: None,
                len: 0x1000,
                at: AT,
            }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"))
            .expect("the EMBEDDED isolate served the verb");
        assert!(
            matches!(reply, VerbReply::Published { .. }),
            "expected a publish, got {reply:?}"
        );
        isolate.checkin(w);
        return;
    }

    // ── the outer run ───────────────────────────────────────────────────────────────
    //
    // ★★★ THE DECOY IS WRITTEN BY A CHILD PROCESS, AND THAT IS LOAD-BEARING — 2026-07-31.
    //
    // It used to be `fs::write` + `set_permissions` right here, and the `exec` two
    // statements below then failed **ETXTBSY ("Text file busy") at 14 in 200** whole-binary
    // runs, always at that `expect`. It reads as a real regression in whatever change is in
    // flight, and it did: it landed in another agent's report tonight as an unexplained red.
    //
    // ⊘ It is NOT a shared or colliding path — the name already carries `process::id()`,
    // and each run's file is its own. The mechanism is the classic multithreaded
    // fork/exec race, and it was confirmed by direct measurement rather than by reading:
    //
    //   `fs::write` holds a WRITE descriptor to this inode for the length of the write.
    //   libtest runs this file's other fifteen tests concurrently, and most of them
    //   `spawn` a real isolate — a `fork`. A child forked inside that window inherits the
    //   descriptor: `O_CLOEXEC` clears it at the child's OWN `exec`, not at the fork, so
    //   for the microseconds until then the kernel still counts a writer on our inode and
    //   refuses `execve` on it with ETXTBSY.
    //
    // ★ Measured, in a standalone probe with eight background forker threads and a fresh
    // unique path per trial, so neither path reuse nor this repo is in it:
    //
    //   | who writes the file we then exec | ETXTBSY |
    //   |---|---|
    //   | this process (`fs::write`)       | **32 / 300** |
    //   | a child process                  | **0 / 300**  |
    //
    // So the fix is structural, not a retry: the write descriptor is moved into a child
    // that exits before we exec, and **this process never opens the decoy for writing at
    // all**. A fork of ours therefore has nothing to inherit. `cp` reads a scratch file we
    // did write — but that scratch file is never executed, so a writer on *it* is
    // harmless. ⊘ A retry loop would have made the red go away while leaving the test
    // unable to run concurrently, which is the property that actually failed.
    let decoy = std::env::temp_dir().join(format!("kayfabe-decoy-isolate-{}", std::process::id()));
    let scratch = decoy.with_extension("src");
    std::fs::write(&scratch, "#!/bin/sh\nexit 3\n").expect("write the decoy's source");
    let copied = std::process::Command::new("cp")
        .args([&scratch, &decoy])
        .status()
        .expect("`cp` must be runnable — the decoy is installed by a child on purpose");
    assert!(
        copied.success(),
        "cp {scratch:?} {decoy:?} failed: {copied:?}"
    );
    let chmodded = std::process::Command::new("chmod")
        .arg("755")
        .arg(&decoy)
        .status()
        .expect("`chmod` must be runnable");
    assert!(
        chmodded.success(),
        "chmod 755 {decoy:?} failed: {chmodded:?}"
    );
    std::fs::remove_file(&scratch).ok();

    // (1) The decoy is real, runnable, and NOT an isolate.
    let ran = std::process::Command::new(&decoy)
        .status()
        .unwrap_or_else(|e| {
            // ★ If this ever comes back, say what it is. An `expect` here printed
            // `Os { code: 26, kind: ExecutableFileBusy }` and nothing about why, and the
            // reader's first guess — a stale file at a shared path — is the wrong one.
            assert_ne!(
                e.raw_os_error(),
                Some(26),
                "ETXTBSY exec'ing {decoy:?}: some thread of THIS process still holds a write \
             descriptor to that inode, so the decoy is being installed in-process again \
             somewhere. The write must happen in a child (see the comment above); a retry \
             here would only hide it. Underlying: {e:?}"
            );
            panic!("the decoy is executable: {e:?}");
        });
    assert_eq!(
        ran.code(),
        Some(3),
        "the fixture is worthless unless the decoy really runs and really is not an isolate"
    );

    // (2) With it named, the embedded isolate is what runs anyway.
    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--exact",
            "an_environment_variable_can_no_longer_redirect_the_isolate",
            "--nocapture",
        ])
        .env("KAYFABE_ISOLATE_BIN", &decoy)
        .env(INNER, "1")
        .output()
        .expect("re-exec this test binary");
    std::fs::remove_file(&decoy).ok();
    assert!(
        out.status.success(),
        "with KAYFABE_ISOLATE_BIN={decoy:?} the isolate must still be the embedded one\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Non-vacuity of the re-exec itself: the inner run must have RUN the test, not filtered
    // it away. `1 passed` is libtest's own word for that.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 passed"),
        "the inner run did not execute the test:\n{stdout}"
    );
}

/// ★★ And the name is gone from the source, so it cannot come back through a different door.
///
/// A ratchet rather than a comment: `locate_program` was deleted, but a future edit could
/// reintroduce "read a path out of the environment" under any name, and the *specific* name
/// this project already shipped is the one a reviewer would not look twice at.
///
/// ★ **Comments are stripped before matching**, the same correction `ci.yml`'s host-pointer
/// gate carries and for the same reason: the rule is about what the code *reads*, and the
/// history of a deleted door is worth writing down at the door. Measured — the first run of
/// this test failed on `spawn_unsafe.rs` and `build.rs`, both of which name the variable in
/// prose explaining why it is gone. Truncating each line at its first `//` can only hide the
/// token *after* a comment marker, which no `env::var` call can be.
#[test]
fn no_source_file_mentions_the_deleted_environment_variable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<pkg> is two levels below the workspace root")
        .to_path_buf();
    // Non-vacuity: the walk must actually find source files.
    let mut seen = 0usize;
    let mut offenders = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                seen += 1;
                // This file names it on purpose, in the test above.
                if path.file_name().is_some_and(|n| n == "real_isolate.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let code: String = text
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");
                if code.contains("KAYFABE_ISOLATE_BIN") {
                    offenders.push(path);
                }
            }
        }
    }
    assert!(
        seen > 50,
        "the walk found only {seen} source files — the instrument is broken, not the tree"
    );
    assert!(
        offenders.is_empty(),
        "the deleted variable is back: {offenders:?}"
    );
}
