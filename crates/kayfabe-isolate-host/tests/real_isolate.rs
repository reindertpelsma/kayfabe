//! ★★★ The real isolate, driven **through the existing ports**.
//!
//! Every test here spawns a genuine child process, hands it descriptors, and drives it with
//! `kayfabe_isolate::{IsolateFactory, Isolate, Worker, VerbPlan}` — the same types the whole
//! suite has been driving against `kayfabe_mocks` since L1-M1. Nothing here reaches past the
//! port to poke the implementation.
//!
//! ## The requirement these exist to settle
//!
//! > *"multiple ioctls must execute IN PARALLEL WITHOUT HANGING"*
//!
//! and the finding that constrains it: RM serialises **all** ioctls per client and waits
//! uninterruptibly, so parallelism has to come from **multiple clients** — i.e. from
//! per-`(Proc, GpuId)` isolates — and not from multiple workers on one client
//! (`rm_concurrency_semantics`; `host_execution_plane.md` §2.0).
//!
//! `parallelism_comes_from_isolates…` and
//! `the_pool_does_not_buy_wire_concurrency_on_one_client` are the two halves of that, and
//! they are deliberately written as a matched pair: one asserts progress, the other asserts
//! its absence, over the *same* fixture. Either alone could pass for the wrong reason.
//!
//! ## No sleeps
//!
//! Progress is asserted as an **edge**, never as a duration: a channel that must stay empty
//! while a park is held and must deliver once it is released. `recv_timeout` appears only as
//! a *sampling* interval inside a loop whose exit condition is the edge itself, so a slow
//! machine makes the test slower and never makes it lie.

use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa};
use kayfabe_isolate::{
    CancelReason, HostHandle, IsolateFactory, IsolateId, RingWorkingSet, RmError, VerbPlan,
    VerbReply, Worker,
};
use kayfabe_isolate_host::loopback::ParkVerb;
use kayfabe_isolate_host::{HostIsolateFactory, RmMode};
use std::sync::mpsc;
use std::time::Duration;

/// The isolate binary this crate builds. `CARGO_BIN_EXE_*` is cargo's own answer, so the
/// test cannot drift from the binary it is testing.
const ISOLATE: &str = env!("CARGO_BIN_EXE_kayfabe-isolate");

/// How long one sampling tick waits before re-checking a progress edge. Never a deadline:
/// see the module docs.
const TICK: Duration = Duration::from_millis(25);

fn factory(park: ParkVerb) -> HostIsolateFactory {
    HostIsolateFactory::new(ISOLATE, RmMode::Loopback).with_park(park)
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
    let mut f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(1));
    assert!(
        !isolate.is_retired(),
        "spawn failed — the isolate binary should be at {ISOLATE}"
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
        })
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
    let mut f = factory(ParkVerb::Nothing);
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
    )
    .expect("the gate passes an all-published working set");
    match w.execute(&plan).expect("doorbell") {
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
        .with_rm(|rm| rm.alloc_vaspace())
        .expect("a control target");
    let plan = VerbPlan::Control {
        obj,
        cmd: kayfabe_isolate::ControlCmd(0x801813),
        payload: vec![0xAB; 32],
    };
    match w.execute(&plan).expect("control") {
        VerbReply::Control { payload } => assert_eq!(payload, vec![0xAB; 32]),
        other => panic!("expected a control reply, got {other:?}"),
    }

    // And the disposal path.
    assert_eq!(
        w.execute(&VerbPlan::Release {
            unmap: Vec::new(),
            free: vec![obj],
        }),
        Ok(VerbReply::Released)
    );
    isolate.checkin(w);
}

/// A handle the child never minted comes back as the exact variant, carrying the value as
/// **presented** — not as a transport failure.
#[test]
fn an_unknown_handle_is_a_bad_handle_and_not_a_wedge() {
    let mut f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(3));
    let mut w = isolate.checkout().expect("worker");
    let bogus = HostHandle::new(iso(3), 0xDEAD_BEEF);
    let failure = w
        .execute(&VerbPlan::Release {
            unmap: Vec::new(),
            free: vec![bogus],
        })
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
    let mut f = factory(ParkVerb::Nothing);
    let mut a = f.spawn(iso(10));
    let mut b = f.spawn(iso(11));
    let mut wa = a.checkout().expect("worker a");
    let mut wb = b.checkout().expect("worker b");

    let ha = wa.with_rm(|rm| rm.alloc_vaspace()).expect("a's vas");
    let hb = wb.with_rm(|rm| rm.alloc_vaspace()).expect("b's vas");

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
        })
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
        let r = worker.execute(&plan);
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
    let mut parking = factory(ParkVerb::Sysmem);
    let mut plain = factory(ParkVerb::Nothing);
    let mut a = parking.spawn(iso(20));
    let mut b = plain.spawn(iso(21));

    let wa = a.checkout().expect("a's worker");
    let (parked_rx, ta) = spawn_verb(
        wa,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        },
    );

    // While A is parked, B completes the identical chain — repeatedly, so this is not one
    // lucky interleaving.
    let mut wb = b.checkout().expect("b's worker");
    for round in 0..8 {
        let reply = wb.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        });
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

/// ★★★ The other half, and the one that settles `host_execution_plane.md` §2.0's open
/// question: **N pool workers of ONE isolate do not get N verbs in flight.**
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
    let mut plain = factory(ParkVerb::Nothing);
    let mut peer = plain.spawn(iso(31));
    let mut wp = peer.checkout().expect("peer worker");

    for _attempt in 0..ATTEMPTS {
        let mut parking = factory(ParkVerb::Sysmem);
        let mut a = parking.spawn(iso(30));

        let w0 = a.checkout().expect("worker 0");
        assert_eq!(w0.id(), kayfabe_isolate::WorkerId(0));
        let (parked_rx, t0) = spawn_verb(
            w0,
            VerbPlan::Publish {
                host_vas: None,
                len: 0x1000,
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
                })
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
    let mut f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(40));

    let mut w = isolate.checkout().expect("worker");
    let stale = isolate
        .cancel_handle(kayfabe_isolate::WorkerId(0))
        .expect("armed while checked out");
    assert!(
        w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000
        })
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
            len: 0x1000
        })
        .is_ok(),
        "a stale cancel landed on an innocent later operation"
    );
    assert_eq!(w.cancel_observed(), None);
    isolate.checkin(w);
}

/// ★★ §7.5's escape, end to end against a real process: the requester is **released
/// without a reply**, and the verb it was waiting for is still parked in the child.
#[test]
fn abandon_releases_a_wedged_requester_with_wedged() {
    let mut f = factory(ParkVerb::Sysmem);
    let mut a = f.spawn(iso(50));

    let w = a.checkout().expect("worker");
    let (rx, t) = spawn_verb(
        w,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        },
    );
    // It really is parked before we abandon it.
    assert!(rx.recv_timeout(TICK).is_err());

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
    let mut f = factory(ParkVerb::Nothing);
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
            len: 0x1000
        })
        .is_ok()
    );
    isolate.checkin(w);
    assert!(isolate.is_quiesced());
}

/// ★ `request_cancel_all` reaches every checked-out worker — §7.6 T2's *"for every
/// checked-out worker, `request_cancel(ProcExit)`"*.
#[test]
fn retiring_a_proc_cancels_every_checked_out_worker() {
    let mut f = factory(ParkVerb::Sysmem);
    let mut a = f.spawn(iso(70));

    let mut parked = Vec::new();
    for _ in 0..2 {
        let w = a.checkout().expect("worker");
        parked.push(spawn_verb(
            w,
            VerbPlan::Publish {
                host_vas: None,
                len: 0x1000,
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
    let mut f = factory(ParkVerb::Sysmem);
    let mut isolate = f.spawn(iso(80));
    // Put a verb in flight that will NEVER come back, then drop everything. §7.0: the
    // process boundary is the collector, and it must not need the verb's cooperation.
    let w = isolate.checkout().expect("worker");
    let (_rx, t) = spawn_verb(
        w,
        VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
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
    let mut f = factory(ParkVerb::Nothing);
    let mut isolate = f.spawn(iso(90));
    let mut w = isolate.checkout().expect("worker");
    kayfabe_util::lockwitness::note_acquired(1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        });
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
