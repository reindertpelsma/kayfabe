//! # nvkvm-fwd — intent recovery → unprivileged host ops
//!
//! The forwarding plane (`mode2_rust_rewrite_architecture.md` §4.2, lesson L2):
//! translate what the guest *means* into **unprivileged** host userspace operations
//! through the owning `Proc`'s isolate — never replay privileged GSP-internal
//! controls. Correctness = observable end-states only.
//!
//! ## Implemented this milestone (the pure-logic slice the simulation drives)
//!
//! - [`publish_backing`] — the data-plane materialization path: back a guest VA range
//!   in a specific **`Vas`** (never "the process" — address ops key on PDB, decision
//!   #14) with a fresh GPA-arena allocation + a host mapping in *that Vas's own host
//!   VAS*. This function IS the #14 fix in code: two procs' identical guest VAs run
//!   through disjoint arenas and disjoint host VASes by construction.
//! - [`handle_doorbell`] — **the ONE ring path** (there is no other function that
//!   reaches `RmBackend::ring_doorbell`): `Arch::decode_doorbell` → vChid →
//!   `by_vchid` → `(Proc, Channel)` → **the #14 ring-gate** (the channel's Vas
//!   working set must be host-published — structural, not caller discipline) →
//!   materialize/schedule that channel on **its proc's own** exec plane (nothing
//!   one-shot, nothing scalar — crack ⚠4) → ring its host token on **its proc's
//!   own** isolate.
//! - [`deliver_completions`] / [`poll_completions`] — glue from the core's completion
//!   policy to `Vmm::raise_irq` (the SWGEN0 edge; transport encoding is `nvkvm-gsp`'s
//!   job once it ports).
//!
//! ## Ports here later (documented skeleton)
//!
//! The ONE pushbuffer method parser (SEM_EXECUTE / MEM_OP / LAUNCH_DMA — address-table
//! §7), the Case-1 shadow-forwarding / Case-2 ack-only tables, CE PT-write capture
//! feed (#13), channel/TSG lifecycle. Each arrives with its regression tests
//! (testing strategy §2).
//!
//! ## Concurrency (decision #17) — the route/act split (L1 cardinal rule R4)
//!
//! This crate is stateless — free functions over the core's types, so the
//! concurrency contract is inherited verbatim from `nvkvm-core` (see its crate
//! docs). Every mixed entry point is factored into the shape the L1 sharding
//! design requires (`l1_concurrency.md` §3.4):
//!
//! - **route** — a pure read of the device-global [`Spine`] (`&Spine`: token
//!   decode, `by_vchid`/`by_pdb` lookup, arch tables). Runs under L1's device
//!   *read* lock; touches no proc.
//! - **act** — the mutation of exactly the routed target (`&mut Proc`, plus
//!   `&Spine` where the act needs routing tables). Runs under that proc's `Mutex`.
//!   ★ Since stage 3 the act phase itself splits into **plan / execute / commit**
//!   (R1): the locked phases only read/decide and re-validate; every `RmBackend`
//!   verb runs between them, lock-free, on a checked-out [`nvkvm_isolate::Worker`].
//! - The original `&mut Gpu` entry points remain as **split-borrow compositions**
//!   of route+act — the single-threaded / degenerate-one-lock shape the tests and
//!   L1-M1 drive.
//!
//! Functions taking `&Gpu`/`&Spine`/`&Proc` are concurrent-safe under shared
//! borrows; functions taking `&mut` require caller-provided exclusivity — and the
//! `&mut Proc` ones ([`publish_backing`], the act phases) parallelize per-proc
//! (disjoint borrows, no shared lock).

use nvkvm_arch::Aperture;
use nvkvm_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use nvkvm_completion::{CompletionError, OsEventRef, PostBatch};
use nvkvm_core::gpu::{Channel, Gpu, Proc, Spine};
use nvkvm_core::{ChanId, ProcId};
use nvkvm_isolate::{ChannelHandles, HostHandle, RmError, VerbPlan, VerbReply, Worker};
use nvkvm_mmu::AddressTable;
use nvkvm_mmu::{AddressFault, Binding};
use nvkvm_vmm::{FbMeta, IrqSpec, Present, PresentError, SurfaceHandle, Vmm};

/// The MSI-X vector completions are raised on. Abstract placeholder until the
/// interrupt-tree model ports (`nvkvm-regs`-equivalent); the mocks assert on it.
pub const COMPLETION_VECTOR: IrqSpec = IrqSpec::Msix(0);

/// Forwarding-plane faults. Loud by design: a routing miss is never resolved by
/// guessing (no content-pick, no MRU scan — those do not exist in the rewrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FwdFault {
    /// The doorbell token did not decode for this architecture (hostile bytes).
    MalformedToken {
        /// The raw token.
        token: u64,
    },
    /// The decoded vChid has no registered channel **on this target GPU** — forward-
    /// population never saw its channel-alloc. MISS=FAULT (the C's `bar1_wpg` MRU
    /// fallback is designed out). Carries its target: a `VChid` is a per-GPU namespace,
    /// so the miss is scoped to the GPU whose doorbell trapped (MG-3).
    UnknownVchid {
        /// The target GPU the doorbell addressed.
        gpu: GpuId,
        /// The decoded vChid.
        vchid: VChid,
    },
    /// The routed proc exists but is retired (cross-teardown consumption refused —
    /// lesson L10).
    RetiredProc(ProcId),
    /// The channel is not bound to any declared VAS and system routing does not
    /// apply — refusing to guess an address space.
    NoVas(ChanId),
    /// The target proc has no `Vas` for this `(GpuId, PDB)` (data-plane routing miss).
    /// Carries its target: a `Pdb` is a per-GPU namespace (MG-3).
    UnknownPdb {
        /// The target GPU.
        gpu: GpuId,
        /// The PDB that missed on that target.
        pdb: Pdb,
    },
    /// A per-`(Proc, GpuId)` host isolate/arena was not materialized for an op's
    /// target GPU (an internal inconsistency — the derivation ensures one per touched
    /// target). Loud, never a silent cross-GPU reach.
    NoTarget {
        /// The proc.
        proc: ProcId,
        /// The target GPU with no materialized isolate/arena.
        gpu: GpuId,
    },
    /// The address table refused (miss/overlap).
    Address(AddressFault),
    /// The proc's GPA arena is exhausted.
    Arena,
    /// Reading guest memory failed while parsing a pushbuffer (`Vmm::gpa_read`
    /// refused a GPFIFO range). Distinct from [`FwdFault::Arena`] by design: this is
    /// a guest-side read failure, not a host arena-exhaustion condition.
    GpaRead {
        /// The guest-physical address the refused read started at.
        gpa: u64,
    },
    /// The isolate's RM backend refused the op.
    Rm(RmError),
    /// A class the guest tried to alloc as an engine object is not one this arch
    /// recognizes as an engine — MISS=FAULT (never guessed into a GR/CE object).
    NotAnEngine(ClassId),
    /// A completion-arm operation was issued on a channel whose [`EngineKind`]
    /// signals through a *different* arm (e.g. arming a mapped fence on a
    /// GR-compute channel, whose completion is the shared-sema arm). The channel's
    /// engine kind selects the arm — exact, never guessed (§2.4's tie-in).
    WrongArm {
        /// The channel the operation targeted.
        chan: ChanId,
        /// Its engine kind (which selects a different arm).
        engine: EngineKind,
    },
    /// The present/display sink refused a GR-graphics scanout.
    Present(PresentError),
    /// The owning proc's completion queue is full — a hostile guest triggered more
    /// completions than it drained. Loud-fault, never unbounded growth (boundary-1).
    Completion(CompletionError),
    /// Every worker in the `(proc, gpu)` isolate's **bounded pool** is in flight
    /// (`l1_concurrency.md` §7.2). This is **backpressure, not failure**: an L1
    /// caller that can wait releases ALL its locks, waits for a return, and re-enters
    /// from the top with full R5 re-validation. It surfaces as a fault only to
    /// callers that chose not to wait (the single-threaded composed entry points).
    PoolSaturated {
        /// The proc whose pool is saturated.
        proc: ProcId,
        /// The target GPU whose isolate pool is saturated.
        gpu: GpuId,
    },
    /// ★ **R5**: the world moved while a verb was in flight lock-free, so the commit
    /// phase's target is no longer what the plan named. MISS=FAULT extends to
    /// staleness — the op surfaces this refusal and does **not** "finish what it
    /// started" against a world that no longer contains its target
    /// (`l1_concurrency.md` §3.3 R5, §11 B5).
    Stale(Stale),
}

/// Which re-validation a commit phase failed (`FwdFault::Stale`). Each variant is a
/// distinct way the world can move across the lock-free verb gap; naming them apart
/// is what makes the §8.4 staleness canaries assert something specific instead of
/// "an error happened".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stale {
    /// The proc retired (or was reaped) while its verb was in flight.
    Proc(ProcId),
    /// The channel the plan targeted was torn down.
    Channel(ChanId),
    /// The `Vas` the plan targeted is gone.
    Vas {
        /// Its target GPU.
        gpu: GpuId,
        /// Its PDB.
        pdb: Pdb,
    },
    /// An `apply`/refresh rewrote routing: `(gpu, vchid)` no longer resolves to the
    /// `(proc, chan)` the plan was made against.
    Route {
        /// The target GPU.
        gpu: GpuId,
        /// The vChid whose route moved.
        vchid: VChid,
    },
    /// The commit's target adopted DIFFERENT host state while this verb was in
    /// flight (a sibling thread's commit won the race). Adopting ours on top would
    /// silently orphan theirs, so the loser refuses and releases what it allocated.
    Rebound,
    /// The `(proc, gpu)` isolate/arena the plan named is gone.
    Target {
        /// The proc.
        proc: ProcId,
        /// The target GPU.
        gpu: GpuId,
    },
}

impl From<CompletionError> for FwdFault {
    fn from(e: CompletionError) -> Self {
        FwdFault::Completion(e)
    }
}

impl From<AddressFault> for FwdFault {
    fn from(f: AddressFault) -> Self {
        FwdFault::Address(f)
    }
}
impl From<RmError> for FwdFault {
    fn from(e: RmError) -> Self {
        FwdFault::Rm(e)
    }
}

// =================================================================================
// ★ THE PLAN / EXECUTE / COMMIT SEAM (`l1_concurrency.md` §3.3, R1's "consequence
// for the core shape"; stage 3 closing the §12.6 gap).
//
// A verb-issuing act phase runs under the owning proc's lock, so it can no longer
// call a blocking `RmBackend` verb in line. Every such site is split in three:
//
//   plan    — under device-read + proc lock: read core state, decide, and EMIT a
//             typed `VerbPlan` plus the ID-shaped hints the commit will need.
//             Emits; does not call. Takes `&Proc` (a pure read) wherever it can.
//   execute — NO locks held: `Worker::execute` runs the chain on a checked-out
//             worker, chaining its own intermediate results (host VAS handle →
//             memory handle → mapped VA) with zero core access. That door asserts
//             R1, so this phase cannot be run under a lock even by accident.
//   commit  — locks re-acquired: RE-VALIDATE (R5) by re-resolving through IDs, then
//             apply the reply to core state — or refuse loudly and hand back the
//             host objects it could not adopt.
//
// Plan products are IDs, never held references (R5's enforcement note), so a commit
// physically cannot dereference something the gap freed. The composed `&mut Proc` /
// `&mut Gpu` entry points below remain, now as *compositions* of the three phases
// that run the round trip on a checked-out worker with no lock held — which is why
// calling one under a lock is an immediate R1 panic instead of a silent violation.
// =================================================================================

/// A locked plan phase's product: the ID-shaped hints its commit needs, plus the
/// verb chain to run lock-free. `verbs = None` means **no host work at all** — the
/// site resolved entirely from core state (an idempotent engine-object replay), so
/// no worker is checked out and the pool is never touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned<P> {
    /// The ID-shaped plan the commit re-validates against.
    pub plan: P,
    /// The verb chain, or `None` when the site needs no host work.
    pub verbs: Option<VerbPlan>,
}

/// Host objects a **refused commit** could not adopt.
///
/// R5's disposition rule made explicit: a commit that refuses must not silently leak
/// what its execute phase already allocated. The caller runs
/// [`Orphans::release_plan`] on the SAME worker, still lock-free, before checking it
/// back in. (The one case with no such caller is a proc that vanished entirely — then
/// the whole isolate is retired and its handle namespace dies with it, which is the
/// retire path owning the disposition instead. Both dispositions are decided, neither
/// is a leak.)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Orphans {
    /// `(host VAS, host GPU VA)` mappings to undo first.
    pub unmap: Vec<(HostHandle, u64)>,
    /// Objects to free.
    pub free: Vec<HostHandle>,
}

impl Orphans {
    /// True if there is nothing to dispose of.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unmap.is_empty() && self.free.is_empty()
    }

    /// The verb chain that disposes of these orphans.
    #[must_use]
    pub fn release_plan(&self) -> VerbPlan {
        VerbPlan::Release {
            unmap: self.unmap.clone(),
            free: self.free.clone(),
        }
    }
}

/// A commit phase's loud refusal: why, what it could not adopt, and whether the op
/// should be re-planned from the top.
///
/// ★ **The two shapes of "the world moved" (a stage-3 finding, `l1_concurrency.md`
/// §12.9).** R5 says a commit whose target vanished must refuse. But not every
/// staleness is a vanishing: first-touch materialization (host VAS, host channel,
/// engine object) is a **compare-and-swap** across the lock-free gap, and two sibling
/// threads of ONE proc racing it is the ordinary case, not an error. The loser has
/// nothing wrong with its request — someone else simply did the work it wanted —
/// so it must **re-resolve**: release its duplicate and re-plan against the winner's
/// state. Refusing there would turn a legal concurrent submission into a spurious
/// guest-visible fault, which is a worse bug than the one R5 prevents.
///
/// So: `retry = true` ⇒ *converging* staleness (re-plan, bounded); `retry = false` ⇒
/// *divergent* staleness (the target is gone — MISS=FAULT, surface it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The fault to surface to the guest (if this is not retried).
    pub fault: FwdFault,
    /// Host objects the caller must release (see [`Orphans`]).
    pub orphans: Orphans,
    /// True if re-planning from the top is the correct resolution (see above).
    pub retry: bool,
}

impl Refusal {
    /// A divergent refusal with nothing to dispose of: the target is gone.
    fn bare(fault: FwdFault) -> Self {
        Refusal {
            fault,
            orphans: Orphans::default(),
            retry: false,
        }
    }
}

/// The reply shape did not match the plan that produced it — an internal wiring
/// error (the adapter handed a commit someone else's reply), never guest-reachable.
fn wrong_reply<T>(what: &str) -> Result<T, Refusal> {
    panic!("commit phase received a {what} reply that does not match its plan")
}

/// ★ Check a worker OUT of `proc`'s isolate for `gpu` — pool bookkeeping, run under
/// the proc lock (`l1_concurrency.md` §7.3). Moves the worker's handle out to the
/// calling thread; the round trip then runs with no lock held.
///
/// `Ok(None)` is **backpressure**: every worker is in flight (or the isolate is
/// retiring and refuses new checkouts). The caller releases all locks, waits, and
/// re-enters from the top — never spins, never waits under a lock.
pub fn checkout(proc: &mut Proc, gpu: GpuId) -> Result<Option<Worker>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    Ok(proc
        .isolates
        .get_mut(&gpu)
        .ok_or(FwdFault::NoTarget { proc: pid, gpu })?
        .checkout())
}

/// Return a checked-out worker to its pool slot (proc lock; §7.3). If the target
/// isolate is gone the worker is dropped with it — a retired isolate's slots are not
/// resurrected.
pub fn checkin(proc: &mut Proc, gpu: GpuId, worker: Worker) {
    if let Some(iso) = proc.isolates.get_mut(&gpu) {
        iso.checkin(worker);
    }
}

/// The single-threaded composition of the three phases, used by the `&mut Proc`
/// entry points: check a worker out, run the chain **with no lock held**, commit,
/// dispose of any orphans, check the worker back in.
///
/// L1's `SharedDevice` deliberately does NOT call this — it interleaves the same
/// three phases with lock acquire/release and a pool-full wait. This form exists for
/// callers that already hold exclusive `&mut Proc` (tests, bring-up, the degenerate
/// single-threaded shape), and it inherits R1's teeth for free: it reaches the
/// backend through [`Worker::execute`], which panics if any lock is held.
fn round_trip<T>(
    proc: &mut Proc,
    gpu: GpuId,
    planned_verbs: Option<VerbPlan>,
    commit: impl FnOnce(&mut Proc, Option<VerbReply>) -> Result<T, Refusal>,
) -> Result<T, FwdFault> {
    let Some(verbs) = planned_verbs else {
        return commit(proc, None).map_err(|r| r.fault);
    };
    let pid = proc.id;
    let Some(mut worker) = checkout(proc, gpu)? else {
        return Err(FwdFault::PoolSaturated { proc: pid, gpu });
    };
    let executed = worker.execute(&verbs);
    let out = match executed {
        Ok(reply) => commit(proc, Some(reply)).map_err(|r| {
            if !r.orphans.is_empty() {
                let _ = worker.execute(&r.orphans.release_plan());
            }
            r.fault
        }),
        Err(e) => Err(FwdFault::Rm(e)),
    };
    checkin(proc, gpu, worker);
    out
}

/// Result of one backing publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// The GPA carved from the proc's private arena.
    pub gpa: u64,
    /// The host GPU VA inside this Vas's own host VAS.
    pub host_va: u64,
}

/// Back `[va, va+len)` in the `Vas` identified by `(gpu, pdb)` inside `proc`:
/// carve GPA from the proc's **per-target** arena, allocate host memory + map it into
/// the Vas's own host VAS via the proc's **per-target** isolate, and forward-populate
/// the address table.
///
/// Keying discipline (decision #14 + MG-3/MG-5): the caller routes here via
/// `Gpu::by_pdb[(gpu, pdb)]`; a `Pdb` is a per-GPU namespace, so the target GPU is
/// part of the address op's identity. The `Proc` owns one arena + isolate PER target
/// (a bug on GPU0 cannot reach GPU1's host handles).
pub fn publish_backing(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<Published, FwdFault> {
    let planned = plan_publish(proc, gpu, pdb, va, len)?;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_publish(proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_publish`] re-validates against. Identities only —
/// never a held reference into core state (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishPlan {
    /// The owning proc.
    pub proc: ProcId,
    /// The target GPU (isolate + arena key).
    pub gpu: GpuId,
    /// The `Vas`'s PDB.
    pub pdb: Pdb,
    /// The guest VA being backed.
    pub va: GpuVa,
    /// Length.
    pub len: u64,
    /// The `Vas`'s host VAS **as observed at plan time** — `None` means the chain
    /// allocates one, and the commit must refuse if someone else materialized one in
    /// the gap (Stale::Rebound) rather than orphaning theirs.
    pub host_vas: Option<HostHandle>,
}

/// PLAN (R1): decide `publish_backing`'s host work from core state and emit it.
/// A pure `&Proc` read — nothing is mutated until the commit.
pub fn plan_publish(
    proc: &Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<Planned<PublishPlan>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    let vas = proc
        .vases
        .get(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    // The arena and the isolate must both exist BEFORE any host verb runs: a target
    // miss is an internal inconsistency, and finding it after the allocs would mean
    // allocating host state for a target we then refuse.
    if !proc.arenas.contains_key(&gpu) || !proc.isolates.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    let host_vas = vas.host_vas;
    Ok(Planned {
        plan: PublishPlan {
            proc: pid,
            gpu,
            pdb,
            va,
            len,
            host_vas,
        },
        verbs: Some(VerbPlan::Publish { host_vas, len }),
    })
}

/// COMMIT (R5): re-resolve everything through IDs and apply the reply — carve the
/// GPA from the proc's own arena and forward-populate the address table — or refuse
/// loudly and hand back what could not be adopted.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Published`] its plan asked for (an adapter
/// wiring error, never guest-reachable).
pub fn commit_publish(
    proc: &mut Proc,
    plan: &PublishPlan,
    reply: Option<VerbReply>,
) -> Result<Published, Refusal> {
    let Some(VerbReply::Published {
        host_vas: fresh_vas,
        memory,
        host_va,
    }) = reply
    else {
        return wrong_reply("publish");
    };
    // Everything this commit could fail to adopt, in release order.
    let orphans = |vas_used: HostHandle, with_vas: Option<HostHandle>| Orphans {
        unmap: vec![(vas_used, host_va)],
        free: with_vas.into_iter().chain([memory]).collect(),
    };
    let vas_used = fresh_vas
        .or(plan.host_vas)
        .expect("chain produced a host VAS");

    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Proc(plan.proc)),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    }
    let pid = proc.id;
    let Proc { vases, arenas, .. } = proc;
    let Some(vas) = vases.get_mut(&(plan.gpu, plan.pdb)) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Vas {
                gpu: plan.gpu,
                pdb: plan.pdb,
            }),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    };
    // R5 on the host VAS itself: the plan decided whether to allocate one by reading
    // `vas.host_vas`; if that answer changed in the gap, a sibling thread won and our
    // fresh VAS (plus everything mapped into it) is an orphan.
    match (plan.host_vas, fresh_vas) {
        (None, Some(fresh)) => {
            if vas.host_vas.is_some() {
                // Converging: a sibling materialized this Vas's host VAS first. Free
                // ours and re-plan — the retry maps into the winner's VAS.
                return Err(Refusal {
                    fault: FwdFault::Stale(Stale::Rebound),
                    orphans: orphans(fresh, Some(fresh)),
                    retry: true,
                });
            }
            vas.host_vas = Some(fresh);
        }
        (Some(known), None) => {
            if vas.host_vas != Some(known) {
                return Err(Refusal {
                    fault: FwdFault::Stale(Stale::Rebound),
                    orphans: orphans(known, None),
                    retry: true,
                });
            }
        }
        _ => unreachable!("the publish chain allocates a host VAS iff the plan had none"),
    }
    let Some(arena) = arenas.get_mut(&plan.gpu) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Target {
                proc: pid,
                gpu: plan.gpu,
            }),
            orphans: orphans(vas_used, None),
            retry: false,
        });
    };
    let gpa = arena.alloc(plan.len, 0x1000).map_err(|_| Refusal {
        fault: FwdFault::Arena,
        orphans: orphans(vas_used, None),
        retry: false,
    })?;
    vas.table
        .bind(
            plan.pdb,
            plan.va,
            plan.len,
            Binding {
                phys: gpa.0,
                aperture: Aperture::SysmemCoherent,
                host_va: Some(host_va),
            },
        )
        .map_err(|e| Refusal {
            fault: FwdFault::Address(e),
            orphans: orphans(vas_used, None),
            retry: false,
        })?;
    Ok(Published {
        gpa: gpa.0,
        host_va,
    })
}

/// ROUTE: which proc owns `(target, pdb)`? A pure spine read (`by_pdb`) — the
/// data-plane routing half of the route/act split. MISS=FAULT.
pub fn route_pdb(spine: &Spine, target: GpuId, pdb: Pdb) -> Result<ProcId, FwdFault> {
    spine
        .by_pdb
        .get(&(target, pdb))
        .copied()
        .ok_or(FwdFault::UnknownPdb { gpu: target, pdb })
}

/// Resolve `va` in `proc`'s `Vas` identified by `(target, pdb)` — the per-proc
/// read half of [`resolve`] (L1: device read lock + that proc's lock). Pure
/// lookup; MISS=FAULT.
pub fn resolve_in(
    proc: &Proc,
    target: GpuId,
    pdb: Pdb,
    va: GpuVa,
) -> Result<(Binding, u64), FwdFault> {
    let vas = proc
        .vases
        .get(&(target, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: target, pdb })?;
    Ok(vas.table.resolve(pdb, va)?)
}

/// Resolve `va` in the `Vas` identified by `(gpu, pdb)`. Pure lookup; MISS=FAULT.
/// Composition of [`route_pdb`] + [`resolve_in`].
pub fn resolve(gpu: &Gpu, target: GpuId, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), FwdFault> {
    let pid = route_pdb(&gpu.spine, target, pdb)?;
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    resolve_in(proc, target, pdb, va)
}

/// Outcome of a doorbell dispatch, for assertions and tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellOutcome {
    /// The proc the token routed to.
    pub proc: ProcId,
    /// The channel it routed to.
    pub chan: ChanId,
    /// The host token that was rung.
    pub host_token: u64,
    /// True if this dispatch had to schedule the channel first (first submission).
    pub scheduled_now: bool,
}

/// Check every VA in `working_set` resolves **host-published** in `table` — the
/// #14 gate condition. Bound-but-unpublished (`host_va = None`, the exact #14
/// EXECUTION fault: the shadow had it, the host VAS did not) and unbound are both
/// loud faults, never a guess (`execution_plane.md` §2.4).
fn gate_vas(
    table: &AddressTable,
    pdb: Pdb,
    working_set: impl IntoIterator<Item = GpuVa>,
) -> Result<(), FwdFault> {
    for va in working_set {
        let (binding, _off) = table.resolve(pdb, va)?; // MISS=FAULT
        if binding.host_va.is_none() {
            return Err(FwdFault::Address(AddressFault::Miss { pdb, va }));
        }
    }
    Ok(())
}

/// A routed doorbell: everything the act phase needs, resolved by a pure spine
/// read (the ROUTE half of L1 cardinal rule R4). Carries the routing identities so
/// act-phase faults name the same `(GpuId, VChid)` the trap addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellRoute {
    /// The owning proc the token routed to.
    pub proc: ProcId,
    /// The channel it routed to.
    pub chan: ChanId,
    /// The target GPU the doorbell addressed (the BAR that trapped).
    pub gpu: GpuId,
    /// The decoded vChid (per-GPU runlist index).
    pub vchid: VChid,
    /// The raw token (recorded per-proc for poll-kick replay).
    pub token: u64,
}

/// ROUTE (R4): decode a doorbell token and demux it to its owning `(Proc, Channel)`
/// — a **pure read of the spine** (`Arch::decode_doorbell` + `by_vchid`), no proc
/// touched, no `&mut` anywhere. In L1 this runs under the device *read* lock only.
///
/// ★ MG-3: the vChid demux is keyed on `(target GPU, vChid)` — the doorbell's
/// target names WHICH GPU (the BAR that trapped); a vChid is a per-GPU runlist
/// index, so identical vChids on two GPUs route to their own channels.
pub fn route_doorbell(
    spine: &Spine,
    target_gpu: GpuId,
    token: u64,
) -> Result<DoorbellRoute, FwdFault> {
    let target = spine
        .arch
        .decode_doorbell(token)
        .ok_or(FwdFault::MalformedToken { token })?;
    let (pid, cid) =
        *spine
            .by_vchid
            .get(&(target_gpu, target.vchid))
            .ok_or(FwdFault::UnknownVchid {
                gpu: target_gpu,
                vchid: target.vchid,
            })?;
    Ok(DoorbellRoute {
        proc: pid,
        chan: cid,
        gpu: target_gpu,
        vchid: target.vchid,
        token,
    })
}

/// ACT (R4): run the routed doorbell against **its owning proc only** —
/// `&mut Proc`, never `&mut Gpu`. Ring-gate → lazy materialization/schedule → ring.
///
/// ★ **The single-threaded composition of [`plan_doorbell`] / `Worker::execute` /
/// [`commit_doorbell`]** (R1). It reaches the backend through the worker door, which
/// asserts R1 — so calling THIS under a proc lock is an immediate named panic, not a
/// silent convoy. L1's `SharedDevice` drives the three phases itself, interleaved
/// with its lock acquire/release and its pool-full wait.
///
/// `working_set` is the set of VAs this submission's work touches, as recovered by
/// the caller (launch descriptors / submit parse). A declared VA that is unbound or
/// bound-but-unpublished (`host_va = None` — the emulator's shadow had it, the
/// channel's OWN host VAS did not) is a loud fault BEFORE the channel is even
/// materialized, never a cross-proc content-pick. (An empty `working_set` is an
/// honest "this submission touches no tracked VA" — there is nothing to fault on,
/// and no host state is at risk.)
///
/// Materialization is lazy and **per-proc**: the first doorbell on a channel
/// allocates + schedules its host channel in its Vas's host VAS through its own
/// isolate (no warm-up assumption — testing strategy `wo_channel_alloc_then_
/// immediate_doorbell`), and the "already scheduled" state lives on the proc's
/// [`nvkvm_core::gpu::ExecPlane`] — there is no global one-shot to leave a second
/// proc's channel off-runlist (#12's CTX2 bug, crack ⚠4).
pub fn exec_doorbell(
    spine: &Spine,
    proc: &mut Proc,
    route: &DoorbellRoute,
    working_set: &[GpuVa],
) -> Result<DoorbellOutcome, FwdFault> {
    let planned = plan_doorbell(proc, route, working_set)?;
    let gpu = planned.plan.cgpu;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_doorbell(spine, proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_doorbell`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The channel.
    pub chan: ChanId,
    /// The GPU the doorbell trapped on (routing key with `vchid`).
    pub gpu: GpuId,
    /// The channel's OWN target GPU (its isolate/arena key).
    pub cgpu: GpuId,
    /// The decoded vChid.
    pub vchid: VChid,
    /// The raw token (recorded per-proc for poll-kick replay).
    pub token: u64,
    /// The channel's declared VAS, if any.
    pub vas_pdb: Option<Pdb>,
    /// The channel's host handles **as observed at plan time** (`None` = the chain
    /// materializes them).
    pub channel: Option<ChannelHandles>,
    /// Whether this submission must schedule the channel first.
    pub schedule: bool,
}

/// PLAN (R1) for the ONE ring path. Runs the #14 ring-gate **before any host op**
/// exactly as before — the gate now lives in the phase that holds the lock, which is
/// strictly stronger: it is checked against the same consistent snapshot the plan is
/// derived from.
///
/// A pure `&Proc` read; nothing is mutated until the commit.
pub fn plan_doorbell(
    proc: &Proc,
    route: &DoorbellRoute,
    working_set: &[GpuVa],
) -> Result<Planned<DoorbellPlan>, FwdFault> {
    let pid = route.proc;
    let cid = route.chan;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    let chan: &Channel = proc.channels.get(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: route.gpu,
        vchid: route.vchid,
    })?;
    let cgpu = chan.gpu;
    if !proc.isolates.contains_key(&cgpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        });
    }

    // ---- The #14 ring-gate, BEFORE any host op (the ONE ring path always runs it). ----
    match chan.vas_pdb {
        Some(pdb) => {
            let vas = proc
                .vases
                .get(&(cgpu, pdb))
                .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
            gate_vas(&vas.table, pdb, working_set.iter().copied())?;
        }
        // No declared VAS (GSP-managed, system-routed): there is no address space to
        // have published a working set into — only an empty declaration is gateable.
        None if working_set.is_empty() => {}
        None => return Err(FwdFault::NoVas(cid)),
    }

    let channel = chan.host_channel.zip(chan.host_token);
    // Lazy per-proc materialization: the channel's graph-derived `EngineKind` rides
    // the alloc so the adapter lands it on the RIGHT runlist (GR-1: the C's
    // `dma_copy_class_alloc_params` engineType=0 → 401 class, designed out).
    let host_vas = if channel.is_none() {
        let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
        proc.vases
            .get(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?
            .host_vas
    } else {
        None
    };
    let schedule = !proc.exec.scheduled.contains(&cid);
    Ok(Planned {
        plan: DoorbellPlan {
            proc: pid,
            chan: cid,
            gpu: route.gpu,
            cgpu,
            vchid: route.vchid,
            token: route.token,
            vas_pdb: chan.vas_pdb,
            channel,
            schedule,
        },
        verbs: Some(VerbPlan::Doorbell {
            host_vas,
            channel,
            engine: chan.engine,
            schedule,
        }),
    })
}

/// COMMIT (R5) for the ring path: re-resolve the route through the spine and the
/// channel through its `ChanId`, then adopt the materialized host handles and record
/// the submission. Refuses — releasing whatever it allocated — if the route moved,
/// the channel was torn down, or a sibling commit rebound the same channel/VAS.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Doorbell`] its plan asked for.
pub fn commit_doorbell(
    spine: &Spine,
    proc: &mut Proc,
    plan: &DoorbellPlan,
    reply: Option<VerbReply>,
) -> Result<DoorbellOutcome, Refusal> {
    let Some(VerbReply::Doorbell {
        host_vas: fresh_vas,
        channel: fresh_chan,
        scheduled,
    }) = reply
    else {
        return wrong_reply("doorbell");
    };
    let orphans = || Orphans {
        unmap: Vec::new(),
        free: fresh_chan
            .map(|(h, _)| h)
            .into_iter()
            .chain(fresh_vas)
            .collect(),
    };
    // Converging staleness (someone else materialized what we were materializing)
    // re-plans; divergent staleness (the target is gone) is a loud refusal.
    let refuse = |what: Stale| {
        Err(Refusal {
            fault: FwdFault::Stale(what),
            orphans: orphans(),
            retry: matches!(what, Stale::Rebound),
        })
    };
    if proc.is_retired() || proc.id != plan.proc {
        return refuse(Stale::Proc(plan.proc));
    }
    // R5 on the ROUTE: an `apply`/refresh may have rewritten `by_vchid` in the gap.
    // The plan was made for `(gpu, vchid) → (proc, chan)`; if that is no longer the
    // routing truth, this submission belongs to a world that no longer exists.
    if spine.by_vchid.get(&(plan.gpu, plan.vchid)) != Some(&(plan.proc, plan.chan)) {
        return refuse(Stale::Route {
            gpu: plan.gpu,
            vchid: plan.vchid,
        });
    }
    let Proc {
        vases,
        channels,
        exec,
        poll,
        ..
    } = proc;
    let Some(chan) = channels.get_mut(&plan.chan) else {
        return refuse(Stale::Channel(plan.chan));
    };
    if let Some(fresh) = fresh_vas {
        let pdb = plan
            .vas_pdb
            .expect("materialization requires a declared VAS");
        let Some(vas) = vases.get_mut(&(plan.cgpu, pdb)) else {
            return refuse(Stale::Vas {
                gpu: plan.cgpu,
                pdb,
            });
        };
        if vas.host_vas.is_some() {
            return refuse(Stale::Rebound);
        }
        vas.host_vas = Some(fresh);
    }
    match fresh_chan {
        // We materialized: nobody else may have, or one of the two host channels is
        // instantly orphaned (and the guest's vChid would ring the wrong one).
        Some((hchan, htok)) => {
            if chan.host_channel.is_some() {
                return refuse(Stale::Rebound);
            }
            chan.host_channel = Some(hchan);
            chan.host_token = Some(htok);
        }
        // We reused what the plan read: it must still be what the channel holds.
        None => {
            if chan.host_channel.zip(chan.host_token) != plan.channel {
                return refuse(Stale::Rebound);
            }
        }
    }
    if scheduled {
        exec.scheduled.insert(plan.chan);
    }
    poll.last_token = Some(plan.token);
    let host_token = chan.host_token.expect("materialized above");
    Ok(DoorbellOutcome {
        proc: plan.proc,
        chan: plan.chan,
        host_token,
        scheduled_now: scheduled,
    })
}

/// ★ THE ONE ring path — the exec-plane demux, **structurally gated** (#14,
/// `execution_plane.md` §2.4; the C's "one exec path" refactor-debt lesson): one
/// guest doorbell write → gate → the owning proc's channel rung on the owning
/// proc's isolate. No ungated sibling exists; nothing else in the workspace calls
/// `RmBackend::ring_doorbell`.
///
/// The **split-borrow composition** of [`route_doorbell`] (pure spine read) +
/// [`exec_doorbell`] (owning-proc act) — L1 cardinal rule R4 factored in the core.
/// The gate is **structural, not caller discipline**: [`exec_doorbell`] is the ONLY
/// function that reaches `RmBackend::ring_doorbell`, and it ALWAYS runs the gate
/// before any host op — a caller cannot choose an ungated door because none exists
/// (the removed `ring_gated` sibling was the debt).
pub fn handle_doorbell(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    token: u64,
    working_set: &[GpuVa],
) -> Result<DoorbellOutcome, FwdFault> {
    let Gpu { spine, procs, .. } = gpu;
    let route = route_doorbell(spine, target_gpu, token)?;
    let proc = procs
        .get_mut(&route.proc)
        .ok_or(FwdFault::RetiredProc(route.proc))?;
    exec_doorbell(spine, proc, &route, working_set)
}

/// Post any composable completion batch for target `gpu_target` and raise the SWGEN0
/// edge (MG-6: per-target GSP queue). Returns the posted batch, if any. (Queue
/// *encoding* is `nvkvm-gsp`'s job once it ports.)
pub fn deliver_completions(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    gpu_target: GpuId,
) -> Option<PostBatch> {
    let batch = gpu.pump_completions(gpu_target)?;
    vmm.raise_irq(COMPLETION_VECTOR).ok()?;
    Some(batch)
}

/// A proc's completion-poll RPC arrived (`MC_SERVICE_INTERRUPTS`-shaped) on target
/// `gpu_target`: re-post its un-acked completions off its OWN poll and raise the edge
/// — the #14 round-8 starvation is impossible by construction (§4.3.2), per target.
pub fn poll_completions(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    gpu_target: GpuId,
    pid: ProcId,
) -> Option<PostBatch> {
    let now = vmm.now();
    let batch = gpu.completion_poll(gpu_target, pid, now)?;
    vmm.raise_irq(COMPLETION_VECTOR).ok()?;
    Some(batch)
}

// =================================================================================
// GR/CE context lifecycle — the Case-1 forward / Case-2 ack-only split
// (`execution_plane.md` §2.2 / §2.5). The core is routing-only: it forwards the
// Case-1 allocs so the HOST kernel-RM builds + self-promotes its OWN context (golden
// ctx on real silicon), and ACKs the Case-2 GSP-internal controls the guest still
// issues (their effect is already achieved host-side). Zero new identity — the GR/CE
// context IS the `(Channel, Vas)` pair the graph already derives.
// =================================================================================

/// How a control routed through the Case-1/Case-2 split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRoute {
    /// **Case 1** — forwarded ~1:1 to the host through the owning proc's isolate
    /// (the RPC *is* the userspace op).
    Forwarded,
    /// **Case 2** — GSP-internal / ROUTE_TO_PHYSICAL with no unprivileged equivalent
    /// (`PROMOTE_CTX`, `GET_CTX_BUFFER_INFO`, …). ACKed to the guest, nothing done on
    /// the host — its effect is already achieved by the Case-1 forwarding. Replaying
    /// it on an unprivileged isolate would be a "wrong layer" error, never done.
    AckOnly,
}

/// The outcome of a Case-1 engine-object forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectForwarded {
    /// The engine kind the object made this channel's context (routing tag).
    pub engine: EngineKind,
    /// The host engine-object handle the forward returned.
    pub host_object: HostHandle,
    /// True if this forward materialized the host channel first (idempotent re-sends
    /// do not re-materialize).
    pub materialized_channel: bool,
    /// True if this forward was a **replay** resolved from the channel's
    /// idempotency table ([`Channel::host_engine_objects`]) — no host alloc was
    /// issued; `host_object` is the ORIGINAL host object (§2.2: re-sends are
    /// idempotent, the same discipline as the graph's alloc/DUP replay).
    pub reused: bool,
}

/// A routed Case-1 engine-object alloc (the ROUTE half — same split as
/// [`DoorbellRoute`]): the arch resolved the class to an [`EngineKind`] and
/// `by_vchid` resolved the owning `(Proc, Channel)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectRoute {
    /// The owning proc.
    pub proc: ProcId,
    /// The channel the object allocs on.
    pub chan: ChanId,
    /// The target GPU the alloc addressed.
    pub gpu: GpuId,
    /// The channel's vChid (per-GPU), for act-phase fault naming.
    pub vchid: VChid,
    /// The engine kind the arch mapped `class` to.
    pub engine: EngineKind,
}

/// ROUTE: resolve a Case-1 engine-object alloc to its owning `(Proc, Channel)` —
/// a pure spine read (`Arch::engine_of_object` + `by_vchid`). A class the arch
/// does not recognize as an engine object is a loud `NotAnEngine` (MISS=FAULT).
pub fn route_engine_object(
    spine: &Spine,
    target_gpu: GpuId,
    vchid: VChid,
    class: ClassId,
) -> Result<EngineObjectRoute, FwdFault> {
    let engine = spine
        .arch
        .engine_of_object(class)
        .ok_or(FwdFault::NotAnEngine(class))?;
    let (pid, cid) = *spine
        .by_vchid
        .get(&(target_gpu, vchid))
        .ok_or(FwdFault::UnknownVchid {
            gpu: target_gpu,
            vchid,
        })?;
    Ok(EngineObjectRoute {
        proc: pid,
        chan: cid,
        gpu: target_gpu,
        vchid,
        engine,
    })
}

/// ACT: forward the routed engine-object alloc on **its owning proc only**
/// (`&mut Proc`): lazily materialize the host channel (same per-proc discipline as
/// the doorbell act), then alloc the engine object via the proc's own isolate.
///
/// The single-threaded composition of [`plan_engine_object`] / `Worker::execute` /
/// [`commit_engine_object`] — same R1 shape as [`exec_doorbell`].
///
/// **Idempotent under replay** (§2.2; the protocol is order-/repeat-independent): a
/// re-sent alloc for a class already forwarded on this channel resolves from
/// [`Channel::host_engine_objects`] and returns the ORIGINAL host object — the host
/// never sees a duplicate engine-object alloc (the guest-retry hazard the graph
/// already covers for alloc/DUP, extended to the host-forward plane).
pub fn exec_engine_object(
    spine: &Spine,
    proc: &mut Proc,
    route: &EngineObjectRoute,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let planned = plan_engine_object(proc, route, class, params)?;
    let gpu = planned.plan.cgpu;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_engine_object(spine, proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_engine_object`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The channel the object allocs on.
    pub chan: ChanId,
    /// The GPU the alloc addressed (routing key with `vchid`).
    pub gpu: GpuId,
    /// The channel's own target GPU (isolate key).
    pub cgpu: GpuId,
    /// The channel's vChid.
    pub vchid: VChid,
    /// The engine kind the arch mapped `class` to.
    pub engine: EngineKind,
    /// The engine-object class.
    pub class: ClassId,
    /// The channel's declared VAS, if any.
    pub vas_pdb: Option<Pdb>,
    /// The channel's host handles as observed at plan time.
    pub channel: Option<ChannelHandles>,
    /// Set when the alloc resolved from the channel's idempotency table — no host
    /// work at all, so no worker is checked out (`verbs = None`).
    pub replay: Option<HostHandle>,
}

/// PLAN (R1) for the Case-1 engine-object forward.
///
/// **Idempotent under replay** (§2.2): a re-sent alloc for a class already forwarded
/// on this channel resolves here, from core state, and emits **no verbs at all** —
/// the host never sees a duplicate, and the replay never touches the worker pool.
pub fn plan_engine_object(
    proc: &Proc,
    route: &EngineObjectRoute,
    class: ClassId,
    params: &[u8],
) -> Result<Planned<EngineObjectPlan>, FwdFault> {
    let pid = route.proc;
    let cid = route.chan;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: route.gpu,
        vchid: route.vchid,
    })?;
    let cgpu = chan.gpu;
    if !proc.isolates.contains_key(&cgpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        });
    }
    let channel = chan.host_channel.zip(chan.host_token);
    // A replay is only representable once the channel materialized (the idempotency
    // table is populated by a forward, which requires a host channel).
    let replay = channel
        .is_some()
        .then(|| chan.host_engine_objects.get(&class).copied())
        .flatten();
    let host_vas = if channel.is_none() {
        let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
        proc.vases
            .get(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?
            .host_vas
    } else {
        None
    };
    let plan = EngineObjectPlan {
        proc: pid,
        chan: cid,
        gpu: route.gpu,
        cgpu,
        vchid: route.vchid,
        engine: route.engine,
        class,
        vas_pdb: chan.vas_pdb,
        channel,
        replay,
    };
    let verbs = if replay.is_some() {
        None
    } else {
        Some(VerbPlan::EngineObject {
            host_vas,
            channel,
            engine: chan.engine,
            class,
            params: params.to_vec(),
        })
    };
    Ok(Planned { plan, verbs })
}

/// COMMIT (R5) for the Case-1 forward: same route/channel re-resolution as the
/// doorbell, then adopt the host engine object into the channel's idempotency table.
///
/// # Panics
/// If `reply` is not the [`VerbReply::EngineObject`] its plan asked for.
pub fn commit_engine_object(
    spine: &Spine,
    proc: &mut Proc,
    plan: &EngineObjectPlan,
    reply: Option<VerbReply>,
) -> Result<EngineObjectForwarded, Refusal> {
    let (fresh_vas, fresh_chan, object) = match (plan.replay, reply) {
        // Replay: nothing ran, nothing to adopt — but the target must still exist.
        (Some(original), None) => (None, None, Some(original)),
        (
            None,
            Some(VerbReply::EngineObject {
                host_vas,
                channel,
                object,
            }),
        ) => (host_vas, channel, Some(object)),
        _ => return wrong_reply("engine-object"),
    };
    let object = object.expect("both arms produce a host object");
    let orphans = || Orphans {
        unmap: Vec::new(),
        free: (plan.replay.is_none())
            .then_some(object)
            .into_iter()
            .chain(fresh_chan.map(|(h, _)| h))
            .chain(fresh_vas)
            .collect(),
    };
    // Converging staleness (someone else materialized what we were materializing)
    // re-plans; divergent staleness (the target is gone) is a loud refusal.
    let refuse = |what: Stale| {
        Err(Refusal {
            fault: FwdFault::Stale(what),
            orphans: orphans(),
            retry: matches!(what, Stale::Rebound),
        })
    };
    if proc.is_retired() || proc.id != plan.proc {
        return refuse(Stale::Proc(plan.proc));
    }
    if spine.by_vchid.get(&(plan.gpu, plan.vchid)) != Some(&(plan.proc, plan.chan)) {
        return refuse(Stale::Route {
            gpu: plan.gpu,
            vchid: plan.vchid,
        });
    }
    let Proc {
        vases, channels, ..
    } = proc;
    let Some(chan) = channels.get_mut(&plan.chan) else {
        return refuse(Stale::Channel(plan.chan));
    };
    if let Some(fresh) = fresh_vas {
        let pdb = plan
            .vas_pdb
            .expect("materialization requires a declared VAS");
        let Some(vas) = vases.get_mut(&(plan.cgpu, pdb)) else {
            return refuse(Stale::Vas {
                gpu: plan.cgpu,
                pdb,
            });
        };
        if vas.host_vas.is_some() {
            return refuse(Stale::Rebound);
        }
        vas.host_vas = Some(fresh);
    }
    match fresh_chan {
        Some((hchan, htok)) => {
            if chan.host_channel.is_some() {
                return refuse(Stale::Rebound);
            }
            chan.host_channel = Some(hchan);
            chan.host_token = Some(htok);
        }
        None => {
            if chan.host_channel.zip(chan.host_token) != plan.channel {
                return refuse(Stale::Rebound);
            }
        }
    }
    if plan.replay.is_none() {
        // A sibling thread may have forwarded the SAME class in the gap; the table is
        // the idempotency authority, so the loser refuses and frees its duplicate
        // rather than overwriting (which would orphan the winner's object silently).
        if chan.host_engine_objects.contains_key(&plan.class) {
            return refuse(Stale::Rebound);
        }
        chan.host_engine_objects.insert(plan.class, object);
    }
    Ok(EngineObjectForwarded {
        engine: plan.engine,
        host_object: object,
        materialized_channel: fresh_chan.is_some(),
        reused: plan.replay.is_some(),
    })
}

/// **Case 1**: forward an engine-object alloc (compute / graphics / CE / NVENC) on the
/// channel identified by `vchid`, so the host kernel-RM builds and self-promotes its
/// OWN context. The **split-borrow composition** of [`route_engine_object`] +
/// [`exec_engine_object`] (same route/act discipline as the doorbell).
///
/// `class` is the guest's engine-object class; `params` is the ABI-lowered alloc
/// blob (Axis A). MISS=FAULT throughout.
pub fn forward_engine_object(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    vchid: VChid,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let Gpu { spine, procs, .. } = gpu;
    let route = route_engine_object(spine, target_gpu, vchid, class)?;
    let proc = procs
        .get_mut(&route.proc)
        .ok_or(FwdFault::RetiredProc(route.proc))?;
    exec_engine_object(spine, proc, &route, class, params)
}

/// ROUTE: classify a `GSP_RM_CONTROL` through the Case-1/Case-2 split — a pure
/// spine read (`Arch::is_case2_control`), no proc touched.
#[must_use]
pub fn classify_control(spine: &Spine, cmd: ControlCmd) -> ControlRoute {
    if spine.arch.is_case2_control(cmd) {
        // Case 2: ack-only. The host already did it (Case-1). Do NOT replay — an
        // unprivileged replay returns InsufficientPermissions ("wrong layer").
        ControlRoute::AckOnly
    } else {
        ControlRoute::Forwarded
    }
}

/// ACT: forward a Case-1 control on **its owning proc only** (`&mut Proc`), on the
/// op's TARGET GPU's isolate (MG-5): the control object `obj` is a handle in that
/// isolate's namespace; routing it elsewhere is unrepresentable.
///
/// The single-threaded composition of [`plan_control`] / `Worker::execute` /
/// [`commit_control`] — same R1 shape as [`exec_doorbell`].
pub fn forward_control(
    proc: &mut Proc,
    target_gpu: GpuId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &mut [u8],
) -> Result<(), FwdFault> {
    let planned = plan_control(proc, target_gpu, obj, cmd, payload)?;
    round_trip(proc, target_gpu, planned.verbs, |proc, reply| {
        commit_control(proc, &planned.plan, reply, payload)
    })
}

/// The ID-shaped hints [`commit_control`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The op's TARGET GPU (MG-5: `obj` is a handle in THAT isolate's namespace).
    pub gpu: GpuId,
    /// The control object.
    pub obj: HostHandle,
    /// The command.
    pub cmd: ControlCmd,
}

/// PLAN (R1) for a Case-1 control forward. The payload is copied into the plan by
/// value: a plan outlives the lock scope that made it, so it may not borrow.
pub fn plan_control(
    proc: &Proc,
    target_gpu: GpuId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &[u8],
) -> Result<Planned<ControlPlan>, FwdFault> {
    let pid = proc.id;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    if !proc.isolates.contains_key(&target_gpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: target_gpu,
        });
    }
    Ok(Planned {
        plan: ControlPlan {
            proc: pid,
            gpu: target_gpu,
            obj,
            cmd,
        },
        verbs: Some(VerbPlan::Control {
            obj,
            cmd,
            payload: payload.to_vec(),
        }),
    })
}

/// COMMIT (R5) for a control forward: re-validate that the proc and its target
/// isolate still exist, then write the host's reply back into the guest's buffer.
///
/// **Honest note on this site's staleness shape.** The control's host effect has
/// already happened by the time the commit runs; the only thing the commit *owns* is
/// the write-back. So a refusal here means "the answer has nowhere to go", not "the
/// op was undone" — and there is no orphan to release, because the object the
/// control ran on was the guest's, not something this op allocated. That is a real
/// asymmetry with the alloc-shaped sites, stated rather than papered over.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Control`] its plan asked for.
pub fn commit_control(
    proc: &mut Proc,
    plan: &ControlPlan,
    reply: Option<VerbReply>,
    payload: &mut [u8],
) -> Result<(), Refusal> {
    let Some(VerbReply::Control { payload: out }) = reply else {
        return wrong_reply("control");
    };
    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Proc(plan.proc))));
    }
    if !proc.isolates.contains_key(&plan.gpu) {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Target {
            proc: plan.proc,
            gpu: plan.gpu,
        })));
    }
    let n = payload.len().min(out.len());
    payload[..n].copy_from_slice(&out[..n]);
    Ok(())
}

/// Route a `GSP_RM_CONTROL` through the Case-1/Case-2 split. A **Case-2** control is
/// ACKed and NOT forwarded (its host effect is already achieved); a **Case-1** control
/// is forwarded to the host on `obj` through the owning proc's isolate. The
/// **split-borrow composition** of [`classify_control`] + [`forward_control`].
///
/// This is the anti-bolt-on payoff in code: adding an engine adds *rows* to the arch's
/// Case-2 set and its class table — never a new host verb, never a new routing path.
pub fn route_control(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    pid: ProcId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &mut [u8],
) -> Result<ControlRoute, FwdFault> {
    if let ControlRoute::AckOnly = classify_control(&gpu.spine, cmd) {
        return Ok(ControlRoute::AckOnly);
    }
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    forward_control(proc, target_gpu, obj, cmd, payload)?;
    Ok(ControlRoute::Forwarded)
}

/// The guest is waiting on the GR golden-capture completion (a GSP-event the host's
/// in-kernel FECS capture satisfies). Route it to the **system** proc — it is
/// kernel-internal and content-irrelevant (the guest only needs the *completion* its
/// 4-second poll waits on). Returns the observed os-event ref for assertions.
///
/// Typed to the system proc by construction (lesson L5 / the #12 finishPayload rule):
/// forging a completion for a userspace proc is unrepresentable here.
///
/// **Deliberately NOT route/act-split** (the one `&mut Gpu` per-proc-shaped entry
/// left): a `&mut Proc` form would let a caller hand it a *user* proc, dissolving
/// the structural L5 guarantee. It targets `Gpu::system` by name, is a rare
/// bring-up event (once per device), and L1 runs it under the device write lock.
pub fn signal_golden_capture(gpu: &mut Gpu, event: OsEventRef) -> Result<OsEventRef, FwdFault> {
    gpu.system.completion.observe(event)?;
    Ok(event)
}

// =================================================================================
// The ONE pushbuffer parser (`execution_plane.md` §2.3) — the address-table
// populator + sema/fence extractor. It decodes JUST the four fact kinds; everything
// else is opaque and passes through (the anti-emulation boundary, trap-min #6). The
// decode LOGIC is core; the method ENCODINGS come from `Arch::pushbuffer()`.
//
// Two co-equal address-table populate sources meet here (address_table.md, L3): the
// bind-time RPC bindings (batch 1, `Gpu::sync_rpc_mappings`) and the observed CE
// PT-writes captured below. Both land in the same per-`Vas` table.
// =================================================================================

/// Upper bound on a single GPFIFO range's method bytes the parser will read. A
/// hostile GPFIFO entry can declare any length; this caps it to a bounded read so an
/// attacker-controlled length is never an arbitrary allocation (boundary-1). Real
/// pushbuffer segments are far smaller; a range hitting this cap is simply truncated
/// (the surplus decodes to nothing actionable, MISS=FAULT at use).
const MAX_PUSH_RANGE_BYTES: usize = 1 << 20;

/// Upper bound on the TOTAL method bytes one `parse_pushbuffer` call will read across
/// ALL of a ring's GPFIFO ranges. `MAX_PUSH_RANGE_BYTES` bounds any single range, but a
/// hostile ring can declare *many* maxed-out ranges; this caps their sum so the decoded
/// method vector cannot grow without bound either (boundary-1). Ranges past the budget
/// are skipped (their content decodes to nothing actionable — MISS=FAULT at use).
const MAX_PUSH_TOTAL_BYTES: usize = 8 << 20;

/// What one pushbuffer parse observed (for assertions + the caller's next steps).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushbufferOutcome {
    /// CE PT-write destination pages captured (#13): physical PT pages this
    /// pushbuffer's CE copies wrote, added to the channel's `Vas.pt_pages`.
    pub pt_writes: Vec<u64>,
    /// Semaphore releases observed → each `observe`d on the owning proc's queue.
    pub sem_releases: Vec<(GpuVa, u64)>,
    /// TLB invalidates seen (pdb, membar). A membar is honored as a hard barrier
    /// (the parser records it; a real transport blocks advance until refresh).
    pub invalidates: Vec<(Pdb, bool)>,
    /// Count of opaque methods passed through (acted on by no core state).
    pub opaque: usize,
}

/// Decode a byte range of method words into `(header, args)` pairs, arch-driven.
/// Total on any input (a hostile/truncated range yields fewer methods, never a
/// panic or an unbounded read).
fn decode_methods(arch: &dyn nvkvm_arch::Arch, bytes: &[u8]) -> Vec<(u32, Vec<u32>)> {
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")))
        .collect();
    let pb = arch.pushbuffer();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let header = words[i];
        let nargs = pb.method_len(header);
        let start = i + 1;
        let end = start.saturating_add(nargs).min(words.len());
        out.push((header, words[start..end].to_vec()));
        // Always advance past at least the header, so a bogus count cannot stall.
        i = end.max(i + 1);
    }
    out
}

/// ROUTE/read phase of the pushbuffer parse: walk `ring`'s GPFIFO entries (arch
/// format, a pure spine read) and read each range's method words from guest memory
/// via `vmm`, bounded (boundary-1: per-range and total caps). Touches no proc — in
/// L1 this runs under the device *read* lock, before the owning proc's lock.
pub fn read_pushbuffer(
    spine: &Spine,
    vmm: &mut dyn Vmm,
    ring: &[u8],
) -> Result<Vec<(u32, Vec<u32>)>, FwdFault> {
    // Walk the GPFIFO entries (arch format), reading each range's method bytes. A
    // hostile GPFIFO entry can name any length; cap the per-range read so a bogus
    // length is a bounded read, never an arbitrary allocation (boundary-1 posture).
    let ranges = spine.arch.pushbuffer().gpfifo_entries(ring);
    let mut methods = Vec::new();
    let mut total = 0usize;
    for r in ranges {
        if total >= MAX_PUSH_TOTAL_BYTES {
            break; // Total-work budget spent — a hostile many-range ring stops here.
        }
        let len = (r.len as usize)
            .min(MAX_PUSH_RANGE_BYTES)
            .min(MAX_PUSH_TOTAL_BYTES - total);
        let mut buf = vec![0u8; len];
        vmm.gpa_read(r.gpa, &mut buf)
            .map_err(|_| FwdFault::GpaRead { gpa: r.gpa })?;
        methods.extend(decode_methods(spine.arch.as_ref(), &buf));
        total += len;
    }
    Ok(methods)
}

/// ACT phase of the pushbuffer parse: apply the decoded `methods` of channel `cid`
/// to **its owning proc only** (`&mut Proc` + the read-only spine for the arch's
/// method decoder). Feeds: CE-PT-write capture → the channel's `Vas.pt_pages` +
/// address table; `SemRelease` → the proc's `CompletionQueue`; honors
/// `TlbInvalidate` membars; passes opaque methods through.
pub fn apply_pushbuffer(
    spine: &Spine,
    proc: &mut Proc,
    cid: ChanId,
    methods: Vec<(u32, Vec<u32>)>,
) -> Result<PushbufferOutcome, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let chan_pdb = chan.vas_pdb;
    let cgpu = chan.gpu;

    let mut out = PushbufferOutcome::default();
    for (header, args) in methods {
        match spine.arch.pushbuffer().decode_method(header, &args) {
            nvkvm_arch::PushMethod::SetObject { .. } => {
                // Routing confirmation only — no address/completion state changes.
            }
            nvkvm_arch::PushMethod::CeLaunchDma {
                dst,
                len,
                dst_is_virtual,
            } => {
                // #13 CE-PT-write capture: a PHYSICAL destination is a page-table write
                // into this channel's compute VAS. Record the dirtied PT page and
                // forward-populate the same per-`Vas` table (co-equal RPC source).
                if !dst_is_virtual {
                    let pdb = chan_pdb.ok_or(FwdFault::NoVas(cid))?;
                    let vas = proc
                        .vases
                        .get_mut(&(cgpu, pdb))
                        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
                    let page = dst.0 & !0xfffu64;
                    vas.pt_pages.insert(page);
                    out.pt_writes.push(page);
                    // Co-populate the address table with the captured mapping (the leaf
                    // the PT-write publishes), MISS=FAULT on overlap.
                    if vas.table.resolve(pdb, dst).is_err() {
                        vas.table.bind(
                            pdb,
                            dst,
                            len.max(0x1000),
                            Binding {
                                phys: dst.0,
                                aperture: Aperture::Vidmem,
                                host_va: None,
                            },
                        )?;
                    }
                }
            }
            nvkvm_arch::PushMethod::SemRelease { addr, payload } => {
                // Completion observe on the OWNING proc's queue (per-`Proc`, §2.4).
                // A hostile guest flooding sem-releases is loud-capped, not OOM.
                proc.completion.observe(OsEventRef(addr.0 ^ payload))?;
                out.sem_releases.push((addr, payload));
            }
            nvkvm_arch::PushMethod::TlbInvalidate { pdb, membar } => {
                out.invalidates.push((pdb, membar));
                // A membar is a hard barrier: the interpreter honors it before
                // advancing (recorded here; the real transport blocks on refresh).
            }
            nvkvm_arch::PushMethod::Opaque => out.opaque += 1,
        }
    }
    Ok(out)
}

/// Parse the pushbuffer `ring` submitted on channel `cid` of proc `pid`, reading its
/// method words from guest memory via `vmm`. The **split-borrow composition** of
/// [`read_pushbuffer`] (spine read + guest-memory read) + [`apply_pushbuffer`]
/// (owning-proc act).
///
/// **Only runs where the core is already the mediator** (kernel/CeUtils/scrubber
/// channels + the CE-PT-write point). A userspace ring never carries a fact the core
/// must extract (verified safe, address_table.md §opaque-fast-path) — callers pass it
/// through as shared pages, no per-submit parse.
pub fn parse_pushbuffer(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    pid: ProcId,
    cid: ChanId,
    ring: &[u8],
) -> Result<PushbufferOutcome, FwdFault> {
    let methods = read_pushbuffer(&gpu.spine, vmm, ring)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    apply_pushbuffer(&gpu.spine, proc, cid, methods)
}

// =================================================================================
// Per-`Proc` working-set publication + ring-gate — THE #14 fix in code
// (`execution_plane.md` §2.4, decision #7, C: 6de85e7). The proven #14 root cause was
// an EXECUTION fault: the loser's GR channel took a host FAULT_PDE because its
// (identical) guest VAs were never published into its OWN host GR VAS. So before a
// channel's doorbell rings, its working set MUST be forward-populated into that
// channel's Vas's own host VAS; an unpublished VA at ring time is a LOUD fault, never
// a cross-proc content-pick (the exact confused-deputy designed out).
//
// The gate is STRUCTURAL: [`handle_doorbell`] is the ONE ring path (nothing else in
// the workspace reaches `RmBackend::ring_doorbell`) and it gates the caller-recovered
// working set against the channel's `Vas` table on every ring — there is no ungated
// sibling to bypass (the C's
// "one exec path" refactor-debt lesson, closed by construction). `gate_working_set`
// below is the read-only QUERY form of the same predicate; it cannot ring anything.
// =================================================================================

/// Read-only query: would `working_set` pass channel `cid`'s ring-gate right now
/// (every VA published into that channel's Vas's own host VAS)? A VA with no host
/// publication (`host_va = None`) is a loud [`FwdFault`], never guessed.
///
/// This is the load-bearing per-`Vas` publication check: two procs' identical guest
/// VAs each resolve in their OWN Vas (keyed by PDB), so the gate passes for both only
/// because each published into its OWN host VAS (distinct `HostHandle`s). The
/// ENFORCING form lives inside [`handle_doorbell`] — this query cannot ring.
pub fn gate_working_set(
    gpu: &Gpu,
    pid: ProcId,
    cid: ChanId,
    working_set: &[GpuVa],
) -> Result<(), FwdFault> {
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    gate_working_set_in(proc, cid, working_set)
}

/// The per-proc form of [`gate_working_set`] (`&Proc` only — in L1: under that
/// proc's lock, no device-wide access needed). Same predicate, same loud faults.
pub fn gate_working_set_in(
    proc: &Proc,
    cid: ChanId,
    working_set: &[GpuVa],
) -> Result<(), FwdFault> {
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let vas = proc
        .vases
        .get(&(chan.gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: chan.gpu, pdb })?;
    gate_vas(&vas.table, pdb, working_set.iter().copied())
}

// =================================================================================
// Completion pattern (e) — the mapped-fence arm (`execution_plane.md` §1.2/§2.4;
// NVENC's fence-not-event shape, bench-proven in `nvenc_101`: the worker reads a
// GPU-written mapped fence with NO syscall). The channel's EngineKind selects the
// arm — exact at the Channel, never guessed from a parse. Distinct from the
// event-delivery path by construction: a fired fence never enters a
// CompletionQueue, never rides a DeliveryPlane batch, never raises SWGEN0.
// =================================================================================

/// Which completion arm a channel's [`EngineKind`] signals through (§2.4's
/// per-engine tie-in — the ONE place engine variety touches the completion plane).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionArm {
    /// Patterns (a)/(c): a semaphore write on a shared/published page (GR-compute,
    /// GR-graphics, CE) — passthrough-polled or parser-observed.
    SharedSema,
    /// Pattern (e): a mapped coherent fence the GPU writes and the guest worker
    /// reads with no syscall (NVENC).
    MappedFence,
}

/// The arm selection, keyed on the channel's [`EngineKind`] (which the `Channel`
/// carries — NVENC vs GR-compute is distinguishable at the channel, not just at
/// parse). NVDEC's completion shape is unproven (the declared honest gap): it stays
/// on the default shared-sema arm until bench-proven, never guessed onto the fence.
#[must_use]
pub fn completion_arm(engine: EngineKind) -> CompletionArm {
    match engine {
        EngineKind::NvEnc => CompletionArm::MappedFence,
        _ => CompletionArm::SharedSema,
    }
}

/// Arm a mapped-fence completion (pattern **e**) on channel `cid`: fire once the
/// fence at `addr` (in the channel's Vas) is observed at/after `target`, starting
/// from `current`. Returns `Ok(Some(event))` if the target is already reached at
/// arm time.
///
/// Discipline, all loud (MISS=FAULT):
/// - the channel's engine must select the fence arm ([`completion_arm`]) — arming
///   a fence on a sema-signalling channel is a [`FwdFault::WrongArm`];
/// - `addr` must be **mapped and host-published** in the channel's OWN Vas (the
///   host GPU writes it; an unpublished fence could never advance) — the same
///   per-`Vas` publication rule as the ring-gate;
/// - re-arms follow the retried-RPC discipline (identical = idempotent,
///   conflicting = loud) and the armed table is capacity-bounded (boundary-1);
/// - firing respects the #12 jump guard (`MAX_FENCE_JUMP`).
pub fn arm_fence(
    gpu: &mut Gpu,
    pid: ProcId,
    cid: ChanId,
    addr: GpuVa,
    current: u32,
    target: u32,
    event: OsEventRef,
) -> Result<Option<OsEventRef>, FwdFault> {
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    arm_fence_in(proc, cid, addr, current, target, event)
}

/// The per-proc ACT form of [`arm_fence`] (`&mut Proc` only — a fence arm touches
/// nothing device-global; in L1 it runs under that proc's lock).
pub fn arm_fence_in(
    proc: &mut Proc,
    cid: ChanId,
    addr: GpuVa,
    current: u32,
    target: u32,
    event: OsEventRef,
) -> Result<Option<OsEventRef>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    if completion_arm(chan.engine) != CompletionArm::MappedFence {
        return Err(FwdFault::WrongArm {
            chan: cid,
            engine: chan.engine,
        });
    }
    let cgpu = chan.gpu;
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let vas = proc
        .vases
        .get(&(cgpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
    // The fence must be a mapped, host-published address in this channel's OWN Vas.
    gate_vas(&vas.table, pdb, [addr])?;
    Ok(proc.fences.arm((pdb.0, addr.0), current, target, event)?)
}

/// A host write to the fence at `(pdb, addr)` was observed carrying `value` (the
/// adapter feeds this from its fence-page observation point). Routes by PDB — the
/// data-plane identity — to the owning proc's fence arms; fires at/after target
/// under the #12 jump guard. A value on an un-armed fence is inert (`Ok(None)`).
pub fn fence_observed(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    pdb: Pdb,
    addr: GpuVa,
    value: u32,
) -> Result<Option<OsEventRef>, FwdFault> {
    let pid = route_pdb(&gpu.spine, target_gpu, pdb)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    fence_observed_in(proc, pdb, addr, value)
}

/// The per-proc ACT form of [`fence_observed`] (`&mut Proc` only; the caller
/// routed by PDB via [`route_pdb`] — in L1: device read lock + that proc's lock).
pub fn fence_observed_in(
    proc: &mut Proc,
    pdb: Pdb,
    addr: GpuVa,
    value: u32,
) -> Result<Option<OsEventRef>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    Ok(proc.fences.observe((pdb.0, addr.0), value)?)
}

// =================================================================================
// The abstract present/display seam — GR-graphics's home (`execution_plane.md` §2.6).
// GR-graphics is the SAME engine as GR-compute (EngineKind::GrGraphics); the ONLY
// added surface is routing its scanout buffer to a `Present` sink, host-agnostic
// (QEMU/PRIME later; MockPresent now). The present-complete is fed back as a synthetic
// vblank via the OWNING proc's completion queue — never NVKMS.
// =================================================================================

/// Route proc `pid`'s GR-graphics scanout `buffer` — a [`SurfaceHandle`] minted by
/// that proc's own isolate (`RmBackend::export_surface`, the host-VRAM PRIME export;
/// guest-RAM handles do not typecheck here, GR-2a) — to the abstract [`Present`]
/// sink, then feed the present-complete back as a synthetic vblank on that proc's
/// completion queue (§2.4's graphics arm). Keeps display hypervisor/host-agnostic:
/// the core names only the [`Present`] seam; the concrete adapter (QEMU/PRIME) is a
/// later fill.
pub fn present_scanout(
    gpu: &mut Gpu,
    pid: ProcId,
    present: &mut dyn Present,
    buffer: SurfaceHandle,
    meta: FbMeta,
) -> Result<u64, FwdFault> {
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    present_scanout_in(proc, present, buffer, meta)
}

/// The per-proc ACT form of [`present_scanout`] (`&mut Proc` + the caller-owned
/// [`Present`] sink — nothing device-global; in L1: that proc's lock).
pub fn present_scanout_in(
    proc: &mut Proc,
    present: &mut dyn Present,
    buffer: SurfaceHandle,
    meta: FbMeta,
) -> Result<u64, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let vblank = present.present(buffer, meta).map_err(FwdFault::Present)?;
    // Synthetic vblank → the proc's completion queue (the graphics completion arm).
    proc.completion.observe(OsEventRef(vblank.seq))?;
    Ok(vblank.seq)
}

// The concurrency contract, compile-time-asserted (decision #17).
nvkvm_util::assert_send_sync!(
    FwdFault,
    Published,
    DoorbellOutcome,
    DoorbellRoute,
    ControlRoute,
    CompletionArm,
    EngineObjectForwarded,
    EngineObjectRoute,
    PushbufferOutcome,
    Stale,
    Orphans,
    Refusal,
    PublishPlan,
    DoorbellPlan,
    EngineObjectPlan,
    ControlPlan,
    Planned<PublishPlan>,
);
