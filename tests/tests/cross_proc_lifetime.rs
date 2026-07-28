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
//!
//! ## ★★ Section 5 — the reference that OUTLIVES its owner (`l1_concurrency.md` §12.33)
//!
//! Sections 1–4 all ask the *refusal* question: may one component reach another's host
//! objects? (No.) Section 5 asks the **survival** question, which is the other half and
//! was untested: when a kernel client holds a `DUP_OBJECT` of a user process's resource
//! and that process is killed, does the resource stay alive, stay *usable*, and
//! eventually get freed?
//!
//! It is the genuine cross-`Proc` reference, and after §12.27 it is the ONLY one: two
//! *user* clients that share are, by the grouping rule, **the same `Proc`** — user↔user
//! cross-proc sharing does not exist by construction. Kernel↔user does, because a dup
//! into a kernel client is a *reference*, never a merge.
//!
//! Grounding, so this models RM rather than us:
//!
//! - RM keeps a dup'd object alive by refcount — `memCopyConstruct_IMPL` does
//!   `pHwResource->refCount++` / `memdescAddRef` / `DupCount++`
//!   (`ogkm: src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`), and
//!   `clientFreeResource_IMPL` therefore destroys nothing while an alias remains.
//! - A kernel reference **can** outlive the owning process: `uvm_va_space` is bound to
//!   the *file*, not the process (`ogkm: kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`),
//!   and `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` says resources are freed "when the
//!   last reference to the file is dropped rather than when this process exits"
//!   (`ogkm: kernel-open/nvidia-uvm/uvm.h:160-167`).
//!
//! Our model matches **because** attribution is by **origin** (`project.rs`: "a user
//! object dup'd into the UVM session stays in the USER component") and every runtime
//! component — isolate, arena, host VAS — is derived from *live* resources. So the
//! surviving dup keeps the owner's boundary alive, and the *last* reference going is
//! what retires the proc.
//!
//! ### ★ What section 5 FOUND (see `l1_concurrency.md` §12.33)
//!
//! "It stays alive" and "it is eventually freed" are different claims, and they do not
//! both hold:
//!
//! - **Alive and usable: yes**, and at *object granularity*. The dup'd VASpace survives
//!   and accepts new host work; the owner's channels — which nothing dup'd — are
//!   reclaimed **per object**, children before parents, on the owner's own isolate. That
//!   split is RM's refcount made executable.
//! - **Freed at refcount 0: NO — not per object.** The last reference dropping retires
//!   the owner inside the same `Gpu::apply`, and `Spine::refresh` step 3 removes a
//!   vanished component's `Proc` *without* a `sync_proc_to_boundary`, so
//!   `stage_dropped_vases` never even queues its host objects. No `Free` verb is ever
//!   issued for them; their disposition is §7.0 namespace death at the reap. Unlike the
//!   ordinary teardown (where the adapter can reclaim off core state before the guest's
//!   own root-free — see [`the_ledger_balances_across_every_teardown_ordering`]), here
//!   the retiring free arrives through a **foreign client**, so no pre-reclaim window
//!   exists at all. [`the_last_reference_dropping_retires_the_owner_but_frees_nothing_per_object`]
//!   asserts that truth rather than the claim we wanted.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::ClientKey;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Orphans};
use kayfabe_isolate::{HostHandle, IsolateId, RmError, VerbFailure, VerbPlan, VerbReply, WorkerId};
use kayfabe_mocks::{HostLedger, MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_tests::{Guarded, ResidueClaim, Scenario, identical_handles};

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

/// The system isolate's id. `Gpu` spawns every isolate as `IsolateId::new(pid.0, GPU)` and the
/// system proc is `ProcId(0)`, so this is a derived fact, not a guess.
const SYSTEM_ISOLATE: IsolateId = IsolateId::new(0, GPU);

/// Two guest compute processes (`OWNER`, `OTHER`) on GPU0, plus the shared verb
/// recorder that backs the conservation ledger.
fn two_proc_gpu() -> (Guarded<Gpu>, ProcId, ProcId, SharedRecorder) {
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
    (
        Guarded::new("cross_proc_lifetime::two_proc_gpu", gpu, recorder.clone()),
        owner,
        other,
        recorder,
    )
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

/// ★ An address plane in which **nothing** is host-published — this file's
/// [`kayfabe_isolate::RingWorkingSet`] for the one place it must build a ring plan.
///
/// It exists because `VerbPlan::gated_doorbell` will not hand out a `VerbPlan::Doorbell`
/// without an address plane to gate against, and this file's subject is the *foreign
/// handle* gate, not the #14 one. Paired with an **empty** working set it is the honest
/// spelling of "this submission claims no addresses": vacuously gated, and therefore a
/// clean isolation of the second gate. Paired with a non-empty one it refuses
/// everything — which is what `an_ungated_working_set_cannot_become_a_ring_plan` uses it
/// for, in `l1_verb_seam.rs`.
struct NothingPublished;

impl kayfabe_isolate::RingWorkingSet for NothingPublished {
    fn is_host_published(&self, _va: GpuVa) -> bool {
        false
    }
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
        IsolateId::new(owner.0, GPU),
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
            .get(&IsolateId::new(owner.0, GPU))
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
    // ★ The ring plan can no longer be hand-built: `VerbPlan::Doorbell` is
    // `#[non_exhaustive]`, so the struct expression this test used to write is a compile
    // error outside `kayfabe-isolate` (pinned by that crate's `tests/ui/`
    // compile-fail row), and the #14 ring-gate runs inside the only constructor.
    //
    // ★ That makes this assertion STRONGER, not weaker, and it is worth saying why: the
    // plan below has now passed the #14 gate — vacuously, over an empty working set,
    // which is what a submission claiming no addresses legitimately gets — and is
    // *still* refused, by the second, independent gate inside `Worker::execute`. A
    // ring that is #14-clean and namespace-dirty is exactly the case that must refuse,
    // and before this rewrite the test could not distinguish it from a plan that had
    // simply never been gated at all.
    let doorbell = VerbPlan::gated_doorbell(
        &NothingPublished,
        &[],
        None,
        Some((owned, 0x1234)),
        kayfabe_arch::ids::EngineKind::Ce,
        true,
    )
    .expect("an empty working set passes the #14 gate — nothing is claimed");
    assert_eq!(
        attempt_on_system(&mut gpu, &doorbell),
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
    // ★ §12.35 — DECLARED RESIDUE. This proc dies VIOLENTLY (`retire_proc`: its worker
    // HUPped, its component is condemned), so `Proc::retire` stops its isolates at once
    // and the staged release is refused: §12.17's no-resurrect rule outranks per-object
    // reclaim, because a sandbox that just lost a worker must not be handed more verbs.
    // The disposition of record is therefore §7.0 namespace death — the reap drops the
    // isolate and a real one's death frees RM's whole client tree. The clean-death path
    // (`Spine::vacate`) keeps its isolates live and DOES reclaim; the split is deliberate.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId::new(owner.0, GPU),
            "condemned owner: `retire_proc` stops the isolate before the staged release \
             can drain (§12.17 no-resurrect), so its host VAS + backing are disposed of \
             by the session's own death (§7.0)",
        )
        .objects(kayfabe_mocks::VerbKind::AllocVaSpace, 1)
        .objects(kayfabe_mocks::VerbKind::AllocSysmem, 1)
        .maps(1),
    );
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
        gpu.retire_proc(owner),
        "the owner was live when its worker died"
    );
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU, OWNER_PDB),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(ClientKey::first(OWNER))
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
    let condemned_iso = IsolateId::new(owner.0, GPU);
    for (iso, outstanding) in &l.leaked {
        if outstanding.is_empty() {
            continue;
        }
        assert!(
            *iso == condemned_iso || *iso == IsolateId::new(other.0, GPU),
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

    gpu.procs
        .get_mut(&owner)
        .expect("owner")
        .retire()
        .discharge_all();
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
        let device = gpu.map(|g| SharedDevice::new(g, mode));

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
        let gpu = device.map(SharedDevice::into_gpu);
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
    assert!(gpu.retire_proc(other));
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

// =================================================================================
// 5 — ★★ THE KERNEL REFERENCE THAT OUTLIVES ITS OWNER
//     (`l1_concurrency.md` §12.33; the survival half of §12.26/§12.27)
// =================================================================================

/// ★ THE one UVM session client, as measured (§12.27: `GSPALLOC hClient=0xc1d00069
/// processID=0xffffffff`). One per `nvidia_uvm` module load; every guest process dups
/// into it, and it is always the *destination*, never the source.
const UVM: HClient = HClient(0xc1d0_0069);
/// UVM's own device handle.
const UVM_DEV: HObject = HObject(0x9000_0001);
/// UVM's own VASpace handle (the session client allocates address spaces of its own).
const UVM_VAS: HObject = HObject(0x9000_0010);
/// UVM's PDB — its own, distinct from every guest process's.
const UVM_PDB: Pdb = Pdb(0x2efa_6c000);
/// The handle UVM's `DUP_OBJECT` of the owner's VASpace lands on, in UVM's namespace.
const UVM_ALIAS: HObject = HObject(0x9000_00a7);
/// A third guest VA, published *after* the owner is dead — the "still usable" probe.
const VA3: GpuVa = GpuVa(0x2_0040_0000);

/// One guest compute process (`OWNER`) whose VASpace the **kernel/UVM session client**
/// has dup'd — the only cross-`Proc` reference §12.27 leaves in existence.
///
/// Deliberately built with [`Scenario::uvm_dup`] and not [`Scenario::peer_dup`]: the
/// destination's declared [`kayfabe_arch::ClientKind`] is the *entire* difference
/// between a reference and a merge (§12.27), and a `peer_dup` here would produce one
/// `Proc` and test nothing.
fn uvm_referenced_gpu() -> (Guarded<Gpu>, ProcId, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");

    let mut s = Scenario::new();
    let owner_vas = s.compute_process_on_gpu(
        OWNER,
        OWNER_PDB,
        identical_handles(OWNER_GR.0, OWNER_CE.0),
        None,
    );
    s.memory(OWNER, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    s.uvm_dup(
        UVM,
        HObject(UVM.0),
        UVM_DEV,
        UVM_VAS,
        UVM_PDB,
        UVM_ALIAS,
        owner_vas,
    );
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let owner = gpu.spine.by_pdb[&(GPU, OWNER_PDB)];
    (
        Guarded::new(
            "cross_proc_lifetime::uvm_referenced_gpu",
            gpu,
            recorder.clone(),
        ),
        owner,
        recorder,
    )
}

/// The guest kernel frees the dead process's client root — the clean kill (T2's path).
fn free_owner_root(gpu: &mut Gpu) {
    gpu.apply(RmEvent::Free {
        client: OWNER,
        handle: identical_handles(OWNER_GR.0, OWNER_CE.0).client_root,
    })
    .expect("the owner's client root frees");
}

/// Run T0's opportunistic drain for `(pid, GPU)` — the raw-`Gpu` stand-in for the L1
/// shell's `SharedDevice::drain_pending_releases`. Returns how many host objects +
/// mappings were disposed of. Panics if any residue survives the attempt, because a
/// residue here would be a *different* finding wearing this one's clothes.
fn drain_pending(gpu: &mut Gpu, pid: ProcId) -> usize {
    let proc = gpu.procs.get_mut(&pid).expect("a live proc");
    let (worker, orphans) = kayfabe_fwd::checkout_and_drain(proc, GPU).expect("live, materialized");
    let n = orphans.free.len() + orphans.unmap.len();
    let mut worker = worker.expect("a free worker");
    let residue = kayfabe_fwd::dispose_on(&mut worker, orphans);
    assert_eq!(
        residue,
        Orphans::default(),
        "the drain disposed of everything it queued"
    );
    kayfabe_fwd::checkin(proc, GPU, worker);
    n
}

/// Every `Free` the OWNER's isolate issued, in order — the exact-verb half of the
/// reclamation claims below (`frees` is device-wide; this is per-namespace and ordered,
/// because "children before parents" is itself an assertion).
fn frees_on_owner(rec: &SharedRecorder, owner: ProcId) -> Vec<HostHandle> {
    frees(rec)
        .into_iter()
        .filter(|&(iso, _)| iso == IsolateId::new(owner.0, GPU))
        .map(|(_, h)| h)
        .collect()
}

/// ★★ **Steps 1–4: a kernel dup keeps its owner's object alive AND USABLE after the
/// owning guest process is cleanly killed** — and the half nothing referenced is
/// reclaimed per object, in RM's order, right then.
///
/// The existing `rmgraph_order_independence.rs` sibling proves the object is still
/// *present*. Present is not the claim that matters: a reference RM keeps alive is one
/// UVM will keep *using* (`uvm_va_space` outlives the process because it hangs off the
/// file — `ogkm: kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`). So this test
/// **exercises** it: after the owner is dead it publishes a brand-new range through the
/// surviving VASpace and asserts the resulting host verbs really ran, on the owner's own
/// still-live isolate, into the owner's original host VAS.
///
/// The four things it pins, in order:
///
/// 1. the reference is genuinely cross-`Proc` — the kernel client is the **system**
///    component's (§12.27) with its own isolate, and the backing is minted in the
///    OWNER's namespace ([`HostHandle::isolate`]);
/// 2. the owner is killed the clean way (its client root freed);
/// 3. the owner's `Proc`, isolate, arena and `by_pdb` route all survive it — because
///    attribution is by **origin** and the dup'd resource is still live
///    (`ogkm: .../mem_mgr/mem.c:1027-1031`);
/// 4. ★ the surviving reference is **usable**: a fresh `publish_backing` succeeds, and
///    ★ the *unreferenced* half — the owner's channels, which nothing dup'd — is freed
///    **per object**, engine object before channel, so the exec plane now faults with
///    the exact [`FwdFault::UnknownVchid`]. That split is the refcount, executable.
#[test]
fn a_kernel_reference_keeps_its_owners_object_alive_and_usable_after_the_owner_is_killed() {
    let _wd = watchdog("kernel_ref_usable", Duration::from_secs(60));
    let (mut gpu, owner, rec) = uvm_referenced_gpu();

    // ---- 1. The reference is cross-`Proc`, and the backing is the OWNER's. ----
    assert!(
        gpu.system.client_values().contains(&UVM),
        "★ the UVM session client is the SYSTEM component's — a dup INTO a kernel \
         client is a reference, never a merge (§12.27)"
    );
    assert!(
        !gpu.procs[&owner].client_values().contains(&UVM),
        "…so it is emphatically not part of the owner's `Proc`: this is a genuine \
         cross-`Proc` reference and not a relabelled intra-proc one"
    );
    assert_eq!(
        gpu.system.isolate(GPU).expect("system isolate").id(),
        SYSTEM_ISOLATE,
        "the referencing side runs in its own isolate namespace",
    );

    // A full workload, so BOTH planes are host-materialized before the owner dies.
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
        0x1000,
    )
    .expect("the owner publishes a second range");
    kayfabe_fwd::handle_doorbell(&mut gpu, GPU, MockArch::token_for(OWNER_GR), &[VA])
        .expect("the owner rings its GR channel");
    kayfabe_fwd::forward_engine_object(
        &mut gpu,
        GPU,
        OWNER_GR,
        kayfabe_mocks::mock_classes::COMPUTE,
        &[],
    )
    .expect("the owner forwards a compute object");

    let backing = backing_of(&gpu, owner, OWNER_PDB, VA);
    let host_vas = gpu.procs[&owner].vases[&(GPU, OWNER_PDB)]
        .host_vas
        .expect("the owner's Vas materialized its host VAS");
    let arena = gpu.procs[&owner].arenas[&GPU].range.clone();
    let owner_iso = gpu.procs[&owner].isolates[&GPU].id();
    assert_eq!(
        backing.isolate(),
        owner_iso,
        "the backing is minted in the OWNER's RM client namespace, and says so"
    );
    // The host objects the channels own — the half NOTHING references.
    let (host_chan, host_engine) = {
        let ch = gpu.procs[&owner]
            .channels
            .values()
            .find(|c| c.vchid == OWNER_GR)
            .expect("the GR channel materialized");
        (
            ch.host_channel.expect("host channel"),
            *ch.host_engine_objects
                .values()
                .next()
                .expect("the forwarded compute object"),
        )
    };
    assert!(
        frees_on_owner(&rec, owner).is_empty(),
        "precondition: nothing has been freed yet"
    );

    // ---- 2. The guest process is killed, cleanly. ----
    free_owner_root(&mut gpu);

    // ---- 3. …and its `Proc` survives, whole. ----
    assert!(
        gpu.procs.contains_key(&owner),
        "★ the owning `Proc` must NOT retire while a kernel dup still references a \
         resource it allocated — that retire reclaims host memory RM says is live \
         (`ogkm: .../mem_mgr/mem.c:1027-1031`)",
    );
    assert_eq!(
        gpu.procs[&owner].isolates[&GPU].id(),
        owner_iso,
        "same isolate"
    );
    assert_eq!(
        gpu.procs[&owner].arenas[&GPU].range, arena,
        "same GPA arena"
    );
    assert_eq!(
        gpu.spine.by_pdb.get(&(GPU, OWNER_PDB)),
        Some(&owner),
        "the dup-kept VASpace still routes to its ALLOCATOR's proc, not to the \
         referencer's — attribution is by origin",
    );
    assert!(
        !gpu.procs[&owner].is_retired(),
        "and it is live, not a retired husk still sitting in the map"
    );

    // ---- 4a. The unreferenced half is reclaimed PER OBJECT, children first. ----
    assert!(
        gpu.procs[&owner].channels.is_empty(),
        "the owner's channels hung off its client root and nothing dup'd them, so RM's \
         refcount does not keep them: they are gone"
    );
    assert_eq!(
        gpu.procs[&owner].pending_release_len(),
        2,
        "…and T0/G2 staged their host objects for release rather than dropping the \
         handles on the floor (`l1_os_shell.md` §7.6)"
    );
    assert_eq!(drain_pending(&mut gpu, owner), 2, "the drain took both");
    assert_eq!(
        frees_on_owner(&rec, owner),
        vec![host_engine, host_chan],
        "★ the exact `Free` verbs reached the backend, engine object BEFORE channel — \
         RM frees children ahead of parents (`ogkm: .../resserv/src/rs_server.c:963-981`)",
    );

    // The exec plane is genuinely gone, and says which key missed.
    assert_eq!(
        kayfabe_fwd::handle_doorbell(&mut gpu, GPU, MockArch::token_for(OWNER_GR), &[VA]),
        Err(FwdFault::UnknownVchid {
            gpu: GPU,
            vchid: OWNER_GR
        }),
        "a channel the reference does not cover is unreachable — MISS=FAULT, named",
    );

    // ---- 4b. ★ The referenced half is not merely present: it is USABLE. ----
    let (binding, _off) = kayfabe_fwd::resolve(&gpu, GPU, OWNER_PDB, VA)
        .expect("the published range still resolves through the surviving VASpace");
    assert_eq!(
        binding.host.expect("still published").memory,
        backing,
        "the published host backing survived the guest client's free"
    );

    let before = rec.lock().expect("recorder").log.len();
    let fresh = kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA3,
        0x1000,
    )
    .expect("★ NEW host work through a VASpace whose allocating process is DEAD");
    let after: Vec<(IsolateId, RmVerb)> = rec.lock().expect("recorder").log[before..].to_vec();
    let fresh_mem = backing_of(&gpu, owner, OWNER_PDB, VA3);
    assert_eq!(
        after,
        vec![
            (
                owner_iso,
                RmVerb::AllocSysmem {
                    handle: fresh_mem,
                    len: 0x1000
                }
            ),
            (
                owner_iso,
                RmVerb::MapGpuVa {
                    vas: host_vas,
                    memory: fresh_mem,
                    len: 0x1000,
                    va: fresh.host_va
                }
            ),
        ],
        "★★ the surviving reference is EXERCISED, not inspected: the fresh range's \
         host verbs really ran, on the OWNER's still-live isolate, into the OWNER's \
         original host VAS — which is what `uvm_va_space` outliving its process means \
         (`ogkm: .../uvm_va_space_mm.c:75-81`)",
    );
    assert_ne!(
        fresh_mem, backing,
        "a genuinely new object, not a re-report of the old one"
    );

    let l = ledger(&rec);
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "nothing was released twice and nothing was reached across a namespace",
    );
}

/// ★★ **Step 5: the last reference dropping retires the owner AND frees its objects,
/// per object** (`l1_concurrency.md` §12.33 → §12.35 — this is the test §12.33 said
/// would have to change the day someone closed it).
///
/// §12.33 measured the opposite and said so: 2 objects + 1 mapping outstanding, ledger
/// `is_balanced() == false`, *"not one `Free` verb ever issued"*. The cause was
/// structural rather than incidental — `Spine::refresh` step 3 removed a vanished
/// component's `Proc` with `procs.remove(id)` + `Proc::retire()` and **no**
/// `sync_proc_to_boundary`, so `stage_dropped_vases` never ran and the host VAS and its
/// backings were not even *queued*; from that instant the core could not name them, and
/// both downstream doors were already shut (a retired isolate refuses the disposal; the
/// reap runs under the device write lock where R1 forbids a verb).
///
/// §12.35 closed it exactly where §12.33 predicted (*"§12.18 already settles every
/// retirement in `plan_refresh` before a `Proc` is touched, so a 'these procs are about
/// to retire' edge exists to hang a pre-retire drain on"*), by making **removal itself**
/// the central, final step: `decide → stage → drain → remove`.
///
/// - **decide** — `plan_refresh` names `RefreshPlan::vanishing` before any proc is
///   touched;
/// - **stage** — `Spine::vacate` is the one removal site and it runs the ordinary
///   `stage_dropped_vases` / `stage_dropped_channels` first (bookkeeping, so it may run
///   under the lock);
/// - **drain** — `Proc::drop`, lock-free and `is_quiesced`-gated, on isolates that
///   `Proc::vacate` deliberately left **live** (a clean death is not a condemnation);
/// - **remove** — only then does the value fall.
///
/// So this test now asserts the claim §12.33 wanted: refcount 0 ⇒ the `Free` verb for
/// that exact [`HostHandle`] reaches the backend, and the ledger balances. The asymmetry
/// §12.33 named is still real and is still what makes the case interesting — the
/// retiring free arrives through a **foreign client** (UVM's), inside a single
/// `Gpu::apply`, so there is no pre-reclaim window at any *caller*. There does not need
/// to be one: the reclamation is not the caller's any more.
#[test]
fn the_last_reference_dropping_retires_the_owner_and_frees_its_objects_per_object() {
    let _wd = watchdog("last_reference_drops", Duration::from_secs(60));
    let (mut gpu, owner, rec) = uvm_referenced_gpu();
    let owner_iso = IsolateId::new(owner.0, GPU);

    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    kayfabe_fwd::handle_doorbell(&mut gpu, GPU, MockArch::token_for(OWNER_GR), &[VA])
        .expect("the owner rings");
    let backing = backing_of(&gpu, owner, OWNER_PDB, VA);
    let host_vas = gpu.procs[&owner].vases[&(GPU, OWNER_PDB)]
        .host_vas
        .expect("host VAS");
    let arena = gpu.procs[&owner].arenas[&GPU].range.clone();

    // The owner dies; the kernel reference keeps its `Proc` alive (previous test), and
    // the channel half is reclaimed per object at the drain.
    free_owner_root(&mut gpu);
    let reclaimed_with_owner = drain_pending(&mut gpu, owner);
    assert_eq!(
        reclaimed_with_owner, 1,
        "the GR channel's host object was freed at the owner's death (no engine object \
         was forwarded here, so it is the only one)"
    );
    let freed_before = frees_on_owner(&rec, owner);
    assert!(
        !freed_before.contains(&backing) && !freed_before.contains(&host_vas),
        "★ and NOT the dup-referenced half — freeing that would be the C's 2026-06-18 \
         bug, a backing yanked out from under a live kernel reference: {freed_before:?}",
    );

    // ---- The LAST reference goes: UVM releases its dup (`FreeDupedHandle` at
    // `uvm_va_space_destroy`). Refcount 0. ----
    gpu.apply(RmEvent::Free {
        client: UVM,
        handle: UVM_ALIAS,
    })
    .expect("the session frees its dup");

    assert!(
        !gpu.procs.contains_key(&owner),
        "the LAST reference going is what retires the proc"
    );
    assert_eq!(gpu.retired_len(), 1, "retired, awaiting the quiesce point");
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU, OWNER_PDB),
        Err(FwdFault::UnknownPdb {
            gpu: GPU,
            pdb: OWNER_PDB
        }),
        "and its address plane is unroutable — MISS=FAULT, named. `UnknownPdb` and not \
         `Condemned`, deliberately: this proc left through `Spine::refresh` step 3 (its \
         component vanished), which files no condemned entry. A clean death is not a \
         condemnation, and the two must not report as each other (§12.13).",
    );

    let reaped = gpu.reap_retired();
    assert_eq!(
        reaped.len(),
        1,
        "exactly one proc reaped — no phantom churn"
    );
    assert!(
        reaped.orphaned().is_empty(),
        "★ the GPA half IS conserved: the arena routed home to its own window, never \
         dropped (§12.19 G7)"
    );
    drop(reaped);

    // ---- ★★ §12.35: THE CLOSED FINDING. ----
    let mut expected = freed_before.clone();
    expected.extend([backing, host_vas]);
    expected.sort_unstable();
    let mut actually_freed = frees_on_owner(&rec, owner);
    actually_freed.sort_unstable();
    assert_eq!(
        actually_freed, expected,
        "★★ the dup-referenced half IS freed per object at refcount 0 — the backing and \
         the host VAS both, on the owner's own isolate. §12.33 measured `freed_before` \
         here (not one further `Free`); §12.35's `decide → stage → drain → remove` is \
         what changed it.",
    );

    let l = ledger(&rec);
    assert_eq!(
        l.leaked_on(owner_iso),
        std::collections::BTreeSet::new(),
        "★ nothing is left outstanding on the owner's isolate — the §7.0 namespace-death \
         backstop is no longer load-bearing on this path",
    );
    assert_eq!(
        l.leaked_maps
            .get(&owner_iso)
            .map(std::collections::BTreeSet::len)
            .unwrap_or(0),
        0,
        "…and the backing's GPU mapping was unmapped before it was freed (RM's own \
         children-before-parents order, `ogkm: rs_client.c:830-849`)",
    );
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "no double-free, and no isolate ever reached across a namespace — the two \
         failure modes that WOULD be bugs rather than dispositions",
    );
    assert!(
        l.is_balanced(),
        "stated once more, unambiguously: the ledger balances"
    );

    // The arena really did return: a fresh process gets the range back (#80's class).
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        OTHER,
        OTHER_PDB,
        identical_handles(OTHER_GR.0, OTHER_CE.0),
        None,
    );
    for ev in s.events {
        gpu.apply(ev).expect("a fresh process starts");
    }
    let next = gpu.spine.by_pdb[&(GPU, OTHER_PDB)];
    assert_eq!(
        gpu.procs[&next].arenas[&GPU].range, arena,
        "the reaped arena was recycled to the very next process — GPA is conserved even \
         where per-object host reclaim is not",
    );
}

/// ★★ **The violent kill: a CONDEMNED owner is not kept usable by its kernel
/// reference — and does not take the kernel session down with it.**
///
/// The clean kill and the condemnation are different paths and must be tested as such.
/// A condemnation is an out-of-band isolate-worker death (§12.13): the owner's RM client
/// namespace is *gone*, so "the reference keeps it alive" is exactly the claim that must
/// NOT hold — there is nothing left to keep. Answering it any other way would be the
/// resurrect the §12.17 no-resurrect rule refuses.
///
/// Four halves, because three of them are what make it a lifetime proof:
///
/// 1. the component is condemned even though a live kernel reference exists;
/// 2. every use of it faults with the exact [`FwdFault::Condemned`], naming the anchor —
///    not a generic miss, and emphatically not a success;
/// 3. ★ the **UVM session client is NOT condemned** and the system proc keeps serving:
///    one dead guest process must never take down the session every *other* guest
///    process shares (the §12.26 device-fatal argument, from the other side);
/// 4. ★ condemnation clears only when the guest frees the **owner's** client root —
///    freeing the referencer's dup does not clear it — which is the documented recovery
///    (§12.17) and the reason a condemned entry is bounded rather than permanent.
#[test]
fn a_condemned_owner_is_not_kept_usable_by_its_kernel_reference() {
    let _wd = watchdog("condemned_owner_kernel_ref", Duration::from_secs(60));
    let (mut gpu, owner, rec) = uvm_referenced_gpu();
    let owner_iso = IsolateId::new(owner.0, GPU);
    // ★ §12.35 — DECLARED RESIDUE. This proc dies VIOLENTLY (`retire_proc`: its worker
    // HUPped, its component is condemned), so `Proc::retire` stops its isolates at once
    // and the staged release is refused: §12.17's no-resurrect rule outranks per-object
    // reclaim, because a sandbox that just lost a worker must not be handed more verbs.
    // The disposition of record is therefore §7.0 namespace death — the reap drops the
    // isolate and a real one's death frees RM's whole client tree. The clean-death path
    // (`Spine::vacate`) keeps its isolates live and DOES reclaim; the split is deliberate.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId::new(owner.0, GPU),
            "condemned owner: `retire_proc` stops the isolate before the staged release \
             can drain (§12.17 no-resurrect), so its host VAS + backing are disposed of \
             by the session's own death (§7.0)",
        )
        .objects(kayfabe_mocks::VerbKind::AllocVaSpace, 1)
        .objects(kayfabe_mocks::VerbKind::AllocSysmem, 1)
        .maps(1),
    );

    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the owner publishes");
    let backing = backing_of(&gpu, owner, OWNER_PDB, VA);
    let host_vas = gpu.procs[&owner].vases[&(GPU, OWNER_PDB)]
        .host_vas
        .expect("host VAS");

    // ---- 1. Out-of-band worker death: the component is condemned. ----
    assert!(
        gpu.retire_proc(owner),
        "the owner was live when its worker died"
    );
    assert_eq!(gpu.spine.condemned_len(), 1, "one condemned component");
    assert!(
        gpu.spine.is_condemned(OWNER),
        "…and it is the owner's, despite the live kernel reference"
    );

    // ---- 2. Every use faults, with the exact variant. ----
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU, OWNER_PDB),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(ClientKey::first(OWNER))
        }),
        "★ a dup does not resurrect a dead namespace: the reference outlives the \
         process, never the isolate that held the objects (§12.17)",
    );
    assert_eq!(
        kayfabe_fwd::resolve(&gpu, GPU, OWNER_PDB, VA).map(|_| ()),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(ClientKey::first(OWNER))
        }),
        "the data plane says the same thing as the routing plane",
    );

    // ---- 3. The kernel session is untouched and still serving. ----
    // Asserted BEFORE the reap on purpose: the claim is about what condemnation did,
    // and a bite that mis-groups the session must be caught by *this* assertion rather
    // than by some later count that happens to shift with it.
    assert!(
        !gpu.spine.is_condemned(UVM),
        "★★ the UVM session client is NOT dragged into the condemnation — it is the \
         system component's, and it is shared by every OTHER guest process",
    );
    assert!(
        gpu.system.client_values().contains(&UVM),
        "…and it is still a member of the live system component"
    );
    assert!(
        gpu.system
            .isolate(GPU)
            .expect("the system isolate survives")
            .idle_workers()
            > 0,
        "the system proc keeps serving",
    );
    assert_eq!(
        gpu.reap_retired().len(),
        1,
        "exactly ONE corpse reaps — the owner's. The session was never a corpse.",
    );

    // ---- 4. Recovery: the OWNER's root-free clears it; the referencer's does not. ----
    gpu.apply(RmEvent::Free {
        client: UVM,
        handle: UVM_ALIAS,
    })
    .expect("the session releases its dup");
    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "★ releasing the REFERENCE does not clear the condemnation — the entry is keyed \
         on the dead component's clients, and OWNER has not been freed",
    );
    assert!(gpu.spine.is_condemned(OWNER));

    free_owner_root(&mut gpu);
    assert_eq!(
        gpu.spine.condemned_len(),
        0,
        "★ …and the guest freeing the DEAD client's root is what clears it — the \
         documented recovery (§12.17), which is why condemnation is bounded",
    );
    assert!(
        gpu.procs.is_empty(),
        "nothing was resurrected on the way out"
    );

    // The ledger: namespace death is the disposition, and nothing worse happened.
    let l = ledger(&rec);
    assert_eq!(
        l.leaked_on(owner_iso),
        std::collections::BTreeSet::from([host_vas, backing]),
        "the condemned isolate's own objects are the §7.0 residue — that is namespace \
         death, stated rather than papered over",
    );
    assert_eq!(
        l.leaked_on(SYSTEM_ISOLATE),
        std::collections::BTreeSet::new(),
        "and the referencing isolate owns nothing at all: the system proc has no data \
         plane, so a reference can never make it the owner of host memory",
    );
}

// =================================================================================
// ★★★ Section 6 — the reference that outlives the guest KERNEL's namespace
// (`l1_concurrency.md` §12.39, finding 1)
//
// Section 5 is the user→kernel direction: a kernel client references a user process's
// resource, the process dies, and the owning `Proc` must SURVIVE because RM's refcount
// says the resource is live. This is the mirror, and the mirror is where the
// classification stops being free: the surviving resource's origin namespace is the
// guest KERNEL's, and the projection had nothing to read once its root was gone.
// `anchor_of` answered "not kernel", i.e. **user**, and minted the guest kernel a real
// user data plane — the exact state `FwdFault::SystemDataPlane` exists to forbid.
// =================================================================================

/// The user handle the guest process aliases the kernel's VASpace under.
const KERNEL_ALIAS: HObject = HObject(0x7a00_0001);

/// ★★ **An orphaned resource of the guest KERNEL's namespace stays in the SYSTEM
/// component** (`l1_concurrency.md` §12.39, finding 1).
///
/// The setup is section 5's, reflected: the guest kernel (UVM's session client — a
/// declared [`kayfabe_arch::ClientKind::Kernel`]) allocates a VASpace and binds a page
/// directory to it; a **user** process then `DUP_OBJECT`s that VASpace into its own
/// namespace; and the guest kernel's client root is freed while the alias lives. RM's
/// refcount keeps the VASpace alive (`ogkm: .../mem_mgr/mem.c:1027-1031`), so it is still
/// reported at its origin `(UVM, UVM_VAS)` and still mints its component's boundary —
/// which is correct and is section 5's own claim.
///
/// What was not correct is **which side of §12.27's line that component landed on**.
/// `project`'s `is_kernel` read `RmGraph::client_kinds`, which only knows namespaces with
/// a **live root**; with the root freed the answer was an absence, `anchor_of` filed the
/// namespace as *not kernel* — i.e. a **user** component — and the spine handed the guest
/// kernel's own PDB a live user `Proc`: an isolate, a GPA arena and a routable `Vas` it
/// can publish host memory into. That is the guest-kernel-obtains-a-user-data-plane shape
/// this whole file's rule 2 exists to refuse, reached by omission rather than by a
/// declaration — the same "absence read as user" defect §12.27 removed from the grouping
/// predicate, still live in the assignment pass.
///
/// The fix is a **recorded fact, not a filter**: every resource carries the
/// [`kayfabe_arch::ClientKind`] its allocating namespace declared
/// (`RmGraph::client_declarations`), so an orphan is classified by what the guest said
/// about itself. Filtering the orphan out instead would retire a `Proc` RM says is live —
/// section 5's finding, in the other direction.
///
/// The assertions lead with the **breach** (a user `Proc` for a kernel namespace, and
/// host memory minted into it), and only then state the mechanism.
#[test]
fn an_orphaned_kernel_resource_never_becomes_a_user_data_plane() {
    let _wd = watchdog("kernel_orphan_no_user_plane", Duration::from_secs(30));
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Guarded::new(
        "cross_proc_lifetime::kernel_orphan",
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes"),
        rec.clone(),
    );

    // The guest kernel's session client, with a VASpace of its own, and a user process
    // that aliases it. `uvm_dup` builds exactly this pair — kernel root, kernel device,
    // kernel VASpace + PDB, then the dup — so the shape is the measured one.
    let mut s = Scenario::new();
    let owner_vas = s.compute_process_on_gpu(
        OWNER,
        OWNER_PDB,
        identical_handles(OWNER_GR.0, OWNER_CE.0),
        None,
    );
    s.uvm_dup(
        UVM,
        HObject(UVM.0),
        UVM_DEV,
        UVM_VAS,
        UVM_PDB,
        UVM_ALIAS,
        owner_vas,
    );
    // ★ The mirror edge: the USER process aliases the KERNEL's VASpace. A dup whose src
    // is a kernel client merges nothing (§12.27), so this is a pure reference — and it is
    // what keeps the kernel's VASpace alive past its own root's free.
    s.push(RmEvent::Dup {
        src: kayfabe_core::rmgraph::NodeKey::new(UVM, UVM_VAS),
        dst: kayfabe_core::rmgraph::NodeKey::new(OWNER, KERNEL_ALIAS),
    });
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    assert!(
        gpu.system.client_values().contains(&UVM),
        "precondition: while its root is live the session client IS the system component's"
    );
    let user_procs_before = gpu.procs.len();
    let planes_before = live_planes(&gpu);

    // ---- The guest kernel's client root is freed while the user's alias lives.
    gpu.apply(RmEvent::Free {
        client: UVM,
        handle: HObject(UVM.0),
    })
    .expect("the session's client root frees");
    assert!(
        gpu.spine
            .rmgraph
            .origin_of(kayfabe_core::rmgraph::NodeKey::new(OWNER, KERNEL_ALIAS))
            .is_some(),
        "precondition: RM's refcount keeps the kernel's VASpace alive through the alias",
    );

    // ---- ★★ THE BREACH, asserted first.
    assert_eq!(
        gpu.procs.len(),
        user_procs_before,
        "★★ a USER `Proc` was minted for the guest KERNEL's namespace — the guest kernel \
         obtained a live user component (isolate + GPA arena + routable `Vas`) simply by \
         having one of its resources outlive its client root",
    );
    assert_eq!(
        live_planes(&gpu),
        planes_before,
        "★★ …and it was handed a data plane of its own: an isolate (a host RM client \
         namespace) plus a GPA arena, for a namespace the guest kernel has closed",
    );
    assert_eq!(
        gpu.spine.by_pdb.get(&(GPU, UVM_PDB)),
        Some(&Gpu::SYSTEM_PROC),
        "★★ the guest kernel's own PDB must route to the SYSTEM proc, which has no data \
         plane — routing it to a user proc is what makes the plane publishable",
    );
    assert!(
        gpu.system.client_values().contains(&UVM),
        "the orphaned namespace stays on the KERNEL side of §12.27's line: it is a fact \
         the guest DECLARED about itself, not something the absence of a root revokes",
    );
    assert!(
        gpu.procs
            .values()
            .all(|p| !p.client_values().contains(&UVM)),
        "★★ no user component may hold the guest kernel's client"
    );

    // ---- …and host memory genuinely cannot be minted for it, by name.
    assert_eq!(
        kayfabe_fwd::publish_backing(&mut gpu.system, GPU, UVM_PDB, VA3, 0x1000),
        Err(FwdFault::SystemDataPlane),
        "★★ the surviving kernel VASpace must refuse to mint host memory — `SystemDataPlane`, \
         not `UnknownPdb`: the refusal is about WHO owns the plane",
    );

    // ---- The user side is untouched: its own plane still works, in its own lane.
    let owner = gpu.spine.by_pdb[&(GPU, OWNER_PDB)];
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
        0x1000,
    )
    .expect("the user process is undisturbed by the kernel namespace's death");
    assert!(
        !gpu.procs[&owner].client_values().contains(&UVM),
        "and it did not absorb the dead kernel namespace"
    );

    let l = ledger(&rec);
    assert_eq!(
        l.leaked_on(SYSTEM_ISOLATE),
        std::collections::BTreeSet::new(),
        "the system isolate owns nothing, before or after the orphaning",
    );
    // T0: the user proc's own publication is still staged/reachable, so the guard is
    // satisfied by core state alone — nothing here is a declared residue.
    kayfabe_tests::unpublish_and_release(
        gpu.procs.get_mut(&owner).expect("owner"),
        GPU,
        OWNER_PDB,
        VA,
    )
    .expect("the user process releases what it published");
}

/// Every `(Proc, GpuId)` data plane the device currently holds live — one isolate + one
/// GPA arena per pair (MG-5). The count a phantom component grows.
fn live_planes(gpu: &Gpu) -> (usize, usize) {
    let isolates =
        gpu.system.isolates.len() + gpu.procs.values().map(|p| p.isolates.len()).sum::<usize>();
    let arenas =
        gpu.system.arenas.len() + gpu.procs.values().map(|p| p.arenas.len()).sum::<usize>();
    (isolates, arenas)
}
