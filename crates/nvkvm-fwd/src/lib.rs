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
//! ## Concurrency (decision #17)
//!
//! This crate is stateless — free functions over the core's types, so the
//! concurrency contract is inherited verbatim from `nvkvm-core` (see its crate
//! docs): functions taking `&Gpu` ([`resolve`], [`gate_working_set`]) are
//! concurrent-safe under a shared borrow; functions taking `&mut Gpu` or
//! `&mut Proc` require caller-provided exclusivity — and the `&mut Proc` ones
//! ([`publish_backing`]) parallelize per-proc (disjoint borrows out of
//! `Gpu::procs`, no shared lock).

use nvkvm_arch::Aperture;
use nvkvm_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use nvkvm_completion::{CompletionError, OsEventRef, PostBatch};
use nvkvm_core::gpu::{Channel, Gpu, Proc, Vas};
use nvkvm_core::{ChanId, ProcId};
use nvkvm_isolate::{HostHandle, RmError};
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

/// Ensure `vas` has its own host VAS object, materializing it through the OWNING
/// proc's isolate on first touch. Per-Vas host separation is the proven #14 fix.
fn ensure_host_vas(
    vas: &mut Vas,
    rm: &mut dyn nvkvm_isolate::RmBackend,
) -> Result<nvkvm_isolate::HostHandle, FwdFault> {
    if let Some(h) = vas.host_vas {
        return Ok(h);
    }
    let h = rm.alloc_vaspace()?;
    vas.host_vas = Some(h);
    Ok(h)
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
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    let Proc {
        vases,
        arenas,
        isolates,
        ..
    } = proc;
    let vas = vases
        .get_mut(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    let arena = arenas
        .get_mut(&gpu)
        .ok_or(FwdFault::NoTarget { proc: pid, gpu })?;
    let rm = isolates
        .get_mut(&gpu)
        .ok_or(FwdFault::NoTarget { proc: pid, gpu })?
        .rm();

    let gpa = arena.alloc(len, 0x1000).map_err(|_| FwdFault::Arena)?;
    let hvas = ensure_host_vas(vas, rm)?;
    let mem = rm.alloc_sysmem(len)?;
    let host_va = rm.map_gpu_va(hvas, mem, len)?;

    vas.table.bind(
        pdb,
        va,
        len,
        Binding {
            phys: gpa.0,
            aperture: Aperture::SysmemCoherent,
            host_va: Some(host_va),
        },
    )?;
    Ok(Published {
        gpa: gpa.0,
        host_va,
    })
}

/// Resolve `va` in the `Vas` identified by `(gpu, pdb)`. Pure lookup; MISS=FAULT.
pub fn resolve(gpu: &Gpu, target: GpuId, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), FwdFault> {
    let pid = *gpu
        .by_pdb
        .get(&(target, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: target, pdb })?;
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let vas = proc
        .vases
        .get(&(target, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: target, pdb })?;
    Ok(vas.table.resolve(pdb, va)?)
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

/// ★ THE ONE ring path — the exec-plane demux, **structurally gated** (#14,
/// `execution_plane.md` §2.4; the C's "one exec path" refactor-debt lesson): one
/// guest doorbell write → gate → the owning proc's channel rung on the owning
/// proc's isolate. No ungated sibling exists; nothing else in the workspace calls
/// `RmBackend::ring_doorbell`.
///
/// `working_set` is the set of VAs this submission's work touches, as recovered by
/// the caller (launch descriptors / submit parse). The gate is **structural, not
/// caller discipline**: this is the ONLY function that reaches
/// `RmBackend::ring_doorbell`, and it ALWAYS runs the gate before any host op — a
/// caller cannot choose an ungated door because none exists (the removed `ring_gated`
/// sibling was the debt). A declared VA that is unbound or bound-but-unpublished
/// (`host_va = None` — the emulator's shadow had it, the channel's OWN host VAS did
/// not) is a loud fault BEFORE the channel is even materialized, never a cross-proc
/// content-pick. (An empty `working_set` is an honest "this submission touches no
/// tracked VA" — there is nothing to fault on, and no host state is at risk.)
///
/// Materialization is lazy and **per-proc**: the first doorbell on a channel
/// allocates + schedules its host channel in its Vas's host VAS through its own
/// isolate (no warm-up assumption — testing strategy `wo_channel_alloc_then_
/// immediate_doorbell`), and the "already scheduled" state lives on the proc's
/// [`nvkvm_core::gpu::ExecPlane`] — there is no global one-shot to leave a second
/// proc's channel off-runlist (#12's CTX2 bug, crack ⚠4).
pub fn handle_doorbell(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    token: u64,
    working_set: &[GpuVa],
) -> Result<DoorbellOutcome, FwdFault> {
    let target = gpu
        .arch
        .decode_doorbell(token)
        .ok_or(FwdFault::MalformedToken { token })?;
    // ★ MG-3: the vChid demux is keyed on `(target GPU, vChid)` — the doorbell's
    // target names WHICH GPU (the BAR that trapped); a vChid is a per-GPU runlist
    // index, so identical vChids on two GPUs route to their own channels.
    let (pid, cid) =
        *gpu.by_vchid
            .get(&(target_gpu, target.vchid))
            .ok_or(FwdFault::UnknownVchid {
                gpu: target_gpu,
                vchid: target.vchid,
            })?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }

    let Proc {
        vases,
        channels,
        exec,
        isolates,
        poll,
        ..
    } = proc;
    let chan: &mut Channel = channels.get_mut(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: target_gpu,
        vchid: target.vchid,
    })?;
    let cgpu = chan.gpu;
    let rm = isolates
        .get_mut(&cgpu)
        .ok_or(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        })?
        .rm();

    // ---- The #14 ring-gate, BEFORE any host op (the ONE ring path always runs it). ----
    match chan.vas_pdb {
        Some(pdb) => {
            let vas = vases
                .get(&(cgpu, pdb))
                .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
            gate_vas(&vas.table, pdb, working_set.iter().copied())?;
        }
        // No declared VAS (GSP-managed, system-routed): there is no address space to
        // have published a working set into — only an empty declaration is gateable.
        None if working_set.is_empty() => {}
        None => return Err(FwdFault::NoVas(cid)),
    }

    // Lazy per-proc materialization. The channel's graph-derived `EngineKind` rides
    // the alloc so the adapter lands it on the RIGHT runlist (GR-1: the C's
    // `dma_copy_class_alloc_params` engineType=0 → 401 class, designed out).
    if chan.host_channel.is_none() {
        let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
        let vas = vases
            .get_mut(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
        let hvas = ensure_host_vas(vas, rm)?;
        let (hchan, htok) = rm.alloc_channel(hvas, chan.engine)?;
        chan.host_channel = Some(hchan);
        chan.host_token = Some(htok);
    }
    let mut scheduled_now = false;
    if !exec.scheduled.contains(&cid) {
        rm.schedule(chan.host_channel.expect("materialized above"))?;
        exec.scheduled.insert(cid);
        scheduled_now = true;
    }

    poll.last_token = Some(token);
    let host_token = chan.host_token.expect("materialized above");
    rm.ring_doorbell(host_token)?;
    Ok(DoorbellOutcome {
        proc: pid,
        chan: cid,
        host_token,
        scheduled_now,
    })
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

/// **Case 1**: forward an engine-object alloc (compute / graphics / CE / NVENC) on the
/// channel identified by `vchid`, so the host kernel-RM builds and self-promotes its
/// OWN context. Materializes the host channel lazily (same per-proc discipline as
/// `handle_doorbell`), then allocs the engine object on it via the proc's own isolate.
///
/// `class` is the guest's engine-object class; the arch maps it to an [`EngineKind`]
/// (a class the arch does not recognize as an engine object is a loud `NotAnEngine`).
/// `params` is the ABI-lowered alloc blob (Axis A). MISS=FAULT throughout.
///
/// **Idempotent under replay** (§2.2; the protocol is order-/repeat-independent): a
/// re-sent alloc for a class already forwarded on this channel resolves from
/// [`Channel::host_engine_objects`] and returns the ORIGINAL host object — the host
/// never sees a duplicate engine-object alloc (the guest-retry hazard the graph
/// already covers for alloc/DUP, extended to the host-forward plane).
pub fn forward_engine_object(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    vchid: VChid,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let engine = gpu
        .arch
        .engine_of_object(class)
        .ok_or(FwdFault::NotAnEngine(class))?;
    let (pid, cid) = *gpu
        .by_vchid
        .get(&(target_gpu, vchid))
        .ok_or(FwdFault::UnknownVchid {
            gpu: target_gpu,
            vchid,
        })?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }

    let Proc {
        vases,
        channels,
        isolates,
        ..
    } = proc;
    let chan = channels.get_mut(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: target_gpu,
        vchid,
    })?;
    let cgpu = chan.gpu;
    let rm = isolates
        .get_mut(&cgpu)
        .ok_or(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        })?
        .rm();

    // Lazily materialize the host channel (first-touch, per-proc) so the engine object
    // has a channel to be allocated on — the host builds its GR ctx against it.
    let materialized_channel = chan.host_channel.is_none();
    if materialized_channel {
        let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
        let vas = vases
            .get_mut(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
        let hvas = ensure_host_vas(vas, rm)?;
        // The channel's graph-derived `EngineKind` rides the alloc (GR-1, wrong-runlist
        // class): the adapter cannot invent the runlist, so the core declares it.
        let (hchan, htok) = rm.alloc_channel(hvas, chan.engine)?;
        chan.host_channel = Some(hchan);
        chan.host_token = Some(htok);
    }
    let hchan = chan.host_channel.expect("materialized above");
    // Replay resolves from the idempotency table — exactly one host object per
    // declared (channel, class), no matter how often the guest re-sends.
    if let Some(&host_object) = chan.host_engine_objects.get(&class) {
        return Ok(EngineObjectForwarded {
            engine,
            host_object,
            materialized_channel,
            reused: true,
        });
    }
    let host_object = rm.alloc_engine_object(hchan, class, params)?;
    chan.host_engine_objects.insert(class, host_object);
    Ok(EngineObjectForwarded {
        engine,
        host_object,
        materialized_channel,
        reused: false,
    })
}

/// Route a `GSP_RM_CONTROL` through the Case-1/Case-2 split. A **Case-2** control is
/// ACKed and NOT forwarded (its host effect is already achieved); a **Case-1** control
/// is forwarded to the host on `obj` through the owning proc's isolate.
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
    if gpu.arch.is_case2_control(cmd) {
        // Case 2: ack-only. The host already did it (Case-1). Do NOT replay — an
        // unprivileged replay returns InsufficientPermissions ("wrong layer").
        return Ok(ControlRoute::AckOnly);
    }
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    // Forward on the op's TARGET GPU's isolate (MG-5): the control object `obj` is a
    // handle in that isolate's namespace; routing it elsewhere is unrepresentable.
    let rm = proc
        .isolates
        .get_mut(&target_gpu)
        .ok_or(FwdFault::NoTarget {
            proc: pid,
            gpu: target_gpu,
        })?
        .rm();
    rm.control(obj, cmd, payload)?;
    Ok(ControlRoute::Forwarded)
}

/// The guest is waiting on the GR golden-capture completion (a GSP-event the host's
/// in-kernel FECS capture satisfies). Route it to the **system** proc — it is
/// kernel-internal and content-irrelevant (the guest only needs the *completion* its
/// 4-second poll waits on). Returns the observed os-event ref for assertions.
///
/// Typed to the system proc by construction (lesson L5 / the #12 finishPayload rule):
/// forging a completion for a userspace proc is unrepresentable here.
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

/// Parse the pushbuffer `ring` submitted on channel `cid` of proc `pid`, reading its
/// method words from guest memory via `vmm`. Feeds: CE-PT-write capture → the
/// channel's `Vas.pt_pages` + address table; `SemRelease` → the proc's
/// `CompletionQueue`; honors `TlbInvalidate` membars; passes opaque methods through.
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
    // Walk the GPFIFO entries (arch format), reading each range's method bytes. A
    // hostile GPFIFO entry can name any length; cap the per-range read so a bogus
    // length is a bounded read, never an arbitrary allocation (boundary-1 posture).
    let ranges = gpu.arch.pushbuffer().gpfifo_entries(ring);
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
        methods.extend(decode_methods(gpu.arch.as_ref(), &buf));
        total += len;
    }

    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let chan_pdb = chan.vas_pdb;
    let cgpu = chan.gpu;

    let mut out = PushbufferOutcome::default();
    for (header, args) in methods {
        match gpu.arch.pushbuffer().decode_method(header, &args) {
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
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
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
    let pid = *gpu
        .by_pdb
        .get(&(target_gpu, pdb))
        .ok_or(FwdFault::UnknownPdb {
            gpu: target_gpu,
            pdb,
        })?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
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
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
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
    ControlRoute,
    CompletionArm,
    EngineObjectForwarded,
    PushbufferOutcome,
);
