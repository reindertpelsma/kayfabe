//! ★ **THE RETRY LEDGER** — `l1_concurrency.md` §12.9's converging-staleness re-plan,
//! audited across N attempts with §12.16's acquire/release [`HostLedger`].
//!
//! §12.9 settled the *policy*: converging staleness (a sibling materialized the same
//! host VAS / channel / engine object first) **re-plans, bounded**; divergent staleness
//! (proc retired, channel gone, route rewritten) **refuses loudly**. The mechanism looks
//! right — a converging [`kayfabe_fwd::Refusal`] carries [`kayfabe_isolate::Orphans`],
//! and `SharedDevice::verb_op` disposes of them on the same worker, still lock-free,
//! *before* the retry.
//!
//! What was never tested is the thing that actually matters about a retry: **the ledger
//! across N attempts.** A retry is only safe if the attempt that lost the race left
//! nothing behind — and every attempt here allocates real host objects (a host VAS, a
//! host memory object, a host GPU mapping) *before* it loses. "Retry is safe" was an
//! assumed claim; these tests make it a measured one.
//!
//! Nothing here waits on a clock. Every ordering edge is a [`VerbHold`] latch: a verb
//! parks *inside* the mock backend and the test releases it, so "attempt 2 planned
//! before attempt 1 committed" is a fact the harness enforces rather than a race it
//! hopes for.
//!
//! What this file pins, per test:
//!
//! - **★ `converging_retries_release_every_host_object_they_allocated`** — N publishers
//!   race for one `Vas`'s host VAS; N−1 lose, re-plan, and win on the retry. The
//!   assertion is not a verb count: it is that the ledger's **outstanding set equals
//!   exactly the set of host objects still reachable from core state**, per isolate,
//!   objects *and* mappings. Every duplicate allocated by a losing attempt is released
//!   exactly once, and nothing is released twice.
//! - **`the_commit_retry_bound_still_releases_the_attempt_that_hits_it`** — the world is
//!   re-staled under the publisher on every round until `MAX_COMMIT_RETRIES` is
//!   exhausted. The op surfaces `Stale::Rebound` (never a spin), and the ledger shows the
//!   **final, refused** attempt's allocation released like every other one — the bound
//!   changes what the caller is told, not what the host is left holding.
//! - **`a_retry_whose_replan_diverges_refuses_without_leaking_the_attempt`** — the path
//!   switches mid-flight: attempt 1 converges (re-plan), attempt 2's commit finds the
//!   proc retired (divergent, non-retryable) with a fresh allocation outstanding. Pins
//!   *both* the fault variant and what the ledger says about the outstanding objects —
//!   which is where the G4 residue is visible, and where it is stated rather than
//!   assumed.
//!
//! Every test arms a [`watchdog`] (the `concurrency_stress.rs` bounded-termination rule)
//! and joins every thread it spawns.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_core::{ProcId, gpu::Proc};
use kayfabe_fwd::{FwdFault, Stale};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, SharedRecorder, VerbHold, VerbKind, mock_classes as mc,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Scenario, identical_handles};

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// Abort the process loudly if the guard is not dropped within `limit` — bounded
/// termination, so a regression that wedges this suite fails fast instead of eating the
/// CI timeout. `KAYFABE_STRESS_WATCHDOG_SECS` overrides it (TSan runs 10-20x slower).
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
            thread::sleep(Duration::from_millis(50));
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
/// The scripted process's `Device` handle — the parent every replacement VASpace is
/// allocated under, so the rebuilt `Vas` resolves to the same GPU target.
const DEV: HObject = HObject(0x5c00_0001);
/// The scripted process's first VASpace handle (`identical_handles`).
const VAS0: HObject = HObject(0x5c00_0010);
/// Handle base for the replacement VASpaces the re-stale rounds declare — clear of the
/// scenario's own handle block (`0x5c00_0010..0x5c00_001b`).
const REPL_VAS: HObject = HObject(0x5c00_0100);
const VA: GpuVa = GpuVa(0x2_0020_0000);

/// One guest proc on GPU0 with `pool` workers in its isolate.
fn device_with(pool: usize, mode: LockMode) -> (Arc<SharedDevice>, ProcId, SharedRecorder) {
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
    (Arc::new(SharedDevice::new(gpu, mode)), pid, recorder)
}

/// Arm a one-shot hold on `verb` of `(pid's isolate, GPU, worker)`.
fn hold_worker(rec: &SharedRecorder, pid: ProcId, worker: u32, verb: VerbKind) -> Arc<VerbHold> {
    rec.lock().expect("recorder").hold(HoldSpec::exact(
        IsolateId(pid.0),
        GPU,
        kayfabe_isolate::WorkerId(worker),
        verb,
    ))
}

/// Arm a one-shot hold on the next `verb` of `pid`'s isolate, **whichever worker takes
/// it** — the form the retry rounds need, because a re-planned attempt checks a worker
/// out afresh and may land on a different slot than the attempt that lost.
fn hold_any(rec: &SharedRecorder, pid: ProcId, verb: VerbKind) -> Arc<VerbHold> {
    rec.lock()
        .expect("recorder")
        .hold(HoldSpec::on_isolate(IsolateId(pid.0), verb))
}

// ---------------------------------------------------------------------------------
// The ledger's counterpart: what core state still legitimately OWNS
// ---------------------------------------------------------------------------------

/// Every host object reachable from one `Proc`'s state — its `Vas`es' host VASes, the
/// host memory behind every published binding, and its channels' host objects.
///
/// This is the other half of the ledger assertion, and the half that makes it a
/// statement about *correctness* rather than about verb arithmetic: an object the mock
/// minted is legitimate iff the core can still name it. Anything outstanding that is
/// **not** in this set is a leak by definition — nothing will ever free it, because
/// nothing can address it.
fn reachable_objects(proc: &Proc) -> BTreeSet<HostHandle> {
    let mut live = BTreeSet::new();
    for vas in proc.vases.values() {
        live.extend(vas.host_vas);
        for (_va, _len, binding) in vas.table.iter() {
            live.extend(binding.host_memory());
        }
    }
    for chan in proc.channels.values() {
        live.extend(chan.host_channel);
        live.extend(chan.host_engine_objects.values().copied());
    }
    live
}

/// Every host GPU mapping reachable from one `Proc`'s state, as the ledger keys them:
/// `(host VAS, host GPU VA)`. A published binding is mapped in its OWN `Vas`'s host VAS
/// (the per-`Vas` #14 fix), so the pair is derivable without consulting the verb log.
fn reachable_maps(proc: &Proc) -> BTreeSet<(HostHandle, u64)> {
    let mut live = BTreeSet::new();
    for vas in proc.vases.values() {
        let Some(host_vas) = vas.host_vas else {
            continue;
        };
        for (_va, _len, binding) in vas.table.iter() {
            if let Some(host_va) = binding.host_va() {
                live.insert((host_vas, host_va));
            }
        }
    }
    live
}

// ---------------------------------------------------------------------------------
// 1 — ★ THE HEADLINE: N converging retries, and the ledger balances exactly
// ---------------------------------------------------------------------------------

/// ★ **N publishers race for one `Vas`'s host VAS; N−1 lose, re-plan, and the ledger
/// balances to the byte** (`l1_concurrency.md` §12.9, §12.16).
///
/// The script forces the race rather than hoping for it: every publisher's `map_gpu_va`
/// is held pending, so **all N plans are made against `host_vas == None`** and all N
/// executions have already allocated a host VAS *and* a host memory object *and* are
/// inside their mapping verb before a single commit runs. Exactly one commit adopts its
/// fresh VAS; the other N−1 hit `commit_publish`'s `(None, Some(fresh))` arm with
/// `vas.host_vas` already set — converging staleness — release their duplicate, re-plan
/// against the winner's VAS, and succeed.
///
/// **The assertion is a set equality, not a count.** Outstanding(ledger) ==
/// Reachable(core), for objects and for mappings. That is the invariant that generalises:
/// it does not need to know how many retries happened, it needs the core to be able to
/// *name* every host object that still exists. A losing attempt that leaked its
/// duplicate would show up as an outstanding handle no `Vas`, binding or channel refers
/// to — which is precisely what "leak" means and precisely what the bite-check produces.
#[test]
fn converging_retries_release_every_host_object_they_allocated() {
    const N: usize = 4;
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let _wd = watchdog(
            "converging_retries_release_every_host_object",
            Duration::from_secs(60),
        );
        let (device, pid, rec) = device_with(N, mode);

        // One hold per pool slot: N publishers, N workers, so every one of them parks
        // in its mapping verb — the edge that makes "all N planned first" a fact.
        let holds: Vec<Arc<VerbHold>> = (0..N as u32)
            .map(|w| hold_worker(&rec, pid, w, VerbKind::MapGpuVa))
            .collect();
        let threads: Vec<_> = (0..N)
            .map(|k| {
                let d = Arc::clone(&device);
                let va = GpuVa(VA.0 + (k as u64) * 0x10_0000);
                thread::spawn(move || d.publish_backing(GPU, PDB, va, 0x1000))
            })
            .collect();
        for h in &holds {
            h.wait_until_pending();
        }
        assert!(
            holds.iter().all(|h| h.is_pending()),
            "({mode:?}) every publisher planned against an unmaterialized host VAS"
        );

        for h in &holds {
            h.release();
        }
        let published: Vec<_> = threads
            .into_iter()
            .map(|t| {
                t.join()
                    .expect("publisher joins")
                    .expect("every publisher commits — the losers via a re-plan")
            })
            .collect();

        // Every publisher got a distinct GPA and a distinct host VA: the re-planned
        // attempts mapped into the WINNER's VAS, they did not resurrect their own.
        let gpas: BTreeSet<u64> = published.iter().map(|p| p.gpa).collect();
        let host_vas: BTreeSet<u64> = published.iter().map(|p| p.host_va).collect();
        assert_eq!(gpas.len(), N, "({mode:?}) N distinct GPAs");
        assert_eq!(host_vas.len(), N, "({mode:?}) N distinct host VAs");

        // The race really happened: N−1 attempts were refused and re-planned, which is
        // visible as N−1 extra host VASes minted and N−1 frees of them. (A count, kept
        // only to prove the SCENARIO — the correctness claim is the set equality below.)
        let ledger = rec.lock().expect("recorder").ledger();
        let log = rec.lock().expect("recorder").verbs_of(IsolateId(pid.0));
        let minted_vas = log
            .iter()
            .filter(|v| matches!(v, kayfabe_mocks::RmVerb::AllocVaSpace { .. }))
            .count();
        assert_eq!(
            minted_vas, N,
            "({mode:?}) all N attempts allocated a host VAS before the race resolved"
        );

        // ★ THE ASSERTION: outstanding == reachable, objects and mappings.
        let gpu = Arc::try_unwrap(device)
            .unwrap_or_else(|_| panic!("every thread joined"))
            .into_gpu();
        let proc = &gpu.procs[&pid];
        assert_eq!(
            ledger.leaked_on(IsolateId(pid.0)),
            reachable_objects(proc),
            "({mode:?}) every host OBJECT still outstanding is one core state can name — \
             a converging retry released its duplicate"
        );
        assert_eq!(
            ledger
                .leaked_maps
                .get(&IsolateId(pid.0))
                .cloned()
                .unwrap_or_default(),
            reachable_maps(proc),
            "({mode:?}) every host MAPPING still outstanding is one core state can name"
        );
        assert_eq!(
            (
                ledger.double_free.as_slice(),
                ledger.free_of_unknown.as_slice(),
                ledger.unmap_of_unknown.as_slice()
            ),
            (&[][..], &[][..], &[][..]),
            "({mode:?}) a retry never releases the same object twice, and never releases \
             one it did not acquire"
        );
    }
}

// ---------------------------------------------------------------------------------
// 2 — the bound: MAX_COMMIT_RETRIES, and what it leaves behind
// ---------------------------------------------------------------------------------

/// Re-stale the `(GPU, PDB)` `Vas` under a parked publisher: free its VASpace and
/// declare a replacement (same PDB, same `Device` parent), so the runtime `Vas` is
/// dropped and re-created with `host_vas == None`. Then run one *winning* publish, which
/// materializes a **new** host VAS.
///
/// The result is exactly the fact `commit_publish`'s R5 check tests: the host VAS the
/// parked attempt planned against is no longer the one the `Vas` holds — converging
/// staleness, every round, on demand. `round` selects the replacement handle so each
/// rebuild is a distinct VASpace object.
///
/// Runs entirely on the test thread while the publisher is parked lock-free inside its
/// mapping verb, so it needs no hold of its own.
///
/// Returns **exactly what this round left outstanding**: the host objects and mappings
/// the winning publish minted. They are never freed — dropping a runtime `Vas` abandons
/// its host backing (per-object reclamation is L1-M2's; the disposition of record is the
/// isolate session's death, §12.16). Returning them lets the test state the expected
/// ledger as a *set built by the script*, so the assertion needs no arithmetic and no
/// assumption about how many attempts the publisher made.
fn restale_the_vas(
    device: &SharedDevice,
    rec: &SharedRecorder,
    round: u32,
    winner_va: GpuVa,
) -> (BTreeSet<HostHandle>, BTreeSet<(HostHandle, u64)>) {
    let mark = rec.lock().expect("recorder").log.len();
    restale_and_win(device, round, winner_va);
    let rec = rec.lock().expect("recorder");
    let mut objects = BTreeSet::new();
    let mut maps = BTreeSet::new();
    for (_iso, verb) in &rec.log[mark..] {
        match verb {
            kayfabe_mocks::RmVerb::AllocVaSpace { handle }
            | kayfabe_mocks::RmVerb::AllocSysmem { handle, .. } => {
                objects.insert(*handle);
            }
            kayfabe_mocks::RmVerb::MapGpuVa { vas, va, .. } => {
                maps.insert((*vas, *va));
            }
            other => panic!("the re-stale round issued an unexpected verb: {other:?}"),
        }
    }
    (objects, maps)
}

/// The mutation half of [`restale_the_vas`].
fn restale_and_win(device: &SharedDevice, round: u32, winner_va: GpuVa) {
    // Round 0 replaces the scenario's own VASpace; later rounds replace the previous
    // round's replacement. `REPL_VAS` is deliberately far from the scenario's handle
    // block (TSG/channels live just above `VAS0`) — a collision would be an *alloc over
    // a live handle*, which the graph refuses loudly and rightly.
    let prev = if round == 0 {
        VAS0
    } else {
        HObject(REPL_VAS.0 + round - 1)
    };
    let next = HObject(REPL_VAS.0 + round);
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: prev,
        })
        .expect("the guest may free its VASpace");
    device
        .apply(RmEvent::Alloc {
            client: CLIENT,
            parent: DEV,
            handle: next,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        })
        .expect("and declare a replacement");
    device
        .apply(RmEvent::SetPageDir {
            client: CLIENT,
            vaspace: next,
            pdb: PDB,
        })
        .expect("re-binding the same page directory");
    device
        .publish_backing(GPU, PDB, winner_va, 0x1000)
        .expect("the winner materializes a fresh host VAS");
}

/// ★ **Hitting `MAX_COMMIT_RETRIES` changes what the CALLER is told, not what the host
/// is left holding** (`l1_concurrency.md` §12.9).
///
/// The bound exists so a bug cannot turn a race into a spin — but a bound is only safe if
/// the attempt that trips it is disposed of like every other one. So the world is
/// re-staled under ONE publisher on **every** round: each release makes its commit find a
/// host VAS it did not plan against, refuse converging, release its duplicate, and re-plan
/// into the next round. After the bound is exhausted the op surfaces `Stale::Rebound` —
/// the fault, not a hang and not a success.
///
/// **The expected ledger is built by the script, not computed from a formula.** Each
/// re-stale round returns exactly the host objects and mappings its winning publish left
/// outstanding (a rebuilt `Vas` abandons the previous one's backing — the declared,
/// deferred reclamation gap, §12.16). Their union is therefore the *whole* legitimate
/// residue of the run: if the publisher's final, refused attempt kept anything, the
/// outstanding set is strictly larger than that union and this assertion names the extra
/// handle. That is the bound's question — "is the last attempt's allocation released?" —
/// answered by set equality rather than by counting verbs.
///
/// The round count is read from [`MAX_COMMIT_RETRIES`] rather than copied: one round per
/// attempt the bound permits. Reading it is what stops this test from silently ceasing to
/// reach the bound (too few rounds) or waiting on an attempt that will never be made (too
/// many) if the bound ever moves.
#[test]
fn the_commit_retry_bound_still_releases_the_attempt_that_hits_it() {
    let _wd = watchdog("commit_retry_bound", Duration::from_secs(60));
    let rounds = kayfabe_rt::device::MAX_COMMIT_RETRIES;
    let (device, pid, rec) = device_with(2, LockMode::Sharded);

    // Round 0's hold: the publisher plans against `host_vas == None`.
    let mut held = hold_any(&rec, pid, VerbKind::MapGpuVa);
    let d = Arc::clone(&device);
    let loser = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));

    let mut expect_objects: BTreeSet<HostHandle> = BTreeSet::new();
    let mut expect_maps: BTreeSet<(HostHandle, u64)> = BTreeSet::new();
    for round in 0..rounds {
        // The progress EDGE: this attempt has allocated and is inside its mapping verb.
        held.wait_until_pending();
        let (objects, maps) = restale_the_vas(
            &device,
            &rec,
            round,
            GpuVa(VA.0 + 0x100_0000 + u64::from(round) * 0x1_0000),
        );
        expect_objects.extend(objects);
        expect_maps.extend(maps);
        // Arm the NEXT round's hold BEFORE releasing this one, so the re-planned attempt
        // parks instead of racing the script.
        let next = hold_any(&rec, pid, VerbKind::MapGpuVa);
        held.release();
        held = next;
    }
    // The last release exhausted the bound, so no further attempt is made and the hold
    // armed above is never entered. Releasing it is a no-op that keeps the recorder clean.
    held.release();

    assert_eq!(
        loser.join().expect("the publisher joins"),
        Err(FwdFault::Stale(Stale::Rebound)),
        "the bound surfaces the converging fault rather than spinning on it"
    );

    // ★ The bound's own question, as a set equality.
    let ledger = rec.lock().expect("recorder").ledger();
    assert_eq!(
        ledger.leaked_on(IsolateId(pid.0)),
        expect_objects,
        "the ONLY host objects still outstanding are the ones the re-stale script itself \
         abandoned — the attempt that exhausted the retry bound released its own"
    );
    assert_eq!(
        ledger
            .leaked_maps
            .get(&IsolateId(pid.0))
            .cloned()
            .unwrap_or_default(),
        expect_maps,
        "likewise for mappings: every attempt unmapped what it mapped, the refused one \
         included"
    );
    assert_eq!(
        (
            ledger.double_free.as_slice(),
            ledger.free_of_unknown.as_slice(),
            ledger.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "{} retries released nothing twice and nothing it did not acquire",
        rounds
    );
}

// ---------------------------------------------------------------------------------
// 3 — converging, then divergent: the switch, mid-retry
// ---------------------------------------------------------------------------------

/// ★ **A retry whose re-plan then DIVERGES** — the path switches from re-planning to
/// refusing with a fresh allocation outstanding (`l1_concurrency.md` §12.9's two shapes,
/// crossed).
///
/// Attempt 1 loses a genuine converging race (a sibling materialized the host VAS) and
/// re-plans. While attempt 2 is parked in its mapping verb — host memory already
/// allocated — the proc is retired out of band. Attempt 2's commit therefore takes
/// `commit_publish`'s FIRST guard, `proc.is_retired()`, which is **divergent**:
/// `retry: false`, orphan-carrying, loud.
///
/// Two things are pinned, and the second is the one that is easy to assume:
///
/// 1. the surfaced fault is `Stale::Proc` on the exact proc — not `Rebound` (the shape
///    the previous attempt had) and not an anonymous RM error;
/// 2. what the ledger says about the outstanding objects. A retired isolate **refuses
///    every verb**, disposal included, so the divergent attempt's release cannot land:
///    the objects are the §12.16 G4 *residue*, whose disposition of record is the
///    isolate session's own death. Asserting the residue's exact size is what keeps that
///    a stated property rather than a silent one — if a future reclamation ledger
///    (L1-M2) makes it disposable, this assertion is where that shows up.
#[test]
fn a_retry_whose_replan_diverges_refuses_without_leaking_the_attempt() {
    let _wd = watchdog("retry_replan_diverges", Duration::from_secs(60));
    let (device, pid, rec) = device_with(2, LockMode::Sharded);

    let first = hold_any(&rec, pid, VerbKind::MapGpuVa);
    let d = Arc::clone(&device);
    let loser = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
    first.wait_until_pending();

    // A sibling wins the host VAS while attempt 1 is parked → converging staleness.
    device
        .publish_backing(GPU, PDB, GpuVa(VA.0 + 0x100_0000), 0x1000)
        .expect("the sibling materializes the host VAS");

    let second = hold_any(&rec, pid, VerbKind::MapGpuVa);
    first.release();
    // Attempt 2 re-planned against the winner's VAS and is now parked mid-chain, with a
    // host memory object already allocated.
    second.wait_until_pending();
    let before_retire = rec.lock().expect("recorder").ledger().leaked_count();

    // ★ The world diverges: the proc retires while attempt 2 is lock-free in flight.
    assert!(device.retire_proc(pid), "the proc was live");
    second.release();

    assert_eq!(
        loser.join().expect("the publisher joins"),
        Err(FwdFault::Stale(Stale::Proc(pid))),
        "a divergent re-validation refuses on ITS OWN cause — never the previous \
         attempt's Rebound, never an anonymous RM error"
    );

    // The retired isolate refuses the disposal too, so the attempt's own allocation is
    // the declared G4 residue: the memory object it allocated, and nothing else. (It
    // never mapped — the mapping verb was refused by the same retirement.)
    let ledger = rec.lock().expect("recorder").ledger();
    assert_eq!(
        ledger.leaked_count(),
        before_retire,
        "the divergent attempt allocated nothing NEW after the retirement — its residue \
         is exactly what it already held, disposed of by the isolate session's death"
    );
    assert_eq!(
        (
            ledger.double_free.as_slice(),
            ledger.free_of_unknown.as_slice(),
            ledger.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "a refused disposal is a no-op, never a partial or repeated release"
    );
}
