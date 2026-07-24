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
//! - [`handle_doorbell`] — the exec-plane demux: `Arch::decode_doorbell` → vChid →
//!   `by_vchid` → `(Proc, Channel)` → materialize/schedule that channel on **its
//!   proc's own** exec plane (nothing one-shot, nothing scalar — crack ⚠4) → ring its
//!   host token on **its proc's own** isolate.
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

use nvkvm_arch::Aperture;
use nvkvm_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa, Pdb, VChid};
use nvkvm_completion::{OsEventRef, PostBatch};
use nvkvm_core::gpu::{Channel, Gpu, Proc, Vas};
use nvkvm_core::{ChanId, ProcId};
use nvkvm_isolate::{HostHandle, RmError};
use nvkvm_mmu::{AddressFault, Binding};
use nvkvm_vmm::{IrqSpec, Vmm};

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
    /// The decoded vChid has no registered channel — forward-population never saw
    /// its channel-alloc. MISS=FAULT (the C's `bar1_wpg` MRU fallback is designed out).
    UnknownVchid {
        /// The decoded vChid.
        vchid: VChid,
    },
    /// The routed proc exists but is retired (cross-teardown consumption refused —
    /// lesson L10).
    RetiredProc(ProcId),
    /// The channel is not bound to any declared VAS and system routing does not
    /// apply — refusing to guess an address space.
    NoVas(ChanId),
    /// The target proc has no `Vas` for this PDB (data-plane routing miss).
    UnknownPdb(Pdb),
    /// The address table refused (miss/overlap).
    Address(AddressFault),
    /// The proc's GPA arena is exhausted.
    Arena,
    /// The isolate's RM backend refused the op.
    Rm(RmError),
    /// A class the guest tried to alloc as an engine object is not one this arch
    /// recognizes as an engine — MISS=FAULT (never guessed into a GR/CE object).
    NotAnEngine(ClassId),
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

/// Back `[va, va+len)` in the `Vas` identified by `pdb` inside `proc`:
/// carve GPA from the proc's arena, allocate host memory + map it into the Vas's
/// own host VAS via the proc's isolate, and forward-populate the address table.
///
/// Keying discipline (decision #14): the caller routes here via `Gpu::by_pdb` —
/// this function takes the `Proc` only because the `Proc` *owns* the arena and
/// isolate; the address op itself keys on the `Vas`.
pub fn publish_backing(
    proc: &mut Proc,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<Published, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let Proc { vases, arena, isolate, .. } = proc;
    let vas = vases.get_mut(&pdb).ok_or(FwdFault::UnknownPdb(pdb))?;
    let rm = isolate.rm();

    let gpa = arena.alloc(len, 0x1000).map_err(|_| FwdFault::Arena)?;
    let hvas = ensure_host_vas(vas, rm)?;
    let mem = rm.alloc_sysmem(len)?;
    let host_va = rm.map_gpu_va(hvas, mem, len)?;

    vas.table.bind(
        pdb,
        va,
        len,
        Binding { phys: gpa.0, aperture: Aperture::SysmemCoherent, host_va: Some(host_va) },
    )?;
    Ok(Published { gpa: gpa.0, host_va })
}

/// Resolve `va` in the `Vas` identified by `pdb`. Pure lookup; MISS=FAULT.
pub fn resolve(gpu: &Gpu, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), FwdFault> {
    let pid = *gpu.by_pdb.get(&pdb).ok_or(FwdFault::UnknownPdb(pdb))?;
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let vas = proc.vases.get(&pdb).ok_or(FwdFault::UnknownPdb(pdb))?;
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

/// The exec-plane demux: one guest doorbell write → the owning proc's channel rung
/// on the owning proc's isolate.
///
/// Materialization is lazy and **per-proc**: the first doorbell on a channel
/// allocates + schedules its host channel in its Vas's host VAS through its own
/// isolate (no warm-up assumption — testing strategy `wo_channel_alloc_then_
/// immediate_doorbell`), and the "already scheduled" state lives on the proc's
/// [`nvkvm_core::gpu::ExecPlane`] — there is no global one-shot to leave a second
/// proc's channel off-runlist (#12's CTX2 bug, crack ⚠4).
pub fn handle_doorbell(gpu: &mut Gpu, token: u64) -> Result<DoorbellOutcome, FwdFault> {
    let target = gpu
        .arch
        .decode_doorbell(token)
        .ok_or(FwdFault::MalformedToken { token })?;
    let (pid, cid) = *gpu
        .by_vchid
        .get(&target.vchid)
        .ok_or(FwdFault::UnknownVchid { vchid: target.vchid })?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    proc.poll.last_token = Some(token);

    let Proc { vases, channels, exec, isolate, .. } = proc;
    let chan: &mut Channel = channels.get_mut(&cid).ok_or(FwdFault::UnknownVchid {
        vchid: target.vchid,
    })?;
    let rm = isolate.rm();

    // Lazy per-proc materialization.
    if chan.host_channel.is_none() {
        let pdb = chan.vas.ok_or(FwdFault::NoVas(cid))?;
        let vas = vases.get_mut(&pdb).ok_or(FwdFault::UnknownPdb(pdb))?;
        let hvas = ensure_host_vas(vas, rm)?;
        let (hchan, htok) = rm.alloc_channel(hvas)?;
        chan.host_channel = Some(hchan);
        chan.host_token = Some(htok);
    }
    let mut scheduled_now = false;
    if !exec.scheduled.contains(&cid) {
        rm.schedule(chan.host_channel.expect("materialized above"))?;
        exec.scheduled.insert(cid);
        scheduled_now = true;
    }

    let host_token = chan.host_token.expect("materialized above");
    rm.ring_doorbell(host_token)?;
    Ok(DoorbellOutcome { proc: pid, chan: cid, host_token, scheduled_now })
}

/// Post any composable completion batch and raise the SWGEN0 edge. Returns the
/// posted batch, if any. (Queue *encoding* is `nvkvm-gsp`'s job once it ports.)
pub fn deliver_completions(gpu: &mut Gpu, vmm: &mut dyn Vmm) -> Option<PostBatch> {
    let batch = gpu.pump_completions()?;
    vmm.raise_irq(COMPLETION_VECTOR).ok()?;
    Some(batch)
}

/// A proc's completion-poll RPC arrived (`MC_SERVICE_INTERRUPTS`-shaped): re-post
/// its un-acked completions off its OWN poll and raise the edge — the #14 round-8
/// starvation is impossible by construction (§4.3.2).
pub fn poll_completions(gpu: &mut Gpu, vmm: &mut dyn Vmm, pid: ProcId) -> Option<PostBatch> {
    let now = vmm.now();
    let batch = gpu.completion_poll(pid, now)?;
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
}

/// **Case 1**: forward an engine-object alloc (compute / graphics / CE / NVENC) on the
/// channel identified by `vchid`, so the host kernel-RM builds and self-promotes its
/// OWN context. Materializes the host channel lazily (same per-proc discipline as
/// `handle_doorbell`), then allocs the engine object on it via the proc's own isolate.
///
/// `class` is the guest's engine-object class; the arch maps it to an [`EngineKind`]
/// (a class the arch does not recognize as an engine object is a loud `NotAnEngine`).
/// `params` is the ABI-lowered alloc blob (Axis A). MISS=FAULT throughout.
pub fn forward_engine_object(
    gpu: &mut Gpu,
    vchid: VChid,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let engine = gpu.arch.engine_of_object(class).ok_or(FwdFault::NotAnEngine(class))?;
    let (pid, cid) = *gpu
        .by_vchid
        .get(&vchid)
        .ok_or(FwdFault::UnknownVchid { vchid })?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }

    let Proc { vases, channels, isolate, .. } = proc;
    let chan = channels.get_mut(&cid).ok_or(FwdFault::UnknownVchid { vchid })?;
    let rm = isolate.rm();

    // Lazily materialize the host channel (first-touch, per-proc) so the engine object
    // has a channel to be allocated on — the host builds its GR ctx against it.
    let materialized_channel = chan.host_channel.is_none();
    if materialized_channel {
        let pdb = chan.vas.ok_or(FwdFault::NoVas(cid))?;
        let vas = vases.get_mut(&pdb).ok_or(FwdFault::UnknownPdb(pdb))?;
        let hvas = ensure_host_vas(vas, rm)?;
        let (hchan, htok) = rm.alloc_channel(hvas)?;
        chan.host_channel = Some(hchan);
        chan.host_token = Some(htok);
    }
    let hchan = chan.host_channel.expect("materialized above");
    let host_object = rm.alloc_engine_object(hchan, class, params)?;
    Ok(EngineObjectForwarded { engine, host_object, materialized_channel })
}

/// Route a `GSP_RM_CONTROL` through the Case-1/Case-2 split. A **Case-2** control is
/// ACKed and NOT forwarded (its host effect is already achieved); a **Case-1** control
/// is forwarded to the host on `obj` through the owning proc's isolate.
///
/// This is the anti-bolt-on payoff in code: adding an engine adds *rows* to the arch's
/// Case-2 set and its class table — never a new host verb, never a new routing path.
pub fn route_control(
    gpu: &mut Gpu,
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
    proc.isolate.rm().control(obj, cmd, payload)?;
    Ok(ControlRoute::Forwarded)
}

/// The guest is waiting on the GR golden-capture completion (a GSP-event the host's
/// in-kernel FECS capture satisfies). Route it to the **system** proc — it is
/// kernel-internal and content-irrelevant (the guest only needs the *completion* its
/// 4-second poll waits on). Returns the observed os-event ref for assertions.
///
/// Typed to the system proc by construction (lesson L5 / the #12 finishPayload rule):
/// forging a completion for a userspace proc is unrepresentable here.
pub fn signal_golden_capture(gpu: &mut Gpu, event: OsEventRef) -> OsEventRef {
    gpu.system.completion.observe(event);
    event
}
