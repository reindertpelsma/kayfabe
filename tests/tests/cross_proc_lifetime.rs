//! ★★ **The cross-`Proc` host-lifetime rule** (`l1_concurrency.md` §12.26) — the C's
//! 2026-06-18 bench disproof, reproduced in this model and refused.
//!
//! ## What the C measured, and why it is the right test to port
//!
//! `C: src/qemu/nvkvm_gpu_emul.c:2055-2065` records a bench-proven failure: releasing a
//! user client's GPGA overlays at that client's root-free *"yanks the backing out from
//! under the still-polling scrub"* — a **kernel** client (CeUtils, `0xc1e00007`) was
//! reading its ring/finishPayload out of memory owned by a **different** client
//! (`0xc1d00003`). RM keeps such a reference alive with a refcount
//! (`ogkm: src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031` — `memCopyConstruct_IMPL`:
//! `pHwResource->refCount++`, `memdescAddRef`, `DupCount++`). The C's fix was **not**
//! per-proc reclamation; it was deferring every backing reap to a **global** quiesce
//! point (`C: :2074-2128`, consumed at the GSP re-handshake, `C: :3458`).
//!
//! We have neither of those mechanisms, and this file is the argument — executable —
//! that we need neither, because in this model the state they defend against is
//! **refused rather than managed**:
//!
//! 1. **Host memory is minted per isolate and named per isolate.** A [`HostHandle`]
//!    carries the [`IsolateId`] whose RM client namespace it lives in, and
//!    [`kayfabe_isolate::Worker::execute`] refuses — before running anything — a plan
//!    naming a handle from another namespace ([`RmError::ForeignHandle`]). So the C's
//!    exact move (one client's disposal reaching another client's backing) is not a
//!    lifetime *race* here; it is a typed refusal with no timing component at all.
//! 2. **The system proc has no data plane.** [`kayfabe_fwd::publish_backing`] refuses on
//!    `Gpu::system` ([`FwdFault::SystemDataPlane`]), so the system proc never mints host
//!    memory and can never be the *owner* half of a cross-proc pair either. Guest-kernel
//!    completions the C forged (the CeUtils scrub, the GR golden capture) stay forged.
//! 3. **The system component is unconditionable.** Its clients are the guest kernel's,
//!    so condemning it is device-fatal by definition — [`SignalOutcome::DeviceFatal`],
//!    not a condemnation entry, and emphatically not the silent no-op it used to be.
//!
//! ## The invariant every test here shares
//!
//! > **No host object is ever released, unmapped, or operated on through an isolate
//! > other than the one that minted it — on any teardown ordering, however adversarial.**
//!
//! Measured, not argued: [`HostLedger`]'s `free_of_unknown` / `unmap_of_unknown` /
//! `double_free` are the cross-namespace-reach detectors, and they must be empty on
//! **every** path below. Where per-object reclamation is possible the ledger must
//! additionally *balance*; where it is not (a condemned owner, whose isolate is gone),
//! the honest assertion is that the only unreleased objects are that isolate's own —
//! namespace death is a real disposition, and a different one from a leak.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Orphans};
use kayfabe_isolate::{HostHandle, IsolateId, RmError, VerbFailure, VerbPlan, VerbReply, WorkerId};
use kayfabe_mocks::{HostLedger, MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_tests::{Scenario, identical_handles};

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// Bounded termination (the `concurrency_stress.rs` rule): a regression that wedges a
/// teardown must fail fast rather than eat the CI timeout.
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
/// U — the *owner*: the guest compute process whose isolate mints the backing.
const OWNER: HClient = HClient(0xA0);
/// R — the *referencer* stand-in for a second user process (the bystander whose objects
/// must never be reached either).
const OTHER: HClient = HClient(0xB0);
const OWNER_PDB: Pdb = Pdb(0x3400_0000);
const OTHER_PDB: Pdb = Pdb(0x3405_000);
const OWNER_GR: VChid = VChid(0x100);
const OWNER_CE: VChid = VChid(0x200);
const OTHER_GR: VChid = VChid(0x300);
const OTHER_CE: VChid = VChid(0x400);
const MEM: HObject = HObject(0x6000_0000);
const VA: GpuVa = GpuVa(0x2_0020_0000);
const VA2: GpuVa = GpuVa(0x2_0030_0000);

/// The system isolate's id. `Gpu` spawns every isolate as `IsolateId(pid.0)` and the
/// system proc is `ProcId(0)`, so this is a derived fact, not a guess.
const SYSTEM_ISOLATE: IsolateId = IsolateId(0);

/// Two guest compute processes (`OWNER`, `OTHER`) on GPU0, plus the shared verb
/// recorder that backs the conservation ledger.
fn two_proc_gpu() -> (Gpu, ProcId, ProcId, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");

    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        OWNER,
        OWNER_PDB,
        identical_handles(OWNER_GR.0, OWNER_CE.0),
        None,
    );
    s.memory(OWNER, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    s.compute_process_on_gpu(
        OTHER,
        OTHER_PDB,
        identical_handles(OTHER_GR.0, OTHER_CE.0),
        None,
    );
    s.memory(OTHER, HObject(0x5c00_0002), MEM, 0x9_1000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let owner = gpu.spine.by_pdb[&(GPU, OWNER_PDB)];
    let other = gpu.spine.by_pdb[&(GPU, OTHER_PDB)];
    (gpu, owner, other, recorder)
}

/// Everything `proc` owns on `gpu`, in RM's release order (unmaps first) — the same
/// walk `teardown_reclaim.rs` uses, so the two files agree on what "reclaimed" means.
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

/// Run `orphans`' release chain on a worker of `proc`'s own `gpu` isolate.
fn release_on_own_isolate(proc: &mut Proc, gpu: GpuId, orphans: &Orphans) {
    if orphans.is_empty() {
        return;
    }
    let mut w = proc
        .isolate_mut(gpu)
        .expect("materialized isolate")
        .checkout()
        .expect("a free worker");
    let outcome = w.execute(&orphans.release_plan());
    assert_eq!(
        outcome,
        Ok(VerbReply::Released),
        "a proc releasing its OWN objects must succeed"
    );
    proc.isolate_mut(gpu).expect("isolate").checkin(w);
}

/// ★ The move the C measured: run `plan` on a worker of the **system** isolate. Returns
/// the verb outcome so each test can assert the exact refusal.
fn attempt_on_system(gpu: &mut Gpu, plan: &VerbPlan) -> Result<VerbReply, VerbFailure> {
    let mut w = gpu
        .system
        .isolate_mut(GPU)
        .expect("the system isolate is materialized at realize time")
        .checkout()
        .expect("a free system worker");
    let out = w.execute(plan);
    gpu.system
        .isolate_mut(GPU)
        .expect("system isolate")
        .checkin(w);
    out
}

/// The host memory handle backing `va` in `(gpu, pdb)` of `pid`.
fn backing_of(gpu: &Gpu, pid: ProcId, pdb: Pdb, va: GpuVa) -> HostHandle {
    let (binding, _off) = kayfabe_fwd::resolve_in(&gpu.procs[&pid], GPU, pdb, va)
        .expect("the range resolves in its owner's Vas");
    binding.host.expect("the range is host-published").memory
}

/// The ledger, plus the two assertions every test in this file shares: **nothing was
/// ever reached across a namespace**.
fn ledger(rec: &SharedRecorder) -> HostLedger {
    let l = rec.lock().expect("recorder").ledger();
    assert!(
        l.free_of_unknown.is_empty(),
        "a free reached across an isolate namespace — the C's exact hazard: {:?}",
        l.free_of_unknown
    );
    assert!(
        l.unmap_of_unknown.is_empty(),
        "an unmap reached across an isolate namespace: {:?}",
        l.unmap_of_unknown
    );
    assert!(
        l.double_free.is_empty(),
        "an object was released twice: {:?}",
        l.double_free
    );
    l
}

/// Every `Free` in the log, as `(issuing isolate, object)`.
fn frees(rec: &SharedRecorder) -> Vec<(IsolateId, HostHandle)> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(iso, v)| match v {
            RmVerb::Free { obj } => Some((*iso, *obj)),
            _ => None,
        })
        .collect()
}

// =================================================================================
// 1 — the C's 2026-06-18 disproof, reproduced
// =================================================================================

/// ★★ **The C's bench disproof, in this model.** A system-proc verb naming a user
/// proc's backing is refused **before it runs**, so there is nothing to yank and no
/// window to yank it in.
///
/// The assertion is deliberately three-part, because two of the three are what make it
/// a lifetime proof rather than an error-code test:
///
/// - the **exact** fault is [`RmError::ForeignHandle`], naming both the offending handle
///   (which carries the namespace it really belongs to) and the isolate that refused it;
/// - the failure carries **no orphans** — the gate runs ahead of the first verb, so the
///   all-or-nothing promise is trivially kept rather than unwound;
/// - the verb log contains **no `Free` at all**, i.e. the host was never asked.
///
/// **Why the mock cannot be trusted to catch this on its own, which is the whole reason
/// the gate lives in the type.** `MockRmBackend` namespaces its fake handle *values*
/// (`(id+1) << 32 | n`), so a foreign handle is provably invalid there and comes back
/// `BadHandle`. A real host does not: RM mints client-scoped handles from one shared base
/// (`ogkm: src/nvidia/generated/g_resserv_nvoc.h:173`, one `serverSetClientHandleBase` at
/// `.../rmapi.c:105`), so the same raw value is **live and different** in every other
/// client — the free would succeed, on a bystander. The mock's answer is survivable; the
/// host's answer is the C's bug. Only the stamp distinguishes them.
#[test]
fn c_2026_06_18_a_system_verb_on_a_user_procs_backing_is_refused_before_it_runs() {
    let _wd = watchdog("c_2026_06_18_refused", Duration::from_secs(30));
    let (mut gpu, owner, _other, rec) = two_proc_gpu();

    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes its backing");
    let owned = backing_of(&gpu, owner, OWNER_PDB, VA);
    assert_eq!(
        owned.isolate(),
        IsolateId(owner.0),
        "the backing is minted in the OWNER's namespace"
    );

    // The C's move: the kernel/system side disposes of a handle it does not own.
    let out = attempt_on_system(
        &mut gpu,
        &VerbPlan::Release {
            unmap: vec![],
            free: vec![owned],
        },
    );

    assert_eq!(
        out,
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: owned,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default(),
        }),
        "the system worker must refuse a foreign handle, with nothing attempted and \
         nothing orphaned"
    );
    assert!(
        frees(&rec).is_empty(),
        "the host was never asked to free anything: {:?}",
        frees(&rec)
    );

    // …and the owner's backing is untouched — the thing the C's scrub lost.
    let (binding, _off) =
        kayfabe_fwd::resolve(&gpu, GPU, OWNER_PDB, VA).expect("the owner's range still resolves");
    assert_eq!(
        binding.host.expect("still published").memory,
        owned,
        "the owner's backing survived the foreign disposal attempt intact"
    );
    let _ = ledger(&rec);
}

/// ★ The same refusal for the **unmap** half, which is the half that would corrupt
/// rather than merely free: an unmap names `(host VAS, host GPU VA)`, and *both* are
/// foreign here. On a real host this would tear down a mapping in a live bystander's
/// address space.
///
/// Split out because `Orphans` carries the two lists separately and
/// [`VerbPlan::handles`] must enumerate both — a gate that scanned only `free` would
/// pass the test above and still leave this door open.
#[test]
fn a_foreign_unmap_is_refused_as_loudly_as_a_foreign_free() {
    let _wd = watchdog("foreign_unmap", Duration::from_secs(30));
    let (mut gpu, owner, _other, rec) = two_proc_gpu();
    let published = kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    let host_vas = gpu.procs[&owner].vases[&(GPU, OWNER_PDB)]
        .host_vas
        .expect("the owner's Vas materialized its host VAS");

    let out = attempt_on_system(
        &mut gpu,
        &VerbPlan::Release {
            unmap: vec![(host_vas, published.host_va)],
            free: vec![],
        },
    );
    assert_eq!(
        out,
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: host_vas,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default(),
        }),
        "a foreign host VAS is refused before the unmap is issued"
    );
    let l = ledger(&rec);
    assert_eq!(
        l.leaked_maps
            .get(&IsolateId(owner.0))
            .map(std::collections::BTreeSet::len),
        Some(1),
        "the owner's mapping is still exactly where it was"
    );
}

/// ★ And the same for every other plan shape that names an input handle — a
/// [`VerbPlan::Control`] on a foreign object, and a [`VerbPlan::Doorbell`] on a foreign
/// channel. These are the two paths a *forwarded* system verb would most plausibly take
/// once the system proc forwards anything at all, so the gate has to cover the plan
/// vocabulary and not just the disposal path.
#[test]
fn every_plan_shape_that_names_a_foreign_handle_is_refused() {
    let _wd = watchdog("every_plan_shape", Duration::from_secs(30));
    let (mut gpu, owner, _other, rec) = two_proc_gpu();
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    let host_vas = gpu.procs[&owner].vases[&(GPU, OWNER_PDB)]
        .host_vas
        .expect("host VAS");
    let owned = backing_of(&gpu, owner, OWNER_PDB, VA);

    let foreign = RmError::ForeignHandle {
        handle: host_vas,
        worker_isolate: SYSTEM_ISOLATE,
    };
    assert_eq!(
        attempt_on_system(
            &mut gpu,
            &VerbPlan::Publish {
                host_vas: Some(host_vas),
                len: 0x1000,
            },
        ),
        Err(VerbFailure {
            err: foreign,
            orphans: Orphans::default()
        }),
        "publishing into another proc's host VAS is refused"
    );
    assert_eq!(
        attempt_on_system(
            &mut gpu,
            &VerbPlan::Control {
                obj: owned,
                cmd: kayfabe_isolate::ControlCmd(0x2080_0110),
                payload: vec![0; 4],
            },
        ),
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: owned,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default()
        }),
        "controlling another proc's object is refused"
    );
    assert_eq!(
        attempt_on_system(
            &mut gpu,
            &VerbPlan::Doorbell {
                host_vas: None,
                channel: Some((owned, 0x1234)),
                engine: kayfabe_arch::ids::EngineKind::Ce,
                schedule: true,
            },
        ),
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: owned,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default()
        }),
        "ringing another proc's channel is refused"
    );
    assert!(frees(&rec).is_empty(), "nothing was disposed of");
    let _ = ledger(&rec);
}

// =================================================================================
// 2 — the owner dies, both ways, while the reference is attempted
// =================================================================================

/// ★★ **The C's failure, completed: the owner dies CLEANLY and the system reference
/// still cannot dangle.**
///
/// Script: owner publishes → system attempts the foreign release (refused) → owner
/// reclaims its own objects per-object → the guest frees the owner's client root →
/// `refresh` retires it → reap. Then the system tries **again**, on a handle whose
/// isolate is now gone.
///
/// The second attempt is the point. A dangling reference is only dangerous *after* the
/// owner dies; a rule that refuses before and permits after is no rule. It refuses
/// identically, because the handle's namespace is a property of the value, not a lookup
/// against live state — there is nothing to have gone stale.
#[test]
fn the_owner_dying_cleanly_cannot_dangle_a_system_reference() {
    let _wd = watchdog("owner_dies_cleanly", Duration::from_secs(30));
    let (mut gpu, owner, _other, rec) = two_proc_gpu();
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA2,
        0x2000,
    )
    .expect("…twice");
    let owned = backing_of(&gpu, owner, OWNER_PDB, VA);

    let foreign_release = VerbPlan::Release {
        unmap: vec![],
        free: vec![owned],
    };
    assert!(
        matches!(
            attempt_on_system(&mut gpu, &foreign_release),
            Err(VerbFailure {
                err: RmError::ForeignHandle { .. },
                ..
            })
        ),
        "refused while the owner is alive"
    );

    // Clean death: the owner reclaims per-object, then the guest frees its client root.
    let plan = reclaim_plan(&gpu.procs[&owner], GPU);
    release_on_own_isolate(gpu.procs.get_mut(&owner).expect("owner"), GPU, &plan);
    gpu.apply(RmEvent::Free {
        client: OWNER,
        handle: identical_handles(OWNER_GR.0, OWNER_CE.0).client_root,
    })
    .expect("the guest frees the owner's client root");
    assert!(
        !gpu.procs.contains_key(&owner),
        "the owner left the live set"
    );
    assert_eq!(gpu.reap_retired().len(), 1, "…and was reaped");

    // The reference is attempted AGAIN, now that the owning isolate is gone.
    assert_eq!(
        attempt_on_system(&mut gpu, &foreign_release),
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: owned,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default(),
        }),
        "a handle whose isolate is DEAD is refused for exactly the same reason — the \
         namespace is a property of the value, so there is nothing to go stale"
    );

    let l = ledger(&rec);
    assert!(
        l.is_balanced(),
        "a clean owner death conserves every host resource: {l:?}"
    );
    assert!(
        !frees(&rec).iter().any(|&(iso, _)| iso == SYSTEM_ISOLATE),
        "the system isolate issued no free at all"
    );
}

/// ★★ **And the violent half: the owner is CONDEMNED (an out-of-band worker death)
/// while the reference is outstanding.**
///
/// This is the ordering the C could not survive, because its reap ran at the client-root
/// free and a condemnation has no client-root free to wait for. Here the owner's isolate
/// dies with its process — the §7.0 backstop — and the assertion is the precise one the
/// ledger supports: **no cross-namespace reach anywhere**, and every object still
/// outstanding belongs to the condemned isolate, whose namespace death *is* its
/// disposition. Anything freed on the system isolate would be a bystander kill.
#[test]
fn a_condemned_owner_cannot_dangle_a_system_reference() {
    let _wd = watchdog("owner_condemned", Duration::from_secs(60));
    let (mut gpu, owner, other, rec) = two_proc_gpu();
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&other).expect("other"),
        GPU,
        OTHER_PDB,
        VA,
        0x1000,
    )
    .expect("the bystander publishes at the IDENTICAL guest VA (#14's shape)");
    let owned = backing_of(&gpu, owner, OWNER_PDB, VA);
    let bystander = backing_of(&gpu, other, OTHER_PDB, VA);
    assert_ne!(
        owned, bystander,
        "identical guest VAs, disjoint host objects (#14 by construction)"
    );

    // Out-of-band death: the owner's component is condemned, its isolate is gone.
    assert!(
        gpu.spine.retire_proc(&mut gpu.procs, owner),
        "the owner was live when its worker died"
    );
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU, OWNER_PDB),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(OWNER)
        }),
        "the owner's component is condemned"
    );
    assert_eq!(gpu.reap_retired().len(), 1, "and reaped");

    // Both the dead owner's handle AND the live bystander's are refused on the system
    // isolate. The second is the one that would corrupt a *running* process.
    for h in [owned, bystander] {
        assert_eq!(
            attempt_on_system(
                &mut gpu,
                &VerbPlan::Release {
                    unmap: vec![],
                    free: vec![h],
                },
            ),
            Err(VerbFailure {
                err: RmError::ForeignHandle {
                    handle: h,
                    worker_isolate: SYSTEM_ISOLATE,
                },
                orphans: Orphans::default(),
            }),
            "the system isolate reaches neither the dead owner's objects nor a live \
             bystander's"
        );
    }

    // The bystander is completely unaffected: its binding still resolves, host-published.
    let (binding, _off) =
        kayfabe_fwd::resolve(&gpu, GPU, OTHER_PDB, VA).expect("the bystander still resolves");
    assert_eq!(binding.host.expect("published").memory, bystander);

    let l = ledger(&rec);
    let condemned_iso = IsolateId(owner.0);
    for (iso, outstanding) in &l.leaked {
        if outstanding.is_empty() {
            continue;
        }
        assert!(
            *iso == condemned_iso || *iso == IsolateId(other.0),
            "only the condemned isolate (namespace death is its disposition) and the \
             still-running bystander may hold outstanding objects; {iso:?} does: \
             {outstanding:?}"
        );
    }
    assert!(
        !l.leaked_on(condemned_iso).is_empty(),
        "the condemned isolate's objects ARE still outstanding — that is namespace \
         death, and the test says so rather than pretending they were reclaimed"
    );
}

/// ★ **A reference taken *during* the owner's teardown** — the window the C's deferred
/// reap exists to cover. The owner is retired but not yet reaped (its isolate is alive,
/// refusing new checkouts), and the system attempts the foreign release right there.
///
/// Refused, for a reason that has nothing to do with the window: the gate is a fact about
/// the handle, so the hazardous interval has zero width.
#[test]
fn a_reference_taken_during_the_owners_teardown_is_refused_too() {
    let _wd = watchdog("reference_during_teardown", Duration::from_secs(30));
    let (mut gpu, owner, _other, rec) = two_proc_gpu();
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    let owned = backing_of(&gpu, owner, OWNER_PDB, VA);

    gpu.procs.get_mut(&owner).expect("owner").retire();
    assert!(
        gpu.procs[&owner].is_retired(),
        "retired, not yet reaped — the isolate is still alive"
    );

    assert_eq!(
        attempt_on_system(
            &mut gpu,
            &VerbPlan::Release {
                unmap: vec![],
                free: vec![owned],
            },
        ),
        Err(VerbFailure {
            err: RmError::ForeignHandle {
                handle: owned,
                worker_isolate: SYSTEM_ISOLATE,
            },
            orphans: Orphans::default(),
        }),
        "mid-teardown is not a special case: the refusal is the same one"
    );
    let _ = ledger(&rec);
}

// =================================================================================
// 3 — the system plane's own rules
// =================================================================================

/// ★★ **The system proc has NO data plane** — the rule that keeps the whole hazard
/// unrepresentable rather than merely absent.
///
/// If the system proc could publish, it would own host memory, and the question "may a
/// user proc reference it / may it reference a user proc's" would be live in both
/// directions. It cannot, so it owns none, so neither direction exists. The guest-kernel
/// work that would otherwise need a backing — the CeUtils scrub, the GR golden capture —
/// is **forged** onto the system proc's completion queue (`C: nvkvm_gpu_emul.c:4032-4058`,
/// scoped to kernel CeUtils only), never forwarded.
///
/// Asserted as the exact fault, because "it happens to fail" is not the property: before
/// this rule existed the call failed with `UnknownPdb` — the right outcome for entirely
/// the wrong reason, and one that would silently start succeeding the moment anything
/// gave the system proc a `Vas`.
#[test]
fn the_system_proc_has_no_data_plane() {
    let _wd = watchdog("system_no_data_plane", Duration::from_secs(30));
    let (mut gpu, _owner, _other, rec) = two_proc_gpu();
    assert_eq!(
        kayfabe_fwd::publish_backing(&mut gpu.system, GPU, OWNER_PDB, VA, 0x1000),
        Err(FwdFault::SystemDataPlane),
        "the system proc must never mint host memory"
    );
    // …and it stays refused even if the system proc is handed the routing key of a
    // real, live Vas — the refusal is about WHO is publishing, not about what resolves.
    assert_eq!(
        kayfabe_fwd::plan_publish(&gpu.system, GPU, OWNER_PDB, VA, 0x1000).map(|_| ()),
        Err(FwdFault::SystemDataPlane),
    );
    assert!(
        gpu.system.vases.is_empty(),
        "the system proc holds no address plane at all"
    );
    let l = ledger(&rec);
    assert_eq!(
        l.leaked_on(SYSTEM_ISOLATE),
        std::collections::BTreeSet::new(),
        "the system isolate owns nothing"
    );
}

/// ★★ **A system worker death is DEVICE-FATAL, not a condemnation — and it used to be a
/// silent no-op.**
///
/// `SignalOutcome::WorkerDied`'s consequence is retire + condemn, whose recovery story is
/// *"a fresh RM client is a different component"*. The system component's clients are the
/// **guest kernel's**, held for the module's lifetime, so that recovery requires the guest
/// kernel to mint clients — exactly what would have been condemned. Condemning it is
/// device-fatal by definition (RM's analogue: `gpuMarkDeviceForReset` +
/// `NV2080_NOTIFIERS_GPU_UNAVAILABLE`, `ogkm: .../kernel_gsp.c:2779-2789`, at **device**
/// level, never client level).
///
/// What actually happened before: `SharedDevice::signal_source` called
/// `Spine::retire_proc(SYSTEM_PROC)`, which reached for the system proc in a `ProcSet`
/// that does not contain it, missed, and returned `false` into a discarded result. The
/// device carried on with a permanently dead system worker slot and **no fault anywhere**.
///
/// So this test asserts all four halves: the typed outcome, that the slot really did die,
/// that nothing was condemned or retired, and that the system proc is still serving.
#[test]
fn a_system_worker_death_is_device_fatal_not_a_silent_no_op() {
    let _wd = watchdog("system_worker_death", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, _owner, _other, _rec) = two_proc_gpu();
        let pool_before = gpu
            .system
            .isolate(GPU)
            .expect("system isolate")
            .idle_workers();
        let device = SharedDevice::new(gpu, mode);

        let hup = device.register_source(SourceKind::Worker {
            proc: Gpu::SYSTEM_PROC,
            gpu: GPU,
            worker: WorkerId(0),
        });
        assert_eq!(
            device.signal_source(hup),
            SignalOutcome::DeviceFatal {
                gpu: GPU,
                worker: WorkerId(0),
            },
            "({mode:?}) a system worker HUP is device-fatal, and says so"
        );

        assert_eq!(
            device.retired_len(),
            0,
            "({mode:?}) the system proc was NOT retired"
        );
        let gpu = device.into_gpu();
        assert_eq!(
            gpu.spine.condemned_len(),
            0,
            "({mode:?}) and its component was NOT condemned — that would kill the guest \
             driver permanently"
        );
        let iso = gpu
            .system
            .isolate(GPU)
            .expect("the system isolate survives");
        assert_eq!(
            iso.idle_workers(),
            pool_before - 1,
            "({mode:?}) the dead slot is really dead — never a respawn (§7.3)"
        );
        assert!(
            iso.idle_workers() > 0,
            "({mode:?}) …and the system proc keeps serving on its remaining workers"
        );
    }
}

/// ★★ The core-level half of the same rule, isolated so a fix that only patches the
/// adapter is still caught: [`kayfabe_core::gpu::Spine::retire_proc`] **refuses** the
/// system proc by rule, and does not merely fail to find it.
///
/// **The distinction is the whole finding, and it needs the hazard constructed to be
/// visible.** Today `Gpu::procs` does not contain `SYSTEM_PROC`, so a `false` from a map
/// miss and a `false` from a rule are indistinguishable — which is exactly why the
/// original behaviour survived unnoticed. But `SharedDevice::proc_cell` *does* resolve
/// the system proc (a per-proc op may legitimately route to it) while `ExclusiveProcs`
/// does not, and an asymmetry like that is precisely what a later "let's make these
/// consistent" change removes. So this test puts a proc at the `SYSTEM_PROC` key of a
/// real [`ProcSet`] — the future mistake, written down — and asserts the refusal holds
/// anyway: nothing removed, nothing retired, **nothing condemned**.
///
/// Without the guard the same call condemns a component, and in production that
/// component's clients are the guest kernel's: the guest driver would be permanently
/// dead, with recovery ("mint a fresh RM client") available only to the thing that was
/// just condemned.
#[test]
fn the_spine_refuses_to_retire_the_system_proc_even_when_it_is_in_the_proc_set() {
    let _wd = watchdog("spine_refuses_system_retire", Duration::from_secs(30));
    let (mut gpu, owner, other, _rec) = two_proc_gpu();

    // Construct the hazard: a live `Proc` sitting at the SYSTEM_PROC key.
    let victim = gpu.procs.remove(&owner).expect("the owner is live");
    let mut set: std::collections::BTreeMap<ProcId, Proc> = std::collections::BTreeMap::new();
    set.insert(Gpu::SYSTEM_PROC, victim);

    assert!(
        !gpu.spine.retire_proc(&mut set, Gpu::SYSTEM_PROC),
        "the system proc is unconditionable — by rule, not by lookup failure"
    );
    assert!(
        set.contains_key(&Gpu::SYSTEM_PROC),
        "and it was not even removed from the set"
    );
    assert_eq!(
        gpu.spine.condemned_len(),
        0,
        "★ nothing was condemned: condemning the system component is device-fatal, and \
         a device-fatal condition must never be filed as a per-client condemnation entry"
    );
    assert_eq!(gpu.retired_len(), 0, "and nothing was retired");

    // The ordinary path still works — the guard is narrow, not a blanket refusal.
    assert!(gpu.spine.retire_proc(&mut gpu.procs, other));
    assert_eq!(gpu.spine.condemned_len(), 1);
}

// =================================================================================
// 4 — CONSERVATION across every teardown ordering
// =================================================================================

/// Which proc dies first, and how, in [`the_ledger_balances_across_every_teardown_ordering`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ordering2 {
    /// The owner dies first, then the bystander.
    OwnerFirst,
    /// The bystander dies first, then the owner.
    ReferencerFirst,
    /// Both die before either is reaped.
    BothBeforeReap,
    /// The bystander attempts its foreign reach *between* the owner's retire and reap.
    ReachDuringTeardown,
}

/// ★★ **CONSERVATION.** For every scripted ordering: every host object acquired is
/// released exactly once, every mapping unmapped exactly once, and nothing is ever
/// touched across a namespace.
///
/// The adversarial ingredient is that at each step the *other* proc's isolate attempts
/// the foreign disposal the C's kernel client performed — so the ledger is not merely
/// proving that a tidy script tidies up, it is proving that a script actively trying to
/// reach across the boundary changes nothing about the accounting.
///
/// Both procs are torn down *cleanly* here (per-object reclaim, then the guest frees the
/// client root) precisely so `is_balanced()` is the assertable property; the condemned
/// case has its own test above, where namespace death is the disposition and pretending
/// otherwise would be dishonest.
#[test]
fn the_ledger_balances_across_every_teardown_ordering() {
    let _wd = watchdog("ledger_orderings", Duration::from_secs(60));
    for ord in [
        Ordering2::OwnerFirst,
        Ordering2::ReferencerFirst,
        Ordering2::BothBeforeReap,
        Ordering2::ReachDuringTeardown,
    ] {
        let (mut gpu, owner, other, rec) = two_proc_gpu();
        for (pid, pdb) in [(owner, OWNER_PDB), (other, OTHER_PDB)] {
            for va in [VA, VA2] {
                kayfabe_fwd::publish_backing(
                    gpu.procs.get_mut(&pid).expect("live"),
                    GPU,
                    pdb,
                    va,
                    0x1000,
                )
                .unwrap_or_else(|e| panic!("({ord:?}) {pid:?} publishes {va:?}: {e:?}"));
            }
        }
        let a_backing = backing_of(&gpu, owner, OWNER_PDB, VA);
        let b_backing = backing_of(&gpu, other, OTHER_PDB, VA);

        // Each proc's isolate attempts to dispose of the OTHER's backing — the C's move,
        // in both directions, before anything dies.
        for (pid, foreign) in [(owner, b_backing), (other, a_backing)] {
            let plan = VerbPlan::Release {
                unmap: vec![],
                free: vec![foreign],
            };
            let mut w = gpu
                .procs
                .get_mut(&pid)
                .expect("live")
                .isolate_mut(GPU)
                .expect("isolate")
                .checkout()
                .expect("worker");
            assert!(
                matches!(
                    w.execute(&plan),
                    Err(VerbFailure {
                        err: RmError::ForeignHandle { .. },
                        ..
                    })
                ),
                "({ord:?}) a cross-proc disposal is refused in every direction"
            );
            gpu.procs
                .get_mut(&pid)
                .expect("live")
                .isolate_mut(GPU)
                .expect("isolate")
                .checkin(w);
        }

        let order: [(ProcId, HClient); 2] = match ord {
            Ordering2::ReferencerFirst => [(other, OTHER), (owner, OWNER)],
            _ => [(owner, OWNER), (other, OTHER)],
        };

        // Per-object reclaim, then the guest's own client-root free (T2's path).
        let mut freed_roots = Vec::new();
        for (pid, client) in order {
            let plan = reclaim_plan(&gpu.procs[&pid], GPU);
            release_on_own_isolate(gpu.procs.get_mut(&pid).expect("live"), GPU, &plan);
            let root = if client == OWNER {
                identical_handles(OWNER_GR.0, OWNER_CE.0).client_root
            } else {
                identical_handles(OTHER_GR.0, OTHER_CE.0).client_root
            };
            gpu.apply(RmEvent::Free {
                client,
                handle: root,
            })
            .unwrap_or_else(|e| panic!("({ord:?}) client-root free: {e:?}"));
            freed_roots.push(pid);

            if ord == Ordering2::ReachDuringTeardown {
                // Retired, not yet reaped: the survivor reaches for the corpse's backing.
                let corpse = if pid == owner { a_backing } else { b_backing };
                assert_eq!(
                    attempt_on_system(
                        &mut gpu,
                        &VerbPlan::Release {
                            unmap: vec![],
                            free: vec![corpse],
                        },
                    ),
                    Err(VerbFailure {
                        err: RmError::ForeignHandle {
                            handle: corpse,
                            worker_isolate: SYSTEM_ISOLATE,
                        },
                        orphans: Orphans::default(),
                    }),
                    "({ord:?}) the mid-teardown reach is refused like any other"
                );
            }
            if ord != Ordering2::BothBeforeReap {
                assert_eq!(
                    gpu.reap_retired().len(),
                    1,
                    "({ord:?}) {pid:?} reaps at its own quiesce point"
                );
            }
        }
        if ord == Ordering2::BothBeforeReap {
            assert_eq!(gpu.reap_retired().len(), 2, "({ord:?}) both reap together");
        }
        assert!(gpu.procs.is_empty(), "({ord:?}) every proc is gone");

        let l = ledger(&rec);
        assert!(
            l.is_balanced(),
            "({ord:?}) conservation failed: {l:?} (leaked {})",
            l.leaked_count()
        );
    }
}
