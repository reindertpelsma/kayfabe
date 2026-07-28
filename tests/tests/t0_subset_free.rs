//! ★★ **T0 — THE GUEST FREES A SUBSET AND KEEPS RUNNING** (`l1_os_shell.md` §7.6 T0,
//! audit gap **G2**), pinned deterministically.
//!
//! §7.0's load-bearing fact is that *the isolate process boundary is the garbage
//! collector*: when a `(Proc, GpuId)` isolate dies, the kernel closes its descriptors and
//! RM frees the entire object tree under its client, whether or not we knew about it. That
//! backstop covers every teardown trigger — a proc retiring, a worker HUP, a reap — with
//! exactly one exception, which is the sharpest thing the L1-M2 audit found:
//!
//! > **The guest frees a *subset* of its objects while the process keeps running.**
//! > `Spine::refresh`'s `p.vases.retain(…)` / `p.channels.retain(…)` drop `Vas` and
//! > `Channel` values holding `host_vas`, bindings' `host`, `host_channel`, `host_token`,
//! > `host_engine_objects` and `GpaBlock`s — **and nothing dies**, so nothing reclaims.
//!
//! That is not an exotic case. It is a training job's steady state, an inference server's
//! steady state, and the workload this project exists for. On this path per-object
//! reclamation is not an optimisation; it is the only reclamation there is.
//!
//! **How this was found, and in which order.** The composed mean run (`l1_mean.rs`) grew
//! the conservation ledger first and reported the honest baseline: with a `t0_churn`
//! phase added, **24 host objects (6 VAS + 6 sysmem + 6 channel + 6 engine object), 6
//! mappings and 24 KiB of GPA** outstanding on *live* procs and nameable by nothing —
//! exactly 4 objects + 1 mapping + one 4 KiB block per free, i.e. linear in the number of
//! subset-frees. Before that phase existed the census read **zero**, because the mean
//! script's two existing subset-frees deliberately targeted channels left VIRGIN. A true
//! negative that says the script never reached T0, not that T0 was safe.
//!
//! What each test here pins:
//!
//! - [`freeing_a_vaspace_queues_its_host_state_and_the_next_op_releases_it`] — the
//!   address plane, including the **unmap-before-free** order and the fact that the queue
//!   is filled synchronously by the `apply` but drained only on a worker.
//! - [`freeing_a_channel_frees_its_engine_objects_before_the_channel`] — the exec plane,
//!   children before parents.
//! - [`freeing_a_vaspace_returns_its_gpa_to_the_procs_own_arena`] — R6's intra-arena half:
//!   the `GpaBlock` travels with the host objects, or the arena leaks one level down.
//! - [`a_quiet_proc_is_drained_by_the_backstop_sweep`] — the proc that stops issuing ops.
//! - [`★ the_drain_never_races_a_verb_in_flight_on_the_same_isolate`] — the regression for
//!   a real bug this fix introduced and the composed retry-ledger script caught: freeing
//!   host objects underneath a parked verb turned our own reclamation into a
//!   use-after-free (`RmError::BadHandle`).
//! - [`a_retired_procs_queue_is_left_to_the_session_death_backstop`] — the one disposition
//!   T0 deliberately does *not* own.
//!
//! Everything here is single-threaded except where a latch is the point, and every
//! assertion is an exact value or an exact fault variant.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::ProcId;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_fwd::{FwdFault, Stale};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbHold, VerbKind,
    mock_classes as mc,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{
    Guarded, ResidueClaim, Scenario, identical_handles, reachable_maps, reachable_objects,
};

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// Abort the process loudly if the guard is not dropped within `limit` — the standing
/// bounded-termination rule, so a regression that wedges this suite fails fast.
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
/// The scenario's own PDB (its VASpace is `VAS0`, its channels hang off its TSG).
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
/// The `Device` handle every VASpace and TSG in the scenario hangs off.
const DEV: HObject = HObject(0x5c00_0001);
/// The scenario's TSG — the parent of every extra channel these tests declare.
const TSG: HObject = HObject(0x5c00_0012);

/// The **throw-away** VASpace: declared, published into, then freed while the proc lives.
const SCRATCH_VAS: HObject = HObject(0x5c00_0200);
/// Its page-directory base (distinct from [`PDB`], so the two `Vas`es are distinct).
const SCRATCH_PDB: Pdb = Pdb(0x3700_0000);
/// A second throw-away VASpace, for the in-flight-race regression.
const SCRATCH_VAS2: HObject = HObject(0x5c00_0201);
/// Its page-directory base.
const SCRATCH_PDB2: Pdb = Pdb(0x3800_0000);
/// The **throw-away** channel: declared, rung, given an engine object, then freed.
const SCRATCH_CHAN: HObject = HObject(0x5c00_0300);
/// Its vChid.
const SCRATCH_VCHID: VChid = VChid(0x300);

/// A VA in the scratch VASpace.
const VA_SCRATCH: GpuVa = GpuVa(0x40_0000_0000);
/// A VA in the proc's main VASpace (the op that keeps the proc alive and busy).
const VA_MAIN: GpuVa = GpuVa(0x50_0000_0000);

/// One guest proc on GPU0 with `pool` workers in its isolate.
fn device_with(
    pool: usize,
    mode: LockMode,
) -> (Guarded<Arc<SharedDevice>>, ProcId, SharedRecorder) {
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
    (
        Guarded::new(
            "t0_subset_free::device_with",
            Arc::new(SharedDevice::new(gpu, mode)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare a VASpace with its own PDB — a `Vas` of its own in the proc.
fn declare_vaspace(device: &SharedDevice, handle: HObject, pdb: Pdb) {
    device
        .apply(RmEvent::Alloc {
            client: CLIENT,
            parent: DEV,
            handle,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        })
        .expect("the guest declares a VASpace");
    device
        .apply(RmEvent::SetPageDir {
            client: CLIENT,
            vaspace: handle,
            pdb,
        })
        .expect("and binds a page directory to it");
}

/// Declare a GR channel at `vchid` on the proc's main VASpace.
fn declare_channel(device: &SharedDevice, handle: HObject, vchid: VChid) {
    device
        .apply(RmEvent::Alloc {
            client: CLIENT,
            parent: TSG,
            handle,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(identical_handles(0, 0).vaspace),
                userd_flags: MockArch::userd_flags_for(vchid),
                ..Default::default()
            },
        })
        .expect("the guest declares a channel");
}

/// The guest frees one of its own objects and keeps running.
fn guest_free(device: &SharedDevice, handle: HObject) {
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle,
        })
        .expect("the guest may free its own object");
}

/// Every verb `pid`'s isolate has issued since `mark`, in order.
fn verbs_since(rec: &SharedRecorder, pid: ProcId, mark: usize) -> Vec<RmVerb> {
    rec.lock().expect("recorder").log[mark..]
        .iter()
        .filter(|(i, _)| *i == IsolateId::new(pid.0, GPU))
        .map(|(_, v)| v.clone())
        .collect()
}

/// How many verbs are in the log (a mark for [`verbs_since`]).
fn mark(rec: &SharedRecorder) -> usize {
    rec.lock().expect("recorder").log.len()
}

/// Assert `Outstanding(ledger) == Reachable(core state)`, objects and mappings, and that
/// nothing was released twice or released unacquired — the strongest form of the §7.8
/// conservation invariant, the one `retry_ledger.rs` proved out.
fn assert_conserved(device: Guarded<Arc<SharedDevice>>, pid: ProcId, rec: &SharedRecorder) {
    let ledger = rec.lock().expect("recorder").ledger();
    let gpu = device.map(|d| {
        Arc::try_unwrap(d)
            .unwrap_or_else(|_| panic!("every thread joined"))
            .into_gpu()
    });
    let proc = &gpu.procs[&pid];
    assert_eq!(
        ledger.leaked_on(IsolateId::new(pid.0, GPU)),
        reachable_objects(proc),
        "every host OBJECT still outstanding must be one core state can name"
    );
    assert_eq!(
        ledger
            .leaked_maps
            .get(&IsolateId::new(pid.0, GPU))
            .cloned()
            .unwrap_or_default(),
        reachable_maps(proc),
        "every host MAPPING still outstanding must be one core state can name"
    );
    assert_eq!(
        (
            ledger.double_free.as_slice(),
            ledger.free_of_unknown.as_slice(),
            ledger.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "T0 released nothing twice, and nothing it did not acquire"
    );
}

// ---------------------------------------------------------------------------------
// 1 — the address plane
// ---------------------------------------------------------------------------------

/// ★★ **Freeing a VASpace queues its host state; the proc's next op releases it**
/// (`l1_os_shell.md` §7.6 T0, "fill before you drop" + "drain lock-free").
///
/// The split is the design, and both halves are asserted separately because conflating
/// them is exactly the bug: the fill **must** happen inside `refresh` (it is the last
/// moment the values exist) and the drain **must not** (R1 forbids issuing a host verb
/// under the device write lock). So after the `Free` this test requires the queue to be
/// non-empty and the verb log to be *untouched*; only the next verb-issuing op may
/// release.
///
/// The released chain is asserted as an exact sequence: **unmap, then free the memory,
/// then free the host VAS**. That is RM's own order — `clientFreeResource_IMPL` unmaps a
/// resource's inter-mappings before `objDelete`
/// (`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_client.c:830-849`; `ogkm-610:` is
/// byte-identical at the same lines) and the server frees children and dependents ahead
/// of parents (`ogkm-580: .../rs_server.c:963-981`, `ogkm-610:` idem) — and it is what
/// keeps our external mirror of those mappings from going stale.
#[test]
fn freeing_a_vaspace_queues_its_host_state_and_the_next_op_releases_it() {
    let _wd = watchdog("t0_vaspace", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = device_with(2, mode);
        declare_vaspace(&device, SCRATCH_VAS, SCRATCH_PDB);
        let published = device
            .publish_backing(GPU, SCRATCH_PDB, VA_SCRATCH, 0x1000)
            .expect("the guest publishes into the VASpace it is about to free");

        // The host VAS and the host memory object — read from CORE STATE, never guessed
        // from the log, so the assertion below names the right objects and would catch
        // the two being swapped (which is the shape of an unmap against the wrong VAS).
        let host_vas = device
            .with_proc(pid, |p| p.vases[&(GPU, SCRATCH_PDB)].host_vas)
            .expect("the proc is live")
            .expect("the publication materialized a host VAS");
        let (binding, _) = device
            .resolve(GPU, SCRATCH_PDB, VA_SCRATCH)
            .expect("the publication resolves");
        assert_eq!(binding.host_va(), Some(published.host_va));
        let memory = binding.host.expect("published").memory;
        assert_ne!(
            host_vas, memory,
            "({mode:?}) the two handles are distinct objects"
        );

        // ---- the FREE: fills the queue, issues NOTHING.
        let m = mark(&rec);
        guest_free(&device, SCRATCH_VAS);
        assert_eq!(
            verbs_since(&rec, pid, m),
            vec![],
            "({mode:?}) ★ R1: `refresh` runs under the device write lock, so the subset \
             free must issue no host verb at all — only queue one"
        );
        assert_eq!(
            device.resolve(GPU, SCRATCH_PDB, VA_SCRATCH),
            Err(FwdFault::UnknownPdb {
                gpu: GPU,
                pdb: SCRATCH_PDB
            }),
            "({mode:?}) the freed VASpace is gone from core state — MISS=FAULT, named"
        );

        // ---- the next op: releases, on its own checked-out worker, before its own work.
        let m = mark(&rec);
        device
            .publish_backing(GPU, PDB, VA_MAIN, 0x1000)
            .expect("the proc keeps running");
        let issued = verbs_since(&rec, pid, m);
        assert_eq!(
            &issued[..3],
            &[
                RmVerb::UnmapGpuVa {
                    vas: host_vas,
                    va: published.host_va
                },
                RmVerb::Free { obj: memory },
                RmVerb::Free { obj: host_vas },
            ][..],
            "({mode:?}) ★ the release chain is unmap → free(memory) → free(host VAS), in \
             that order, and it runs BEFORE the op's own work: {issued:?}"
        );

        assert_conserved(device, pid, &rec);
    }
}

// ---------------------------------------------------------------------------------
// 2 — the exec plane
// ---------------------------------------------------------------------------------

/// ★ **Freeing a channel frees its engine objects FIRST, then the channel** (§7.6 T0, the
/// exec plane's half).
///
/// An engine object is allocated *on* a host channel, so it is the child, and RM frees
/// children ahead of parents. `host_token` gets no entry of its own on purpose: it is not
/// a handle, it is the work-submit doorbell token the channel object owns, and it dies
/// with it — a "release" for it would be a free of something that was never allocated,
/// which is precisely what `HostLedger::free_of_unknown` exists to catch.
#[test]
fn freeing_a_channel_frees_its_engine_objects_before_the_channel() {
    let _wd = watchdog("t0_channel", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = device_with(2, mode);
        declare_channel(&device, SCRATCH_CHAN, SCRATCH_VCHID);
        let rung = device
            .doorbell(GPU, MockArch::token_for(SCRATCH_VCHID), &[])
            .expect("the guest rings the channel it is about to free");
        let forwarded = device
            .forward_engine_object(GPU, SCRATCH_VCHID, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("and forwards an engine object onto it");
        let host_channel = host_channel_of(&rec, pid);

        let m = mark(&rec);
        guest_free(&device, SCRATCH_CHAN);
        assert_eq!(
            verbs_since(&rec, pid, m),
            vec![],
            "({mode:?}) the subset free issues no host verb"
        );
        assert_eq!(
            device.doorbell(GPU, MockArch::token_for(SCRATCH_VCHID), &[]),
            Err(FwdFault::UnknownVchid {
                gpu: GPU,
                vchid: SCRATCH_VCHID
            }),
            "({mode:?}) the freed channel no longer routes — MISS=FAULT, named"
        );

        let m = mark(&rec);
        device
            .publish_backing(GPU, PDB, VA_MAIN, 0x1000)
            .expect("the proc keeps running");
        let issued = verbs_since(&rec, pid, m);
        assert_eq!(
            &issued[..2],
            &[
                RmVerb::Free {
                    obj: forwarded.host_object
                },
                RmVerb::Free { obj: host_channel },
            ][..],
            "({mode:?}) ★ children before parents: the engine object is freed, then the \
             channel it lives on: {issued:?}"
        );
        assert!(
            !issued
                .iter()
                .any(|v| matches!(v, RmVerb::Free { obj } if obj.raw() == rung.host_token)),
            "({mode:?}) the work-submit token is not a handle and must never be freed"
        );

        assert_conserved(device, pid, &rec);
    }
}

/// The host channel handle the mock minted for the scratch channel, read off the log.
fn host_channel_of(rec: &SharedRecorder, pid: ProcId) -> HostHandle {
    verbs_since(rec, pid, 0)
        .into_iter()
        .find_map(|v| match v {
            RmVerb::AllocChannel { handle, .. } => Some(handle),
            _ => None,
        })
        .expect("the ring allocated a host channel")
}

// ---------------------------------------------------------------------------------
// 3 — R6: the GPA half travels with the host half
// ---------------------------------------------------------------------------------

/// ★ **The `GpaBlock` travels with the host objects** (§7.8's R6 row, gap G6).
///
/// `GpaBlock` is move-only precisely so that a double free is unrepresentable — which
/// also means a block dropped with its `Vas` is unrecoverable, and the proc's arena
/// monotonically fills. That is the C's #80 leak (`teardown_hardening_done`) reproduced
/// one level down after being fixed one level up, and it is why the GPA return is done in
/// the same act as the host queueing rather than in a separate pass someone can forget.
///
/// It runs **under the device write lock and that is correct**: returning a block to a
/// `GpaArena` issues no host verb, so R1 does not apply to it. Only the host disposal has
/// to wait for a worker.
#[test]
fn freeing_a_vaspace_returns_its_gpa_to_the_procs_own_arena() {
    let _wd = watchdog("t0_gpa", Duration::from_secs(60));
    let (device, pid, rec) = device_with(2, LockMode::Sharded);

    let baseline = live_bytes(&device, pid);
    declare_vaspace(&device, SCRATCH_VAS, SCRATCH_PDB);
    device
        .publish_backing(GPU, SCRATCH_PDB, VA_SCRATCH, 0x1000)
        .expect("publish into the scratch VASpace");
    assert_eq!(
        live_bytes(&device, pid),
        baseline + 0x1000,
        "the publication carved one page out of the proc's own arena"
    );

    guest_free(&device, SCRATCH_VAS);
    assert_eq!(
        live_bytes(&device, pid),
        baseline,
        "★ R6: the dropped `Vas`'s GPA block came back to the proc's arena in the same \
         act that queued its host objects — an arena that only ever fills is #80 one \
         level down"
    );

    // …and the returned range is genuinely reusable, not merely uncounted.
    declare_vaspace(&device, SCRATCH_VAS2, SCRATCH_PDB2);
    let again = device
        .publish_backing(GPU, SCRATCH_PDB2, VA_SCRATCH, 0x1000)
        .expect("the reclaimed GPA is handed out again");
    assert_eq!(
        live_bytes(&device, pid),
        baseline + 0x1000,
        "the reused range came out of the free list, not off the end of the cursor"
    );
    assert!(
        again.gpa >= arena_range(&device, pid).start && again.gpa < arena_range(&device, pid).end,
        "the reissued GPA is inside this proc's own arena"
    );

    device.drain_pending_releases();
    assert_conserved(device, pid, &rec);
}

/// Bytes `pid`'s GPU0 arena still has handed out.
fn live_bytes(device: &SharedDevice, pid: ProcId) -> u64 {
    device
        .with_proc(pid, |p| {
            p.arenas
                .get(&GPU)
                .map_or(0, kayfabe_core::gpa::GpaArena::live_bytes)
        })
        .expect("the proc is live")
}

/// `pid`'s GPU0 arena range.
fn arena_range(device: &SharedDevice, pid: ProcId) -> core::ops::Range<u64> {
    device
        .with_proc(pid, |p| p.arenas[&GPU].range.clone())
        .expect("the proc is live")
}

// ---------------------------------------------------------------------------------
// 4 — the proc that goes quiet
// ---------------------------------------------------------------------------------

/// ★ **A proc that stops issuing ops still gets its queue drained** (§7.6 T0, "with the
/// executor as the backstop for a proc that goes quiet").
///
/// The opportunistic path rides out on the next verb-issuing op, which covers a busy
/// proc for free. A guest process that frees its last VASpace and then blocks on a fence
/// issues no next op — and on this path there is no process-boundary backstop either, so
/// without the sweep those host objects are held for the life of the process.
///
/// The count is exact, and it is the count the queue itself reports before the sweep runs:
/// the assertion is *"the sweep disposed of everything that was owed"*, not *"the sweep
/// did some work"*.
#[test]
fn a_quiet_proc_is_drained_by_the_backstop_sweep() {
    let _wd = watchdog("t0_quiet", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pid, rec) = device_with(2, mode);
        declare_vaspace(&device, SCRATCH_VAS, SCRATCH_PDB);
        declare_channel(&device, SCRATCH_CHAN, SCRATCH_VCHID);
        device
            .publish_backing(GPU, SCRATCH_PDB, VA_SCRATCH, 0x1000)
            .expect("publish");
        device
            .doorbell(GPU, MockArch::token_for(SCRATCH_VCHID), &[])
            .expect("ring");
        device
            .forward_engine_object(GPU, SCRATCH_VCHID, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("engine object");

        guest_free(&device, SCRATCH_CHAN);
        guest_free(&device, SCRATCH_VAS);
        // 1 unmap + free(memory) + free(host VAS) + free(engine object) + free(channel).
        let owed = device
            .with_proc(pid, kayfabe_core::gpu::Proc::pending_release_len)
            .expect("live");
        assert_eq!(
            owed, 5,
            "({mode:?}) the two frees queued exactly one unmap and four objects"
        );

        // …and then the guest goes quiet. No further op is ever issued.
        assert_eq!(
            device.drain_pending_releases(),
            owed,
            "({mode:?}) ★ the backstop sweep disposed of everything the queue owed"
        );
        assert_eq!(
            device.drain_pending_releases(),
            0,
            "({mode:?}) and it is idempotent — a second sweep finds nothing"
        );
        assert_eq!(
            device
                .with_proc(pid, kayfabe_core::gpu::Proc::pending_release_len)
                .expect("live"),
            0,
            "({mode:?}) the queue is empty, not merely reported drained"
        );
        assert_conserved(device, pid, &rec);
    }
}

// ---------------------------------------------------------------------------------
// 5 — ★ the regression: reclamation must never race a verb in flight
// ---------------------------------------------------------------------------------

/// ★★ **The drain must not run while a verb of that isolate is in flight** — the
/// regression for a real bug the fix introduced and a *composed* script caught.
///
/// The first version of T0 drained on every checkout. `retry_ledger.rs`'s scripted
/// re-stale wedged immediately, and the cause was ours, not the mock's: a publisher was
/// parked **inside its mapping verb**, holding a host VAS, when the guest freed that
/// VASpace; the drain then freed the VAS underneath the parked verb, and the verb came
/// back `RmError::BadHandle`. Our own reclamation had become a use-after-free, and it
/// surfaced to the guest as an anonymous host error instead of as staleness.
///
/// The guard is [`kayfabe_core::gpu::Proc::checkout_with_pending_release`]'s idle test —
/// the same `Isolate::is_quiesced` predicate the reap uses, for the same reason (§12.16
/// G3: *"the reap would otherwise tear an isolate down under a live connection held by a
/// foreign thread"*). This test scripts the exact shape and pins **both** halves:
///
/// 1. while the verb is parked, a sibling op on the same isolate releases **nothing**;
/// 2. the parked op's own outcome is the *staleness* refusal its script earned —
///    `Stale::Vas`, naming the very `(gpu, pdb)` the guest freed, never `Rm(BadHandle)`.
///    That is the §12.10 polarity: when the world genuinely moved, the fault says so; an
///    anonymous host error in its place is a wrong-cause answer, and it was precisely what
///    the racing drain produced.
#[test]
fn the_drain_never_races_a_verb_in_flight_on_the_same_isolate() {
    let _wd = watchdog("t0_in_flight", Duration::from_secs(60));
    let (device, pid, rec) = device_with(2, LockMode::Sharded);

    // A scratch VASpace with real host state, and a publisher parked inside its map verb
    // against it.
    declare_vaspace(&device, SCRATCH_VAS, SCRATCH_PDB);
    device
        .publish_backing(GPU, SCRATCH_PDB, VA_SCRATCH, 0x1000)
        .expect("the scratch Vas materializes a host VAS");
    let held: Arc<VerbHold> = rec.lock().expect("recorder").hold(HoldSpec::on_isolate(
        IsolateId::new(pid.0, GPU),
        VerbKind::MapGpuVa,
    ));
    let d = Arc::clone(&device);
    let parked = thread::spawn(move || {
        d.publish_backing(GPU, SCRATCH_PDB, GpuVa(VA_SCRATCH.0 + 0x10_0000), 0x1000)
    });
    held.wait_until_pending();

    // ★ The guest frees the VASpace the parked verb is mapping into.
    let m = mark(&rec);
    guest_free(&device, SCRATCH_VAS);
    assert_eq!(
        device
            .with_proc(pid, kayfabe_core::gpu::Proc::pending_release_len)
            .expect("live"),
        3,
        "the free queued the unmap + memory + host VAS of the dropped Vas"
    );

    // A sibling op runs to completion on the same isolate — and must NOT drain, because
    // the parked verb still names the host VAS the queue wants to free.
    device
        .publish_backing(GPU, PDB, VA_MAIN, 0x1000)
        .expect("a sibling op makes progress");
    let issued = verbs_since(&rec, pid, m);
    assert!(
        !issued
            .iter()
            .any(|v| matches!(v, RmVerb::Free { .. } | RmVerb::UnmapGpuVa { .. })),
        "★ the drain ran while a verb was in flight on the same isolate — that is a \
         use-after-free of our own making: {issued:?}"
    );
    // The backstop sweep must decline for the same reason, not merely the fast path.
    assert_eq!(
        device.drain_pending_releases(),
        0,
        "★ the backstop sweep must decline a busy isolate too"
    );

    held.release();
    assert_eq!(
        parked.join().expect("the parked publisher joins"),
        Err(FwdFault::Stale(Stale::Vas {
            gpu: GPU,
            pdb: SCRATCH_PDB
        })),
        "★ the parked op's outcome is the STALENESS its script earned, naming the very \
         VASpace the guest freed — never an anonymous `Rm(BadHandle)` caused by our own \
         reclamation"
    );

    // …and once the isolate is genuinely idle, the sweep does its work.
    assert_eq!(
        device.drain_pending_releases(),
        3,
        "the queue survived the busy window intact and drains at the idle point"
    );
    assert_conserved(device, pid, &rec);
}

// ---------------------------------------------------------------------------------
// 6 — the disposition T0 does NOT own
// ---------------------------------------------------------------------------------

/// **A retired proc's queue is left to §7.0's process-boundary backstop, deliberately.**
///
/// A retired isolate refuses every verb, disposal included, so there is nothing T0 can do
/// and nothing it should try: the queue dies with the `Proc` at the reap, and the isolate
/// session's death frees the whole client tree. Stating it as a test is what keeps it a
/// *chosen* disposition rather than a silent one — and it is the assertion that would
/// change if a future stage ever made a retired isolate's namespace reclaimable per-object.
#[test]
fn a_retired_procs_queue_is_left_to_the_session_death_backstop() {
    let _wd = watchdog("t0_retired", Duration::from_secs(60));
    let (mut device, pid, rec) = device_with(2, LockMode::Sharded);
    // ★ §12.35 — DECLARED RESIDUE: this test's entire subject, stated to the guard in the
    // guard's own vocabulary. `retire_proc` is the VIOLENT death, so `Proc::retire` stops
    // the isolates and the staged queue can never drain. (The clean death —
    // `Spine::vacate` — keeps them live and does reclaim; that is
    // `freeing_a_vaspace_queues_its_host_state_and_the_next_op_releases_it`.)
    device.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pid.0, GPU),
            "the one disposition T0 deliberately does NOT own: an out-of-band `retire_proc` \
             stops the isolate, so the queued host VAS + backing are disposed of in bulk \
             by the session's death (§7.0)",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 1)
        .maps(1),
    );
    declare_vaspace(&device, SCRATCH_VAS, SCRATCH_PDB);
    device
        .publish_backing(GPU, SCRATCH_PDB, VA_SCRATCH, 0x1000)
        .expect("publish");
    guest_free(&device, SCRATCH_VAS);
    assert!(device.retire_proc(pid), "the proc was live");

    let m = mark(&rec);
    assert_eq!(
        device.drain_pending_releases(),
        0,
        "a retired isolate refuses every verb, so the sweep disposes of nothing"
    );
    assert_eq!(
        verbs_since(&rec, pid, m),
        vec![],
        "…and it does not even try: no verb reaches a retired isolate"
    );
    assert_eq!(
        device.reap_retired(),
        1,
        "the proc reaps at the quiesce point"
    );

    // The residue is real, named, and disposed of by the session's death — exactly what
    // `HostLedger`'s own docs mean by "bulk disposal at namespace death is a different
    // disposition from per-object reclaim".
    let ledger = rec.lock().expect("recorder").ledger();
    let outstanding: BTreeSet<HostHandle> = ledger.leaked_on(IsolateId::new(pid.0, GPU));
    assert_eq!(
        outstanding.len(),
        2,
        "the retired proc's host VAS and memory object are the §7.0 residue: {outstanding:?}"
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
