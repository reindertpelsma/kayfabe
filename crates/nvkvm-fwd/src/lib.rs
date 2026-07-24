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
use nvkvm_arch::ids::{GpuVa, Pdb, VChid};
use nvkvm_completion::PostBatch;
use nvkvm_core::gpu::{Channel, Gpu, Proc, Vas};
use nvkvm_core::{ChanId, ProcId};
use nvkvm_isolate::RmError;
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
