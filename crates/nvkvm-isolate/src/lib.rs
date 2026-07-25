//! # nvkvm-isolate — the per-process unprivileged-sandbox port
//!
//! Two abstract seams live here:
//!
//! - [`RmBackend`] — **RM verbs, not ioctls** (arch doc §4.2, crate `nvkvm-rm` folded
//!   into this crate until the wire protocol lands): the unprivileged host-RM
//!   operation surface an isolate can issue. Abstract by design — the Windows-host
//!   door stays open, and NO real NVOS struct/ioctl number appears here (Axis A is
//!   quarantined to `nvkvm-abi`).
//! - [`Isolate`] / [`IsolateFactory`] — the per-guest-process sandboxed host worker.
//!   One isolate per `Proc` (`session_id == ProcId`), giving **blast-radius
//!   containment**: a bug forwarding process A cannot touch process B's host
//!   handles/mappings (threat-model boundary 2, arch doc §4.3.5).
//!
//! The real implementation (spawn, `CLONE_NEW*` namespaces, pivot_root, seccomp,
//! socket wire protocol — the Mode-1 stub posture) is an adapter crate concern.
//! This crate is pure interface + value types so the core and its tests never touch
//! an OS. `nvkvm-mocks::{MockRmBackend, MockIsolate}` are the test impls.
//!
//! ## ★ R1's teeth live here (`l1_concurrency.md` §3.3, stage 3)
//!
//! A host RM verb is the archetypal blocking call, so **R1 — no blocking call under
//! ANY lock, ever — must be asserted at this port's door, not at a wrapper someone
//! must remember to use** (the §12.6 gap stage 3 closes). Two mechanisms, together:
//!
//! - **Ownership shape.** [`RmBackend`] is not reachable from an [`Isolate`] by
//!   reference. It lives inside the isolate's bounded pool of [`Worker`]s, and
//!   [`Isolate::checkout`] **moves a worker OUT** to the calling thread (§7.3). A
//!   locked core phase therefore has nothing to call: it can *emit* a [`VerbPlan`]
//!   and check a worker out, and that is all.
//! - **Runtime assert.** [`Worker::execute`] (and the [`Worker::with_rm`] escape
//!   hatch) call [`nvkvm_util::lockwitness::assert_lock_free`] before touching the
//!   backend. Holding any ranked lock at a verb is an immediate panic naming R1.
//!
//! Full compile-time enforcement of "no guard is alive on this thread" is not
//! expressible in safe Rust; the ownership shape makes violations contortions
//! instead of accidents, and the assert is the real teeth.
//!
//! ## Concurrency (decision #17)
//!
//! [`Isolate`] and [`IsolateFactory`] are **`Send + Sync` supertraits**: the core
//! *stores* them (each `Proc` owns its `Box<dyn Isolate>`, the `Gpu` owns the
//! factory), so they inherit the core's shareability requirement. Their `&self`
//! surface is pure reads (`id`/`is_retired`); every mutation takes `&mut self`, so
//! exclusivity comes from the caller's borrow, as everywhere in the core.
//! [`RmBackend`] is `Send + Sync` **because the pool stores it**, not because any
//! call path shares one: it is reachable exclusively through a [`Worker`] the caller
//! `&mut`-owns, so a shared reference never exists. The bound is nonetheless
//! load-bearing and not droppable — an [`Isolate`] owns N idle [`Worker`]s, a `Proc`
//! owns the isolate, and the core's `Gpu` is `Sync`, so every boxed backend sitting
//! in a pool slot must be `Sync` for that chain to hold. (Before the §7.2 pool, no
//! `Box<dyn RmBackend>` was ever *stored* in core state, and the crate carried a
//! `Send`-only exception here; storing the pool is what cashed it in. Cost to real
//! impls: no `Rc`/`Cell` in a backend's private state — which the `&mut`-only
//! surface never needed anyway.)

pub use nvkvm_arch::ids::ControlCmd;
use nvkvm_arch::ids::{ClassId, EngineKind, GpuId};
use nvkvm_vmm::SurfaceHandle;

/// A host-side RM object handle, scoped to ONE isolate's handle namespace.
///
/// Handles from different isolates must never be interchangeable — the mock backend
/// enforces this in tests (boundary-2 blast-radius assertion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostHandle(pub u64);

/// Errors an RM verb can return, in core terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmError {
    /// The op requires privilege the isolate deliberately lacks. This is
    /// **"wrong layer," never "gain privilege"** (lesson L2: a Case-2 GSP-internal
    /// control replayed on the host gets exactly this) — callers must treat it as a
    /// design error in the forwarding decision, not retry with more privilege.
    InsufficientPermissions,
    /// Unknown handle in this isolate's namespace.
    BadHandle(HostHandle),
    /// Host resource exhaustion.
    NoMemory,
    /// Any other backend-reported failure (opaque status for diagnostics).
    Other(u32),
}

/// # The unprivileged host-RM verb surface
///
/// The complete vocabulary of host operations the forwarding plane may request.
/// Everything here must be issuable by an **unprivileged** host process — that
/// unprivilege, not the keying, is the load-bearing host security boundary
/// (lesson L8). There is deliberately no verb that could express a privileged
/// GSP-internal replay.
///
/// Two verb tiers, both abstract:
///
/// - **Generic verbs** (`alloc`/`free`/`control`) — ABI-typed passthrough for
///   Case-1 shadow-forwarding, parameter encoding supplied by the ABI adapter.
/// - **Intent verbs** (`alloc_vaspace`/`alloc_sysmem`/`alloc_channel`/`schedule`/…)
///   — named *intents* the adapter lowers to the correct per-version NVOS sequence
///   (lesson L2: translate guest intent; class/param selection is the adapter's
///   Axis-A job, never the core's).
///
/// Object-safe; implemented by the linux-ioctl adapter inside the sandbox, and by
/// `MockRmBackend` in tests (scriptable failures, recorded verb log).
///
/// `Send + Sync` — see the crate docs: only ever reached via `&mut`, so shared
/// cross-thread references are unrepresentable, but the pool *stores* boxed
/// backends inside a `Sync` `Proc`, which makes the bound structural.
pub trait RmBackend: Send + Sync {
    /// Allocate an RM object of `class` under `parent`. `params` is an opaque,
    /// already-encoded parameter blob (encoding is the ABI adapter's job).
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate a fresh **host GPU virtual address space** for one
    /// guest `Vas`. Per-Vas host VAS separation is the proven #14 fix: two guest
    /// processes' identical guest VAs publish into *different* host VASes and
    /// cannot collide.
    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate `len` bytes of host-visible system memory.
    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate a host GPU channel bound to host VAS `vas`, on the
    /// runlist/engine named by `engine` — the channel's graph-derived [`EngineKind`],
    /// which the adapter lowers to the host `NV_CHANNEL_ALLOC_PARAMS` engine type.
    /// The engine is declared HERE because the adapter cannot invent it: an
    /// engine-blind channel alloc is the C's proven wrong-runlist bug class
    /// (`dma_copy_class_alloc_params`: `engineType=0` → wrong runlist →
    /// cuCtxCreate 401 — seam audit GR-1). Returns
    /// `(channel_handle, host_work_submit_token)`.
    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError>;

    /// Intent verb: allocate an **engine object** (compute / graphics / CE / NVENC)
    /// of `class` on host channel `chan` — the Case-1 forward that makes the host
    /// kernel-RM build and self-promote its OWN context (golden ctx included, on real
    /// silicon). `params` is the ABI-lowered alloc blob (Axis A: `IS_EXTERNALLY_OWNED`
    /// already stripped, etc.). NOTE the anti-bolt-on property: this is *almost* the
    /// generic [`RmBackend::alloc`] with `parent = chan`; it is named only to state
    /// the intent — the host verb surface does NOT grow to add an engine.
    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError>;

    /// Intent verb: make `chan` runnable (the GPFIFO_SCHEDULE intent). Per-proc,
    /// never a one-shot: #12's CTX2 rang off-runlist because scheduling was a
    /// sticky global in the C.
    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError>;

    /// Free an RM object (and its subtree, per RM semantics).
    fn free(&mut self, obj: HostHandle) -> Result<(), RmError>;

    /// Issue a control command on an object; `payload` is read and written in place.
    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError>;

    /// Map `len` bytes of `memory` into the host GPU VA space owned by `vas`,
    /// returning the host GPU VA. The isolate picks/owns the actual placement —
    /// per-Vas host-VAS separation is what makes two guest processes' identical
    /// guest VAs land in disjoint host mappings (#14's proven fix).
    fn map_gpu_va(&mut self, vas: HostHandle, memory: HostHandle, len: u64)
    -> Result<u64, RmError>;

    /// Unmap a previous [`RmBackend::map_gpu_va`].
    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError>;

    /// Ring the host work-submit doorbell with an (already host-translated) token.
    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError>;

    /// Intent verb: export the host memory object `memory` (a render target in host
    /// VRAM) as a presentable [`SurfaceHandle`] — the **producer half of the display
    /// seam** (`execution_plane.md` §3.3, seam audit GR-2b). The C proved this runs
    /// in the ISOLATE (stub `PRIME_HANDLE_TO_FD` dma-buf export, session-owned —
    /// `present_path_b_done`); the flow is one-way guest→host. The consumer half is
    /// `Present::present`. Anti-bolt-on note: this is the ONE named display verb —
    /// the verb surface does not grow per engine.
    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError>;
}

/// Session identity of an isolate. Equals the owning `ProcId` by construction
/// (arch doc §4.3.4; the isolate infra supports 4096 sessions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsolateId(pub u32);

/// One worker slot inside an isolate's **bounded pool** (`l1_concurrency.md` §7.2,
/// decision #37).
///
/// The pool exists because a single-in-flight worker per isolate serializes a guest
/// process's *own* threads behind each other (the #37 intra-proc blocking gap); N
/// workers per `(Proc, GpuId)` isolate let sibling guest threads have verbs in flight
/// concurrently, while each individual worker stays strictly single-in-flight (a
/// property the type system gives for free: a worker is reached only by `&mut`).
///
/// Dense and **scoped to its owning isolate** — `WorkerId(0)` of proc A's GPU0 isolate
/// and `WorkerId(0)` of proc B's GPU1 isolate are unrelated identities, exactly like
/// [`HostHandle`]. Anything keyed on a worker must carry the `(ProcId, GpuId)` pair
/// alongside it (see `nvkvm_core::reactor::SourceKind::Worker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerId(pub u32);

/// The bounded pool's default size (`l1_concurrency.md` §7.2, "Calibration").
///
/// **A tuning constant, not a design question.** The design is explicit that the pool
/// is *statically* sized first and grows dynamically only when a measured workload
/// proves the bound hurts — a spawn/reap policy, thundering-herd wakeups and
/// worker-lifetime races are all cost with no demonstrated benefit. Order of the vCPU
/// count; four is a small commodity guest.
pub const DEFAULT_POOL_WORKERS: usize = 4;

// =================================================================================
// The verb PLAN — what a locked core phase emits instead of calling (R1's
// "consequence for the core shape", `l1_concurrency.md` §3.3)
// =================================================================================

/// A freshly allocated host channel: `(handle, host work-submit token)`.
pub type ChannelHandles = (HostHandle, u64);

/// ★ A **typed verb chain** — the description of host work a locked core phase
/// emits, executed later by [`Worker::execute`] with no lock held.
///
/// Deliberately NOT a resumable continuation machine. Every site's verbs are
/// data-dependent only on *each other* (host VAS handle → memory handle → mapped VA),
/// never on core state read between two verbs, so a plain chain suffices and the
/// execution step can thread its own intermediate results. If a future site genuinely
/// needs to consult core state mid-chain, that is a design change to argue in the doc
/// — never a hidden lock acquisition inside execution.
///
/// Owned payloads (`Vec<u8>`) rather than borrows: a plan outlives the lock scope
/// that produced it by construction, so it cannot hold a reference into core state.
/// Control/alloc blobs are small; the copy is the price of the invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbPlan {
    /// `publish_backing`'s chain: (optionally) allocate the Vas's own host VAS, then
    /// allocate sysmem and map it into that VAS.
    Publish {
        /// The Vas's already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// Bytes to allocate and map.
        len: u64,
    },
    /// The doorbell chain: (optionally) host VAS → (optionally) host channel →
    /// (optionally) schedule → ring.
    Doorbell {
        /// The Vas's host VAS, or `None` to allocate one (only consulted when
        /// `channel` is `None`).
        host_vas: Option<HostHandle>,
        /// The channel's already-materialized host handles, or `None` to allocate.
        channel: Option<ChannelHandles>,
        /// The channel's graph-derived engine (GR-1: the adapter cannot invent the
        /// runlist, so the core declares it).
        engine: EngineKind,
        /// Whether this submission must make the channel runnable first.
        schedule: bool,
    },
    /// The Case-1 engine-object chain: (optionally) host VAS → (optionally) host
    /// channel → engine-object alloc.
    EngineObject {
        /// The Vas's host VAS, or `None` to allocate one (only when `channel` is
        /// `None`).
        host_vas: Option<HostHandle>,
        /// The channel's host handles, or `None` to materialize it first.
        channel: Option<ChannelHandles>,
        /// The channel's engine (rides the channel alloc, GR-1).
        engine: EngineKind,
        /// The engine-object class.
        class: ClassId,
        /// The ABI-lowered alloc blob.
        params: Vec<u8>,
    },
    /// One Case-1 control, payload carried by value in and out.
    Control {
        /// The control object, in this isolate's namespace.
        obj: HostHandle,
        /// The command.
        cmd: ControlCmd,
        /// The in/out payload.
        payload: Vec<u8>,
    },
    /// ★ The disposition of host objects a **refused commit** could not adopt
    /// (`l1_concurrency.md` §3.3 R5: a commit whose target vanished must not
    /// silently leak what it already allocated). Runs on the same worker, still
    /// lock-free, before the worker is checked back in.
    Release {
        /// `(host VAS, host GPU VA)` pairs to unmap first.
        unmap: Vec<(HostHandle, u64)>,
        /// Objects to free, in the given order.
        free: Vec<HostHandle>,
    },
}

/// What one [`VerbPlan`] produced — the typed reply a commit phase re-enters with.
///
/// `host_vas` / `channel` fields carry only what this execution **freshly allocated**
/// (`None` = the plan reused what core state already held), which is exactly what the
/// commit must adopt or orphan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbReply {
    /// [`VerbPlan::Publish`]'s reply.
    Published {
        /// Freshly allocated host VAS, if the plan asked for one.
        host_vas: Option<HostHandle>,
        /// The allocated host memory object.
        memory: HostHandle,
        /// The host GPU VA it was mapped at.
        host_va: u64,
    },
    /// [`VerbPlan::Doorbell`]'s reply.
    Doorbell {
        /// Freshly allocated host VAS, if any.
        host_vas: Option<HostHandle>,
        /// Freshly allocated host channel, if any.
        channel: Option<ChannelHandles>,
        /// Whether the schedule verb ran.
        scheduled: bool,
    },
    /// [`VerbPlan::EngineObject`]'s reply.
    EngineObject {
        /// Freshly allocated host VAS, if any.
        host_vas: Option<HostHandle>,
        /// Freshly allocated host channel, if any.
        channel: Option<ChannelHandles>,
        /// The host engine object.
        object: HostHandle,
    },
    /// [`VerbPlan::Control`]'s reply — the payload as the host wrote it back.
    Control {
        /// The written-back payload.
        payload: Vec<u8>,
    },
    /// [`VerbPlan::Release`]'s reply.
    Released,
}

/// ★ A checked-out pool worker: **the one door to a host RM verb**.
///
/// Obtained only from [`Isolate::checkout`], which moves it OUT of the isolate's pool
/// (§7.3). While it is out, exactly one thread `&mut`-owns it — so
/// single-in-flight-per-worker stays the borrow checker's guarantee, N times over,
/// and there is no shared in-flight slot table and no txn demux anywhere (§11 B6:
/// concurrency comes from channel COUNT, never from multiplexing one channel).
///
/// `Send`, not `Sync` — it migrates to whichever thread checked it out, but a shared
/// reference to one is unrepresentable.
pub struct Worker {
    id: WorkerId,
    backend: Box<dyn RmBackend>,
}

impl core::fmt::Debug for Worker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Worker").field("id", &self.id).finish()
    }
}

impl Worker {
    /// Wrap a backend as pool slot `id` (isolate implementations only).
    #[must_use]
    pub fn new(id: WorkerId, backend: Box<dyn RmBackend>) -> Self {
        Worker { id, backend }
    }

    /// This worker's slot in its isolate's pool.
    #[must_use]
    pub fn id(&self) -> WorkerId {
        self.id
    }

    /// ★ Run `plan`'s verb chain. **Asserts R1 first** — invoking a host verb with
    /// any ranked lock held panics naming R1 (crate docs).
    ///
    /// Chains its own intermediate results with **zero core access**. On a mid-chain
    /// failure it releases what it already allocated on this same worker and then
    /// returns the error, so a partial chain never leaks a host object — the caller
    /// sees an all-or-nothing verb round trip.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn execute(&mut self, plan: &VerbPlan) -> Result<VerbReply, RmError> {
        nvkvm_util::lockwitness::assert_lock_free("issuing a host RM verb");
        let rm = &mut *self.backend;
        match plan {
            VerbPlan::Publish { host_vas, len } => {
                let (vas, fresh_vas) = match *host_vas {
                    Some(h) => (h, None),
                    None => {
                        let h = rm.alloc_vaspace()?;
                        (h, Some(h))
                    }
                };
                let memory = match rm.alloc_sysmem(*len) {
                    Ok(m) => m,
                    Err(e) => return Err(unwind(rm, fresh_vas.into_iter().collect(), e)),
                };
                let host_va = match rm.map_gpu_va(vas, memory, *len) {
                    Ok(va) => va,
                    Err(e) => {
                        let mut orphans = vec![memory];
                        orphans.extend(fresh_vas);
                        return Err(unwind(rm, orphans, e));
                    }
                };
                Ok(VerbReply::Published {
                    host_vas: fresh_vas,
                    memory,
                    host_va,
                })
            }
            VerbPlan::Doorbell {
                host_vas,
                channel,
                engine,
                schedule,
            } => {
                let (chan, fresh_vas, fresh_chan) = match *channel {
                    Some(c) => (c, None, None),
                    None => {
                        let (vas, fresh_vas) = match *host_vas {
                            Some(h) => (h, None),
                            None => {
                                let h = rm.alloc_vaspace()?;
                                (h, Some(h))
                            }
                        };
                        match rm.alloc_channel(vas, *engine) {
                            Ok(c) => (c, fresh_vas, Some(c)),
                            Err(e) => {
                                return Err(unwind(rm, fresh_vas.into_iter().collect(), e));
                            }
                        }
                    }
                };
                let unwind_set = || {
                    let mut v: Vec<HostHandle> = Vec::new();
                    if let Some((h, _)) = fresh_chan {
                        v.push(h);
                    }
                    v.extend(fresh_vas);
                    v
                };
                if *schedule && let Err(e) = rm.schedule(chan.0) {
                    return Err(unwind(rm, unwind_set(), e));
                }
                if let Err(e) = rm.ring_doorbell(chan.1) {
                    return Err(unwind(rm, unwind_set(), e));
                }
                Ok(VerbReply::Doorbell {
                    host_vas: fresh_vas,
                    channel: fresh_chan,
                    scheduled: *schedule,
                })
            }
            VerbPlan::EngineObject {
                host_vas,
                channel,
                engine,
                class,
                params,
            } => {
                let (chan, fresh_vas, fresh_chan) = match *channel {
                    Some(c) => (c, None, None),
                    None => {
                        let (vas, fresh_vas) = match *host_vas {
                            Some(h) => (h, None),
                            None => {
                                let h = rm.alloc_vaspace()?;
                                (h, Some(h))
                            }
                        };
                        match rm.alloc_channel(vas, *engine) {
                            Ok(c) => (c, fresh_vas, Some(c)),
                            Err(e) => {
                                return Err(unwind(rm, fresh_vas.into_iter().collect(), e));
                            }
                        }
                    }
                };
                match rm.alloc_engine_object(chan.0, *class, params) {
                    Ok(object) => Ok(VerbReply::EngineObject {
                        host_vas: fresh_vas,
                        channel: fresh_chan,
                        object,
                    }),
                    Err(e) => {
                        let mut orphans: Vec<HostHandle> = Vec::new();
                        if let Some((h, _)) = fresh_chan {
                            orphans.push(h);
                        }
                        orphans.extend(fresh_vas);
                        Err(unwind(rm, orphans, e))
                    }
                }
            }
            VerbPlan::Control { obj, cmd, payload } => {
                let mut payload = payload.clone();
                rm.control(*obj, *cmd, &mut payload)?;
                Ok(VerbReply::Control { payload })
            }
            VerbPlan::Release { unmap, free } => {
                // Best-effort by design: this IS the failure path. A refusal here
                // would have nowhere to go, and the isolate's own teardown is the
                // backstop (its whole handle namespace dies with it).
                for &(vas, va) in unmap {
                    let _ = rm.unmap_gpu_va(vas, va);
                }
                for &obj in free {
                    let _ = rm.free(obj);
                }
                Ok(VerbReply::Released)
            }
        }
    }

    /// Escape hatch for adapter/test code that needs the raw verb surface — scoped
    /// to a closure so no bare `&mut dyn RmBackend` escapes into a caller that might
    /// then take a lock. **Asserts R1** exactly like [`Worker::execute`].
    ///
    /// Production forwarding paths use [`VerbPlan`]; this exists so the port stays
    /// usable for bring-up probes without reopening a door that skips the assert.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn with_rm<R>(&mut self, f: impl FnOnce(&mut dyn RmBackend) -> R) -> R {
        nvkvm_util::lockwitness::assert_lock_free("issuing a host RM verb");
        f(&mut *self.backend)
    }
}

/// Release `orphans` (newest first) after a mid-chain verb failure, then surface the
/// ORIGINAL error — the cleanup's own failures are noise on an already-failing path.
fn unwind(rm: &mut dyn RmBackend, orphans: Vec<HostHandle>, err: RmError) -> RmError {
    for obj in orphans {
        let _ = rm.free(obj);
    }
    err
}

/// # One per-process sandboxed host worker pool
///
/// Owns a private host-RM connection with its own handle namespace, fd table, and
/// host process. Lifecycle: created at the process's earliest unambiguous signal (the
/// DUP_OBJECT dup-src registration — arch doc §4.3.4), retired in two stages
/// (`retire()` → drop) so cross-teardown consumption is impossible by construction
/// (lesson L10).
///
/// The isolate remains ONE sandboxed process per `(Proc, GpuId)` — the sandbox, the
/// RM client and the handle namespace are per-process identities and stay singular.
/// [`Worker`]s are slots inside it, each with its own 1-deep request/reply channel
/// (`l1_concurrency.md` §7.2); they share only the RM connection, which is
/// kernel-mediated. A handle minted on one worker is therefore valid on its siblings.
///
/// `Send + Sync`: owned by a `Proc` inside the shared `Gpu` (crate docs, #17).
pub trait Isolate: Send + Sync {
    /// This isolate's session id (== `ProcId`).
    fn id(&self) -> IsolateId;

    /// How many worker slots this isolate's bounded pool has (statically sized,
    /// §7.2). Never changes over the isolate's life.
    fn pool_size(&self) -> usize;

    /// How many workers are currently checked IN (available).
    fn idle_workers(&self) -> usize;

    /// ★ Check a worker OUT: mark a slot busy and move its handle to the calling
    /// thread (§7.3). Runs under device-read + proc lock — pool *bookkeeping* only,
    /// no verb.
    ///
    /// `None` means **backpressure, not failure**: the pool is saturated (or the
    /// isolate is retired and refuses new checkouts, §5.4). The caller must release
    /// ALL locks, wait, and re-enter from the top with full R5 re-validation — never
    /// spin, never wait under a lock.
    fn checkout(&mut self) -> Option<Worker>;

    /// Return a checked-out worker to its slot. Pool bookkeeping; runs under the
    /// proc lock alongside the commit phase.
    fn checkin(&mut self, worker: Worker);

    /// A worker died out of band (its reactor source signalled HUP, §7.3). Retires
    /// the slot permanently — **never a respawn**: a worker that died mid-verb may
    /// have left host state the core cannot reason about. Returns `true` if the slot
    /// was known and is now dead.
    fn worker_died(&mut self, worker: WorkerId) -> bool;

    /// Stage 1 of teardown: stop accepting new ops, begin quiescing in-flight work.
    /// Heavy state is reaped at the proven quiesce point, not here (lesson L10).
    fn retire(&mut self);

    /// True once `retire()` has been called (a retired isolate must refuse ops).
    fn is_retired(&self) -> bool;
}

/// Spawns isolates. The composition root holds one; per-`(Proc, GpuId)` isolates are
/// created through it so the core never knows *how* a sandbox is made.
///
/// `Send + Sync`: owned by the shared `Gpu` (crate docs, #17); spawning takes `&mut`.
pub trait IsolateFactory: Send + Sync {
    /// Spawn (or lazily reserve) the isolate for session `(id, gpu)` — one sandboxed
    /// host worker **per guest process per target GPU** (`multi_gpu_and_mig.md` item
    /// 3: a proc spanning two GPUs gets distinct isolates, so a bug forwarding its
    /// GPU0 traffic cannot reach its GPU1 host handles — the #14 blast-radius
    /// boundary lifted onto the GPU axis).
    fn spawn(&mut self, id: IsolateId, gpu: GpuId) -> Box<dyn Isolate>;
}

// The concurrency contract, compile-time-asserted (decision #17). `dyn RmBackend`
// is the one documented Send-only exception (crate docs).
nvkvm_util::assert_send_sync!(
    HostHandle,
    RmError,
    IsolateId,
    WorkerId,
    VerbPlan,
    VerbReply,
    dyn Isolate,
    dyn IsolateFactory
);
// The backend and the `Worker` that owns one: `Send + Sync` because pool slots live
// inside the `Sync` core (crate docs), even though no call path ever shares one.
nvkvm_util::assert_send_sync!(dyn RmBackend, Worker);
