//! # kayfabe-fwd — intent recovery → unprivileged host ops
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
//! - [`plan_doorbell`] — **the ONE ring gate**: `Arch::decode_doorbell` → vChid →
//!   `by_vchid` → `(Proc, Channel)` (in [`route_doorbell`]) → **the #14 ring-gate** (the
//!   channel's Vas working set must be host-published — structural, not caller
//!   discipline) → a `VerbPlan::Doorbell` that materializes/schedules that channel on
//!   **its proc's own** exec plane (nothing one-shot, nothing scalar — crack ⚠4) and
//!   rings its host token on **its proc's own** isolate.
//!
//!   ★ **corrected 2026-07-27** — this list used to read *"[`handle_doorbell`] — the ONE
//!   ring path (there is no other function that reaches `RmBackend::ring_doorbell`)"*.
//!   That cardinality is false and was found by the whitepaper's verification pass:
//!   `RmBackend::ring_doorbell` has exactly **one** call site and it is inside
//!   `kayfabe_isolate::Worker::execute`, which [`handle_doorbell`] reaches only
//!   indirectly — and the L1 path a real guest MMIO write takes,
//!   `kayfabe_rt::SharedDevice::doorbell`, **never enters [`handle_doorbell`] at all**.
//!   The *safety* property is unchanged, one level down: [`plan_doorbell`] is the sole
//!   constructor of `VerbPlan::Doorbell` in the production crates and it runs the gate
//!   before returning one, so `Worker::execute` has nothing un-gated it could be asked to
//!   ring. [`handle_doorbell`] and `SharedDevice::doorbell` are two **compositions** over
//!   that one gate (single-threaded and L1-sharded); neither is a second door.
//!
//!   ★★ **Closed 2026-07-27**: the residual noted here — *"`VerbPlan` is a public enum,
//!   so the guarantee is over the call graph, not enforced by the type system"* — is
//!   gone. `VerbPlan::Doorbell` is `#[non_exhaustive]` and its only constructor,
//!   [`kayfabe_isolate::VerbPlan::gated_doorbell`], runs the gate; hand-building the
//!   variant no longer compiles anywhere outside `kayfabe-isolate`.
//! - [`deliver_completions`] / [`poll_completions`] — glue from the core's completion
//!   policy to `Vmm::raise_irq` (the SWGEN0 edge; transport encoding is `kayfabe-gsp`'s
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
//! concurrency contract is inherited verbatim from `kayfabe-core` (see its crate
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
//!   verb runs between them, lock-free, on a checked-out [`kayfabe_isolate::Worker`].
//! - The original `&mut Gpu` entry points remain as **split-borrow compositions**
//!   of route+act — the single-threaded / degenerate-one-lock shape the tests and
//!   L1-M1 drive.
//!
//! Functions taking `&Gpu`/`&Spine`/`&Proc` are concurrent-safe under shared
//! borrows; functions taking `&mut` require caller-provided exclusivity — and the
//! `&mut Proc` ones ([`publish_backing`], the act phases) parallelize per-proc
//! (disjoint borrows, no shared lock).

use kayfabe_arch::ids::{
    ClassId, ControlCmd, EngineKind, GpuId, GpuVa, HClient, HObject, Pdb, RunlistId, VChid,
};
use kayfabe_arch::{
    Aperture, CpuOperand, CpuPlane, PhysTarget, PlaneAddr, PushRange, Residency, ResidencyOracle,
};
use kayfabe_completion::{CompletionError, OsEventRef, PostBatch};
use kayfabe_core::gpu::{Channel, Gpu, Proc, Spine};
use kayfabe_core::{ChanId, ProcAnchor, ProcId};
use kayfabe_isolate::{
    CancelReason, ChannelHandles, GuestRamGrant, HostHandle, IsolateId, RmError, VerbPlan,
    VerbReply, Worker, WorkerId,
};
#[doc(inline)]
pub use kayfabe_isolate::{CeExecutor, CeSource, CeSubCopy};
use kayfabe_mmu::AddressTable;
use kayfabe_mmu::{AddressFault, Binding};
use kayfabe_vmm::{FbMeta, IrqSpec, Present, PresentError, SurfaceHandle, Vmm, VmmError};

mod ptdecode;
mod trace;

#[doc(inline)]
pub use ptdecode::{
    Admit, IsolateFb, MAX_PT_META, PT_DECODE_BUDGET, PT_SWEEP_BUDGET, PtDecodeOutcome,
    PtDecodePlan, PtDecodeResult, PtDecodeTask, PtSweepPlan, SweepReason, commit_pt_decode,
    commit_pt_decode_as, commit_pt_sweep, plan_pt_decode, plan_pt_sweep, pt_meta_of, run_pt_decode,
    run_pt_sweep,
};

/// The MSI-X vector completions are raised on. Abstract placeholder until the
/// interrupt-tree model ports (`kayfabe-regs`-equivalent); the mocks assert on it.
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
    /// ★ The op's routing key belongs to a **CONDEMNED component**
    /// (`l1_concurrency.md` §7.3 / §12.13): one of this guest process's isolate workers
    /// died **out of band**, so §7.3's "no resurrect" / `WorkerDied`'s "never a respawn"
    /// make the whole component permanently dead — it has no `Proc`, no isolate, no
    /// GPA arena and no route, and it will get none until the *guest itself* frees
    /// its client root.
    ///
    /// **Why this is a refusal and not a transparent re-materialization.** The isolate
    /// is a process, so the host kernel already reclaimed everything it held; rebuilding
    /// the component's host objects would be *almost* clean. It is wrong anyway, because
    /// **the guest's data died with the isolate** — a published backing is host memory
    /// (`RmBackend::alloc_sysmem`) owned by that isolate's RM client, so a rebuild hands
    /// the guest a **zeroed** backing for a VA it believes still holds its data. Silent
    /// corruption is strictly worse than this fault, which is the semantic real hardware
    /// already has: **sticky-fatal**, like an Xid. It is not a brick — a re-initialising
    /// application (fresh RM client ⇒ different component ⇒ not condemned) is served
    /// normally, and a dying one has its clients freed by the guest kernel.
    ///
    /// Distinct from [`FwdFault::RetiredProc`] because there is no `ProcId` left to
    /// name (the proc was removed and reaped; ids are never reused), and distinct
    /// from [`FwdFault::UnknownPdb`]/[`FwdFault::UnknownVchid`] because the key is
    /// not unknown — it is *forbidden*. The label comes out of the same forward
    /// projection that fills the live routing maps, so naming it costs no reverse
    /// resolution (the `RmGraph::gpu_of` / address-table doctrine).
    Condemned {
        /// The condemned component's deterministic label (its smallest client
        /// handle) — the guest's own identity for the process that lost its worker.
        anchor: ProcAnchor,
    },
    /// The channel is not bound to any declared VAS and system routing does not
    /// apply — refusing to guess an address space.
    NoVas(ChanId),
    /// ★★★ **#177.** The guest rang a channel it never asked us to schedule.
    ///
    /// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) with `bEnable = NV_TRUE` is what
    /// puts a channel into `kayfabe_core::gpu::ExecPlane::requested`; until then a real
    /// GPU would not run its work, because the channel is on no runlist. This fault is
    /// what makes that true here too.
    ///
    /// ⊘ **Why this variant is the whole point of the #177 rung.** Before it,
    /// [`plan_doorbell`] treated scheduling as a *memo*: `let schedule =
    /// !proc.exec.scheduled.contains(&cid)` — a channel that had never been scheduled was
    /// simply scheduled on the fly, so serving `0xa06f0103` would have had **nothing to
    /// perform** and its `NV_OK` would have been unfalsifiable. With this gate the control
    /// has an observable: refuse before, proceed after.
    ///
    /// ⚠ It is deliberately **not** `UnknownVchid` or `NoVas`: the channel is known and
    /// may be perfectly well bound. What is missing is the guest's own declaration.
    NotScheduled {
        /// The channel that was rung.
        chan: ChanId,
        /// Its vChid, as the doorbell decoded it — the identity a bench log can match
        /// against the `gpfifo rings` census line.
        vchid: VChid,
    },
    /// ★★★ **E6.** The proc this submission was routed to holds no channel with this
    /// [`ChanId`].
    ///
    /// ⊘ Distinct from [`FwdFault::UnknownVchid`] by *which namespace missed*. That one
    /// is the **device-wide** exec-plane index failing to route a doorbell's decoded
    /// vChid to any `(proc, chan)` at all; this one is a caller naming a proc-local
    /// channel slot that proc does not have. Folding them would report a caller/adapter
    /// wiring error as a guest routing miss, which is `§12.10`'s wrong-reason conflation
    /// on the arm a bench would be reading.
    ///
    /// ⊘ And distinct from [`FwdFault::NoVas`], which `apply_pushbuffer` returns for the
    /// same absence: that reading predates this variant and is not changed here, because
    /// changing it would move a refusal three corpora assert on. What is not repeated is
    /// the conflation — the **new** site names the truth.
    UnknownChannel {
        /// The proc that was routed to.
        proc: ProcId,
        /// The channel slot it does not hold.
        chan: ChanId,
    },
    /// ★★★ **E6 — the `(proc, gpu)` isolate exists and can NEVER serve a verb.**
    ///
    /// # ⊘ The bug this variant is, and it was reachable for the first time in E6
    ///
    /// `kayfabe_isolate::Isolate::checkout` answers `None` for **two** conditions its own
    /// docs run together: *"the pool is saturated (**or** the isolate is retired and
    /// refuses new checkouts)"*. [`checkout`] used to pass both up as `Ok(None)`, which
    /// `kayfabe_rt::SharedDevice::verb_op` treats as backpressure — release the locks,
    /// **park on the pool gate**, re-enter from the top. That is correct for saturation
    /// and is an **unbounded wait** for the other: a retired isolate (and a
    /// [`kayfabe_isolate::StillbornIsolates`] one, pool width **zero**) never returns a
    /// worker, so the generation the waiter is parked on never moves.
    ///
    /// ⊘ It was **unreachable before E6** only because nothing ever routed that far: the
    /// shipped archive's doorbell died at [`FwdFault::UnknownVchid`] before a checkout was
    /// attempted. The join is what makes a submission reach the pool, and the shipped
    /// default plane **is** the stillborn one — so E6's own control arm (`KAYFABE_ISOLATES`
    /// unset) is precisely the configuration that would have hung a vCPU thread instead of
    /// refusing.
    ///
    /// ⊘ Distinct from [`FwdFault::NoTarget`], which is *no isolate at all* — an internal
    /// inconsistency. This one is *an isolate that is present and permanently dead*, which
    /// is the ordinary state of an archive with no forwarding plane installed, and it is a
    /// legitimate answer to give a guest rather than a bug to report.
    IsolateRetired {
        /// The proc whose isolate refuses.
        proc: ProcId,
        /// The target GPU whose isolate refuses.
        gpu: GpuId,
    },
    /// ★★★ **The `(proc, gpu)` isolate is DECIDED and not yet MATERIALIZED** — R1's spawn
    /// deferral (`l1_concurrency.md` §3.3; `kayfabe_core::gpu::Spine::defer_isolate`).
    ///
    /// Spawning a sandbox is a blocking call, so the spine may only *decide* one under the
    /// device write lock and the shell must spawn it with every guard dropped. Between
    /// those two moments the proc is routable and holds no isolate — a **legal** state,
    /// and the one thing it must not be reported as is [`FwdFault::NoTarget`], whose whole
    /// meaning is *"an internal inconsistency"*.
    ///
    /// ⊘ **Converging staleness, in §12.9's sense — DEFER, not FAULT.** The correct
    /// response is to materialize the pending isolate (lock-free) and re-plan from the
    /// top, which `kayfabe_rt::SharedDevice::verb_op` does; nothing has failed and the
    /// guest must never see it. A caller that cannot re-run surfaces it as the fault it is
    /// for that site — the same per-site categorisation [`checkout`]'s docs make for
    /// `PoolSaturated`. ⊘ Distinct from [`FwdFault::IsolateRetired`], which is permanent.
    IsolatePending {
        /// The proc whose isolate is on its way.
        proc: ProcId,
        /// The target GPU it is being materialized for.
        gpu: GpuId,
    },
    /// ★★★ **E6 — the channel's declared VAS has no HOST address space**, so there is
    /// nowhere to point a copy engine.
    ///
    /// The `Vas` is declared (a PDB resolved, [`FwdFault::UnknownPdb`] did not fire) and
    /// `Vas::host_vas` is still `None`: nothing in this address space has ever been
    /// host-published, so no host VAS was ever materialized.
    ///
    /// ⊘ **A refusal and deliberately not a materialization.** Allocating an empty host
    /// VAS here would let the chain proceed and point a real copy engine at addresses
    /// that resolve to nothing in it — `Xid 31 FAULT_PDE`, which is the failure
    /// [`kayfabe_isolate::CeExecutor`]'s docs name, arrived at by *our* choice rather
    /// than by the guest's. MISS = FAULT applies to the host plane too.
    NoHostVas {
        /// The channel whose submission was refused.
        chan: ChanId,
        /// The PDB of the declared-but-unpublished `Vas`.
        pdb: Pdb,
    },
    /// ★★★ **A guest-RAM pin was asked for at a VA whose backing is not guest RAM.**
    ///
    /// The VA resolves — this is not a miss — and its binding's aperture is something
    /// other than sysmem: the guest's own page tables say these bytes live in the
    /// framebuffer, in a peer's memory, or in fabricated space. Pinning the *hypervisor's*
    /// RAM at that address would publish a range of guest memory under an address the
    /// guest uses for something else entirely.
    ///
    /// ⊘ **Loud, and never downgraded to a skip.** The caller derived a guest-physical
    /// address from this same table before it asked; if the aperture disagrees with what
    /// it derived, the two readings are of different things and neither may be used.
    GuestRamNotSysmem {
        /// The VA that was to be pinned.
        va: GpuVa,
        /// The aperture its binding actually names.
        aperture: Aperture,
    },
    /// ★★★ **A guest-RAM pin was asked for at a VA that is already HOST-PUBLISHED.**
    ///
    /// [`publish_backing`] has minted host sysmem at this address and mapped it there.
    /// Pinning the guest's own pages on top would demand the same host GPU VA twice, and
    /// RM answers a colliding fixed map with `0x51 NV_ERR_NO_MEMORY` — a status that
    /// cannot be told apart from real exhaustion. ⇒ Refused **here**, where the cause is
    /// still legible, rather than at the ioctl where it is not.
    GuestRamAddressTaken {
        /// The VA that was to be pinned.
        va: GpuVa,
        /// The host GPU VA the existing publication occupies.
        host_va: u64,
    },
    /// ★★★★★ **A guest-RAM pin was asked for at a base that IS pinned — for FEWER BYTES.**
    ///
    /// ⊘⊘ **This refusal exists because its absence was a GREEN VERDICT ON A PARTIAL
    /// MAPPING.** Until `w271` the idempotence key was the VA alone, so a 64 KiB request at
    /// a base already described for 32 KiB was answered `already = true` with the 32 KiB
    /// descriptor's own handle, and the second 32 KiB was **never described to RM**. The
    /// caller logged `ALREADY PINNED (idempotent replay) … placed_as_asked=true` and read it
    /// as success. `[measured 2026-08-12, boot `w270_pin`]` the host GPU then faulted at
    /// exactly the first byte past the described extent — `+0x8000`, to the byte — and that
    /// fault is the only reason the truncation was ever visible.
    ///
    /// ⇒ **The pin's identity is the `(base, extent)` PAIR.** A request that asks for more
    /// than is described is not a replay of anything; it is a *new* obligation, and it is
    /// refused **by name, carrying both numbers**, so the caller can describe the remainder.
    ///
    /// ★ Why a refusal rather than a silent widening: this crate may not derive a grant.
    /// `described` and `requested` are the two numbers the **VMM** needs to mint the
    /// remainder's grant from its own layout, and handing them back is the whole content of
    /// this variant. See [`GuestRamGrant::originated_by_the_vmm`]'s name.
    ///
    /// # ★★★★ AND WHY THE RECORD IS NOT REPLACED — the choice, stated where it was made
    ///
    /// [`kayfabe_core::gpu::GuestRamPin`] holds **one** `(host_va, memory, len)`, so a
    /// growing request forces a choice, and it is a correctness question rather than a style
    /// one. The two options were:
    ///
    /// - **(a) replace the record with a larger run.** ⊘ **Refused, and not on taste.** An
    ///   `OS_DESCRIPTOR` is built over a page list fixed at creation — RM has no verb to
    ///   lengthen one — so "replace" means *allocate a second descriptor over a superset of
    ///   the same guest pages*. Between the new map and the old free, RM holds **two
    ///   overlapping descriptors over the same pages**, and the fixed map of the larger one
    ///   lands on a host VA the smaller one still occupies (`0x51 NV_ERR_NO_MEMORY`,
    ///   collision-or-exhaustion, indistinguishable). Unmapping first opens a window in
    ///   which a live engine's operand is unmapped. And dropping the map entry is the only
    ///   record of the pair it named — [a Free can free a NAME, not the object].
    /// - **(b) keep per-run records and describe only the REMAINDER.** ★ Taken. Two
    ///   descriptors, over **disjoint** page sets, at **abutting** VAs. No overlap ever
    ///   exists, nothing is freed, no handle is orphaned, and the guest's addresses resolve
    ///   throughout because neither existing mapping is disturbed.
    ///
    /// ⊘ (b) is also not a new mechanism: a **fragmented** range already becomes several
    /// pins at several bases, and every caller already loops over runs. Growth reaches that
    /// same shape from the other direction.
    ///
    /// [`GuestRamGrant::originated_by_the_vmm`]: kayfabe_isolate::GuestRamGrant::originated_by_the_vmm
    GuestRamPinTooShort {
        /// The base VA, which **is** pinned — for too few bytes.
        va: GpuVa,
        /// How many bytes the live pin at `va` actually describes to RM.
        described: u64,
        /// How many bytes this request named.
        requested: u64,
    },
    /// ★★★★ **A guest-RAM pin's extent COLLIDES with a pin at a different base.**
    ///
    /// The same identity defect as [`FwdFault::GuestRamPinTooShort`], arrived at from the
    /// other side: nothing is pinned at `va` itself, but `[va, va+requested)` contains — or
    /// is reached by — a pin that starts elsewhere. Proceeding would build a second
    /// `OS_DESCRIPTOR` over pages RM has already been given and then ask for a **fixed** GPU
    /// map at a host VA that is occupied, which RM answers `0x51 NV_ERR_NO_MEMORY` — a
    /// status that cannot be told apart from genuine exhaustion.
    ///
    /// ★ [`GuestRamPinOverlap::free_prefix`] is what makes this actionable rather than
    /// merely loud: when the collision starts *after* `va` there is a clear prefix the
    /// caller may describe now, and the rest is reached by continuing past the pin that is
    /// already there. A `free_prefix` of `0` means no progress is possible at this base.
    GuestRamPinOverlaps(GuestRamPinOverlap),
    /// ★★★ **The address table and the page-table WALK disagree about this leaf.**
    ///
    /// The second crossing has two sources for one fact: the guest's own page tables,
    /// read by the CE walker, and this proc's [`kayfabe_mmu::AddressTable`]. When both
    /// answer, they must answer the same — and when they do not, **neither may be used**.
    ///
    /// ⊘ This refusal exists because picking one is the campaign's most expensive
    /// recurring mistake (`two_projections_of_one_fact_disagreeing`, three prior
    /// instances). Backing a leaf at the walk's physical address while the table names a
    /// different one puts a real host GPU object under an address the guest reaches
    /// through the other reading. ⇒ **Refuse, and print both numbers**, so the boot log
    /// carries the disagreement rather than a resolution of it.
    FbLeafDisagrees {
        /// The leaf VA.
        va: GpuVa,
        /// What the guest's own page-table walk said.
        walked: (u64, Aperture),
        /// What this proc's address table said.
        tabled: (u64, Aperture),
    },
    /// ⊘ A framebuffer leaf whose length RM cannot place, refused **by name** rather
    /// than rounded.
    ///
    /// The C rounds the allocation up to 64 KiB and registers the rounded range
    /// (`C: nvkvm_gpu_emul.c:8242-8243`, `asize` vs `tsize`), which makes the object
    /// claim up to 60 KiB of guest framebuffer address space **past the end of the leaf**
    /// — an overhang the establishment copy never fills and the local shadow can no
    /// longer answer for. ⚠ That is the C's, not a defect this port inherits: here a leaf
    /// that is not a whole number of 64 KiB granules is refused, so the overhang is
    /// unrepresentable instead of silent.
    FbLeafGranularity {
        /// The leaf VA.
        va: GpuVa,
        /// Its length, as the walk reported it.
        len: u64,
    },
    /// ⊘ The address table binds this VA, but over a **different range** than the leaf
    /// the walk found. Backing it would either overhang a neighbour or leave a hole, and
    /// which of the two is not something this site can decide.
    FbLeafExtent {
        /// The leaf VA the walk found.
        va: GpuVa,
        /// The length the walk found.
        len: u64,
        /// The `(start, len)` the address table holds instead.
        tabled: (u64, u64),
    },
    /// ★ A copy-engine request partitioned into more sub-copies than [`MAX_CE_SPANS`]
    /// (`#102` stage C2). Guest-influenced on both axes — the request's length and the
    /// address table's fragmentation — so it is bounded, and the bound is a LOUD refusal
    /// rather than a truncation: a partition that stops early silently drops the tail of
    /// a copy.
    CeTooFragmented {
        /// The request's destination.
        dst: GpuVa,
        /// The request's declared length.
        len: u64,
    },
    /// ★★★ **E10b — a copy-engine operand lives in a PEER GPU's memory** and is therefore
    /// neither host-representable in this port's model nor CPU-reachable by the shell.
    ///
    /// A physical operand carrying `SET_{SRC,DST}_PHYS_MODE.TARGET = _PEERMEM`
    /// (`kayfabe_arch::PhysTarget::Peer`), or a fabricated binding whose aperture is
    /// [`Aperture::Peer`]. The residency split gives every `CeExecutor::Ours` sub-copy a
    /// defined [`kayfabe_arch::Residency`] (the framebuffer or guest RAM) — a peer operand
    /// has neither, so it is refused **by name** here rather than silently mistaken for a
    /// framebuffer copy (`clc7b5.h:71`; `PhysTarget::Peer`'s own doc). Loud, never guessed:
    /// modelling peer-to-peer DMA is a deliberate future decision, not something to fall
    /// into through a defaulted plane.
    CePeerOperand {
        /// The operand's address (a physical FB/peer address or a resolved binding phys).
        addr: u64,
    },
    /// ★★★ **The operand's backing is not ours to hold still** —
    /// [`kayfabe_arch::Backing::HostOwned`].
    ///
    /// ⊘ **Not a statement that the case is impossible.** The C artifact ran exactly this at
    /// **host parity** (`mode2_uvm_residency.md`, DECIDED 2026-06-04): a guest managed VA is
    /// backed by a host `cudaMallocManaged` allocation, host UVM owns residency, and real
    /// migration happens below the guest's addressing at GPA→HPA. From here the *plane* is
    /// unchanged — it is still guest RAM through the `Vmm`.
    ///
    /// What is missing is the **interlock**: a CPU copy assumes its operands are stable for
    /// its duration, and a host-owned backing may migrate mid-copy. This port has built no
    /// such interlock, so it stops by name rather than copying from a page that may move.
    /// ★ Separate from [`FwdFault::CePeerOperand`] because *"a second GPU owns it"* and
    /// *"its backing can move under us"* are different findings with different fixes.
    CeUnstableBacking {
        /// The operand's address.
        addr: u64,
    },
    /// ★★★ **E10e — a point resolution was asked of a channel that has no address table
    /// at all** ([`TableOperands::Untracked`]).
    ///
    /// ⊘ Separate from [`AddressFault::Miss`], which is *"this table does not cover that
    /// VA"* — a fact about an address space that exists. This is *"there is no address
    /// space to miss in"*, and the two have different fixes: a miss means the guest never
    /// published the mapping, this means the port never learned the address space. The
    /// distinction is the same one [`FwdFault::NoVas`] draws for a range query, restated
    /// where a **completion semaphore** is the thing being resolved — and a completion
    /// written at a guessed address is `#12`.
    CeNoTable {
        /// The virtual address that had nowhere to resolve.
        va: GpuVa,
    },
    /// ★★★ **E10e — the PUBLISHED-ROOT WALK refused this virtual address.**
    ///
    /// The channel that walls has no `Vas`; its addresses are resolved by descending the
    /// guest's own page tables from the root the guest published (`0x90f10106`), on the
    /// guest's own doorbell demand. That descent has its own refusal vocabulary — an
    /// unmapped or sparse entry **at a named level**, a root in an aperture this device
    /// does not back, a leaf beyond the framebuffer bound — and this is where it crosses
    /// into the forwarding plane.
    ///
    /// ⊘ `kind` is a `&'static str` and not a payload because the walk's finding is
    /// `kayfabe_device::ceresolve::CeResolve`, which this pure crate cannot name and this
    /// `Copy` enum could not hold. It is produced by an **exhaustive match in the crate
    /// that owns that type** (`CeResolve::kind`), so a new walk outcome fails *that* build
    /// until it is named here-compatible. The finding's full detail — the level, the
    /// address, the limit — travels beside the fault in
    /// `kayfabe_rt::ceutils::CeUtilsRefusal`, whose type is free to be as large as the
    /// truth requires.
    CeWalk {
        /// The virtual address the walk was asked for.
        va: GpuVa,
        /// The walk's own finding, by its stable kind.
        kind: &'static str,
    },
    /// ★★★ **E10c — a `CeExecutor::Ours` sub-copy whose needed CPU plane is `None`** — a
    /// straddle no single executor can run.
    ///
    /// A sub-copy is diverted to the shell CPU executor because its `by` is `Ours`, but one
    /// of the ends it must touch is real device memory (`Representability::HostBacked`) or
    /// untracked, which has no [`kayfabe_arch::Residency`]. The shell holds only the emulated
    /// framebuffer and guest RAM, so it cannot reach that end; the isolate cannot run it
    /// either (the other end is fabricated). Refused **by name** rather than guessing a
    /// store — the executor is chosen by *where the bytes live*, and here they live in two
    /// places no one executor spans.
    CpuCeStraddle {
        /// The destination address of the un-runnable sub-copy.
        dst: u64,
        /// Whether the missing plane was the destination's (`true`) or the source's.
        dst_end: bool,
    },
    /// ★★★ **E10c — the emulated framebuffer store refused a CPU CE access.** The copy
    /// resolved to a framebuffer-physical address the `FbStore` would not serve (outside
    /// the advertised framebuffer, or the residency ceiling was reached). Carries the
    /// address and the store's own one-sentence reason (`FbRefused::why`).
    CpuCeFb {
        /// The framebuffer-physical address the access resolved to.
        phys: u64,
        /// The store's reason, verbatim.
        why: &'static str,
    },
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
    ///
    /// ★ **And distinct from [`FwdFault::NonRamGpa`]** — this one means *nothing is
    /// there*; that one means *a device is there*. They are near neighbours over the
    /// same call (`testing_doctrine.md` §2 rule 3), and the day they start reporting as
    /// each other a test must change.
    GpaRead {
        /// The guest-physical address the refused read started at.
        gpa: u64,
    },
    /// ★★★ **The guest aimed a descriptor at a device register window.** A GPFIFO
    /// entry (or any other guest-supplied GPA) named an address that resolves to MMIO
    /// rather than to host RAM, and the port refused to serve it.
    ///
    /// # Why this is its own variant and not a `GpaRead`
    ///
    /// It is not a read failure, it is a **refused lock inversion**. `Vmm::gpa_read` is
    /// in-lock legal (`l1_os_shell.md` §6.1) and this call site runs it under the
    /// device read lock, so an implementation that served a device-aimed GPA would take
    /// the VMM's global lock beneath one of our ranked locks — §6.3's ABBA, constructed
    /// on demand by the guest, and invisible to all four of §6.3's enforcement layers.
    /// `[src] v10.2.0 system/physmem.c:3250` (write) / `:3347` (read) →
    /// `prepare_mmio_access` `:3196-3209`.
    ///
    /// Folding it into [`FwdFault::GpaRead`] would make the one observable signal that
    /// the refusal happened indistinguishable from an ordinary unbacked-page miss — the
    /// §12.10 wrong-reason conflation, on the security-relevant arm.
    NonRamGpa {
        /// The first guest-physical address in the requested range that resolves to a
        /// device region.
        gpa: u64,
    },
    /// ★★★ **A GPFIFO range resolved into an aperture whose bytes are not guest RAM.**
    ///
    /// The address table answered, and the answer was `Vidmem` (or `Peer`), where
    /// [`kayfabe_mmu::Binding::phys`] is a guest **framebuffer** offset — a number in a
    /// different space that happens to be the same width. `Vmm::gpa_read` addresses guest
    /// RAM, so handing it that number would read an unrelated page and hand the method
    /// decoder plausible bytes: the exact "the read succeeded and the bytes are wrong"
    /// failure that [`read_pushbuffer`]'s translation exists to end, one level further in.
    ///
    /// ⊘ Distinct from [`FwdFault::NonRamGpa`] by which plane refused. That one is the
    /// **VMM** saying a guest-physical address names a device; this one is the **address
    /// table** saying the range was never in guest-physical space at all. They are near
    /// neighbours over one call (`testing_doctrine.md` §2 rule 3) and folding them would
    /// lose which plane knew.
    ///
    /// ⊘ It is a refusal and **not** a claim that a real GPU cannot fetch methods out of
    /// video memory. It can; this port serves no framebuffer byte
    /// ([`kayfabe_arch::FbWindow`]: *"Nothing here serves a byte"*), so the honest answer
    /// is that we cannot read it, stated by name.
    ///
    /// ⊘⊘ **CORRECTED `[w235, 2026-08-11]`, above the sentence it corrects: the clause
    /// "this port serves no framebuffer byte" is STALE.** `kayfabe_device::SparseFb` has
    /// served the emulated framebuffer since the BAR1 window landed, and the descent reads
    /// the guest's own ring out of it (`fbRING[p0]@0x1024000=…`). ⇒ this variant is no
    /// longer *"we cannot read it"*; it is **[`VidmemRoute::Refuse`], the default, declining
    /// to**. Pass [`VidmemRoute::OwnFramebuffer`] and a [`FbBytes`] to read it instead.
    /// ★ The stale clause is left in place rather than deleted, because a reader who
    /// remembers the old sentence needs to meet the correction where they look for it.
    PushbufferAperture {
        /// The virtual address whose binding named the aperture.
        va: GpuVa,
        /// The aperture the binding named.
        aperture: Aperture,
    },
    /// ★★★★★ **A vidmem range resolved into our framebuffer, and NOTHING EVER WROTE THAT
    /// PAGE** — forbidden #2 caught on the read side instead of the execute side.
    ///
    /// [`FbBytes::read`] would answer zeros and `Ok`, and a zero-filled GPFIFO ring is
    /// indistinguishable from a legitimately quiet one (`gpfifo_live_entries` stops at the
    /// first zero entry **by design**). So the byte census cannot tell *"the guest wrote no
    /// entries"* from *"we are reading a page the guest never addressed"*, and only
    /// residency can: **a page nothing ever wrote is not in the map.**
    ///
    /// ⊘ Raised **only** when [`FbBytes::page_written`] answers `Some(false)`. A `None` —
    /// *"this store cannot tell you"* — is **unmeasured** and must not be read as `false`;
    /// see [`FbBytes::page_written`] for why that distinction is the `dlen=0` lesson.
    RingFbNeverWritten {
        /// The guest virtual address whose binding resolved into the framebuffer.
        va: GpuVa,
        /// The framebuffer-physical address that has no page.
        phys: u64,
    },
    /// ★★★★★ **A GPGA region's KIND could not be decided truthfully at the bind** —
    /// [`kayfabe_mmu::RegionKindFault`], raised where the mapping is bound rather than
    /// where an operand is classified.
    ///
    /// ⊘ **This replaces `FwdFault::BackingNotGuestVisible`**, and the move is the point.
    /// That fault refused a shadowing backing at *classify* time — after the host object had
    /// been allocated, mapped FIXED at the guest's own VA, and written into the address
    /// table, where every other reader saw a published range. `RegionKind` makes the same
    /// state unconstructible, so the refusal now happens **before** anything is adopted and
    /// the orphans go straight back. ⊘ The old variant is **deleted**, not deprecated: it had
    /// no producer left, and a fault nothing raises is a name a reader will look for in a
    /// census that can never print it.
    ///
    /// Two producers, both decisions:
    /// - `commit_back_fb_leaf` — ruling 3
    ///   ([`kayfabe_mmu::RegionKindFault::FakeFbAtRealGpuVa`]): the FB crossing mints a
    ///   fresh **blank** host vidmem object at a guest framebuffer address whose bytes stay
    ///   in `SparseFb`. `[measured 2026-08-11, w228]` `placed_as_asked=true` **and blank**.
    /// - a bind whose aperture is [`Aperture::Peer`]
    ///   ([`kayfabe_mmu::RegionKindFault::PeerHasNoKind`]).
    RegionKindRefused {
        /// The VA whose region kind could not be decided.
        va: GpuVa,
        /// The address plane's own answer — one vocabulary, not a second one here.
        fault: kayfabe_mmu::RegionKindFault,
    },
    /// ★★★ **The doorbell's ring brought NO decodable GPFIFO entry** — the guest rang for
    /// work and the entry at the cursor read back as nothing.
    ///
    /// # ⊘ Why this is not [`FwdFault::PushTooFragmented`], which is what it used to say
    ///
    /// `[measured 2026-08-09, boots `uvm2_d0fbac0` / `scan1_00865a7` / `vaspan_994bbdc`]`
    /// four consecutive boots reported the UVM channel's wall as
    /// `[FwdFault::PushTooFragmented] { va: GpuVa(0x121010000), len: 0 }`. That name means
    /// *"one range cut into more address-table spans than [`MAX_PUSH_SPANS`]"* — a
    /// **fragmentation limit of ours**, raised at
    /// [`pushbuffer_ranges`]. The measured fact was the opposite: no range existed at all,
    /// because the ring read back as zeros (`scan=64/1024 declared, unread=0,
    /// nonzero=NONE`). ⇒ Every boot log named a bound we never hit, for a submission that
    /// never had a byte to fragment.
    ///
    /// ⊘ A refusal by the **wrong** name is worse than an unnamed one: it is a specific,
    /// actionable, false diagnosis, and `len: 0` is the only thing that ever hinted the
    /// sentence was not about fragmentation at all. `mode2_forwarding_model.md`'s *"refuse
    /// by name"* is a claim about the name being TRUE, not about there being one.
    ///
    /// ★ It carries the index and the declared depth, so *"the cursor is past the end"* and
    /// *"index 0 of a ring the guest never wrote"* are two readings of this variant and not
    /// one.
    RingBroughtNoEntry {
        /// The ring's base GPU virtual address, as the channel declared it.
        ring_va: GpuVa,
        /// The entry index the cursor was at — `0` is the guest's first-ever submission.
        index: u32,
        /// How many entries the channel declared the ring holds.
        entries: u32,
    },
    /// ★★★★ **The submission was READ and DECODED, and not one method in it was a launch
    /// or a release** — so the doorbell moved no bytes and released no semaphore.
    ///
    /// # ⊘ It replaces `NotAnEngine(ClassId(0))`, which was a FALSE name
    ///
    /// `[measured 2026-08-09, boot s19_1dfde1b_cup2]` — `kayfabe-rt/src/ceutils.rs` raised
    /// `FwdFault::NotAnEngine(ClassId(0))` here, with `ClassId(0)` written as a **literal**
    /// in the raise site. No class was ever looked up on this path; `route_engine_object`
    /// — the one place that *does* resolve a class and can honestly report `NotAnEngine` —
    /// is not on the doorbell path at all. So the boot report named an engine-class lookup
    /// that never happened, and `ClassId(0)` was an **absence wearing a number's clothes**:
    /// reading it as *"the channel's class resolved to zero"* is reading a constant.
    ///
    /// This is `RingBroughtNoEntry`'s lesson recurring one layer later, and by the same
    /// mechanism: an existing variant reused for a case it does not describe, whose
    /// argument then had to be invented. ⊘ *"Refuse by name"* is a claim about the name
    /// being **true**.
    ///
    /// # ★★★★ Every field exists to make a null DISCRIMINATE
    ///
    /// *"nothing ran"* has at least four distinct causes and the old refusal could not tell
    /// them apart: the pushbuffer read as empty; it read bytes that decoded to no methods;
    /// it decoded methods the chip's codec recognized none of; or it decoded methods that
    /// are real but belong to a **different engine** — which is the only reading under
    /// which a class was ever the question.
    ///
    /// # ★★★★ §16.66 — THE NAME WAS `SubmissionHasNoLaunch`, AND IT WAS FALSE
    ///
    /// `[measured 2026-08-10, boot `s51_d502ac6_engroute`]` the submission this fired on
    /// **had** a `LAUNCH_DMA`. Its own printed pushbuffer says so:
    /// `[2]sub4/m0x300/Incrementing/n1=0x14`. What it had no *copy*: `0x14` decodes as
    /// `FLUSH_ENABLE | SEMAPHORE_TYPE_RELEASE_FOUR_WORD` with `DATA_TRANSFER_TYPE = NONE`,
    /// a zero-byte timestamped semaphore release, and the codec declined it for a reason
    /// that had nothing to do with a missing method.
    ///
    /// ⊘ *"Submission has no launch"* sends a reader to hunt for a method that is **right
    /// there in the line the fault itself prints**, and it sent several rungs there. The
    /// name is now about what the *decode* produced — no work this executor can perform —
    /// which is true whether the cause is empty bytes, a foreign engine, or a launch whose
    /// shape the codec refuses. ★ Same family as `NotAnEngine(ClassId(0))` and
    /// `PushTooFragmented { len: 0 }` before it, and it is the third time in this file: a
    /// refusal's name is a claim, and a claim that names the wrong noun costs more than
    /// silence because it is *followed*.
    SubmissionDecodedNoWork {
        /// GPFIFO entries this doorbell consumed before decoding.
        entries: u32,
        /// The ring index the cursor started at — **which** submission this is about.
        /// ⊘ Carried because the addressing probe beside this refusal used to describe
        /// entry `0` unconditionally, i.e. a submission an *earlier* doorbell had already
        /// served.
        index: u32,
        /// `(header, args)` method pairs read out of those entries' pushbuffers.
        /// `0` = the ranges carried no method words at all.
        methods: u32,
        /// How many of `methods` the chip's codec turned into [`PushMethod::Opaque`] —
        /// i.e. bytes it read and recognized nothing in. `opaque == methods` with
        /// `methods > 0` is *"we decoded nothing"*; `opaque < methods` is *"we decoded
        /// something, just never a launch"*, and those are different bugs.
        opaque: u32,
        /// ★★★★ **The class the submission's own `SET_OBJECT` named**, straight out of the
        /// guest's method words.
        ///
        /// ⊘ `None` means **no `SET_OBJECT` was present in these bytes** — which is a
        /// different fact from `Some(ClassId(0))` (the guest wrote a `SET_OBJECT` of zero),
        /// and the old refusal collapsed both into the same literal. This is the one honest
        /// answer on this path to *"what engine is this channel driving"*: it is declared by
        /// the guest, in the very bytes that failed to produce a launch, and it is not
        /// recomputed through any resolver.
        set_object: Option<ClassId>,
    },
    /// ★★★ **§16.66 — a `RELEASE_FOUR_WORD_SEMAPHORE` reached an executor with no clock.**
    ///
    /// The record is `{ payload: u64, timestamp: u64 }` and the second half needs a
    /// nanosecond source. ⊘ Refused rather than written as zeros: `0` is a legal `PTIMER`
    /// reading, so a zeroed timestamp is not a smaller answer — it is a **plausible wrong
    /// one**, and a guest that subtracts two of them gets a number it has no way to
    /// distrust. `kayfabe_device::NanoClock`'s own standing rule for this device is *never
    /// answer a free-running counter with a constant*.
    ///
    /// ⚠ Reaching this is a **wiring** fault, not a guest one: the CE session carries
    /// `CePlane::now_ns`, so the only way here is a caller that resolved a four-word
    /// release and then declined to pass the source it had.
    CeReleaseNoClock,
    /// ★★★ **§16.24's admission scope, EXPIRED** — the submission carries a
    /// `GP100_UVM_SW` fault method, so UVM is acting on a fault this port never delivered.
    ///
    /// # ⊘ Why this is a refusal and not a log line
    ///
    /// §16.24 admitted `GP100_UVM_SW` (`0xc076`) with an argument attached: UVM's
    /// `channelAllocate` cannot build a channel without it
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:6110-6122`), and the object's
    /// only in-band use is to hold a subchannel for `FAULT_CANCEL_A` — which nothing can
    /// reach, because this port raises no replayable fault and pushes no shadow-queue
    /// entry. A submission containing one of those methods is that argument being false.
    ///
    /// ⊘ Serving the rest of such a submission would be the *silent* failure this
    /// repository keeps measuring: the copies would run, the cancel would be walked past,
    /// and UVM would proceed believing a fault it is tracking had been cancelled. That is
    /// a wrong ANSWER, not a missing one — the `a_saturated_instrument` family. Refusing
    /// says the true thing.
    ///
    /// ⚠ **Nothing has ever raised this**, and that is the point: `[measured 2026-08-09,
    /// boot s23_10a769c]` nine doorbells served, zero of these, while the routine
    /// `SET_OBJECT GP100_UVM_SW` that heads every UVM CE push was present throughout. It
    /// is a tripwire under an assumption, not a wall anybody stands at.
    UvmFaultMethodWithoutFaultDelivery {
        /// The `NVC076_*` method address read out of the guest's own pushbuffer.
        /// ⊘ Not a constant chosen here — see [`FwdFault::SubmissionDecodedNoWork`] for the
        /// six boots the opposite habit cost.
        method: u32,
    },
    /// A GPFIFO range cut into more address-table spans than [`MAX_PUSH_SPANS`] — a loud
    /// refusal, never a truncated read. See that constant for why the bound exists.
    ///
    /// ⊘ **A bound of OURS, never a statement about the guest** — and never the empty-ring
    /// case, which is [`FwdFault::RingBroughtNoEntry`]. See that variant for the four boots
    /// this one mislabelled.
    PushTooFragmented {
        /// The range's start VA.
        va: GpuVa,
        /// The length the GPFIFO entry declared.
        len: u64,
    },
    /// The isolate's RM backend refused the op.
    ///
    /// ★★★ **§16.105 — it now carries WHICH HOST OBJECT the refused verb was issued
    /// against**, and that is a struct variant rather than a second `RmOn`-shaped
    /// variant on purpose: two variants meaning *"RM refused"* is the
    /// `two_projections_of_one_fact_disagreeing` shape, and every matcher that tests for
    /// one would silently miss the other. Making it a field forces every construction
    /// site to say what it knows, which is what the compiler is for.
    Rm {
        /// Why RM refused.
        err: RmError,
        /// ★ The host object the refused verb named as its target — for a Case-1
        /// engine-object alloc, **the host channel it was attempted on**. `None` for
        /// every verb that has no such target, and for a chain that failed before
        /// reaching one.
        ///
        /// ⊘ **An identity, never a live handle**: for the engine-object alloc whose
        /// channel was materialized inside the same chain, this handle has already been
        /// freed by the unwind that produced the failure. See [`kayfabe_isolate::VerbFailure::on`].
        on: Option<HostHandle>,
    },
    /// A class the guest tried to alloc as an engine object is not one this arch
    /// recognizes as an engine — MISS=FAULT (never guessed into a GR/CE object).
    NotAnEngine(ClassId),
    /// ★★★★★ **§16.80** — an engine-object alloc's `hParent` did not resolve to a channel
    /// this port can name a forwarding target for.
    ///
    /// ⊘ Four distinguishable misses under one variant, because they are four different
    /// facts and a single "unroutable" would make them one. [`EngineParentMiss`] names
    /// which hop declined; none of them is ever guessed past.
    EngineObjectParent {
        /// The client namespace the alloc arrived in.
        client: HClient,
        /// The `hParent` handle the alloc named.
        object: HObject,
        /// Which hop refused.
        why: EngineParentMiss,
    },
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
    /// ★ **The op was CANCELLED** — its requester interrupted the in-flight verb
    /// (`l1_concurrency.md` §5.4, §12.16 gap G4). Not a host failure and not
    /// staleness: the host is fine and the proc is typically still very much alive
    /// (the ordinary case is one guest *thread* dying while its process runs on).
    ///
    /// It exists because without it a cancellation arrives as
    /// [`RmError::Other`] and the failure-path re-validation resolves it to
    /// `FwdFault::Rm(..)` whenever the proc is still live — which is the *normal*
    /// cancellation case. That is §12.10's wrong-reason conflation one layer over: a
    /// canary asserting "it refused" would pass while the fault said "the host
    /// failed" about a host that did nothing wrong.
    ///
    /// **Non-retryable.** Re-issuing work whose requester is gone is not a resolution.
    /// It is §12.9's third staleness shape: non-retryable and orphan-carrying (see
    /// [`kayfabe_isolate::VerbFailure`]).
    ///
    /// The mechanism that produces it — the §5.4 interrupt handshake — is L1-M2's;
    /// this is the vocabulary, landed first so it is not a retrofit.
    Cancelled {
        /// The proc whose op was cancelled. Named because the fault must not read as
        /// "this proc is gone" — it usually is not.
        proc: ProcId,
        /// ★ **WHY** it was cancelled (`l1_os_shell.md` §7.3): a proc exiting, a device
        /// reset, the verb watchdog, or the requesting guest thread taking a signal.
        ///
        /// Carried because *"a fault must name the truth, not the symptom"*, and because
        /// the four are operationally different answers for the guest: `ProcExit` means
        /// the work had no requester left, `Watchdog` means the host was too slow, and a
        /// canary that could not tell them apart would pass on whichever it got — which
        /// is the same shape as §14.8 F4's `VmmError` finding one plane over.
        reason: CancelReason,
    },
    /// ★★ **WEDGED** — the host verb never returned and the requester was released
    /// without a reply (`l1_os_shell.md` §7.5, the two-stage watchdog's second expiry).
    ///
    /// Structurally different from [`FwdFault::Cancelled`] in the way that matters: a
    /// cancellation is a fact about the *requester* and leaves a healthy worker behind;
    /// this is a fact about a **host thread in uninterruptible sleep**, which no
    /// user-space design can kill. What the escape converts is an *unbounded silent
    /// stall* into a *bounded loud failure plus a leak we can name, count and report*.
    ///
    /// It is always accompanied — in the same act, never as a reorderable second step —
    /// by the slot dying permanently and the component being condemned. That pairing is
    /// what makes abandoning the reply safe here and nowhere else (§7.2: the desync
    /// hazard is a *future* reader of that channel, and the escape guarantees there is
    /// none).
    Wedged {
        /// The proc whose verb was abandoned. Its component is condemned by the same
        /// act, so every later op of it faults [`FwdFault::Condemned`].
        proc: ProcId,
        /// The target GPU whose isolate wedged.
        gpu: GpuId,
        /// The pool slot that is now permanently dead.
        worker: WorkerId,
    },
    /// ★ **R5**: the world moved while a verb was in flight lock-free, so the commit
    /// phase's target is no longer what the plan named. MISS=FAULT extends to
    /// staleness — the op surfaces this refusal and does **not** "finish what it
    /// started" against a world that no longer contains its target
    /// (`l1_concurrency.md` §3.3 R5, §11 B5).
    Stale(Stale),
    /// ★ **The SYSTEM proc has no data plane** (`l1_concurrency.md` §12.26): something
    /// asked [`publish_backing`] to allocate host memory on `Gpu::system`.
    ///
    /// This is the rule that keeps a cross-`Proc` host reference *unrepresentable*
    /// rather than merely absent. Guest-kernel work that would need a backing —
    /// the CeUtils scrub, the GR golden capture — is **forged** to the system proc's
    /// completion queue, never forwarded, so the system proc never mints host memory
    /// and can therefore never hold a handle a *user* proc's isolate owns. Every real
    /// byte the guest kernel moves on a user process's behalf is forwarded through
    /// **that user proc's own** isolate, which is also the isolate whose death
    /// reclaims it.
    ///
    /// Loud rather than silent because the day someone needs the system proc to
    /// publish, the lifetime question this rule answers has to be re-opened
    /// deliberately — with a refcount or a global quiesce point — not discovered
    /// afterwards.
    SystemDataPlane,
    /// ★★ **A host backing whose object is not in this publication's own isolate
    /// namespace** (`gpga_address_space.md` §9.3, boundary 2).
    ///
    /// The commit phase is where a host object first enters core state, so it is where
    /// the object's owner scope has to be checked — and under **reservation arenas** the
    /// check stops being belt-and-braces. RM grants *objects, not ranges*: an isolate
    /// that holds an arena handle can map **any** offset in it. So adopting a backing
    /// whose object belongs to another isolate does not hand this proc one range, it
    /// hands it reach over that isolate's entire reservation, and it does so through a
    /// binding that reads as perfectly ordinary everywhere downstream.
    ///
    /// The scope tested is [`kayfabe_isolate::HostHandle`]'s own — `belongs_to`, the
    /// same predicate `Worker::execute`'s foreign-handle gate uses — deliberately, so
    /// there is one notion of "whose object is this" rather than two that can drift.
    ///
    /// Not guest-reachable today (the reply comes from this proc's own worker, which
    /// mints in its own namespace), which is exactly why it is a *loud refusal* and not
    /// an assertion: the day something else fills this reply in — a shared arena
    /// allocator, a replayed capture — the wrong answer must be a refusal rather than a
    /// silently adopted cross-isolate mapping.
    ForeignBacking {
        /// The isolate this publication is for.
        isolate: IsolateId,
        /// The handle that does not belong to it.
        memory: HostHandle,
    },
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
        // ⊘ `on: None` — a bare `RmError` arrived with no verb context, so there is
        // nothing to name. Never guessed from the caller's surroundings.
        FwdFault::Rm { err: e, on: None }
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
/// Re-exported from the isolate port, where it moved when [`kayfabe_isolate::Worker`]
/// gained the ability to return one (§12.16, G4: a mid-chain verb failure has orphans
/// too, and the worker cannot depend on this crate).
///
/// R5's disposition rule made explicit: a commit that refuses must not silently leak
/// what its execute phase already allocated. The caller runs
/// [`Orphans::release_plan`] on the SAME worker, still lock-free, before checking it
/// back in.
///
/// ★ **Correction (§12.16, G4).** This doc used to end "*The one case with no such
/// caller is a proc that vanished entirely — then the whole isolate is retired and its
/// handle namespace dies with it … Both dispositions are decided, neither is a leak.*"
/// Both halves of that were wrong:
///
/// - **The namespace dies at the REAP, not at `retire()`.** `Proc::retire` stops the
///   isolate; the sandbox and its handles survive until `Spine::reap_retired` drops the
///   `Proc` — deferredly, at an adapter-declared quiesce point, and (since G3) only
///   once the isolate is quiesced. Between those two moments the objects are held, not
///   disposed of. That is a *deferred* disposition, which is a fine thing to have and
///   a different thing from what the sentence claimed.
/// - **There is a third disposition, and it was unnamed:** a worker that dies
///   mid-chain. Nothing unwinds, the reply never returns, and everything allocated
///   before the failure point is in no `Orphans`, in no core state, and enumerable from
///   nothing. Its only backstop is the same bulk one the C had — the session's fds
///   closing at reap (`C: src/qemu/virtio_nvgpu.c:100-118`, the #80 reaper).
///
/// See [`kayfabe_isolate::VerbFailure`] for the precise limits of what can be
/// enumerated, including the open question about interrupted allocs.
pub use kayfabe_isolate::Orphans;

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
/// ★ **This is the miss taxonomy at the commit seam** (`kayfabe_core` crate docs):
/// converging staleness is a **DEFER** (the world moved *toward* an answer, so re-plan
/// against it — bounded by `MAX_COMMIT_RETRIES` because a defer must terminate) and
/// divergent staleness is a **FAULT** (the plan's target is gone; nothing that can arrive
/// brings it back). The ledger of that defer — every host object a losing attempt
/// allocated, released exactly once — is pinned by `tests/retry_ledger.rs`.
///
/// So: `retry = true` ⇒ *converging* staleness (re-plan, bounded); `retry = false` ⇒
/// *divergent* staleness (the target is gone — MISS=FAULT, surface it).
///
/// ★ **`#[must_use]` (§12.16, G4):** a dropped `Refusal` silently leaks every host
/// object in its [`Orphans`] — the disposition rule this type exists to carry, undone
/// by a missing semicolon's worth of attention. The compiler is the enforcement.
#[must_use = "a dropped Refusal discards its `orphans` — every host object it names \
              leaks. Release them on the checked-out worker (`orphans.release_plan()`) \
              and surface `fault`, or hand the whole Refusal onward."]
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
///
/// ★ **The miss taxonomy, with the CALLER choosing the category** (`kayfabe_core` crate
/// docs). "No worker available" is a fact that *will* change — a round trip ends and a
/// slot returns — so it is **DEFER** for any caller that can wait: `SharedDevice::verb_op`
/// parks on the pool gate and re-enters with full R5 re-validation, and the wait is
/// counted so saturation is distinguishable from a hang
/// (`kayfabe_rt::device::PoolWaits`). For a caller that *cannot* wait (the single-threaded
/// composed entry points) the same absence surfaces as `FwdFault::PoolSaturated` — a
/// FAULT. Both are correct because the category is a property of the site: the same fact
/// is deferrable exactly when the site can be re-run.
///
/// The missing-isolate arm below is unconditionally FAULT: a `(proc, gpu)` with no
/// isolate is an internal inconsistency, not a fact awaiting arrival.
///
/// ★★★ **E6 — and so is the PERMANENTLY DEAD one, which used to be `Ok(None)`.** The
/// paragraph above chooses the category per *site*; that choice is only sound when the
/// absence really is *"a fact that will change"*. It is not, for an isolate that is
/// retired or has pool width zero: [`kayfabe_isolate::Isolate::checkout`]'s own docs fold
/// saturation and permanent refusal into one `None`, and a caller that parks on the second
/// one parks forever. See [`FwdFault::IsolateRetired`] for why E6 is where this became
/// reachable.
pub fn checkout(proc: &mut Proc, gpu: GpuId) -> Result<Option<Worker>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    // ★★★ R1's deferral: "no isolate" now has TWO meanings and only one of them is an
    // inconsistency. See [`FwdFault::IsolatePending`] and [`missing_isolate`].
    let absent = missing_isolate(proc, gpu);
    let iso = proc.isolates.get_mut(&gpu).ok_or(absent)?;
    match iso.checkout() {
        Some(w) => Ok(Some(w)),
        None if never_serves(&**iso) => Err(FwdFault::IsolateRetired { proc: pid, gpu }),
        None => Ok(None),
    }
}

/// ★★★ **E6 — will this isolate EVER hand out a worker again?** The predicate that
/// separates [`kayfabe_isolate::Isolate::checkout`]'s two `None`s.
///
/// Both conditions are "never", and they are checked rather than assumed of each other: a
/// [`kayfabe_isolate::StillbornIsolates`] one is retired **and** zero-width, a real isolate
/// that retired mid-life is retired and non-zero-width, and only a live pool that is
/// momentarily full is neither. [`kayfabe_isolate::Isolate::pool_size`] is documented to
/// *"never change over the isolate's life"*, so a zero here cannot become a one.
///
/// ⊘ It lives in **one** function because there are **two** checkout doors ([`checkout`]
/// and [`checkout_and_drain`]) and only the second is on the L1 path. Fixing the first
/// alone is a change that passes every unit test and leaves the hypervisor hanging — which
/// is what happened, and is why the mutation had to be shown to change *behaviour* on the
/// path L1 actually takes rather than merely to change bytes.
fn never_serves(iso: &dyn kayfabe_isolate::Isolate) -> bool {
    iso.is_retired() || iso.pool_size() == 0
}

/// ★★★ **Which absence is this?** — the R1-deferral counterpart of [`never_serves`], and
/// it lives in one function for the identical reason, one order of magnitude louder:
/// [`never_serves`] had **two** doors to keep in step, this has **six**. Five of them are
/// PLAN-phase (`plan_publish`, `plan_doorbell`, `plan_engine_object`, `plan_control`,
/// `plan_ce`) and refuse *before* the checkout is ever reached, so fixing the checkout
/// alone is a change that passes every unit test and still refuses a legal state on the
/// path L1 actually takes.
///
/// ⊘ **That is not hypothetical, it is what happened here**: with only the two checkout
/// doors converted, `a_verb_that_lands_in_the_gap_…` failed with
/// `NoTarget { proc: ProcId(1) }` — `plan_doorbell` had already refused. The tests are the
/// only reason the other four were found.
///
/// - `Proc::wants_isolate` ⇒ [`FwdFault::IsolatePending`] — decided, spawning, DEFER.
/// - otherwise ⇒ [`FwdFault::NoTarget`] — nothing ever asked for one, FAULT.
///
/// ⊘ **Deliberately NOT applied at the COMMIT-phase target check** (`commit_control`):
/// there the isolate was present when the plan was made, so its absence is divergent
/// staleness (`Stale::Target`) and re-materializing it would be finishing what a vanished
/// world started.
fn missing_isolate(proc: &Proc, gpu: GpuId) -> FwdFault {
    if proc.wants_isolate(gpu) {
        FwdFault::IsolatePending { proc: proc.id, gpu }
    } else {
        FwdFault::NoTarget { proc: proc.id, gpu }
    }
}

/// ★★ [`checkout`] **plus T0's opportunistic drain** (`l1_os_shell.md` §7.6 T0, gap G2)
/// — the form every L1 verb-issuing site uses.
///
/// A checked-out worker is exactly the opportunity T0 names ("*opportunistically at the
/// next verb-issuing op for that proc — the worker is checked out anyway, near-zero
/// marginal cost*"), so the queue rides out of the locked phase with the worker rather
/// than needing a mechanism of its own. The returned [`Orphans`] is empty unless a
/// previous `refresh` dropped a `Vas` or a `Channel` of this `(proc, gpu)` while the proc
/// stayed alive **and** that isolate was otherwise idle — see
/// [`Proc::checkout_with_pending_release`] for why the idle test is load-bearing and why
/// the two must be one act.
///
/// The refusals are [`checkout`]'s, unchanged and checked first, so a retired proc or an
/// unmaterialized target never reaches the drain: a retired isolate refuses every verb
/// including the release, and its disposition of record is the session's death (§7.0).
///
/// # Errors
/// - [`FwdFault::RetiredProc`] — the proc is retired.
/// - [`FwdFault::NoTarget`] — no isolate for this `(proc, gpu)`, and none was asked for.
/// - [`FwdFault::IsolatePending`] — one was asked for and is still being spawned (R1).
/// - [`FwdFault::IsolateRetired`] — the isolate is present and can never serve a verb.
pub fn checkout_and_drain(
    proc: &mut Proc,
    gpu: GpuId,
) -> Result<(Option<Worker>, Orphans), FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    if !proc.isolates.contains_key(&gpu) {
        // ★★★ R1's deferral — the SAME discrimination [`checkout`] makes, and it has to be
        // made here too because **this** is the door L1 goes through. See
        // [`missing_isolate`].
        return Err(missing_isolate(proc, gpu));
    }
    let pid = proc.id;
    let (worker, orphans) = proc.checkout_with_pending_release(gpu);
    // ★★★ E6 — the SAME discrimination [`checkout`] makes, and it has to be made here too
    // because **this** is the door L1 goes through (`Staged::check_out`). See
    // [`never_serves`] and [`FwdFault::IsolateRetired`].
    //
    // ⊘ Nothing is dropped by refusing here: `Proc::checkout_with_pending_release` only
    // detaches the pending-release queue when it hands out a worker, so a `None` worker
    // always carries an empty `Orphans`.
    if worker.is_none()
        && proc
            .isolates
            .get(&gpu)
            .is_some_and(|iso| never_serves(&**iso))
    {
        debug_assert!(orphans.is_empty(), "a refused checkout detaches no queue");
        return Err(FwdFault::IsolateRetired { proc: pid, gpu });
    }
    Ok((worker, orphans))
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
                // Residue of a failed release has no sink in the core yet — see
                // `dispose_on` and §12.16's "what remains".
                let _ = worker.execute(&r.orphans.release_plan());
            }
            r.fault
        }),
        // ★ G4 (§12.16): cancellation is named apart from host failure here too, and
        // the failure's own orphans get a disposal attempt on the same worker before
        // it is checked back in.
        Err(f) => {
            let reason = worker.cancel_observed();
            // ★ Read BEFORE `f.orphans` is moved out below — see `VerbFailure::on`.
            let on = f.on;
            // ★★ §7.5 — a WEDGED worker cannot dispose of anything: it is still inside
            // the ioctl that wedged it. Asking it to would produce a second wedge, so
            // the chain's intermediates go straight onto the proc's `pending_release`
            // queue, where §12.35's audit can NAME them. Every other failure still gets
            // its disposal attempt on the same live worker first — and what that could
            // not dispose of is staged as well, closing the `let _ =` §12.16 left here.
            let residue = if f.err == RmError::Wedged {
                f.orphans
            } else {
                dispose_on(&mut worker, f.orphans)
            };
            proc.stage_release(gpu, residue);
            Err(match f.err {
                RmError::Wedged => FwdFault::Wedged {
                    proc: pid,
                    gpu,
                    worker: worker.id(),
                },
                e => verb_fault(pid, e, reason, on),
            })
        }
    };
    checkin(proc, gpu, worker);
    out
}

/// Best-effort disposal of a verb failure's `orphans` on the SAME worker, still
/// lock-free — and it hands back **what it still could not dispose of**
/// (`l1_concurrency.md` §12.16, gap G4).
///
/// The residue has no core-side sink yet: recording undisposed host objects across a
/// proc's lifetime is the reclamation ledger, which is L1-M2's to design. Until it
/// exists the disposition of record is the one the C also relied on — the isolate's
/// whole handle namespace dying when its session is reaped
/// (`C: src/qemu/virtio_nvgpu.c:100-118`). This function exists so that the residue is
/// a **named, returned value** at every call site rather than a swallowed `let _ =`,
/// which is what makes the ledger an addition later instead of a retrofit.
#[must_use = "the returned residue is the set of host objects that STILL exist and \
              could not be disposed of — bind it and say what happens to it."]
pub fn dispose_on(worker: &mut Worker, orphans: Orphans) -> Orphans {
    if orphans.is_empty() {
        return orphans;
    }
    match worker.execute(&orphans.release_plan()) {
        Ok(_) => Orphans::default(),
        Err(f) => f.orphans,
    }
}

/// ★ Surface a lock-free verb failure as a forwarding fault, keeping **cancellation
/// distinct from host failure** (`l1_concurrency.md` §12.16, gap G4; §12.10 one layer
/// over).
///
/// [`RmError::Interrupted`] is a fact about the *requester* — a guest thread died or
/// took a signal and its in-flight verb was interrupted (§5.4). Reporting it as
/// `FwdFault::Rm` would say "the host refused" about a host that did exactly what it
/// was asked. Every other `RmError` is a genuine host refusal and stays one.
///
/// ★ `reason` is what the worker's own cancel seam **observed**
/// ([`kayfabe_isolate::Worker::cancel_observed`]), read lock-free by the executing
/// thread. `None` with an `Interrupted` error means the break signal landed but nobody
/// recorded why — a backend bug, not a guest condition — so it is surfaced as
/// [`CancelReason::GuestSignal`], the §5.4 founding case, rather than guessed at or
/// silently re-typed as a host failure.
///
/// ★★★ **§16.105 — `on` is [`kayfabe_isolate::VerbFailure::on`], passed through.** It is
/// a parameter rather than something re-derived here because the only party that ever
/// knew it is the worker: a Case-1 engine-object alloc that materializes its own channel
/// has `EngineObjectPlan::channel == None`, and the unwind frees the channel it built. ⊘
/// It is deliberately dropped on the `Interrupted` arm: a cancellation is a fact about
/// the *requester*, and naming a host object there would invite the reading that the host
/// object is what went wrong.
#[must_use]
pub fn verb_fault(
    proc: ProcId,
    err: RmError,
    reason: Option<CancelReason>,
    on: Option<HostHandle>,
) -> FwdFault {
    match err {
        RmError::Interrupted => FwdFault::Cancelled {
            proc,
            reason: reason.unwrap_or(CancelReason::GuestSignal),
        },
        e => FwdFault::Rm { err: e, on },
    }
}

/// Result of one backing publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// The GPA carved from the proc's private arena.
    pub gpa: u64,
    /// The host GPU VA this range is addressable at — **equal to the guest VA**
    /// (`#102`, address identity). It is reported rather than dropped because a caller
    /// that just published wants the fact confirmed, not re-derived.
    pub host_va: u64,
    /// ★ #102 — the host memory object backing the range, in the minting isolate's
    /// namespace.
    ///
    /// Added because address identity took a fact away: the host VA used to encode which
    /// isolate produced it (the mock minted it out of `(proc, GPU)` bit lanes, and half a
    /// dozen tests read provenance straight off those bits). It cannot any more — the
    /// address is now the *guest's* number and says nothing about who mapped it. A
    /// [`HostHandle`] does say, exactly and by type: it is `(Proc, GpuId)`-scoped, so
    /// `memory.isolate()` is the provenance those tests were approximating.
    pub memory: HostHandle,
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

/// ★★★★ What [`FwdFault::GuestRamPinOverlaps`] carries. A struct rather than four inline
/// fields because the caller acts on the *combination* — `free_prefix` is only meaningful
/// beside the base it is a prefix of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamPinOverlap {
    /// The base VA that was asked for.
    pub va: GpuVa,
    /// How many bytes the request named.
    pub requested: u64,
    /// The base of the pin that is in the way.
    pub existing_base: u64,
    /// How many bytes that pin describes.
    pub existing_len: u64,
    /// ★ How much of `[va, va+requested)` is clear of it — `existing_base - va` when the
    /// collision starts inside the request, and **`0`** when the pin in the way starts
    /// below `va` and reaches into it. ⊘ Zero means *no progress is possible at this base*,
    /// and a caller that loops on this fault must treat it as terminal or it will spin.
    pub free_prefix: u64,
}

/// ★★★★★ **What one guest-RAM pin produced** — [`pin_guest_ram`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamPinned {
    /// The host GPU VA the guest's pages were placed at. Equal to the VA asked for, or
    /// the verb refused with [`kayfabe_isolate::RmError::PlacementRefused`] and this
    /// value never existed.
    pub host_va: u64,
    /// The `OS_DESCRIPTOR` object RM built over the guest pages.
    pub memory: HostHandle,
    /// ★★★ Whether this call did the work, or found it already done.
    ///
    /// ⊘ Reported rather than hidden, and it is the field a caller on a **doorbell** needs
    /// most: the pin is idempotent and a doorbell repeats, so a caller that could not tell
    /// "pinned now" from "was already pinned" would log a first-time event on every ring
    /// and a reader would conclude the descriptor was being re-created.
    ///
    /// ⚠ **`already` is now only ever true for a FULLY COVERED replay.** A request that asks
    /// for more bytes than the live pin describes is [`FwdFault::GuestRamPinTooShort`], not
    /// an `already`. Before `w271` it was the latter, and that was a green verdict on a
    /// partial mapping.
    pub already: bool,
    /// ★★★★★ **How many bytes are described to RM at [`GuestRamPinned::host_va`] after this
    /// call** — the extent, reported beside the address so a caller can print *requested*
    /// and *described* side by side.
    ///
    /// ⊘ For a fresh pin this equals the grant's length by construction. For a replay it is
    /// the **live pin's** length, which is `>=` what was asked (a shorter live pin refuses).
    /// It is carried explicitly because *"a mismatch should be read, not inferred"* is the
    /// whole lesson of the boot that produced this field.
    pub described: u64,
}

/// The ID-shaped hints [`commit_pin_guest_ram`] re-validates against. Identities only.
///
/// ⊘ **The grant is carried whole and is never recomputed here.** It is the VMM's
/// statement; the core's job is to route it, not to check it. A core that re-derived an
/// offset would need a layout, and the only layout it could build is one derived from the
/// guest-physical address — which is `kayfabe_vmm_qemu::layout`'s `-m 8G` bug, arrived at
/// from the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinGuestRamPlan {
    /// The owning proc.
    pub proc: ProcId,
    /// The target GPU (isolate key).
    pub gpu: GpuId,
    /// The `Vas`'s PDB.
    pub pdb: Pdb,
    /// The guest VA the pages must be addressable at.
    pub va: GpuVa,
    /// The VMM's grant.
    pub grant: GuestRamGrant,
    /// The `Vas`'s host VAS as observed at plan time.
    pub host_vas: Option<HostHandle>,
    /// ★ The pin this plan found already live, if any — the idempotent replay.
    pub existing: Option<kayfabe_core::gpu::GuestRamPin>,
}

/// ★★★★★ **Pin `[va, va+grant.len())`'s GUEST pages into the host VAS at `va`.**
///
/// `guest_ram_crossing.md` §5.8, step 3. The difference from [`publish_backing`] is the
/// whole point and is one word: the bytes are the **guest's**. `publish_backing` mints
/// host sysmem and maps it at the guest's address, which is right for a range the guest
/// has never written and wrong for a ring the guest is polling.
///
/// # Errors
/// Every arm of [`FwdFault`] the plan can raise, plus whatever the host refused.
pub fn pin_guest_ram(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    grant: GuestRamGrant,
) -> Result<GuestRamPinned, FwdFault> {
    let planned = plan_pin_guest_ram(proc, gpu, pdb, va, grant)?;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_pin_guest_ram(proc, &planned.plan, reply)
    })
}

/// ★★★★ **Is any live guest-RAM pin in the way of `[va, va+len)`?** — the extent key's
/// second half, asked of a base that is NOT itself pinned.
///
/// ⊘ Callers must have already handled the exact-base case; this deliberately ignores a pin
/// at `va` itself so that the two questions cannot answer each other. Returns the **nearest**
/// obstruction, so a caller that loops makes progress in the direction it is walking.
fn overlapping_pin(
    vas: &kayfabe_core::gpu::Vas,
    va: GpuVa,
    len: u64,
) -> Option<GuestRamPinOverlap> {
    // ⊘ A zero-length request cannot collide with anything, and treating it as if it could
    // would refuse an empty grant for a reason that is not true of it.
    if len == 0 {
        return None;
    }
    let end = va.0.saturating_add(len);
    // 1. A pin that starts BELOW `va` and reaches into the request. `free_prefix = 0`: there
    //    is no clear byte at `va` at all, so no caller can make progress here.
    if let Some((&base, pin)) = vas.guest_ram_pins.range(..va.0).next_back()
        && base.saturating_add(pin.len) > va.0
    {
        return Some(GuestRamPinOverlap {
            va,
            requested: len,
            existing_base: base,
            existing_len: pin.len,
            free_prefix: 0,
        });
    }
    // 2. A pin that starts INSIDE the request. Everything below it is clear, and that
    //    prefix is what the caller may describe now.
    let (&base, pin) = vas.guest_ram_pins.range(va.0..end).next()?;
    Some(GuestRamPinOverlap {
        va,
        requested: len,
        existing_base: base,
        existing_len: pin.len,
        free_prefix: base - va.0,
    })
}

/// PLAN (R1): decide the pin's host work from core state. A pure `&Proc` read.
///
/// ## What it checks, and the one thing it deliberately does NOT
///
/// ★★★★★ **AMENDED 2026-08-12 (`w271`), above the sentence it qualifies.** The paragraph
/// below said the length is checked against *nothing*, and that was read — including by this
/// function's own author — as forbidding the comparison `w270`'s wall turned out to need.
/// ⊘ **It does not, and the distinction is the whole of `w271`.** There are two different
/// questions and only one of them is an echo:
///
/// - *"is this length CORRECT?"* — unanswerable here, exactly as written below. The layout
///   that produced it is the hypervisor's; the only thing the core could check it against is
///   the request itself. That reasoning stands, unweakened.
/// - *"does this request name MORE than the extent WE ALREADY DESCRIBED to RM?"* — an
///   ordinary question about **our own record of work we performed**. `GuestRamPin::len` is
///   not guest input and not hypervisor input; it is what a previous call on this path put
///   there. Comparing against it is no more an echo than comparing against `host_vas` is.
///
/// ⇒ The comparison is made, and only that one. `[measured 2026-08-12, boot `w270_pin`]`
/// its absence let a 64 KiB request replay a 32 KiB descriptor and report success, and the
/// host GPU faulted at the first undescribed byte.
///
/// It checks facts about **this address space**: that the VA resolves at all, that its
/// binding is sysmem, that nothing is already host-published there, and whether a pin is
/// already live **and long enough**. ⊘ It does **not** check the grant's offset or length
/// *for correctness* against anything. There is nothing in the core to check them against —
/// the layout that produced them is the hypervisor's — and a check invented here would be a
/// check of a request against itself, which is [an echo is unverifiable by its reply].
///
/// # Errors
/// [`FwdFault::RetiredProc`], [`FwdFault::SystemDataPlane`], [`FwdFault::UnknownPdb`],
/// [`FwdFault::NoTarget`], [`FwdFault::Address`] (a miss),
/// [`FwdFault::GuestRamNotSysmem`], [`FwdFault::GuestRamAddressTaken`], or the isolate
/// deferral.
pub fn plan_pin_guest_ram(
    proc: &Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    grant: GuestRamGrant,
) -> Result<Planned<PinGuestRamPlan>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    // ★ §12.26's system-plane rule, for the same reason `plan_publish` states it: the
    // system proc's isolate exists to serve the device's own bring-up and never the data
    // plane, and a guest-RAM pin is as data-plane as it gets.
    if proc.id == Gpu::SYSTEM_PROC {
        return Err(FwdFault::SystemDataPlane);
    }
    let pid = proc.id;
    let vas = proc
        .vases
        .get(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    if !proc.isolates.contains_key(&gpu) {
        return Err(missing_isolate(proc, gpu));
    }
    // ★★★★★ THE IDEMPOTENCE ARM, and its key is the `(base, extent)` PAIR.
    //
    // It is FIRST among the address checks on purpose: a live pin makes every check below
    // true-by-construction, so asking them first would refuse a replay for a condition the
    // replay itself created.
    //
    // ⊘⊘ **The extent half of the key is `w271`, and its absence was measurable.** Until
    // then this arm asked only `get(&va.0)`, so a 64 KiB request at a base described for
    // 32 KiB replayed the 32 KiB descriptor and the caller printed `ALREADY PINNED …
    // placed_as_asked=true`. `[measured 2026-08-12, boot `w270_pin`]` the host GPU faulted
    // at the first byte past the described extent. A green supply row held the wall in
    // place, and only an `Xid` from an independent authority made it visible.
    if let Some(existing) = vas.guest_ram_pins.get(&va.0).copied() {
        // ⊘ `<`, not `!=`: a request for FEWER bytes than are described is genuinely
        // covered. Refusing it would turn every re-derivation that happens to name a
        // shorter run into a fault, and there is nothing wrong with a shorter ask.
        if existing.len < grant.len() {
            return Err(FwdFault::GuestRamPinTooShort {
                va,
                described: existing.len,
                requested: grant.len(),
            });
        }
        return Ok(Planned {
            plan: PinGuestRamPlan {
                proc: pid,
                gpu,
                pdb,
                va,
                grant,
                host_vas: vas.host_vas,
                existing: Some(existing),
            },
            // ⊘ No verbs at all. `verb_op` commits straight through and never touches the
            // isolate pool — the same shape an idempotent engine-object re-send takes.
            verbs: None,
        });
    }
    // ★★★★ …and the same identity question from the OTHER side: nothing is pinned at `va`,
    // but something may be pinned INSIDE `[va, va+len)`, or may start below `va` and reach
    // into it. Both are collisions, and both would otherwise reach RM as a *fixed* map at an
    // occupied host VA — answered `0x51 NV_ERR_NO_MEMORY`, indistinguishable from real
    // exhaustion. ⇒ Refused here, where the cause is still legible.
    if let Some(overlap) = overlapping_pin(vas, va, grant.len()) {
        return Err(FwdFault::GuestRamPinOverlaps(overlap));
    }
    // ★★★ The guest's own page tables are the authority on what lives at `va`, and this
    // is where their answer is consulted. MISS = FAULT: an unbound VA is refused rather
    // than pinned speculatively.
    let (binding, _off) = vas.table.resolve(pdb, va)?;
    match binding.aperture() {
        Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => {}
        aperture => return Err(FwdFault::GuestRamNotSysmem { va, aperture }),
    }
    if let Some(host_va) = binding.host_va() {
        return Err(FwdFault::GuestRamAddressTaken { va, host_va });
    }
    let host_vas = vas.host_vas;
    Ok(Planned {
        plan: PinGuestRamPlan {
            proc: pid,
            gpu,
            pdb,
            va,
            grant,
            host_vas,
            existing: None,
        },
        verbs: Some(VerbPlan::PinGuestRam {
            host_vas,
            grant,
            at: va,
        }),
    })
}

/// COMMIT (R5): re-resolve through IDs and record the pin, or refuse and hand back what
/// could not be adopted.
///
/// # Panics
/// If `reply` is not the [`VerbReply::GuestRamPinned`] its plan asked for.
///
/// # Errors
/// [`Refusal`] carrying [`Stale`] when the proc or the `Vas` moved under the chain.
pub fn commit_pin_guest_ram(
    proc: &mut Proc,
    plan: &PinGuestRamPlan,
    reply: Option<VerbReply>,
) -> Result<GuestRamPinned, Refusal> {
    // ★ The replay arm. No verbs ran, so there is nothing to adopt and nothing to orphan.
    if let Some(existing) = plan.existing {
        let None = reply else {
            return wrong_reply("pin_guest_ram replay");
        };
        return Ok(GuestRamPinned {
            host_va: existing.host_va,
            memory: existing.memory,
            already: true,
            // ★ The LIVE pin's extent, not the request's. `plan_pin_guest_ram` has already
            // refused the case where this would be smaller than what was asked, so a caller
            // printing `requested` beside this can only ever see `described >= requested`
            // on a replay — and if it ever sees otherwise, the plan arm has regressed.
            described: existing.len,
        });
    }
    let Some(VerbReply::GuestRamPinned {
        host_vas: fresh_vas,
        mapped,
        memory,
        host_va,
    }) = reply
    else {
        return wrong_reply("pin_guest_ram");
    };
    // ⚠ `mapped` is NOT in this list, and cannot be: `Orphans` frees RM objects and
    // unmaps GPU VAs. The isolate's own window onto the guest pages is released when the
    // isolate dies, which `GuestRamPlane`'s ownership makes a destructor rather than a
    // step — see that type. So a refused commit leaks a mapping until proc teardown and
    // **nothing else**, which is stated here rather than left for a reader to derive.
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
    let isolate = IsolateId::new(pid.0, plan.gpu);
    if !memory.belongs_to(isolate) {
        return Err(Refusal {
            fault: FwdFault::ForeignBacking { isolate, memory },
            // Only what is OURS goes on the release list — the same judgement
            // `commit_publish` makes about a foreign object.
            orphans: Orphans::default(),
            retry: false,
        });
    }
    let Some(vas) = proc.vases.get_mut(&(plan.gpu, plan.pdb)) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Vas {
                gpu: plan.gpu,
                pdb: plan.pdb,
            }),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    };
    // ★★ R5's rebind check, in the shape `commit_publish` uses: a sibling thread may have
    // materialized a host VAS in the gap, and adopting ours over theirs would orphan a
    // VAS the address table already names.
    if let Some(theirs) = vas.host_vas
        && let Some(ours) = fresh_vas
        && theirs != ours
    {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Rebound),
            orphans: orphans(ours, fresh_vas),
            retry: true,
        });
    }
    // ★ And the same for the pin itself: a sibling may have pinned this VA in the gap.
    // Refuse rather than overwrite — the map entry is the only record of the objects, so
    // replacing one silently would leak the pair it named.
    //
    // ⊘ `retry: true` here, and it is `w271`'s change: the sibling's pin may be SHORTER than
    // ours, in which case a retry re-plans, meets `GuestRamPinTooShort`, and the caller
    // describes the remainder. Refusing terminally would leave the same truncation the
    // extent key exists to close, arrived at through a race instead of through a replay.
    if let Some(theirs) = vas.guest_ram_pins.get(&plan.va.0).copied() {
        return Err(Refusal {
            fault: FwdFault::GuestRamAddressTaken {
                va: plan.va,
                host_va: theirs.host_va,
            },
            orphans: orphans(vas_used, fresh_vas),
            retry: true,
        });
    }
    // ★★ R5's half of the extent key: a sibling may have pinned a range that OVERLAPS ours
    // in the gap. The plan checked this against state that has since moved, and adopting our
    // descriptor now would leave two live maps over one set of guest pages.
    if let Some(overlap) = overlapping_pin(vas, plan.va, plan.grant.len()) {
        return Err(Refusal {
            fault: FwdFault::GuestRamPinOverlaps(overlap),
            orphans: orphans(vas_used, fresh_vas),
            retry: true,
        });
    }
    if let Some(h) = fresh_vas {
        vas.host_vas = Some(h);
    }
    vas.guest_ram_pins.insert(
        plan.va.0,
        kayfabe_core::gpu::GuestRamPin {
            host_va,
            memory,
            mapped,
            len: plan.grant.len(),
        },
    );
    Ok(GuestRamPinned {
        host_va,
        memory,
        already: false,
        // ⊘ The GRANT's length, which is what was described to RM — not a length this crate
        // computed. `GuestRamPin::len` above is filled from the same number for the same
        // reason, and the two must never be allowed to drift apart.
        described: plan.grant.len(),
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
    // ★ §12.26 — the system-plane rule, enforced at the ONE site that mints host
    // memory, and BEFORE any host verb exists (so there is nothing to orphan).
    // ★ §12.26 — the system-plane rule, enforced at the ONE site that mints host
    // memory, and BEFORE any host verb exists (so there is nothing to orphan).
    if proc.id == Gpu::SYSTEM_PROC {
        return Err(FwdFault::SystemDataPlane);
    }
    let pid = proc.id;
    let vas = proc
        .vases
        .get(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    // The arena and the isolate must both exist BEFORE any host verb runs: a target
    // miss is an internal inconsistency, and finding it after the allocs would mean
    // allocating host state for a target we then refuse.
    if !proc.arenas.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    // ★★★ R1's spawn deferral: a missing isolate has two meanings and only one of them is
    // an inconsistency (see [`missing_isolate`]).
    if !proc.isolates.contains_key(&gpu) {
        return Err(missing_isolate(proc, gpu));
    }
    // ★★★ #102 — refuse an already-bound VA HERE, before a single host verb exists.
    //
    // Address identity made this necessary *and* possible. Necessary: the host VAS is
    // occupied at exactly this address too, so the map verb would now fail inside the
    // driver — and the caller would learn about the collision as `Rm(NoMemory)` after
    // allocating a VAS and a memory object, with the core's own `Overlap` vocabulary
    // shadowed by the host's. Possible: before identity the core could not know, because
    // the host chose the address and every map got a fresh one.
    //
    // This is the plan-side half only. `commit_publish`'s bind still refuses (R5) — a
    // sibling thread can bind this range in the gap between the read and the commit, and
    // *that* is the case the commit check exists for. Checking twice is not redundancy:
    // the cheap check avoids host work, the late check is the correctness one.
    if vas.table.resolve(pdb, va).is_ok() {
        return Err(FwdFault::Address(AddressFault::Overlap { pdb, va }));
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
        // ★★★ #102 — the guest VA travels INTO the host verb. The plan no longer says
        // "map this somewhere and tell me where"; it says "map this at the address the
        // guest named", which is the only request whose answer a forwarded pushbuffer
        // can use.
        verbs: Some(VerbPlan::Publish {
            host_vas,
            len,
            at: va,
        }),
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

    // ★★ §9.3 — OWNER SCOPE, checked where a host object ENTERS core state.
    //
    // ★ Ordered AFTER the R5 identity guard, and that ordering was corrected rather than
    // chosen: placed before it, this refusal fired on a commit applied to the *wrong
    // proc* and reported "foreign handle" about a plan/proc mismatch — §12.10's
    // wrong-reason conflation, masking the root cause with a symptom of it
    // (`l1_verb_seam.rs::commit_publish_and_doorbell_proc_guards_refuse_on_either_term_alone`
    // caught it). Here the proc's identity is already established, so "is this object
    // ours" is a well-posed question rather than a consequence of a different failure.
    // See [`FwdFault::ForeignBacking`].
    let isolate = IsolateId::new(pid.0, plan.gpu);
    if !memory.belongs_to(isolate) {
        return Err(Refusal {
            fault: FwdFault::ForeignBacking { isolate, memory },
            // Only what is OURS goes on the release list. `memory` is another isolate's
            // object: we have no standing to free it, and queueing it would ask this
            // proc's worker to free across the very boundary this refusal names.
            orphans: Orphans {
                unmap: vec![(vas_used, host_va)],
                free: fresh_vas.into_iter().collect(),
            },
            retry: false,
        });
    }

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
    let block = arena.alloc(plan.len, 0x1000).map_err(|_| Refusal {
        fault: FwdFault::Arena,
        orphans: orphans(vas_used, None),
        retry: false,
    })?;
    let gpa = block.gpa;
    // ★★★ THE DECISION, at the bind site: the owner's **kind 3, real GPU memory**. We
    // allocated a host object and mapped it at the guest's own VA, and it is the range's
    // only memory.
    //
    // ★ G1 (§12.16): the ALLOCATION travels with the PLACEMENT. Storing only `host_va`
    // here is what made the host memory object unreachable from core state — a bound range
    // no reclaim path could ever free. `HostBacking` makes that omission untypeable, and
    // `Binding::real_gpu_memory` now makes *kind 3 without an object* untypeable too.
    //
    // ★ `whole` and not `slice` (`gpga_address_space.md` §8.2): this chain allocates a
    // fresh host object per publication, so the binding IS the object and its release frees
    // it. Arena sub-allocation is the OTHER constructor, and nothing mints it yet —
    // `VerbReply::Published` has no offset to carry one, and that reply lives on the isolate
    // seam.
    //
    // ★★★★★ **SOLE, and the distinction is measured — see `BackingBytes`.** This chain
    // allocates host sysmem and binds it at a GPA carved from *our own* arena: the guest has
    // no independent path to those bytes, so this object is not a shadow of anything.
    // `Publish`'s own doc scopes it — *"correct for a range the guest has never written"* —
    // and that is exactly the sole case.
    //
    // ⊘ Both arguments that could make `real_gpu_memory` refuse are **literals on this
    // line** — a sysmem aperture and `SoleBacking` — so ruling 3 cannot fire here. Stated,
    // not swallowed.
    let binding = Binding::real_gpu_memory(
        gpa.0,
        Aperture::SysmemCoherent,
        kayfabe_mmu::HostBacking::whole(memory, host_va, kayfabe_mmu::BackingBytes::SoleBacking),
    )
    .expect("host sysmem carved from our own arena is kind 3 — both refusals are literals here");
    if let Err(e) = vas.table.bind(plan.pdb, plan.va, plan.len, binding) {
        // ★ G6: the bind refused, so the GPA is owed straight back. Before the arena
        // had a `free` this range simply leaked for the life of the proc.
        let returned = arena.free(block).is_ok();
        debug_assert!(returned, "a block returns to the arena that cut it");
        return Err(Refusal {
            fault: FwdFault::Address(e),
            orphans: orphans(vas_used, None),
            retry: false,
        });
    }
    // ★ G6: keep the token beside the binding, so the range is reclaimable by name.
    vas.blocks.insert(plan.va.0, block);
    Ok(Published {
        gpa: gpa.0,
        host_va,
        memory,
    })
}

/// ★★★★★ **Which chain a framebuffer leaf is materialized by** — and they are
/// alternatives, never layers.
///
/// `fb_cpu_view.md` §4. A leaf served by both would have two host objects at one VA, so a
/// shell arms exactly one and the type is what says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbLeafBacking {
    /// ⊘ **SUPERSEDED — `w228`'s chain.** Real host **vidmem** at the guest's own VA, with
    /// **no CPU view**: the engine reads the card object and the guest reads the emulator's
    /// fabricated one. Two memories, silent in both directions, no fault and no status.
    ///
    /// It is kept expressible because it is what a leaf *was* and because the difference
    /// between the two chains is the finding — ⊘ but it has no production caller: the shell
    /// arms [`FbLeafBacking::Joined`]. See `fb_cpu_view.md` §0.1 for the measurement that
    /// settles why the card object cannot grow the missing view.
    Vidmem,
    /// ★★★★★ **ONE memory.** A fabricated backing, mapped in the isolate, described to RM as
    /// an `OS_DESCRIPTOR` and placed at the leaf's VA — and handed up to the VMM, which maps
    /// the same pages as the guest's view of the leaf's framebuffer range.
    ///
    /// ⚠ The leaf becomes host **sysmem**; see [`kayfabe_isolate::FbLeafJoined`] for why that
    /// divergence is named rather than silent, and why it is not optional.
    Joined,
}

/// ★★★★★ **What backing ONE framebuffer leaf produced** — [`back_fb_leaf`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLeafBacked {
    /// The host GPU VA the blank vidmem object was placed at. Equal to the leaf's own
    /// guest VA, or the verb refused with [`kayfabe_isolate::RmError::PlacementRefused`]
    /// and this value never existed.
    pub host_va: u64,
    /// The `NV01_MEMORY_LOCAL_USER` object RM built.
    pub memory: HostHandle,
    /// ★ Whether this call did the work or found it already done. Reported for the same
    /// reason [`GuestRamPinned::already`] is: a caller on a doorbell repeats, and one
    /// that could not tell "backed now" from "was already backed" would report a
    /// first-time event on every ring.
    pub already: bool,
    /// ★★★★★ The backing the VMM may `mmap` as the guest's own view of this leaf — `None`
    /// for [`FbLeafBacking::Vidmem`], which has no view to hand over, and `None` on an
    /// idempotent replay.
    ///
    /// ⊘ **`None` on a replay is a statement, not a gap.** The backing crossed once, on the
    /// call that did the work; a second descriptor for the same pages would be a second
    /// lifetime for one file, and a caller that installed it twice would map the same memory
    /// at the same framebuffer address twice. A replay's caller already has its view.
    pub backing: Option<kayfabe_isolate::ExportedBacking>,
}

/// The ID-shaped hints [`commit_back_fb_leaf`] re-validates against. Identities and the
/// two numbers the walk produced — never a held reference into core state (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackFbLeafPlan {
    /// The owning proc.
    pub proc: ProcId,
    /// The target GPU (isolate key).
    pub gpu: GpuId,
    /// The `Vas`'s PDB.
    pub pdb: Pdb,
    /// The leaf's base VA — where the host object must be placed.
    pub va: GpuVa,
    /// The leaf's length.
    pub len: u64,
    /// ★★★ The framebuffer-physical address the **guest's own page-table walk** produced
    /// for this leaf. Carried so the commit can re-check it against the address table
    /// rather than re-deriving it from a second walk at a second instant.
    pub phys: u64,
    /// The `Vas`'s host VAS as observed at plan time.
    pub host_vas: Option<HostHandle>,
    /// ★ The backing this plan found already live, if any — the idempotent replay.
    pub existing: Option<(u64, HostHandle)>,
    /// ★★★★★ Which chain materializes this leaf. Carried on the PLAN so the commit can check
    /// it against the reply's shape: a `Published` arriving for a `Joined` plan is a chain
    /// that ran something other than what was decided, and `wrong_reply` names it.
    pub how: FbLeafBacking,
}

/// ★ RM places a fixed mapping in 64 KiB granules; a leaf that is not a whole number of
/// them cannot be covered exactly. See [`FwdFault::FbLeafGranularity`] for why this port
/// refuses rather than rounds.
const FB_LEAF_GRANULE: u64 = 0x1_0000;

/// ★★★★★ **THE SECOND CROSSING — back ONE framebuffer leaf with real host vidmem.**
///
/// `C: docs/design/mode2_fb_crossing_question.md` §5 (GEN-2), settled 2026-06-04 and built
/// twice in the C artifact. The difference from [`publish_backing`] is the whole point and
/// it is two words: the object comes out of **device-local** memory, and the range it
/// covers is one the guest's **own page tables already bind** — so this call does not carve
/// a GPA, it attaches a host materialization to a leaf that already exists.
///
/// # ★★★ The two sources, and why this function holds both
///
/// The leaf's `(va, len, phys)` comes from a walk of the guest's own page tables. The
/// address table may *also* hold a binding for that VA. This function refuses on any
/// disagreement ([`FwdFault::FbLeafDisagrees`]) instead of preferring either reading —
/// see that variant for the three prior times preferring one cost a campaign week.
///
/// # ⊘ What this does NOT do, stated before a green line is read as more
///
/// - **It does not seed, copy or blank the object.** The C's `copy_content` is a separate
///   one-time establishment bridge (`C: :8281-8290`) and needs a CPU view this port does
///   not have on this path.
/// - **It builds no CPU view**, so the guest's own framebuffer accesses at `phys` still go
///   to the shell's fabricated aperture and **not** to this object. That is the C's
///   `gpu_only` shape (`C: :7354-7368`), chosen there because a CPU view consumes the
///   host's 256 MiB BAR1, and chosen here because the descriptor that would join the
///   isolate's mapping to the shell's framebuffer is not wired to this path.
///   ⇒ **Two memories, and until that is closed they diverge.**
/// - **It does not make anything execute.** Nothing is submitted, no doorbell is routed
///   and the host GR engine is not pointed at the result.
///
/// # ⚠ The gap this INHERITS knowingly
///
/// The C's re-back for VA→GPA re-binding is **sysmem only** (`C: :8396-8445`); its
/// framebuffer path has drop-on-free (`C: :2003-2021`) and no re-seed. This port inherits
/// that: if the guest unbinds and re-creates at the same VA with a different frame, the
/// host object stays where it was. ⊘ Named rather than silently reproduced.
///
/// # Errors
/// [`FwdFault`] — every refusal by name; nothing here falls back.
pub fn back_fb_leaf(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
    phys: u64,
    how: FbLeafBacking,
) -> Result<FbLeafBacked, FwdFault> {
    let planned = plan_back_fb_leaf(proc, gpu, pdb, va, len, phys, how)?;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_back_fb_leaf(proc, &planned.plan, reply)
    })
}

/// PLAN (R1): decide [`back_fb_leaf`]'s host work from core state. A pure `&Proc` read.
///
/// # Errors
/// [`FwdFault`] if the proc, the `Vas`, the isolate or the leaf itself is unusable.
pub fn plan_back_fb_leaf(
    proc: &Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
    phys: u64,
    how: FbLeafBacking,
) -> Result<Planned<BackFbLeafPlan>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    // ★ §12.26 — the system-plane rule, at the site that mints host memory and BEFORE any
    // host verb exists. ⊘ Not relaxed here: the system proc's work is forged precisely so
    // it can hold no host state whose reclaim has no defined point, and a framebuffer
    // object is host state.
    if proc.id == Gpu::SYSTEM_PROC {
        return Err(FwdFault::SystemDataPlane);
    }
    // ★ The granularity gate runs FIRST, before any core state is consulted: a leaf RM
    // cannot place exactly is refused on its own terms, not as a consequence of some
    // other lookup.
    if len < FB_LEAF_GRANULE
        || !len.is_multiple_of(FB_LEAF_GRANULE)
        || !va.0.is_multiple_of(FB_LEAF_GRANULE)
    {
        return Err(FwdFault::FbLeafGranularity { va, len });
    }
    let pid = proc.id;
    let vas = proc
        .vases
        .get(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    if !proc.arenas.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    if !proc.isolates.contains_key(&gpu) {
        return Err(missing_isolate(proc, gpu));
    }
    // ★★★ THE TWO-SOURCE CHECK. `binding_at` and not `resolve`, deliberately: this asks
    // *"is what is already here the same shape as what I am about to attach"*, which is
    // the extent question `resolve` hides on purpose.
    let existing = match vas.table.binding_at(va) {
        None => None,
        Some((start, tlen, b)) => {
            if start != va.0 || tlen != len {
                return Err(FwdFault::FbLeafExtent {
                    va,
                    len,
                    tabled: (start, tlen),
                });
            }
            if b.aperture() != Aperture::Vidmem || b.phys() != phys {
                return Err(FwdFault::FbLeafDisagrees {
                    va,
                    walked: (phys, Aperture::Vidmem),
                    tabled: (b.phys(), b.aperture()),
                });
            }
            // ★ Already backed — the idempotent replay. No host verb at all, so there is
            // nothing to orphan and no second fixed map to collide with the first (which
            // RM would answer `0x51`, a status that cannot be told apart from real
            // exhaustion).
            b.host().map(|h| (h.host_va(), h.memory()))
        }
    };
    let host_vas = vas.host_vas;
    Ok(Planned {
        plan: BackFbLeafPlan {
            proc: pid,
            gpu,
            pdb,
            va,
            len,
            phys,
            host_vas,
            existing,
            how,
        },
        verbs: if existing.is_some() {
            None
        } else {
            // ★★★ ONE decision, made here, from the shell's own arming. ⊘ The two arms are
            // not a fallback for each other: nothing below retries a refused join as a
            // vidmem publish, because a leaf that could not be joined is a leaf whose two
            // memories would then be re-created deliberately.
            Some(match how {
                FbLeafBacking::Vidmem => VerbPlan::PublishVidmem {
                    host_vas,
                    len,
                    at: va,
                },
                FbLeafBacking::Joined => VerbPlan::JoinFbLeaf {
                    host_vas,
                    len,
                    at: va,
                    phys,
                },
            })
        },
    })
}

/// COMMIT (R5): adopt [`plan_back_fb_leaf`]'s host work into core state, or refuse and
/// hand back everything the execute phase allocated.
///
/// # Errors
/// [`Refusal`] — carrying the [`Orphans`] the caller must release on the same worker.
#[allow(clippy::too_many_lines)]
pub fn commit_back_fb_leaf(
    proc: &mut Proc,
    plan: &BackFbLeafPlan,
    reply: Option<VerbReply>,
) -> Result<FbLeafBacked, Refusal> {
    // ★ The replay arm: the plan found a live backing and emitted no verbs, so there is
    // no reply to adopt and nothing to unwind. ⊘ Checked against `reply` being absent —
    // a reply here would mean the chain ran when the plan said it would not.
    if let Some((host_va, memory)) = plan.existing {
        if reply.is_some() {
            return wrong_reply("back_fb_leaf replay");
        }
        return Ok(FbLeafBacked {
            host_va,
            memory,
            already: true,
            // ⊘ `None`, and it is the replay's whole point — see the field's docs.
            backing: None,
        });
    }
    // ★★★ The reply's shape is checked against the PLAN's chain, not merely against one
    // expected variant. A `Published` arriving for a `Joined` plan means the chain executed
    // something other than what was decided, and adopting it would record a leaf as joined
    // when it holds card memory with no view — the exact state this rung ends.
    let (fresh_vas, memory, host_va, backing) = match (plan.how, reply) {
        (
            FbLeafBacking::Vidmem,
            Some(VerbReply::Published {
                host_vas,
                memory,
                host_va,
            }),
        ) => (host_vas, memory, host_va, None),
        (
            FbLeafBacking::Joined,
            Some(VerbReply::FbLeafJoined {
                host_vas,
                joined:
                    kayfabe_isolate::FbLeafJoined {
                        backing,
                        memory,
                        host_va,
                    },
            }),
        ) => (host_vas, memory, host_va, Some(backing)),
        _ => return wrong_reply("back_fb_leaf"),
    };
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
    // ★★ §9.3 — owner scope, checked where a host object ENTERS core state, and ordered
    // after the identity guard for the reason `commit_publish` records.
    let isolate = IsolateId::new(pid.0, plan.gpu);
    if !memory.belongs_to(isolate) {
        return Err(Refusal {
            fault: FwdFault::ForeignBacking { isolate, memory },
            orphans: Orphans {
                unmap: vec![(vas_used, host_va)],
                free: fresh_vas.into_iter().collect(),
            },
            retry: false,
        });
    }
    let Some(vas) = proc.vases.get_mut(&(plan.gpu, plan.pdb)) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Vas {
                gpu: plan.gpu,
                pdb: plan.pdb,
            }),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    };
    match (plan.host_vas, fresh_vas) {
        (None, Some(fresh)) => {
            if vas.host_vas.is_some() {
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
        _ => unreachable!("the vidmem publish chain allocates a host VAS iff the plan had none"),
    }
    // ★★★★★ **THE JOINED ARM STOPS HERE, AND THE STOP IS THE ORDERING FIX.**
    //
    // ⚠ `fb-join` bound the row here, and the shell installed the guest's view three steps
    // later — `bind` → `exports.dup` → `mmap` → `RegPlane::join_fb`. That leaves a window in
    // which the address table says `RegionKind::RealGpuMemory` +
    // `BackingBytes::JoinsGuestWindow` while the guest's framebuffer window still points at
    // the old `SparseFb` page. ⊘ And the window does not merely open and close: if the dup,
    // the `mmap` or the install REFUSES, the row stays, permanently, declaring a join that
    // never happened. That is `w228`'s two memories under the one name that says they are
    // one — strictly worse than `w228`, which at least declared the shadow.
    //
    // ⇒ **The commit adopts the HOST facts and binds nothing.** The caller installs the
    // view, and only then calls [`adopt_joined_fb_leaf`], which binds. The declaration is
    // then backed by an install that has already succeeded, which is the closest a type
    // whose truth-maker lives in another process can be brought to being checked.
    //
    // ★ What IS still done above, and must be: `vas.host_vas` adoption. The execute phase
    // may have allocated a host VAS, and a commit that returned without recording it would
    // leak the one object nothing else can name.
    //
    // ⊘ THE COST, STATED. Between here and `adopt_joined_fb_leaf` the isolate holds a fixed
    // mapping at the leaf's VA that core state does not know about, so a re-ask in that gap
    // re-plans as a FIRST join and RM answers the second fixed map at an occupied address
    // with `0x51` — collision-or-exhaustion, which cannot be told apart. The caller closes
    // it by releasing on any failure (`SharedDevice::adopt_joined_fb_leaf` stages the
    // orphans), so a retry starts from nothing rather than from half a join.
    if matches!(plan.how, FbLeafBacking::Joined) {
        return Ok(FbLeafBacked {
            host_va,
            memory,
            already: false,
            backing,
        });
    }
    bind_backed_fb_leaf(vas, plan, host_va, memory, vas_used)?;
    Ok(FbLeafBacked {
        host_va,
        memory,
        already: false,
        backing,
    })
}

/// ★★ **One framebuffer leaf's identity, whole** — the guest VA it is bound at, its length,
/// and the framebuffer-physical base the guest's **own page-table walk** produced for it.
///
/// ⊘ A type rather than three parameters because the three are re-validated **together**:
/// [`adopt_joined_fb_leaf`] runs at a later instant than the plan that named them and must
/// establish that it is looking at the same leaf, and a caller that could pass two of the
/// three from one reading and the third from another is the `FbLeafDisagrees` class one
/// level up. ⚠ Not retrofitted onto [`plan_back_fb_leaf`] here: that is a wide, purely
/// mechanical change across every caller and test, and mixing it into the ordering fix would
/// bury the fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLeafRange {
    /// The leaf's base guest VA — where the host object is placed.
    pub va: GpuVa,
    /// Its length in bytes.
    pub len: u64,
    /// Its framebuffer-physical base, from the walk.
    pub phys: u64,
}

/// ★★★★★ **BIND A JOINED LEAF, ONCE THE GUEST'S VIEW IS ALREADY INSTALLED** — the second
/// half of the join, and the half that may only run after the first has been made true.
///
/// # ⊘ Why this is a separate call and not the tail of [`commit_back_fb_leaf`]
///
/// [`kayfabe_mmu::BackingBytes::JoinsGuestWindow`] asserts something no type can check: that
/// the guest's own framebuffer window for this range now maps the very pages
/// [`kayfabe_isolate::VerbPlan::JoinFbLeaf`] described to RM. Only the shell can make that
/// true, and it needs the reply's backing to do it. So the commit adopts the host facts and
/// stops; the shell installs; and the bind — the moment the declaration enters core state —
/// happens **here**, with the install already behind it.
///
/// ★ The alternative that was rejected: bind first and install after. That is what `fb-join`
/// did, and it is not merely racy — a refused install leaves the row asserting a join that
/// never happened, **permanently**. `w228`'s two memories under the one word that says they
/// are one.
///
/// ⚠ **This verb issues NO host work.** It is core-state bookkeeping over facts the caller
/// already holds, so it takes them as arguments rather than re-deriving them: `host_va` and
/// `memory` are the execute phase's own answers, carried by the caller across the install.
/// ⊘ They are re-checked, not trusted — `memory.belongs_to` was checked in the commit and the
/// leaf's R5 identity is re-checked below, at this later instant.
///
/// # Errors
/// [`Refusal`] — the same R5 vocabulary [`commit_back_fb_leaf`] refuses with, carrying the
/// orphans the caller must release. ★ A caller that ignores them leaves a fixed mapping at
/// the guest's VA that no core state names.
pub fn adopt_joined_fb_leaf(
    proc: &mut Proc,
    plan: &BackFbLeafPlan,
    host_va: u64,
    memory: HostHandle,
) -> Result<(), Refusal> {
    let orphans = |vas_used: HostHandle| Orphans {
        unmap: vec![(vas_used, host_va)],
        free: vec![memory],
    };
    // ⊘ R5 again, at this instant rather than the commit's: the install is a round trip
    // through another process and a `mmap`, so the proc can have retired inside it.
    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Proc(plan.proc)),
            orphans: plan.host_vas.map_or_else(Orphans::default, orphans),
            retry: false,
        });
    }
    let Some(vas) = proc.vases.get_mut(&(plan.gpu, plan.pdb)) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Vas {
                gpu: plan.gpu,
                pdb: plan.pdb,
            }),
            orphans: plan.host_vas.map_or_else(Orphans::default, orphans),
            retry: false,
        });
    };
    // ★ The host VAS the commit adopted. ⊘ Read from the `Vas` and not from `plan.host_vas`:
    // the plan's copy is what was true at plan time, and the commit may have written a
    // freshly-allocated one over it — an unmap aimed at the stale value would miss.
    let Some(vas_used) = vas.host_vas else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Rebound),
            orphans: Orphans::default(),
            retry: false,
        });
    };
    bind_backed_fb_leaf(vas, plan, host_va, memory, vas_used)
}

/// ★★★ **R5 re-validation of the leaf, and the bind** — the tail both framebuffer-leaf
/// chains end on, factored out because they now reach it at **different times**.
///
/// [`commit_back_fb_leaf`] calls it inline for [`FbLeafBacking::Vidmem`] (where ruling 3
/// refuses it, every time); [`adopt_joined_fb_leaf`] calls it after the caller has installed
/// the guest's view. ⊘ One body rather than two: the R5 checks and the unwind-on-overlap are
/// the part that is easy to get subtly wrong, and a second copy of them would be a second
/// reading of what "this leaf is still the leaf we planned" means.
///
/// `vas_used` is the host VAS every refusal's `Orphans` unmaps from; `host_va`/`memory` are
/// what the execute phase produced.
///
/// # Errors
/// [`Refusal`] — carrying the orphans the caller must release. ★ The `BackingBytes` is
/// derived from `plan.how` and from nothing else, so ruling 3 adjudicates the chain that
/// actually ran.
fn bind_backed_fb_leaf(
    vas: &mut kayfabe_core::gpu::Vas,
    plan: &BackFbLeafPlan,
    host_va: u64,
    memory: HostHandle,
    vas_used: HostHandle,
) -> Result<(), Refusal> {
    let orphans = || Orphans {
        unmap: vec![(vas_used, host_va)],
        free: vec![memory],
    };
    // ★★★ R5 ON THE LEAF ITSELF. The plan read the table; a sibling thread may have bound,
    // re-bound or backed this range in the gap. Every disagreement is refused with the
    // orphans attached — ⊘ never resolved by overwriting, because the map entry is the
    // only record of the host object and replacing one silently leaks it.
    let previous = match vas.table.binding_at(plan.va) {
        None => None,
        Some((start, tlen, b)) => {
            if start != plan.va.0 || tlen != plan.len {
                return Err(Refusal {
                    fault: FwdFault::FbLeafExtent {
                        va: plan.va,
                        len: plan.len,
                        tabled: (start, tlen),
                    },
                    orphans: orphans(),
                    retry: false,
                });
            }
            if b.aperture() != Aperture::Vidmem || b.phys() != plan.phys {
                return Err(Refusal {
                    fault: FwdFault::FbLeafDisagrees {
                        va: plan.va,
                        walked: (plan.phys, Aperture::Vidmem),
                        tabled: (b.phys(), b.aperture()),
                    },
                    orphans: orphans(),
                    retry: false,
                });
            }
            if let Some(h) = b.host() {
                // A sibling won the race. Ours is an orphan; theirs is the answer, and it
                // is a *retry* rather than a failure because re-planning finds it and
                // replays.
                return Err(Refusal {
                    fault: FwdFault::GuestRamAddressTaken {
                        va: plan.va,
                        host_va: h.host_va(),
                    },
                    orphans: orphans(),
                    retry: true,
                });
            }
            Some(b)
        }
    };
    // ★★★★★ **RULING 3, ENFORCED AT THE DECISION — and it refuses the VIDMEM chain
    // outright.**
    //
    // > *"no fake FB ever can be mapped to a real GPU VA of an isolate except the
    // > scratchpad"* — owner, 2026-08-11.
    //
    // ⊘⊘ **CORRECTED — there are now TWO chains here and this text is about one of them.**
    // Read this before the paragraph below. `plan.how` selects between them and the
    // `BackingBytes` each one declares is what ruling 3 adjudicates:
    //
    //   * `FbLeafBacking::Vidmem` — `w228`'s chain, `ShadowsGuestMemory`. Everything below
    //     is true of it, verbatim, and it is still refused. It has no production caller.
    //   * `FbLeafBacking::Joined` — `JoinsGuestWindow`, and it is **ruling 4**, not a
    //     relaxation of ruling 3: an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over host pages the
    //     guest's own framebuffer window has been re-pointed at. There is no second memory,
    //     so the sentence below (*"a SECOND, separate memory at the same address"*) is
    //     simply not true of it.
    //
    // ⚠ And note what the difference is NOT. It is not the aperture: both bind
    // `Aperture::Vidmem`, because `plan.phys` is a framebuffer offset in both and the
    // aperture records what the GUEST declared. Correcting it to sysmem would make
    // `Binding::is_guest_ram` true of a number `Vmm::gpa_read` must never see, and would
    // route `residency_of_aperture` to `CpuPlane::GuestRam` when the joined bytes are
    // reachable through the framebuffer store and nowhere else.
    //
    // This is that sentence's one and only production violator. The binding's `phys` is
    // `plan.phys`: the GUEST's own framebuffer offset, whose bytes live in the device's
    // `SparseFb` and which the guest goes on reading and writing through BAR1/BAR2. The
    // host vidmem object the execute phase just allocated is a SECOND, separate memory at
    // the same address. `[measured 2026-08-11, w228]` `placed_as_asked=true` **and blank**.
    //
    // ⊘ **The enforcement is `Binding::real_gpu_memory` refusing to be constructed**, not a
    // test here: `Aperture::Vidmem` IS the emulated framebuffer in this design, and
    // `BackingBytes::ShadowsGuestMemory` says the same thing a second way. There is no
    // spelling of this state that reaches [`AddressTable::bind`].
    //
    // ★★ **What is deliberately NOT done: the table is left alone.** The `previous` row —
    // the guest-declared `RegionKind::FakeFramebuffer` binding a page-table decode put
    // there — stays exactly as it was. Unbinding it and failing would drop the range to
    // *no row at all*, and an absent row is `Representability::Untracked`, which routes to
    // the **real host GPU**. ⇒ Refusing this crossing must not be allowed to hand the range
    // to hardware; the two derived defaults point opposite ways and this is the seam where
    // that matters.
    //
    // ⊘ The host objects the execute phase allocated go back as ORPHANS. Nothing leaks, and
    // nothing is adopted.
    //
    // ★ The scratchpad carve-out (ruling 4) IS the `Joined` arm of this same path — see the
    // correction at the head of this block. It goes through
    // `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over host pages, and it *does* present a `Vidmem`
    // aperture here, because the region's aperture is the guest's declaration and not the
    // object's class.
    //
    // ★★ **The refusal is ASKED FOR, not restated.** This site calls the constructor and
    // propagates its answer rather than raising `FakeFbAtRealGpuVa` from a literal — a
    // literal here would be a second computation of ruling 3 that agrees with the first
    // today and can drift from it tomorrow, and a mutant weakening the constructor would
    // leave this chain's own tests green.
    //
    // ★★★★★ **AND THIS IS THE ONE FACT ONLY THIS SITE KNOWS.** `BackingBytes` has no default
    // and no inference by construction; the chain that created the memory is the only thing
    // that can say which of the two it is, and it says so from `plan.how` — the field the
    // plan carried precisely so the commit could not re-derive it from the reply's shape.
    let bytes = match plan.how {
        FbLeafBacking::Vidmem => kayfabe_mmu::BackingBytes::ShadowsGuestMemory,
        FbLeafBacking::Joined => kayfabe_mmu::BackingBytes::JoinsGuestWindow,
    };
    let binding = match kayfabe_mmu::Binding::real_gpu_memory(
        plan.phys,
        Aperture::Vidmem,
        kayfabe_mmu::HostBacking::whole(memory, host_va, bytes),
    ) {
        Ok(b) => b,
        Err(fault) => {
            return Err(Refusal {
                fault: FwdFault::RegionKindRefused { va: plan.va, fault },
                orphans: orphans(),
                retry: false,
            });
        }
    };
    // ⊘⊘ **NO LONGER UNREACHED — corrected.** The text this replaces said *"unreached while
    // ruling 3 stands"*, and it was right of the only chain that existed then. `plan.how ==
    // Joined` reaches it: that is the *"once a fake-FB page can be made GPU-reachable as an
    // `OS_DESCRIPTOR`"* case the old text was holding the code open for, and it has arrived.
    // ⊘ Ruling 3 did not move; the `Vidmem` arm is still refused above.
    //
    // ★ Drop the un-backed binding and re-insert it WITH its materialization. `bind` refuses
    // an overlap, so the unbind is not optional — and it is done only after every refusal
    // above, so the table is never left with a hole by a path that then declines.
    if previous.is_some() {
        vas.table.unbind(plan.va);
    }
    if let Err(e) = vas.table.bind(plan.pdb, plan.va, plan.len, binding) {
        // ⊘ Put back exactly what was there. A refusal that left the range unbound would
        // turn a failed *addition* into a removal of something that was working.
        if let Some(b) = previous {
            let _restored = vas.table.bind(plan.pdb, plan.va, plan.len, b);
            debug_assert!(
                _restored.is_ok(),
                "re-binding what was just unbound cannot overlap"
            );
        }
        return Err(Refusal {
            fault: FwdFault::Address(e),
            orphans: orphans(),
            retry: false,
        });
    }
    Ok(())
}

/// ★ G6 — reclaim ONE published backing (`l1_concurrency.md` §12.20): unbind the range,
/// return its GPA to **this proc's own** arena, and hand back the host objects the caller
/// must release.
///
/// This is the intra-proc counterpart of `Spine::reap_retired`, and it exists because
/// `GpaArena` used to have no `free` at all: reclamation was whole-arena-at-proc-death
/// only, so a long-lived process that maps and unmaps repeatedly walked its cursor to the
/// end and took a permanent [`FwdFault::Arena`]. That is the C's #80 leak
/// (`teardown_hardening_done`) reproduced one level down after being fixed one level up.
///
/// Like G1's reclaim, this is the *mechanism*; **when** to call it is the caller's,
/// driven by declared graph facts (the `RmGraph` refcounts DUP_OBJECT from the protocol,
/// so liveness is known rather than inferred — there is deliberately no collector here).
/// The host half travels with it in the returned [`Orphans`] for the same reason the two
/// must not drift apart: a GPA recycled while its host memory is still mapped is the
/// `ALREADY-MAPPED` class, so the pair is one call.
///
/// # Errors
/// - [`FwdFault::NoTarget`] — the proc has no arena for this GPU.
/// - [`FwdFault::UnknownPdb`] — the `Vas` is gone (its VASpace was freed by the guest).
/// - [`FwdFault::Address`] with [`AddressFault::Miss`] — **nothing is owed at this VA**:
///   it was never host-published here, or it was already reclaimed. The arena must never
///   accept a range it does not owe, so this is refused before anything is mutated.
pub fn unpublish_backing(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
) -> Result<Orphans, FwdFault> {
    let pid = proc.id;
    if !proc.arenas.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    let Proc { vases, arenas, .. } = proc;
    let vas = vases
        .get_mut(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    // The token FIRST, and only *read* first: a VA this Vas owes nothing at is refused
    // with the table still untouched, so a double free changes nothing at all.
    if !vas.blocks.contains_key(&va.0) {
        return Err(FwdFault::Address(AddressFault::Miss { pdb, va }));
    }
    let block = vas.blocks.remove(&va.0).expect("checked above");
    let host_vas = vas.host_vas;
    let backing = vas.table.unbind(va).and_then(|(_len, b)| b.host());
    let arena = arenas.get_mut(&gpu).expect("checked above");
    if arena.free(block).is_err() {
        // Unreachable while a live proc keeps its arena: the block names the arena that
        // cut it. Loud rather than a panic, and the range stays out of circulation.
        return Err(FwdFault::Stale(Stale::Target { proc: pid, gpu }));
    }
    let mut out = Orphans::default();
    if let (Some(host_vas), Some(h)) = (host_vas, backing) {
        // The UNMAP is unconditional — the GPU mapping is per-binding either way.
        out.unmap.push((host_vas, h.host_va()));
        // ★★ The FREE is not (`gpga_address_space.md` §8.2/§9.3). `frees_object()` is
        // false for an arena slice: the object serves sibling bindings at other offsets,
        // so freeing it here would unmap the arena out from under them — the first
        // release destroying what the last one owns. The arena is freed by its own
        // owner, and until that owner exists the isolate process boundary is the
        // backstop (§7.0), never this call.
        if h.frees_object() {
            out.free.push(h.memory());
        }
    }
    Ok(out)
}

/// ROUTE: which proc owns `(target, pdb)`? A pure spine read (`by_pdb`) — the
/// data-plane routing half of the route/act split.
///
/// **MISS ⇒ FAULT** (`kayfabe_core` crate docs, the miss taxonomy). This is a *use* site:
/// the guest has addressed a VAS, so "the PDB has not been declared yet" is not a fact
/// that can still arrive **for this operation** — the operation is now. The derivation
/// layer defers (`Gpu::sync_proc_to_boundary`); routing refuses. That pairing is what
/// makes the refusal exact instead of merely early.
///
/// ★ §12.13: a miss is checked against the condemned map before it is reported, so a
/// key whose component lost a worker out of band gets [`FwdFault::Condemned`] — the
/// *specific* refusal — instead of an anonymous `UnknownPdb`. Both are misses; only
/// one of them is a security-relevant fact.
pub fn route_pdb(spine: &Spine, target: GpuId, pdb: Pdb) -> Result<ProcId, FwdFault> {
    if let Some(&pid) = spine.by_pdb.get(&(target, pdb)) {
        return Ok(pid);
    }
    if let Some(anchor) = spine.condemned_pdb(target, pdb) {
        return Err(FwdFault::Condemned { anchor });
    }
    Err(FwdFault::UnknownPdb { gpu: target, pdb })
}

/// Resolve `va` in `proc`'s `Vas` identified by `(target, pdb)` — the per-proc
/// read half of [`resolve`] (L1: device read lock + that proc's lock). Pure lookup.
///
/// **MISS ⇒ FAULT**, both terms: an unknown `(target, pdb)` is `FwdFault::UnknownPdb`,
/// and an unbound VA is `AddressFault::Miss`. Nothing defers here — the address table IS
/// the guest's TLB, and a TLB has no "later" (`kayfabe_mmu` crate docs).
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
    /// ★★★★ **The channel's engine**, off the same `Channel` every other field here came
    /// from.
    ///
    /// # ⊘ Why the outcome carries it rather than the caller re-resolving it
    ///
    /// `kayfabe_rt::device::SharedDevice::doorbell` has to decide, *after* the ring has
    /// been rung, whether the **copy-engine content forward** applies to this doorbell
    /// (`kayfabe_rt::device::ring_content_is_forwardable`). Re-resolving the channel to ask
    /// would be a second lock acquisition and — worse — a **second projection of one fact**,
    /// which this project has measured disagreeing three times. It is read here, inside the
    /// commit, off the same `chan` that yielded [`Self::host_token`], so the engine and the
    /// token can never be attributed to different channels.
    ///
    /// ⊘ It is not a routing input. Nothing upstream of the commit branches on it; the
    /// engine that decided which host runlist this channel lives on rode the *plan*
    /// (`VerbPlan::Doorbell::engine`) and was consumed by `alloc_channel` long before here.
    pub engine: EngineKind,
}

/// Check every VA in `working_set` is **ring-admissible** in `table` — the #14 gate
/// condition, with each refusal carrying its own name (`execution_plane.md` §2.4).
///
/// ★ This is the **query** form, used by [`gate_working_set`] and the address-probe
/// sites. The **enforcing** form is [`VasGate`] below, which the same predicate drives
/// from inside `VerbPlan::gated_doorbell` — one authority, [`ring_admits`], two callers,
/// no second definition to drift.
fn gate_vas(
    table: &AddressTable,
    pdb: Pdb,
    working_set: impl IntoIterator<Item = GpuVa>,
) -> Result<(), FwdFault> {
    for va in working_set {
        ring_admits(table, pdb, va)?;
    }
    Ok(())
}

/// ★★★★★ **THE ring-gate authority — may a real engine be pointed at `va` for the
/// duration of a submission?** ⊘ **TWO answers, not three** — see the integration note.
///
/// # ⊘ The correction this carries (2026-08-11)
///
/// This asked exactly one question — `binding.host.is_some()`, *"does a host object exist
/// at this address"* — which is the **same refuted predicate**
/// [`representability_of`] was corrected off the same day. The correction landed on the
/// copy-engine classifier and **not** here, so the two authorities for one question
/// disagreed: `representability_of` refused a
/// [`kayfabe_mmu::BackingBytes::ShadowsGuestMemory`] backing by name while this gate
/// admitted it to a ring. [`kayfabe_mmu::BackingBytes`]'s own words are the ruling —
/// *"⊘ Fatal for anything the guest reads or polls, **which is what a ring is**"* — so the
/// gate was the site that contradicted the doc, not the doc that over-reached.
///
/// ⚠ **`[NOT MEASURED]` as a live defect, and the census is the argument.** The gate is
/// **vacuous in production today**: the only production caller of
/// `kayfabe_rt::device::SharedDevice::doorbell` passes `&[]`
/// (`kayfabe-qemu-raw/src/shim.rs`, which states the emptiness and its reason —
/// recovering the touched VAs means parsing the ring, increment E4/E5),
/// [`gate_working_set`] has test callers only, and [`arm_fence`] has no production caller
/// at all. So no boot has ever run this predicate over a VA. It is **latent**.
///
/// # ★★★★★ INTEGRATION, 2026-08-11 — the third answer is UNCONSTRUCTIBLE, so it is gone
///
/// Two lanes corrected this same predicate on the same day, and their fixes are **not
/// additive**. This function shipped a three-answer shape whose third answer was
/// *"resolved, host object, but a SECOND memory shadowing what the guest reads"* ⇒
/// `FwdFault::BackingNotGuestVisible`. The region-kind lane then moved that same refusal
/// **to the entrance**: [`kayfabe_mmu::Binding::real_gpu_memory`] is the only constructor
/// that can set `host: Some(..)`, and it refuses [`kayfabe_mmu::BackingBytes`]'s
/// `ShadowsGuestMemory` outright (ruling 3). [`kayfabe_mmu::Binding::declared_by_guest`],
/// the other and only remaining constructor, always sets `host: None`.
///
/// ⇒ **`host().is_some()` ⟺ `kind() == RegionKind::RealGpuMemory`, by construction**, and
/// there is no longer any state for a third arm to name. `FwdFault::BackingNotGuestVisible`
/// was deleted with its last producer. Keeping a three-answer shape would have meant
/// *fabricating* a [`kayfabe_mmu::RegionKindFault`] the address plane never raised —
/// refusing by a name that is not true.
///
/// ★ What survives from each lane, deliberately: the **authority consolidation** (one
/// predicate, [`gate_vas`] and [`host_published`] both derived from it, no second reading
/// of the table) and the **kind reading** (`b.kind()` is literally the same question
/// [`representability_of`] asks, which is what the consolidation was *for*).
///
/// # The two answers
///
/// - **unresolved** ⇒ [`AddressFault::Miss`] — the table IS the guest's TLB, and a TLB has
///   no "later".
/// - **resolved, but not [`kayfabe_mmu::RegionKind::RealGpuMemory`]** ⇒ the same
///   [`AddressFault::Miss`], deliberately: to a ring these are one thing, an address the
///   host VAS cannot translate. This is the #14 EXECUTION fault (the shadow had it, the
///   host VAS did not), and it now also covers the fake framebuffer (kind 2) and the
///   guest's own physical pages (kind 4), neither of which a real engine may be pointed at.
fn ring_admits(table: &AddressTable, pdb: Pdb, va: GpuVa) -> Result<(), FwdFault> {
    let miss = || FwdFault::Address(AddressFault::Miss { pdb, va });
    let Ok((binding, _off)) = table.resolve(pdb, va) else {
        return Err(miss());
    };
    match binding.kind() {
        kayfabe_mmu::RegionKind::RealGpuMemory => Ok(()),
        kayfabe_mmu::RegionKind::FakeFramebuffer | kayfabe_mmu::RegionKind::GuestPhysDma => {
            Err(miss())
        }
    }
}

/// THE gate predicate as the bare bool [`kayfabe_isolate::RingWorkingSet`] wants —
/// **derived from [`ring_admits`]**, never a second reading of the table.
///
/// ★ The derivation is the point: the isolate seam cannot name `FwdFault` (it cannot even
/// name `AddressTable`), so the enforcing form must collapse the authority's answers into
/// one rather than re-asking a weaker question, and `plan_doorbell` re-derives the exact
/// fault from the offending VA — which is the division of labour
/// [`kayfabe_isolate::RingWorkingSet`]'s own doc specifies.
///
/// ⊘ **Both lanes rewrote this same body on 2026-08-11 and they agree**, which is why this
/// is a derivation and not a choice between them: the region-kind lane's
/// `binding.kind() == RegionKind::RealGpuMemory` is exactly what [`ring_admits`] now reads,
/// so spelling it a second time here would reintroduce the second definition
/// [`ring_admits`] exists to prevent.
fn host_published(table: &AddressTable, pdb: Pdb, va: GpuVa) -> bool {
    ring_admits(table, pdb, va).is_ok()
}

/// ★★ The **enforcing** #14 ring-gate: one channel's `Vas`, handed to
/// [`kayfabe_isolate::VerbPlan::gated_doorbell`] — the only constructor of a
/// `VerbPlan::Doorbell` — which runs [`host_published`] over the submission's working
/// set before a plan exists at all.
///
/// Keyed by PDB, which is the whole of #14: two procs' *identical* guest VAs resolve in
/// their OWN `Vas`, so the gate passes for both only because each published into its own
/// host VAS (distinct `HostHandle`s).
struct VasGate<'a>(&'a AddressTable, Pdb);

impl kayfabe_isolate::RingWorkingSet for VasGate<'_> {
    fn is_host_published(&self, va: GpuVa) -> bool {
        host_published(self.0, self.1, va)
    }
}

/// The `Vas`-less view: a GSP-managed, system-routed channel (`Channel::vas_pdb =
/// None`) has no address space to have published anything *into*, so nothing is
/// published and only an **empty** working set is gateable — exactly the pre-existing
/// `None if working_set.is_empty()` arm, now expressed as the address plane it is
/// rather than as a special case beside the gate.
struct NoVasGate;

impl kayfabe_isolate::RingWorkingSet for NoVasGate {
    fn is_host_published(&self, _va: GpuVa) -> bool {
        false
    }
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
    /// The decoded vChid — the chid **within [`Self::runlist`]**, not a per-GPU index.
    pub vchid: VChid,
    /// ★★ The decoded runlist. Carried, **not** routed on: the exec-plane index is keyed
    /// `(GpuId, VChid)` and cannot express it.
    ///
    /// `[measured]` on GA106 that key is nonetheless a channel identity — one global
    /// `CHID_MGR`, so chids are device-unique (`doorbell_token_encoding.md` §4). ⊘ That
    /// is a fact about the **part**, not about the key: on a part with per-runlist
    /// channel RAM two live channels can share a chid, and this field is what a future
    /// `(GpuId, RunlistId, VChid)` key would be built from. It is carried today so that
    /// the decoder is not the thing that lost the information.
    pub runlist: RunlistId,
    /// The raw token (recorded per-proc for poll-kick replay).
    pub token: u64,
}

/// Which exec-plane miss is this? ★ §12.13: a `(gpu, vchid)` that misses `by_vchid`
/// is either genuinely unknown or the exec plane of a **condemned** component; the
/// condemned map answers that forward, out of the same projection.
fn vchid_miss(spine: &Spine, gpu: GpuId, vchid: VChid) -> FwdFault {
    match spine.condemned_vchid(gpu, vchid) {
        Some(anchor) => FwdFault::Condemned { anchor },
        None => FwdFault::UnknownVchid { gpu, vchid },
    }
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
        .arch()
        .decode_doorbell(token)
        .ok_or(FwdFault::MalformedToken { token })?;
    let (pid, cid) = *spine
        .by_vchid
        .get(&(target_gpu, target.vchid))
        .ok_or_else(|| vchid_miss(spine, target_gpu, target.vchid))?;
    Ok(DoorbellRoute {
        proc: pid,
        chan: cid,
        gpu: target_gpu,
        vchid: target.vchid,
        runlist: target.runlist,
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
/// [`kayfabe_core::gpu::ExecPlane`] — there is no global one-shot to leave a second
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
    // ★★★ R1's spawn deferral — see [`missing_isolate`].
    if !proc.isolates.contains_key(&cgpu) {
        return Err(missing_isolate(proc, cgpu));
    }

    // ---- The channel's own `Vas`, resolved BEFORE any host op. A declared PDB whose
    //      `Vas` is absent is a loud refusal here, exactly as before.
    let vas = match chan.vas_pdb {
        Some(pdb) => Some(
            proc.vases
                .get(&(cgpu, pdb))
                .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?,
        ),
        None => None,
    };

    let channel = chan.host_channel.zip(chan.host_token);
    // Lazy per-proc materialization: the channel's graph-derived `EngineKind` rides
    // the alloc so the adapter lands it on the RIGHT runlist (GR-1: the C's
    // `dma_copy_class_alloc_params` engineType=0 → 401 class, designed out).
    let host_vas = if channel.is_none() {
        match vas {
            Some(v) => v.host_vas,
            None => return Err(FwdFault::NoVas(cid)),
        }
    } else {
        None
    };
    // ★★★ #177 — THE GATE. The guest must have asked for this channel to be schedulable
    // before anything it submits is planned. `requested` is written only by
    // `Gpu::schedule_channel`, i.e. only by the guest's own `0xa06f0103`.
    if !proc.exec.requested.contains(&cid) {
        return Err(FwdFault::NotScheduled {
            chan: cid,
            vchid: route.vchid,
        });
    }
    let schedule = !proc.exec.scheduled.contains(&cid);

    // ---- ★★ The #14 ring-gate, BEFORE any host op — and it now runs **inside the
    //      constructor**. `VerbPlan::Doorbell` is `#[non_exhaustive]`, so
    //      `VerbPlan::gated_doorbell` is the only thing in the workspace (or outside it)
    //      that can produce one, and it refuses before returning a plan. There is
    //      therefore no plan-shaped object in existence for an ungated working set —
    //      the invariant is on the type, not on this function remembering to call a
    //      gate first (`ARCHITECTURE.md` invariant 5, closed 2026-07-27).
    let vas_gate = vas.zip(chan.vas_pdb).map(|(v, pdb)| VasGate(&v.table, pdb));
    let no_vas = NoVasGate;
    let gate: &dyn kayfabe_isolate::RingWorkingSet = match &vas_gate {
        Some(g) => g,
        None => &no_vas,
    };
    let verbs =
        VerbPlan::gated_doorbell(gate, working_set, host_vas, channel, chan.engine, schedule)
            // ★ Re-derive the EXACT fault from the offending VA, which is the division of
            // labour `RingWorkingSet`'s doc specifies: the seam carries a bare bool, this
            // crate owns the vocabulary. `ring_admits` is the same authority the gate just
            // ran, so the re-derivation cannot disagree with the refusal it is naming.
            //
            // ⊘ CORRECTED 2026-08-11 (integration): this comment used to say *"there are
            // three answers, not two — a `ShadowsGuestMemory` backing is
            // `BackingNotGuestVisible`, not a `Miss`"*. There are **two**. Ruling 3 made a
            // shadowing backing unbindable, so `ring_admits` now yields only `Miss` and
            // `FwdFault::BackingNotGuestVisible` no longer exists — see `ring_admits`.
            //
            // ★ The re-derivation is kept even though every arm currently answers `Miss`:
            // it is the SHAPE that matters. The bare-bool collapse at the seam is lossy by
            // construction, so the day the authority grows a second refusal this site
            // carries it without being touched. Asking the authority is never wrong;
            // hardcoding `Miss` here would be right today and silently wrong later.
            .map_err(|kayfabe_isolate::UngatedVa(va)| {
                match (vas, chan.vas_pdb) {
                    (Some(v), Some(pdb)) => ring_admits(&v.table, pdb, va)
                        // ⊘ Total, not `unwrap_err`: the gate refused this VA, so the
                        // authority refuses it too — but a panic is not the way to say so.
                        .err()
                        .unwrap_or(FwdFault::Address(AddressFault::Miss { pdb, va })),
                    _ => FwdFault::NoVas(cid),
                }
            })?;

    // ★★★★★ THE TRANSLATION WITNESS (owner directive, 2026-08-12). The store witness in
    // `RmConnection::doorbell` proves the write instruction executes; it cannot say WHAT was
    // translated into WHICH host token, nor on which engine, because by then only a bare
    // `u32` remains. This is the only site that holds both halves at once.
    //
    // ⚠ It settles a standing claim by measurement rather than by reading: task #243 records
    // *"user-proc `GrCompute` doorbells never reach it at all"*, which has been UNTESTED since
    // legs A2/B landed at `w261`/`w262`. `engine=GrCompute` beside `proc=2` on this line
    // refutes it; its absence across a whole boot confirms it.
    //
    // ⊘ `host_token=NONE-YET` is not a failure: it is the lazy-materialization path, where
    // the channel (and therefore its token) is allocated by the verbs this function is about
    // to return. The pairing then appears in `DOORBELL-VERB`.
    eprintln!(
        "kayfabe: DOORBELL-XLATE proc={} chan={} vchid={} engine={:?} guest_token={:#010x} \
         host_token={} schedule={schedule}",
        pid.0,
        cid.0,
        route.vchid,
        chan.engine,
        route.token,
        chan.host_token.map_or_else(
            || "NONE-YET(materializes in these verbs)".to_string(),
            |t| format!("{t:#x}")
        ),
    );
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
        verbs: Some(verbs),
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
    // ★ Off the SAME `chan` binding as `host_token` one line up — see the field's doc for
    // why the outcome carries this rather than the caller resolving it again.
    let engine = chan.engine;
    Ok(DoorbellOutcome {
        proc: plan.proc,
        chan: plan.chan,
        host_token,
        scheduled_now: scheduled,
        engine,
    })
}

/// ★ The **single-threaded composition** of the one gated ring path — the exec-plane
/// demux, **structurally gated** (#14, `execution_plane.md` §2.4; the C's "one exec path"
/// refactor-debt lesson): one guest doorbell write → gate → the owning proc's channel rung
/// on the owning proc's isolate.
///
/// The **split-borrow composition** of [`route_doorbell`] (pure spine read) +
/// [`exec_doorbell`] (owning-proc act) — L1 cardinal rule R4 factored in the core.
///
/// ★ **corrected 2026-07-27** (found by the whitepaper's verification pass). This doc used
/// to claim *"No ungated sibling exists; nothing else in the workspace calls
/// `RmBackend::ring_doorbell`"* and *"[`exec_doorbell`] is the ONLY function that reaches
/// `RmBackend::ring_doorbell`"*. Both are false as stated: the sole `ring_doorbell` call
/// site is inside `kayfabe_isolate::Worker::execute`, and **this function is not on the L1
/// path at all** — a real guest MMIO write goes through `kayfabe_rt::SharedDevice::doorbell`,
/// which drives plan/execute/commit itself and never calls `handle_doorbell`.
///
/// The gate is still **structural, not caller discipline**, and the argument simply moves
/// down one level: [`plan_doorbell`] is the sole constructor of `VerbPlan::Doorbell`
/// *within the production crates*, and it runs the #14 ring-gate before any host op
/// exists — so neither composition can hand `Worker::execute` an un-gated ring, and the
/// removed `ring_gated` sibling stays removed.
///
/// ★★ **And since 2026-07-27 that is a fact about the TYPES, not only the call graph.**
/// The ⚠ that used to stand here said "structural" described the call graph while
/// `VerbPlan` was a public enum with public variant fields and `Worker::execute` was
/// public — so a `VerbPlan::Doorbell` could be hand-built outside this crate and rung
/// with the gate never having run, which `tests/tests/cross_proc_lifetime.rs` did.
/// `VerbPlan::Doorbell` is now `#[non_exhaustive]` (no struct expression exists outside
/// `kayfabe-isolate` — E0639, pinned by that crate's `tests/ui/ungated_doorbell.rs`) and
/// its only constructor, [`kayfabe_isolate::VerbPlan::gated_doorbell`], **is** the gate:
/// it checks every working-set VA host-published in the ringing channel's own `Vas`
/// before a plan exists. The residual is stated at that constructor and is a different,
/// smaller one: the address plane it gates over is caller-supplied, because Rust cannot
/// express "only this crate may call this function".
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
/// *encoding* is `kayfabe-gsp`'s job once it ports.)
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

/// Which hop of [`route_engine_object_by_parent`] declined — the four are different
/// facts, and collapsing them into one "unroutable" would hide which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineParentMiss {
    /// No object at `(client, hParent)` in the RM graph at all. The guest named a handle
    /// this port never saw allocated.
    NoNode,
    /// The parent exists and is **not** a channel — e.g. a TSG (channel *group*) or a
    /// subdevice. ⊘ Never walked through: an engine object's context is the channel's,
    /// and picking one member of a group would be a guess about which.
    NotAChannel,
    /// The channel's `Device` ancestor has not resolved, so the alloc names no **target
    /// GPU**. Deferred by construction (the same `gpu: None` scope `project` uses), never
    /// defaulted to GPU 0 — `deviceInstance` failing open to GPU 0 is a *driver* bug this
    /// port declines to reproduce.
    NoTarget,
    /// The channel's `userd_flags` name no vChid for this arch — the same refusal
    /// `ProjectionError::UnnamedVchid` makes, asked here so the forward cannot substitute
    /// a vChid the projection would have refused.
    UnnamedVchid,
}

/// ★★★★★ **§16.80** — ROUTE an engine-object alloc from the handles the **RPC** carries.
///
/// [`route_engine_object`] is keyed on `(GpuId, VChid)`, which is what a *doorbell* has.
/// A `GSP_RM_ALLOC` has `(hClient, hParent)` instead, so this is the missing hop that
/// makes the Case-1 forward reachable from the wire at all — the reason
/// [`forward_engine_object`] has had zero production callers since it was built.
///
/// ⊘ **Forward-derived, never reverse-resolved.** Both keys come out of the channel's own
/// alloc facts by the *same* computations `kayfabe_core::project` used to build
/// `Spine::by_vchid` — `RmGraph::gpu_of_resource` and `Arch::vchid_from_userd_flags` — and
/// the answer is then put back through [`route_engine_object`], so `by_vchid` stays the
/// single authority. A disagreement surfaces as [`FwdFault::UnknownVchid`], loudly, rather
/// than as a second competing resolution of a question the projection already answered
/// (`two_projections_of_one_fact_disagreeing`, four prior instances).
///
/// # Errors
/// [`FwdFault::EngineObjectParent`] naming the hop, then whatever
/// [`route_engine_object`] refuses with.
pub fn route_engine_object_by_parent(
    spine: &Spine,
    client: HClient,
    parent: HObject,
    class: ClassId,
) -> Result<EngineObjectRoute, FwdFault> {
    // ★★★ THE CLASS GATE RUNS FIRST, and this ordering is a MEASURED correction rather
    // than a preference.
    //
    // `[measured 2026-08-10, boot `w221_49dc3ec_grfwd`]` this function delegated the
    // `engine_of_object` check to `route_engine_object` — i.e. LAST, after the graph
    // lookup — while the comment at its only call site said the opposite ("every
    // non-engine alloc exits before the graph is touched"). The boot printed the truth:
    //
    //   ENGINE-OBJECT class=0x0000 … → REFUSED EngineObjectParent { … NotAChannel }
    //   ENGINE-OBJECT class=0x0080 … → REFUSED EngineObjectParent { … NotAChannel }
    //   ENGINE-OBJECT class=0x2080 … → REFUSED EngineObjectParent { … NotAChannel }
    //
    // ⊘ A client root refused as *"your parent is not a channel"* is a **true sentence
    // about the wrong question** — the class was never an engine object and the parent
    // was never going to matter. And it was not merely untidy: the report is bounded, and
    // nineteen lines of kernel bring-up noise had consumed most of that bound before the
    // one alloc the instrument exists for (`0xc7c0`) arrived.
    //
    // ★ My own test passed over it, because it asserted only that no HOST verb was
    // issued — never WHICH refusal. A claim written in a comment and not asserted
    // anywhere is prose (`a_comment_that_names_an_exception_is_a_bug_report`).
    // ⊘ The result is deliberately DISCARDED: `route_engine_object` at the end of this
    // function asks the same question and remains the one authority for the ANSWER. This
    // call is the GATE — it decides whether to proceed — never a second resolution whose
    // value could come to disagree with the first.
    spine
        .arch()
        .engine_of_object(class)
        .ok_or(FwdFault::NotAnEngine(class))?;
    let miss = |why| FwdFault::EngineObjectParent {
        client,
        object: parent,
        why,
    };
    let node = spine
        .rmgraph
        .node(kayfabe_core::rmgraph::NodeKey::new(client, parent))
        .ok_or_else(|| miss(EngineParentMiss::NoNode))?;
    if !matches!(node.kind, kayfabe_arch::ObjectKind::Channel { .. }) {
        return Err(miss(EngineParentMiss::NotAChannel));
    }
    let gpu = spine
        .rmgraph
        .gpu_of_resource(node.id())
        .ok_or_else(|| miss(EngineParentMiss::NoTarget))?;
    let vchid = spine
        .arch()
        .vchid_from_userd_flags(node.facts.userd_flags)
        .ok_or_else(|| miss(EngineParentMiss::UnnamedVchid))?;
    route_engine_object(spine, gpu, vchid, class)
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
        .arch()
        .engine_of_object(class)
        .ok_or(FwdFault::NotAnEngine(class))?;
    let (pid, cid) = *spine
        .by_vchid
        .get(&(target_gpu, vchid))
        .ok_or_else(|| vchid_miss(spine, target_gpu, vchid))?;
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
    let planned = plan_engine_object(spine, proc, route, class, params)?;
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
    spine: &Spine,
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
    // ★★★ R1's spawn deferral — see [`missing_isolate`].
    if !proc.isolates.contains_key(&cgpu) {
        return Err(missing_isolate(proc, cgpu));
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
            // ★★★★★ **LEG A2** — asked for ONLY when this call is the one that will birth
            // the host channel. ⊘ A channel that already exists was born over whatever it
            // was born over; re-stating its ring here would be a second opinion about a fact
            // RM already holds and cannot be told.
            adopt: if channel.is_none() {
                adopted_guest_ring(spine, proc, chan, cgpu)
            } else {
                None
            },
        })
    };
    Ok(Planned { plan, verbs })
}

/// ★★★★★ **LEG A2 — the guest's own ring, IF the supply side already put it where a host
/// engine can reach it.** `None` on every other day, and `None` is every prior boot.
///
/// # ★★★ THE ARMING IS INHERITED, and that is deliberate rather than a shortcut
///
/// There is no flag here and there must not be one. This returns `Some` **exactly when** the
/// address table already holds a binding at the channel's declared `gpFifoOffset` whose host
/// backing declares [`kayfabe_mmu::BackingBytes::JoinsGuestWindow`] — one memory, the bytes
/// the guest writes and the bytes the engine reads being the same bytes. That binding is
/// written by one path only (`adopt_joined_fb_leaf`, after a join that succeeded), and that
/// path runs only when the shell armed `KAYFABE_GUEST_RING=ring`.
///
/// ⇒ With the supply side disarmed this function is `None` **by construction**, so the
/// default build's behaviour is byte-identical without a second selector that could drift
/// out of step with the first (`a_second_source_of_truth_beside_a_complete_value`).
///
/// # ⊘ THE OWNER INVARIANT — the forbidden state is not reachable from here
///
/// [`kayfabe_mmu::BackingBytes::ShadowsGuestMemory`] — `w228`'s **blank** host vidmem twin at
/// the guest's own VA, *"two memories"* — is refused by the `match` below rather than by a
/// comment. A channel born over that object fetches GPFIFO entries out of a page nothing ever
/// wrote, decodes zeros, never advances `GP_GET`, and reports **no error at all**.
fn adopted_guest_ring(
    spine: &Spine,
    proc: &Proc,
    chan: &kayfabe_core::gpu::Channel,
    cgpu: GpuId,
) -> Option<kayfabe_isolate::AdoptedGuestRing> {
    // ⊘ Off the channel's OWN graph node — the same node `CeChannelFacts::ring_va` reads, so
    // the two projections of "what ring did this channel declare" cannot disagree.
    // ⚠ `gpFifoOffset = 0` is a VALUE (the driver's golden-context channel declares it) and
    // it survives this path intact: what is `Option` here is whether a ring was declared at
    // all, never whether the number is zero.
    // ⊘ ONE read of the node, and both facts come out of it. Two `node_of_resource` calls
    // would be two lookups of one object that a future refactor could point at different
    // revisions — and the ring and the USERD have to be the SAME channel's or the
    // containment test below is being run against the wrong leaf.
    let facts = spine.rmgraph.node_of_resource(chan.key)?.facts;
    let ring = facts.gp_fifo_ring?;
    let userd = facts.userd;
    let pdb = chan.vas_pdb?;
    let (start, len, binding) = proc
        .vases
        .get(&(cgpu, pdb))?
        .table
        .binding_at(kayfabe_arch::ids::GpuVa(ring.va))?;
    let host = binding.host()?;
    // ★★★ THE ONE ARM. ⊘ Not `binding.host().is_some()` — that asks *"does a host object
    // exist here"*, and the question that decides correctness is *"does the guest reach these
    // bytes some other way"*. `[measured 2026-08-11]` `representability_of` made exactly that
    // mistake and it is why `BackingBytes` exists at all.
    if host.bytes() != kayfabe_mmu::BackingBytes::JoinsGuestWindow {
        return None;
    }
    Some(kayfabe_isolate::AdoptedGuestRing {
        memory: host.memory(),
        // Where the joined object is placed — the leaf's own base, which is what
        // `adopt_joined_fb_leaf` bound.
        ring_va: start,
        // ★★ The GUEST's two numbers, passed through untouched. Neither is derived from
        // `ring_va` and neither is one of the adapter's constants — see `RingLayout::entries`
        // for why a wrong modulus is not cosmetic.
        gp_fifo_va: ring.va,
        gp_fifo_entries: ring.entries,
        // ★★★★★ **LEG B**, offered from inside leg A2's own answer so that *"the guest's
        // USERD on a ring of ours"* is unspellable. See `AdoptedGuestRing::userd`.
        userd: adopted_guest_userd(&binding, len, host.memory(), userd),
    })
}

/// Size of one channel's USERD slot on every part this port targets.
///
/// `1 << NV_RAMUSERD_BASE_SHIFT` with shift 9 (`ogkm-580:
/// src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:1545-1556`, `dev_ram.h`
/// (gm107) `:49`), selected by `_GM107` for every non-Tegra chip. ⊘ Named here rather than
/// taken from the guest's own `userdMem.size` because it is what the **containment** check
/// must be run against: a guest that declared a short size would otherwise buy itself a
/// binding whose last bytes are outside the joined leaf.
const USERD_SLOT_BYTES: u64 = 512;

/// ★★★★★ **LEG B — the guest's own USERD, IF it lies inside the leaf its ring was joined
/// through.** `None` on every other day, and `None` is every prior boot.
///
/// # ★★★ THIS IS A CONTAINMENT TEST, NOT A RESOLUTION — and that is the licence
///
/// `kayfabe_mmu`'s `gpga.rs` forbids `fn owner_of(addr)` *"and there never will be"*, and
/// `docs/design/leg_b_userd_adoption_blocker.md` §2.2 refused the BAR1 route on exactly that
/// ground. ⊘ **Nothing here asks who owns an address.** The chain is forward the whole way:
/// the channel names its ring VA, the ring VA names a binding, the binding is a joined object
/// with a known framebuffer base and length. The only question asked of the guest's number is
/// *"is it inside the object I am already holding"*. Had the ring's leaf not been joined,
/// this declines — it does not go looking.
///
/// # Where the guest's number comes from, and why three documents said it did not exist
///
/// `kayfabe_core::rmgraph::DeclaredUserd::resolved` — the guest's **own kernel** resolves
/// `hUserdMemory[0]`/`userdOffset[0]` before it RPCs the GSP, because a fake GSP has no
/// client handle namespace to look a handle up in. See
/// `kayfabe_abi::notifier::ChannelUserdMemWire` and `docs/design/userd_mem_is_on_the_wire.md`.
///
/// # ⊘ The four ways this says no, all of them normal
///
/// - the params did not carry the descriptor (`resolved: None`) — *"we could not read it"*;
/// - the guest put its USERD in **guest RAM** (`UserdMem::Sysmem`) — legal, served by the
///   guest-RAM pin and by no framebuffer join, and refused here **by name** in the sense that
///   the `match` has an arm for it rather than a wildcard;
/// - the descriptor was zero (`UserdMem::Undeclared`) — the guest let RM allocate its USERD;
/// - the address is outside this leaf. ⚠ `[NOT MEASURED]` how often that last one happens; on
///   `w262b` the sixteen walling channels' rings and USERDs share one 2 MB leaf, but that is
///   one workload.
fn adopted_guest_userd(
    binding: &kayfabe_mmu::Binding,
    len: u64,
    memory: kayfabe_isolate::HostHandle,
    userd: Option<kayfabe_core::rmgraph::DeclaredUserd>,
) -> Option<kayfabe_isolate::AdoptedGuestUserd> {
    let base = match userd?.resolved? {
        kayfabe_arch::UserdMem::Framebuffer { base, .. } => base,
        // ⊘ Arms, not a wildcard. `Sysmem` is a REAL and legal USERD location this rung has
        // no crossing for, and `Undeclared` is the guest saying it allocated none; folding
        // either into the `None` above would make two different findings look like a decode
        // that failed.
        kayfabe_arch::UserdMem::Sysmem { .. } | kayfabe_arch::UserdMem::Undeclared { .. } => {
            return None;
        }
    };
    // ⊘ The aperture of the BINDING, checked even though `JoinsGuestWindow` implies it. The
    // guest's address is a *framebuffer* offset; a binding over anything else would make the
    // subtraction below arithmetic between two different address spaces — the exact defect
    // `kayfabe_arch::Aperture`'s own docs name ("vidmem offset X and sysmem offset X are
    // different bytes on different devices").
    if binding.aperture() != kayfabe_arch::Aperture::Vidmem {
        return None;
    }
    let offset = base.checked_sub(binding.phys())?;
    // ★ `checked_add` and `>`, not `>=`: the slot's LAST byte must be inside the leaf. A
    // USERD whose first 8 bytes are in the joined window and whose `GP_GET` is not would be
    // accepted by a start-only check and would fetch forever from a page RM zeroed.
    if offset.checked_add(USERD_SLOT_BYTES)? > len {
        return None;
    }
    Some(kayfabe_isolate::AdoptedGuestUserd { memory, offset })
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

/// ★★★★★ **§16.80** — [`forward_engine_object`] keyed on the handles a `GSP_RM_ALLOC`
/// actually carries. The composition of [`route_engine_object_by_parent`] and
/// [`exec_engine_object`]; see the route for why the vChid is forward-derived.
///
/// # Errors
/// [`FwdFault`], by variant.
pub fn forward_engine_object_by_parent(
    gpu: &mut Gpu,
    client: HClient,
    parent: HObject,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let Gpu { spine, procs, .. } = gpu;
    let route = route_engine_object_by_parent(spine, client, parent, class)?;
    let proc = procs
        .get_mut(&route.proc)
        .ok_or(FwdFault::RetiredProc(route.proc))?;
    exec_engine_object(spine, proc, &route, class, params)
}

/// ROUTE: classify a `GSP_RM_CONTROL` through the Case-1/Case-2 split — a pure
/// spine read (`Arch::is_case2_control`), no proc touched.
#[must_use]
pub fn classify_control(spine: &Spine, cmd: ControlCmd) -> ControlRoute {
    if spine.arch().is_case2_control(cmd) {
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
    // ★★★ R1's spawn deferral — see [`missing_isolate`].
    if !proc.isolates.contains_key(&target_gpu) {
        return Err(missing_isolate(proc, target_gpu));
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
// Two co-equal address-table populate sources meet here (address_table.md, L3) — but
// ⚠ NAME THEM AS THIS WIRE HAS THEM, not as the C artifact had them. This comment used
// to say "the bind-time RPC bindings (batch 1, `Gpu::sync_rpc_mappings`)", and on a
// GSP-client part THAT SOURCE HAS NO PRODUCER: `MAP_MEMORY_DMA`/`UNMAP_MEMORY_DMA` are
// HAL stubs there, so `RmEvent::MapMemoryDma` is never constructed from the wire
// (`kayfabe_rmrpc` module docs, three independent oracles; `decode_map_memory_dma` has
// no caller outside tests). `sync_rpc_mappings` still runs — over an empty set — which
// is why the wrong name survived: the code path is live, so it reads as a live source.
// The two sources here are `GPU_PROMOTE_CTX` (`#93`) and the observed CE PT-writes
// captured below. Both land in the same per-`Vas` table.
// =================================================================================

/// Upper bound on a single GPFIFO range's method bytes the parser will read. A
/// hostile GPFIFO entry can declare any length; this caps it to a bounded read so an
/// attacker-controlled length is never an arbitrary allocation (boundary-1). Real
/// pushbuffer segments are far smaller; a range hitting this cap is simply truncated
/// (the surplus decodes to nothing actionable, MISS=FAULT at use).
pub const MAX_PUSH_RANGE_BYTES: usize = 1 << 20;

/// Upper bound on the TOTAL method bytes one `parse_pushbuffer` call will read across
/// ALL of a ring's GPFIFO ranges. `MAX_PUSH_RANGE_BYTES` bounds any single range, but a
/// hostile ring can declare *many* maxed-out ranges; this caps their sum so the decoded
/// method vector cannot grow without bound either (boundary-1). Ranges past the budget
/// are skipped (their content decodes to nothing actionable — MISS=FAULT at use).
pub const MAX_PUSH_TOTAL_BYTES: usize = 8 << 20;

// ---------------------------------------------------------------------------------
// ★★★ TWO DECISIONS, NOT ONE (`eight_blockers_resolved.md` §11.5 / §12).
//
// The C makes two different decisions about one `LAUNCH_DMA`, on two different
// predicates, and stage B (`379f712`) folded them together — it got the CAPTURE
// predicate right and answered EXECUTE by accident, routing everything non-phys to
// "forward it, let hardware run it".
//
// | decision | the C's predicate | site |
// |---|---|---|
// | EXECUTE — host CE vs our own copy | `m2cexec && !mscrub && !remap && !src_phys && !dst_phys && is_user_ce(chan_client)` | `C: nvkvm_gpu_emul.c:6310` |
// | CAPTURE — is this a page-table write? | the fb-write hook, on the **resolved physical** destination | `C: :6353`, `:6437` |
//
// They are separate here because they read different inputs and can disagree on the
// same command. Read the C's execute row carefully: `is_user_ce` means **every
// guest-kernel CE copy is CPU-emulated there**, including the framebuffer-alias
// page-table write — which is *virtual*-destination and would pass any purely
// operand-carried test — and so are all scrubs and fills.
//
// [`ce_executor_c`] is that predicate, ported. [`classify_ce`] is the capture one.
// ---------------------------------------------------------------------------------

/// Who submitted a copy-engine command — the C's `is_user_ce(chan_client)`
/// (`C: nvkvm_gpu_emul.c:2493` — paraphrased rather than quoted, because the C's own
/// wording names a host user-mode library and the hexagonal gate rightly refuses that
/// vocabulary here: *is this client one of the user-mode driver's CE-copy clients, the
/// user-observable data path? UVM/init clients are not*).
///
/// The port keys it on the **proc**, not on a client list. `kayfabe_core::Gpu::system`
/// already *is* the guest-kernel component — "kernel RM / scrubber / CeUtils traffic",
/// every declared kernel client joined by rule (§12.27) — so the fact the C had to
/// accumulate into `m2_user_ce_clients[]` at runtime is one the projection declares.
/// That is a strengthening, not a departure: the C's list was populated by observation
/// and a client it had not yet seen read as *not* user-CE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOrigin {
    /// A user process's channel — the user-observable CE data path.
    User,
    /// The guest kernel's own channel — CeUtils, the scrubber, UVM.
    GuestKernel,
}

impl ChannelOrigin {
    /// The origin of a channel owned by `pid`.
    #[must_use]
    pub fn of(pid: ProcId) -> Self {
        if pid == Gpu::SYSTEM_PROC {
            ChannelOrigin::GuestKernel
        } else {
            ChannelOrigin::User
        }
    }
}

/// ★ **The C's execute predicate, ported literally** (`C: nvkvm_gpu_emul.c:6310`):
///
/// ```text
/// bool host_ce = s->m2cexec && !mscrub && !remap && !src_phys && !dst_phys &&
///                nvkvm_m2_is_user_ce(s, s->chan_client);
/// ```
///
/// Every conjunct survives except `m2cexec`, which is a **bench debug switch** (the
/// C's "run the copy on the CPU so I can read it" flag), not a design axis — this port
/// has no mode in which execution forwarding is off, so a constant `true` conjunct is
/// not modelled. Named here rather than silently dropped.
///
/// ★★ This is the **baseline**, not the shipped policy. §12's ruling replaces the
/// `is_user_ce` conjunct — a fact about *who submitted the work* — with representability,
/// a fact about the **address**. `ce_c_vs_representability` (integration tests) pins
/// every row where the two answers differ, so each departure is a value a test reads
/// rather than a paragraph.
#[must_use]
pub fn ce_executor_c(
    work: kayfabe_arch::CeWork,
    origin: ChannelOrigin,
    src_is_virtual: bool,
    dst_is_virtual: bool,
) -> CeExecutor {
    let plain_copy = matches!(work, kayfabe_arch::CeWork::Copy);
    if plain_copy && src_is_virtual && dst_is_virtual && origin == ChannelOrigin::User {
        CeExecutor::HostCe
    } else {
        CeExecutor::Ours
    }
}

/// ★★★ **What a copy-engine command's operands CARRY** — the discriminator the whole
/// data plane turns on (`mode2_dataplane_architecture.md`, "The architecture to build").
///
/// Not *who submitted the work* and not *what form the destination took*, but what the
/// operands mean:
///
/// - **VA-operand** — copies and kernels. The operands are GPU virtual addresses the host
///   MMU resolves once the address space is resident. Nothing for the **address plane** to
///   extract: no PTE values are in flight, so there is no capture. There is no software
///   *shadow* of these, and there must not be: the C's shadow-plus-forged-completion was
///   byte-exact and never touched a GPU, which is precisely why nothing noticed the buffer
///   had no host mapping until hardware was finally asked to resolve it.
/// - **Phys-operand** — page-table writes and scrubs. The payload is guest-physical PTE
///   values, which **cannot be handed to hardware**. Capture, so the address plane can
///   decode what the page now describes at the guest's own commit point.
///
/// ★★ **This enum answers CAPTURE ONLY.** It used to say "forward it, let hardware
/// execute it" on the `VaOperand` arm — which is the *execute* decision, a different
/// predicate over different inputs, answered here by accident
/// (`eight_blockers_resolved.md` §11.5). Who runs the copy is [`CeExecutor`]'s question.
/// A command can perfectly well be "not a page-table write" **and** "not hardware's to
/// run": every guest-kernel data copy in the C is exactly that.
///
/// ★★ The classification runs on the **resolved physical destination**, never on
/// `dst_is_virtual`. This port used to gate on `!dst_is_virtual`, which excludes exactly
/// the case #13 is about: the guest kernel's copy-engine utility identity-maps the whole
/// framebuffer into its own address space at 512 MiB pages and issues its page-table
/// writes as **VIRTUAL-destination** copies (`C: nvkvm_gpu_emul.c:4936-4952`). The C hooks
/// on the resolved physical regardless of form (`C: :6353`, `:6437`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeOperands {
    /// Operands the host MMU resolves. Forward; do not intercept.
    VaOperand {
        /// The resolved physical destination (diagnostics only — nothing acts on it).
        phys: u64,
    },
    /// Operands carrying guest-physical PTE values. Intercept.
    PhysOperand {
        /// The 4 KiB page-table page this write landed on.
        page: u64,
        /// The aperture the destination resolved through.
        aperture: Aperture,
        /// ★ The proc whose `Vas` OWNS that page — **not necessarily the proc whose
        /// channel issued the write.** The guest kernel writes user processes' page
        /// tables; that asymmetry is the reason the ownership index is device-global.
        owner: ProcId,
        /// The owning `Vas`'s PDB.
        owner_pdb: Pdb,
    },
}

/// One latched page-table write (`#13`, the C's `m2_cpt` dirty entry).
///
/// **Latched, not decoded.** The hot path records which page was touched and nothing else:
/// a big scrub can hit one page-directory page thousands of times, and decoding its
/// subtree per span **livelocked on the bench** — *"the first per-write attempt hung with
/// State=R busy-poll and no CTX OK"* (`C: :8686-8690`). The decode happens at the guest's
/// own commit point, the semaphore release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtWrite {
    /// The target GPU (a physical page address is per-GPU).
    pub gpu: GpuId,
    /// The 4 KiB page-table page base.
    pub page: u64,
    /// The aperture the write resolved through.
    pub aperture: Aperture,
    /// The proc whose `Vas` owns the page (see [`CeOperands::PhysOperand`]).
    pub owner: ProcId,
    /// The owning `Vas`'s PDB.
    pub owner_pdb: Pdb,
    /// Bytes the copy declared — how much of the page it may have changed.
    pub bytes: u64,
}

/// What one pushbuffer parse observed (for assertions + the caller's next steps).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushbufferOutcome {
    /// ★ Phys-operand commands: page-table pages this pushbuffer's copies wrote, each
    /// carrying the `Vas` that owns it. Latched here; applied by the caller.
    pub pt_writes: Vec<PtWrite>,
    /// ★ VA-operand commands seen and **not captured** — the address plane extracted
    /// nothing from them. Counted so "we did not intercept this" is an assertable fact
    /// rather than an absence, and so a test can tell "classified as data" apart from
    /// "never decoded at all".
    ///
    /// ★★ This is the CAPTURE tally and says **nothing** about who executes the copy —
    /// see [`PushbufferOutcome::host_ce`] / [`PushbufferOutcome::ours`], which partition
    /// the same commands on the other predicate. `data_copies + pt_writes.len()` and
    /// `host_ce + ours` count the same `LAUNCH_DMA`s two different ways.
    pub data_copies: usize,
    /// ★★★ EXECUTE, the **C BASELINE**, arm one: commands [`ce_executor_c`] leaves to
    /// real hardware.
    ///
    /// This is what the C would have done, kept beside what we actually do
    /// ([`PushbufferOutcome::ce_spans`]) so every departure §12's ruling introduces is a
    /// value a test reads rather than a paragraph. Nothing acts on it.
    pub c_execute_host_ce: usize,
    /// ★★★ EXECUTE, the **C BASELINE**, arm two: commands the C runs itself. There,
    /// every guest-kernel CE copy, every scrub and every fill lands here
    /// (`C: nvkvm_gpu_emul.c:6310`).
    pub c_execute_ours: usize,
    /// ★★★ **The partition we act on** (§12): every copy-engine request in this
    /// pushbuffer, split into sub-copies by the representability of its operands, in
    /// submission order. The caller turns these into [`VerbPlan::CeSplit`] and runs them
    /// on a worker — with no lock held (R1).
    pub ce_spans: Vec<CeSpan>,
    /// Semaphore releases observed → each `observe`d on the owning proc's queue.
    pub sem_releases: Vec<(GpuVa, u64)>,
    /// ★★★ How many of [`PushbufferOutcome::sem_releases`] came from a
    /// [`kayfabe_arch::PushMethod::CeRelease`] — a launch that moved **no bytes**.
    ///
    /// ⊘ Counted separately from the total for one reason, and it is a refusal to conflate:
    /// a release behind a copy and a release that IS the submission are different claims
    /// about what ran. A ring of `ce_releases == sem_releases.len()` with `ce_spans` empty
    /// executed nothing and owes nothing — which is exactly UVM's `channel_init` push — and
    /// a report that could not say so would look identical to one that dropped a copy.
    pub ce_releases: usize,
    /// TLB invalidates seen (pdb, membar). A membar is honored as a hard barrier
    /// (the parser records it; a real transport blocks advance until refresh).
    pub invalidates: Vec<(Pdb, bool)>,
    /// Count of opaque methods passed through (acted on by no core state).
    pub opaque: usize,
}

/// ★★★ **THE ONE PLACE the core touches guest-physical memory.**
///
/// Every `Vmm::gpa_read` / `Vmm::gpa_write` in the pure crates goes through here, and a
/// CI gate keeps it that way (`.github/workflows/ci.yml`, the GPA-accessor gate). The
/// point is not tidiness — it is that the *classification* of a refusal must exist in
/// exactly one place, for the same reason `RmGraph::undeclared_namespace` does:
///
/// - [`VmmError::NonRamGpa`] ⇒ [`FwdFault::NonRamGpa`] — the guest aimed a descriptor
///   at a device register window. **Named, never folded**, because this call runs under
///   the device read lock and a backend that served it would take the VMM's global lock
///   beneath one of our ranked locks (`l1_os_shell.md` §6.3 / §10.1 item 6).
/// - [`VmmError::BadGpa`] ⇒ [`FwdFault::GpaRead`] carrying **the port's address, not
///   ours**. ★ **Fixed 2026-07-27 (`l1_os_shell.md` §14.8 F6).** This arm used to fall
///   into the catch-all below, which substitutes the *requested* address — so a
///   straddling descriptor that ran off the end of a window reported where it *started*
///   while its near neighbour ([`VmmError::NonRamGpa`]) reported the **boundary byte**
///   [`kayfabe_vmm::GuestRamMap::resolve`] actually named. Two refusals whose payloads
///   mean different things is the shape `testing_doctrine.md` §2 rule 3 forbids, and it
///   was invisible because §12.43's straddle test uses a RAM→DEVICE range — i.e. the arm
///   that already kept it.
/// - anything else ⇒ [`FwdFault::GpaRead`] naming the address the *request* started at.
///   Nothing reaches this arm today ([`VmmError`]'s other variants are raised by the
///   mapping plane, not by `gpa_read`); it exists so a future variant degrades to a loud
///   refusal rather than to a compile error someone silences.
///
/// A `map_err(|_| …)` at the call site — which is what this replaced — discards the
/// variant, and with it the only evidence the refusal was the security-relevant one.
fn guest_read(vmm: &mut dyn Vmm, gpa: u64, buf: &mut [u8]) -> Result<(), FwdFault> {
    vmm.gpa_read(gpa, buf).map_err(|e| match e {
        VmmError::NonRamGpa { gpa } => FwdFault::NonRamGpa { gpa },
        VmmError::BadGpa { gpa } => FwdFault::GpaRead { gpa },
        _ => FwdFault::GpaRead { gpa },
    })
}

/// Decode a byte range of method words into `(header, args)` pairs, arch-driven.
/// Total on any input (a hostile/truncated range yields fewer methods, never a
/// panic or an unbounded read).
fn decode_methods(arch: &dyn kayfabe_arch::Arch, bytes: &[u8]) -> Vec<(u32, Vec<u32>)> {
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

/// ROUTE phase of the pushbuffer parse: the `ring`'s GPFIFO entries in the arch's own
/// entry format. A **pure spine read** — no proc, no guest memory, no `Vmm` — so it is
/// the half that legitimately runs before the owning proc's lock is taken.
///
/// Each [`PushRange`] names a **GPU virtual address in the issuing channel's address
/// space**; nothing here can or should translate it (see [`read_pushbuffer`]).
#[must_use]
pub fn pushbuffer_ranges(spine: &Spine, ring: &[u8]) -> Vec<PushRange> {
    spine.arch().pushbuffer().gpfifo_entries(ring)
}

/// ★★★ **Upper bound on how many address-table bindings ONE GPFIFO range may be read
/// across.**
///
/// The guest controls both the range's declared length *and* its address table's
/// fragmentation (a promotion or a page-table write may bind as little as one byte), so
/// "how many spans does this range cut into" is guest-influenced and needs a bound
/// (boundary-1). Exceeding it is a **loud refusal** ([`FwdFault::PushTooFragmented`]) and
/// never a truncation: a read that silently stops at span 4096 hands the method decoder a
/// prefix of the guest's pushbuffer and calls it the whole thing, which is `#13 CE-DROP`
/// with a different transport.
///
/// ⊘ It is not a performance knob. With [`MAX_PUSH_RANGE_BYTES`] at 1 MiB and the
/// smallest page a real mapping uses (4 KiB), a legitimate range cuts into at most 256
/// spans; the headroom is there so the bound is never the thing that breaks a real guest.
pub const MAX_PUSH_SPANS: usize = 4096;

/// ★★★ **ACT phase of the pushbuffer parse — and the phase where the GPFIFO entry's
/// address is TRANSLATED.**
///
/// Walks `ranges` (from [`pushbuffer_ranges`]), resolves each one's **GPU virtual**
/// address through the issuing channel's own address table, reads the method words out of
/// guest memory via `vmm`, and decodes them. Bounded per range, in total, and in
/// fragmentation.
///
/// # ★★★ Why this is not the route phase any more — §8.2.3, and it is a PHASE-SHAPE change
///
/// This function used to take `ring` directly, run `gpfifo_entries` itself and hand
/// `PushRange::gpa` to `Vmm::gpa_read` **with no walk**, under the device read lock and
/// touching no proc. That is the defect: a GPFIFO entry holds a GPU **virtual** address
/// (`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`; `pChannel->pbGpuVA` is a
/// `MAP_MEMORY_DMA` `dmaOffset`, `ogkm-580: mem_utils_gm107.c:842`, and every entry is
/// `pbGpuVA + gpOffset`, `:1871-1879`), `[measured]` at rev `c93930d` — two boots
/// differing only in guest RAM (8 GiB, 2 GiB) declared the **byte-identical** ring address
/// `0x1_2006_4000`, which at 2 GiB is outside every usable `e820` range.
///
/// Translating needs the channel's `Vas`, which needs the proc, so the read **moved from
/// the route phase into the act phase**. Three things about that, stated rather than
/// slipped in:
///
/// 1. **No new lock is acquired.** `SharedDevice::route_act` takes device-read (rank 0)
///    then that proc's mutex (rank 1) for a single operation; the read moved from between
///    those two acquisitions to after the second. The lock *set* of the whole op is
///    unchanged and the rank order is unchanged.
/// 2. **The in-lock-legality argument is unchanged, because it never mentioned a rank.**
///    `Vmm::gpa_read` is legal here only because the port refuses a GPA that is not host
///    RAM ([`FwdFault::NonRamGpa`]) — a backend that served a device-aimed GPA would take
///    the VMM's global lock *beneath one of ours*, which is `l1_os_shell.md` §6.3's ABBA
///    whether the lock above it is rank 0 or rank 1.
/// 3. ⊘⊘⊘ **CORRECTED `[w281, 2026-08-12]` — POINT 3 BELOW IS NO LONGER TRUE OF THE
///    PRODUCTION PATH, AND THE REASON IS A LOCK RANK, NOT A CHANGE OF MIND.** This
///    function is now the **no-framebuffer** wrapper over
///    [`plan_pushbuffer`] → [`fetch_pushbuffer`] → [`decode_pushbuffer`], and
///    `kayfabe_rt::device::SharedDevice::parse_pushbuffer` calls those three directly.
///    ⇒ Read point 3 as a statement about *this wrapper*, which really does hold one lock
///    set across the read because it never touches the framebuffer.
///
///    **Why the split became forced.** Reading a **vidmem** pushbuffer needs
///    [`FbBytes`], whose only production implementation takes the plane's mutex —
///    `LockRank::Plane`, **rank 0** — and `route_act` holds ranks 1 and 2. Taking the
///    plane beneath them is `core → plane`, which `check_acquire` refuses **by name**.
///    This is the identical hazard that forced [`plan_gpfifo_ring`] /
///    [`fetch_ring_bytes`] apart in `w235`, one level down, and it is a **deadlock**,
///    which strictly dominates the TOCTOU point 3 weighs.
///
///    ⚠ **The TOCTOU point 3 names is REAL and is now ACCEPTED, with its blast radius
///    stated.** Between plan and fetch the guest may invalidate a translation, so the
///    bytes may come from a page that *was* named by a table the guest owned at plan
///    time and has since been unmapped. That is a **stale read of memory the guest
///    itself named**, never a read of memory it never owned: the runs are computed from
///    that channel's own table under the lock and are never recomputed outside it. The
///    ring has run under exactly this exposure since `w235`. ⊘ It is a widening, it is
///    stated here rather than discovered later, and it is the price of the rank order.
///
/// # The refusals, and why each is its own name
///
/// - **No address space** — the channel is GSP-managed with no declared VAS
///   (`Channel::vas_pdb == None`), or its `Vas` is gone: [`FwdFault::NoVas`] /
///   [`FwdFault::UnknownPdb`]. There is no table to miss in, which is a different fact
///   from missing in one.
/// - **MISS** — no binding covers the VA: [`FwdFault::Address`]`(`[`AddressFault::Miss`]`)`,
///   naming the exact faulting VA. **This is the whole doctrine** (`mode2_address_table.md`:
///   the table *is* the guest's TLB, miss = fault, never a reverse-resolve, never a
///   heuristic). An unresolvable VA is not a zero and not a guess.
/// - **Wrong aperture** — the binding resolves into video or peer memory:
///   [`FwdFault::PushbufferAperture`]. `Vmm::gpa_read` addresses *guest RAM*; a vidmem
///   `Binding::phys` is a guest **framebuffer** offset and handing it to `gpa_read` would
///   read an unrelated page of guest RAM that happens to share the number — the same
///   class of silent wrong-bytes failure as the untranslated read this replaces.
/// - **Too fragmented** — [`FwdFault::PushTooFragmented`], see [`MAX_PUSH_SPANS`].
/// - **The read itself** — [`FwdFault::GpaRead`] / [`FwdFault::NonRamGpa`], unchanged, and
///   still the arm that makes "FAULT" literal for an address that resolved but is not RAM.
///
/// # Errors
///
/// Every bullet above, plus [`FwdFault::RetiredProc`].
pub fn read_pushbuffer(
    spine: &Spine,
    proc: &Proc,
    cid: ChanId,
    vmm: &mut dyn Vmm,
    ranges: &[PushRange],
) -> Result<Vec<(u32, Vec<u32>)>, FwdFault> {
    // ⊘ `false`, and it is not a policy choice here: this wrapper is handed no [`FbBytes`],
    // so `OwnFramebuffer` would plan runs nothing could read. The vidmem decision lives at
    // the three-phase call site, which is the only place that HAS a store.
    let plan = plan_pushbuffer(proc, cid, ranges, false)?;
    let bytes = fetch_pushbuffer(&plan, vmm, None)?;
    Ok(decode_pushbuffer(spine, &bytes))
}

/// ★★★★★ **The PLAN half of the pushbuffer read — everything that needs the core's locks,
/// and NOTHING that touches a byte.** The [`fetch_ring_bytes`] shape, one level down.
///
/// `vidmem` is the switch [`read_pushbuffer`]'s corrected point 3 describes: `false`
/// reproduces the pre-`w281` behaviour exactly (a vidmem range is
/// [`FwdFault::PushbufferAperture`], raised **here**, under the lock, before any byte is
/// planned); `true` plans those runs out of our own framebuffer instead, and
/// [`fetch_pushbuffer`] must then be handed a store or it raises the same refusal.
///
/// ⊘ **The caller decides, and the caller must say so on its own flag.** `w279`'s result
/// ruled that this widening is *"its own flag, never folded into route B"*: route B is the
/// registration of an [`FbSource`], which is a *supply*; this is a *route*, and a boot that
/// cannot tell which of the two produced a byte cannot attribute it.
///
/// # Errors
/// [`FwdFault::RetiredProc`], [`FwdFault::NoVas`], [`FwdFault::UnknownPdb`], and every
/// translation refusal [`read_pushbuffer`] documents.
pub fn plan_pushbuffer(
    proc: &Proc,
    cid: ChanId,
    ranges: &[PushRange],
    vidmem: bool,
) -> Result<PushPlan, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let cgpu = chan.gpu;
    // A channel with no declared VAS has no table to translate in. MISS = FAULT applies
    // to the table's ABSENCE too — it is not an invitation to read the number raw.
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let table = &proc
        .vases
        .get(&(cgpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?
        .table;
    let route = if vidmem {
        VidmemRoute::OwnFramebuffer
    } else {
        VidmemRoute::Refuse
    };

    let mut out: Vec<(usize, Vec<(PushSrc, usize, usize)>)> = Vec::new();
    let mut total = 0usize;
    for r in ranges {
        if total >= MAX_PUSH_TOTAL_BYTES {
            break; // Total-work budget spent — a hostile many-range ring stops here.
        }
        // A hostile GPFIFO entry can name any length; cap the per-range read so a bogus
        // length is a bounded read, never an arbitrary allocation (boundary-1 posture).
        let len = (r.len as usize)
            .min(MAX_PUSH_RANGE_BYTES)
            .min(MAX_PUSH_TOTAL_BYTES - total);
        // TRANSLATE. The clamped length is what gets translated, so the cap above is still
        // the thing that bounds the work — a hostile length cannot make this walk the
        // whole table.
        out.push((len, push_range_gpas(table, pdb, r, len, route)?));
        total += len;
    }
    Ok(PushPlan { ranges: out })
}

/// One pushbuffer read's translated runs, computed under the core's locks and read outside
/// them — [`RingPlan`]'s sibling.
///
/// ⊘ Opaque for [`RingPlan`]'s reason: a run names a framebuffer offset **or** a GPA
/// depending on the aperture, and exposing them as numbers is exactly the confusion
/// [`PushSrc`] exists to prevent.
///
/// ★ Kept **per range**, not flattened: [`decode_methods`] runs per range and a method
/// stream that ran across a range boundary would decode a header out of one range's tail
/// and its operands out of the next one's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPlan {
    ranges: Vec<(usize, Vec<(PushSrc, usize, usize)>)>,
}

impl PushPlan {
    /// How many ranges survived the total-work budget. ⊘ For logging and tests only —
    /// nothing decides on it.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether the budget left nothing to read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// ★★★ Whether any planned run reads our own framebuffer rather than guest RAM — the
    /// one fact a grader needs to tell *"the vidmem route was on"* from *"it was on and
    /// nothing needed it"*.
    #[must_use]
    pub fn touches_fb(&self) -> bool {
        self.ranges
            .iter()
            .any(|(_, runs)| runs.iter().any(|(s, _, _)| matches!(s, PushSrc::Fb { .. })))
    }
}

/// One pushbuffer read's bytes, per range, in plan order. ⊘ Raw bytes and nothing else:
/// decoding needs the arch, which lives behind a lock this phase does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushBytes {
    ranges: Vec<Vec<u8>>,
}

/// ★★★ **The FETCH half — the bytes, with EVERY ranked guard dropped.**
///
/// [`fetch_ring_bytes`]'s obligations apply verbatim, including the one nothing in this
/// signature can assert: the core's locks must be **down**, because the plane mutex this
/// may take is rank 0 and `route_act` holds ranks 1 and 2. A [`PushPlan`] is the only way
/// to reach this function and producing one is the phase that holds the locks.
///
/// ⊘ **`fb = None` is not "no framebuffer pages here"** — it is *"this caller declines to
/// serve them"*, and a planned [`PushSrc::Fb`] run then raises
/// [`FwdFault::PushbufferAperture`] naming the guest's own VA, exactly as the unwidened
/// path did.
///
/// ⚠ **No `RingFbNeverWritten` equivalent is raised here.** The ring needs that guard
/// because an unwritten ring page is byte-identical to a quiet one and decodes to
/// `NoLiveEntries` — self-concealing. The pushbuffer's blank case is **not** self-concealing,
/// but ⊘⊘ **NOT for the reason first written here, which was FALSE and a test caught it.**
///
/// `[w281, measured]` The claim was *"an unwritten page decodes to zero methods, visible as
/// a count of 0"*. It does **not**. A 64-byte zero page decodes to **16 `(0, [])` pairs**, a
/// non-zero count. On GA10x a zero header is `sec_op = GRP0_USE_TERT`, `tert_op =
/// TERT_OP_METHOD` ⇒ `MethodForm::Legacy` with `arg_words = 0`
/// (`kayfabe_abi::submit::method_header_decode`), and `Ga10xPushbuffer::decode_method`
/// answers [`kayfabe_arch::PushMethod::Opaque`] because the form is not `Incrementing`.
///
/// ⇒ The real property, and the one
/// `tests/tests/pushbuffer_out_of_our_own_framebuffer.rs` asserts, is stronger and is about
/// **facts, not counts**: every method a blank page decodes to is `Opaque`, so a blank
/// pushbuffer yields **no `SetObject`, no CE span and no semaphore release** — it cannot
/// imitate work. That is forbidden #2's actual requirement. ⚠ A method *count* would have
/// been a useless discriminator here, which is exactly what the false claim asserted it was.
///
/// # Errors
/// [`FwdFault::PushbufferAperture`] for a vidmem run with no store, or a store that does
/// not back the range; and the `Vmm` read's own refusals.
pub fn fetch_pushbuffer(
    plan: &PushPlan,
    vmm: &mut dyn Vmm,
    mut fb: Option<&mut dyn FbBytes>,
) -> Result<PushBytes, FwdFault> {
    let mut out = Vec::with_capacity(plan.ranges.len());
    for (len, runs) in &plan.ranges {
        let mut buf = vec![0u8; *len];
        for &(src, at, take) in runs {
            match src {
                PushSrc::Gpa(gpa) => guest_read(vmm, gpa, &mut buf[at..at + take])?,
                PushSrc::Fb { phys, va } => {
                    let fb = fb.as_deref_mut().ok_or(FwdFault::PushbufferAperture {
                        va,
                        aperture: Aperture::Vidmem,
                    })?;
                    if !fb.read(phys, &mut buf[at..at + take]) {
                        return Err(FwdFault::PushbufferAperture {
                            va,
                            aperture: Aperture::Vidmem,
                        });
                    }
                }
            }
        }
        out.push(buf);
    }
    Ok(PushBytes { ranges: out })
}

/// The DECODE half — pure, and back under whichever locks the caller wants, because it
/// touches neither guest memory nor the framebuffer.
#[must_use]
pub fn decode_pushbuffer(spine: &Spine, bytes: &PushBytes) -> Vec<(u32, Vec<u32>)> {
    let mut methods = Vec::new();
    for buf in &bytes.ranges {
        methods.extend(decode_methods(spine.arch(), buf));
    }
    methods
}

/// How many **leading** entries of `ring` the guest has actually written, decoded by
/// `pb`'s own codec one entry at a time.
///
/// ★ The stop rule is the guest's, not ours: *"an unwritten entry is zero and decodes to
/// nothing — that is the ring saying 'no more work', not a malformed entry, because RM
/// zero-initialises this buffer (`TRANSFER_FLAGS_SHADOW_INIT_MEM`)"*
/// (`kayfabe_rt::ceutils::run_submission`). So the walk stops at the first entry the codec
/// yields nothing for, or yields a zero-length range for.
///
/// ⊘ Decoded **per entry** rather than by calling `gpfifo_entries` on the whole ring and
/// counting: a codec that filters undecodable entries (`Ga10xPushbuffer` does — it is a
/// `filter_map`) returns a count that is no longer a position, and a cursor built on it
/// would drift past exactly the entries it could not read.
#[must_use]
pub fn gpfifo_live_entries(pb: &dyn kayfabe_arch::PushbufferAbi, ring: &[u8]) -> usize {
    let stride = pb.gpfifo_entry_stride();
    if stride == 0 {
        return 0;
    }
    let mut n = 0usize;
    while (n + 1) * stride <= ring.len() {
        let entry = &ring[n * stride..(n + 1) * stride];
        match pb.gpfifo_entries(entry).first() {
            Some(r) if r.len > 0 => n += 1,
            _ => break,
        }
    }
    n
}

/// The most of one channel's GPFIFO ring this port will read for a single doorbell.
///
/// ★ A bound of **ours**, on a length the guest chooses: `gpFifoEntries` is a guest-declared
/// `u32` and the mapping behind `gpFifoOffset` is a guest-chosen extent, so both are
/// hostile inputs (boundary-1). 64 KiB is comfortably above the shapes measured on this
/// project — RM's own CeUtils ring is `GPFIFO_SIZE = 0x8000` (`ogkm-580:
/// channel_utils.c:243-250`, and `tests/tests/e10e_ceutils_doorbell.rs` encodes it) — and
/// small enough that a ring declared at 4 GiB is a bounded read rather than an allocation.
pub const MAX_GPFIFO_RING_BYTES: usize = 64 * 1024;

/// ★★★ **Where one run of a translated range's bytes must be read FROM.**
///
/// ⊘ Not a `u64` with a flag beside it. A guest-physical address and a framebuffer offset
/// are different address spaces that share a numeric type, and the single most likely way
/// to get this wrong is to carry one and read it with the other's reader — which succeeds,
/// returns bytes, and returns the **wrong** bytes. Making the source a variant means the
/// reader is chosen by a `match` the compiler checks, not by a `bool` a caller remembers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushSrc {
    /// A guest-physical address, for [`Vmm::gpa_read`].
    Gpa(u64),
    /// A framebuffer offset into **our own** [`FbBytes`] store, plus the guest VA it came
    /// from — the VA is carried only so a refusal can name the address the guest used.
    Fb { phys: u64, va: GpuVa },
}

/// ★★★★★ **May a translated range be read out of OUR OWN emulated framebuffer?**
///
/// # ⊘ This is a MEASUREMENT SWITCH, not a design decision
///
/// `[w235, 2026-08-11]` The guest's GPFIFO ring for the 8 `proc 2` doorbells lives in the
/// **emulated framebuffer**, not in guest RAM: the descent already prints its bytes
/// (`fbRING[p0]@0x1024000=0000c002…`). Reading it is *route B*. Route A — influencing the
/// ring's aperture at allocation time so it lands in sysmem — is being measured
/// independently and, if it answers YES, is the better route and this becomes a stepping
/// stone.
///
/// ⚠ **The scope tension, named rather than papered over.** The owner's 2026-08-07 ruling
/// sanctions *"a different executor producing a true end-state"* but **scopes itself to
/// kernel-originated copy-engine work**; these doorbells are **user `proc 2`**, which the
/// same file calls *passthrough, not inspected*. Enumerating the ring is agreed. What may
/// happen **after** enumeration is an open question for the owner, and nothing here decides
/// it. ⇒ [`VidmemRoute::Refuse`] is the default at every production call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VidmemRoute {
    /// ⊘ **The default.** A vidmem range is [`FwdFault::PushbufferAperture`], exactly as
    /// before this switch existed. No call site reaches the other arm without opting in.
    #[default]
    Refuse,
    /// Read vidmem ranges out of the emulated framebuffer this device already serves.
    OwnFramebuffer,
}

/// ★★★★ **The bytes of our own emulated framebuffer, and WHETHER A PAGE WAS EVER WRITTEN.**
///
/// # ⊘ Why `read` alone is not enough, and this is the rung's central hazard
///
/// [`kayfabe_device::FbStore::read`] answers an address inside the aperture that nothing
/// ever wrote with **zeros and `Ok`** — deliberately, and documented. A GPFIFO ring is
/// *supposed* to be mostly zeros (`gpfifo_live_entries` stops at the first zero entry,
/// because RM zero-initialises the buffer). ⇒ **a ring page that was never written and a
/// ring page written with an empty tail are byte-identical**, and the first is
/// `nz0/4096` — the blank operand that is forbidden #2.
///
/// ★ So residency, not bytes, is the discriminator: *a page nothing ever wrote is not in
/// the map.* [`Self::page_written`] is that question, and
/// [`FwdFault::RingFbNeverWritten`] is the refusal. ⊘ Without this the route would read
/// 64 KiB of zeros, report `NoLiveEntries`, and look **exactly** like a correct quiet
/// channel — self-concealing, which is the property that makes forbidden #2 expensive.
pub trait FbBytes {
    /// Fill `buf` from framebuffer-physical address `phys`; `false` if this store does not
    /// back the range at all.
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool;

    /// ★★★ **Was the page containing `phys` ever written?** [`None`] from a store that
    /// cannot answer — which callers must treat as *unmeasured*, never as *no*.
    ///
    /// ⊘ The `Option` is the `dlen=0` lesson in a signature: a store with no origin
    /// tracking must be able to say *"I cannot tell you"* instead of being forced into a
    /// `false` that reads as a positive claim about the guest.
    fn page_written(&self, phys: u64) -> Option<bool>;
}

/// ★★★★ **A SHARED, long-lived source of our own framebuffer's bytes** — what a device
/// holds, as opposed to [`FbBytes`], which is what one read borrows.
///
/// # ⊘ Why this exists as well as [`FbBytes`]
///
/// `FbBytes` takes `&mut self` because a *borrowed* reader may be a connection. A device that
/// wants to answer framebuffer reads for the whole of its life needs something `Send + Sync`
/// it can keep in an `Arc`, and `&mut self` cannot be handed out from an `Arc` without
/// interior mutability. So the stored form takes `&self` and the borrowed form is derived
/// from it ([`FbSourceRef`]).
///
/// ★★★ **Registering one is what turns route B ON.** There is no boolean anywhere: a device
/// with no source refuses vidmem ranges exactly as it did before route B existed
/// ([`VidmemRoute::Refuse`]), because [`read_gpfifo_ring`] derives the route from whether it
/// was handed a reader. ⇒ *A default-off flag that is not off by construction is a
/// default-on flag with a comment.*
pub trait FbSource: Send + Sync + core::fmt::Debug {
    /// Fill `buf` from framebuffer-physical address `phys`; `false` if unbacked.
    fn read(&self, phys: u64, buf: &mut [u8]) -> bool;

    /// ★★★ Was the page containing `phys` ever written? [`None`] = **cannot tell**, which is
    /// *unmeasured* and must never be read as *no*. See [`FbBytes::page_written`].
    fn page_written(&self, phys: u64) -> Option<bool>;
}

/// One read's borrow of a [`FbSource`] — the adapter that makes the stored form usable.
#[derive(Debug)]
pub struct FbSourceRef<'a>(pub &'a dyn FbSource);

impl FbBytes for FbSourceRef<'_> {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        self.0.read(phys, buf)
    }
    fn page_written(&self, phys: u64) -> Option<bool> {
        self.0.page_written(phys)
    }
}

/// ★★★★ **What [`read_gpfifo_ring`] found — the ring, or the NAMED reason there is none.**
///
/// # ⊘ Why this is an enum and not an `Option`
///
/// `[measured 2026-08-10, boot `p2_29e7c25_planereal`]` three guest doorbells were reported
/// `SERVED … forwarded (host channel rung)` and the guest's copy-engine scrubber then died
/// on `lastCompletedPayload == lastSubmittedPayload`. `execution_plane_increments.md`
/// §16.69.5 records the two stories that evidence could not separate: *the GPU ran the work
/// and we lost the completion*, or *the channel was rung with nothing in it*. This function
/// is where the second story is decided — it had **six** distinct `Ok(None)` arms, every one
/// of which makes [`crate::DoorbellOutcome`] report `Served` with **zero bytes forwarded** —
/// and an `Option` collapsed all six into the same silence as "the ring was read and was
/// empty".
///
/// ⊘ **An absence with no name is not a measurement.** Each variant below is a different
/// fact about the guest or about our own address plane, and they are not interchangeable:
/// `NoAddressSpace` is a channel `kayfabe_rt::ceutils` owns, while `RingVaUnbound` is our
/// table failing to hold a mapping the guest is actively using.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingLook {
    /// The mapped prefix of the ring, as bytes. Never empty.
    Ring(Vec<u8>),
    /// The channel's resource has no node in the RM graph — nothing was ever declared.
    NoChannelNode,
    /// The node exists and declares no `gpFifoOffset`/`gpFifoEntries` at all.
    NoRingDeclared,
    /// A ring was declared, and declared **empty** — `gpFifoOffset = 0` or zero entries.
    /// The golden-context channel does this on purpose (`ogkm-580: kernel_graphics.c:2420`).
    RingDeclaredEmpty {
        /// `gpFifoOffset`, as declared.
        va: u64,
        /// `gpFifoEntries`, as declared.
        entries: u32,
    },
    /// The channel has no address space (`vas_pdb == None`), so this table is not the one
    /// its ring lives in. ⊘ Not a fault: the shell's CPU copy-engine path owns it.
    NoAddressSpace,
    /// ★★★ The ring's own VA is **not bound** in the channel's address table. The guest is
    /// submitting through a mapping our forward-populated table never witnessed.
    RingVaUnbound {
        /// The declared `gpFifoOffset` that resolved to nothing.
        va: u64,
    },
    /// The VA is bound, but the binding ends at or before it — zero readable bytes.
    RingMappedZero {
        /// The declared `gpFifoOffset`.
        va: u64,
    },
}

impl RingLook {
    /// A short, stable tag naming this outcome, for a diagnostic line.
    ///
    /// ★ The tag is the *variant*, never a derived judgement: a reader must be able to tell
    /// which of the six absences happened without the printer having decided what it means.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            RingLook::Ring(_) => "RING",
            RingLook::NoChannelNode => "NO-CHANNEL-NODE",
            RingLook::NoRingDeclared => "NO-RING-DECLARED",
            RingLook::RingDeclaredEmpty { .. } => "RING-DECLARED-EMPTY",
            RingLook::NoAddressSpace => "NO-ADDRESS-SPACE",
            RingLook::RingVaUnbound { .. } => "RING-VA-UNBOUND",
            RingLook::RingMappedZero { .. } => "RING-MAPPED-ZERO",
        }
    }
}

/// ★★★ **Read channel `cid`'s own GPFIFO ring out of guest memory**, through that
/// channel's own address table.
///
/// This is the half of the join that had no production caller and therefore no production
/// *source*: [`read_pushbuffer`] translates the ranges a ring names, but nothing fetched
/// the ring. `docs/design/execution_plane_increments.md` §15.5 states the consequence —
/// *"`SERVED` on the real plane means: we rang a doorbell on a host channel into which the
/// guest's methods were never copied"* — and prescribes the order this closes:
/// **wire the ring first, complete second.**
///
/// # ⊘ A NAMED absence is a real answer, and it is the common one
///
/// Six shapes answer a [`RingLook`] absence rather than faulting, each because the *guest*
/// said something rather than because we could not. ⚠ They used to be one `Ok(None)`, which
/// is why §16.69's boot could not say which of them made `3 forwarded` mean nothing:
///
/// 1. **The channel declares no ring at all** (`AllocFacts::gp_fifo_ring == None`), or
///    declares `gpFifoOffset = 0` with zero entries — which a real driver does on purpose
///    for a channel it only uses to build a golden context (`ogkm-580:
///    kernel_graphics.c:2420-2424`, *"Set the gpFifoOffset to zero intentionally"*, and
///    `kayfabe_core::rmgraph::GpFifoRing::va`'s own warning that **0 is a value, not a
///    blank**).
/// 2. **The channel has no address space** (`vas_pdb == None`). A GSP-managed channel's
///    ring is served by the shell's CPU copy-engine path (`kayfabe_rt::ceutils`), which
///    descends the guest's published page tables instead; it is not this table's to read.
/// 3. ⚠ **The ring's own VA is not bound in that table.** Stated as a limit rather than
///    claimed as a design: a declared-but-unmapped ring *is* a fact worth a name, and this
///    rung answers `None` so that a doorbell which is served today keeps being served
///    today. ⊘ `[NOT MEASURED]` — no boot has been run against this, and turning it into a
///    refusal without one would be refusing live traffic on a reading.
///
/// # Errors
/// [`FwdFault::RetiredProc`]; [`FwdFault::UnknownPdb`] if the channel's declared VAS is
/// gone; and every translation refusal [`read_pushbuffer`] documents — MISS, wrong
/// aperture, over-fragmented, and the read itself.
pub fn read_gpfifo_ring(
    spine: &Spine,
    proc: &Proc,
    cid: ChanId,
    vmm: &mut dyn Vmm,
    fb: Option<&mut dyn FbBytes>,
) -> Result<RingLook, FwdFault> {
    match plan_gpfifo_ring(spine, proc, cid, fb.is_some())? {
        RingPlanLook::Planned(plan) => Ok(RingLook::Ring(fetch_ring_bytes(&plan, vmm, fb)?)),
        RingPlanLook::Absent(a) => Ok(a),
    }
}

/// ★★★★★ **The PLAN half — everything that needs the core's locks, and NOTHING that
/// touches a byte.**
///
/// # ⊘ Why this is split at all, and it is a MEASURED hazard, not a style preference
///
/// `[w235, 2026-08-11]` Route B needs the emulated framebuffer's bytes, and the only seam
/// that serves them is [`kayfabe_device::RegPlane::pt_bytes`], **which takes the plane's FSM
/// mutex on every single read**. `kayfabe_rt::device::forward_ring` calls the ring reader
/// inside a `route_act` closure holding the **rank-0 device read lock and the rank-1 proc
/// mutex**. Reading the framebuffer there would take the plane mutex *beneath* two core
/// locks — the **exact inversion** of the established `plane → core` order:
///
/// - `plane.rs`' `ce_session` doc: *"the caller must hold no core lock … the command-policy
///   chain already takes the core's ranked locks under this mutex, so plane→core is the
///   established order and core→plane is its inversion."*
/// - `unranked_locks.rs`' row for `Mutex<PlaneState>`: *"★★★ THE HAZARD … NOTHING may block
///   beneath it … and the R1 witness will not say so."*
///
/// ★★★ **And that last clause is the whole reason this is a function and not a comment.**
/// The plane mutex is a bare `std::sync::Mutex`, **unranked** — so `assert_lock_free` passes
/// **vacuously** while it is held and no existing gate would have failed. The ABBA partner
/// is not hypothetical and is already shipping: the policy chain takes the core's ranked
/// locks under `state.lock()` on another vCPU's MMIO trap, so a guest that rings a doorbell
/// on one vCPU while touching a register on another **builds the deadlock itself**.
///
/// ⇒ The plan is computed under the core's locks; [`fetch_ring_bytes`] reads the bytes with
/// **every ranked guard dropped**. Same shape as `decode_pt_writes_from`'s plan/execute/commit.
///
/// # Errors
/// As [`read_gpfifo_ring`].
pub fn plan_gpfifo_ring(
    spine: &Spine,
    proc: &Proc,
    cid: ChanId,
    vidmem: bool,
) -> Result<RingPlanLook, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let cgpu = chan.gpu;
    // (1) Nothing declared, or declared empty — see the doc's shape 1.
    let Some(node) = spine.rmgraph.node_of_resource(chan.key) else {
        return Ok(RingPlanLook::Absent(RingLook::NoChannelNode));
    };
    let Some(ring) = node.facts.gp_fifo_ring else {
        return Ok(RingPlanLook::Absent(RingLook::NoRingDeclared));
    };
    if ring.va == 0 || ring.entries == 0 {
        return Ok(RingPlanLook::Absent(RingLook::RingDeclaredEmpty {
            va: ring.va,
            entries: ring.entries,
        }));
    }
    // (2) No table to read it in. ⊘ Not a fault: `ceutils` owns this channel's ring.
    let Some(pdb) = chan.vas_pdb else {
        return Ok(RingPlanLook::Absent(RingLook::NoAddressSpace));
    };
    let table = &proc
        .vases
        .get(&(cgpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?
        .table;
    // (3) How much of it the guest actually mapped, bounded by our own ceiling. ⊘ The
    // extent comes from `binding_at` rather than from `entries × stride`: the declared
    // count is the guest's and the mapping is the guest's, and reading past what was
    // mapped would fault on a range the guest never claimed held entries.
    let base = GpuVa(ring.va);
    let Some((start, b_len, _)) = table.binding_at(base) else {
        return Ok(RingPlanLook::Absent(RingLook::RingVaUnbound {
            va: ring.va,
        }));
    };
    let mapped = b_len.saturating_sub(base.0 - start);
    let len = usize::try_from(mapped)
        .unwrap_or(MAX_GPFIFO_RING_BYTES)
        .min(MAX_GPFIFO_RING_BYTES);
    if len == 0 {
        return Ok(RingPlanLook::Absent(RingLook::RingMappedZero {
            va: ring.va,
        }));
    }
    let r = PushRange {
        va: base,
        len: len as u64,
    };
    let route = if vidmem {
        VidmemRoute::OwnFramebuffer
    } else {
        VidmemRoute::Refuse
    };
    Ok(RingPlanLook::Planned(RingPlan {
        runs: push_range_gpas(table, pdb, &r, len, route)?,
        len,
    }))
}

/// ★★★ **The FETCH half — the bytes, with EVERY ranked guard dropped.**
///
/// ⚠ **The caller's obligation is not checkable here**: nothing in this signature can
/// assert that the core's locks are down, because the plane mutex this may take is
/// **unranked** and the R1 witness is vacuous against it (see [`plan_gpfifo_ring`]). The
/// type system carries the discipline instead — a [`RingPlan`] is the only way to reach
/// this function, and producing one is the phase that holds the locks.
///
/// # Errors
/// [`FwdFault::RingFbNeverWritten`] for a framebuffer page nothing ever wrote;
/// [`FwdFault::PushbufferAperture`] if the store refuses the range; and the `Vmm` read's
/// own refusals.
pub fn fetch_ring_bytes(
    plan: &RingPlan,
    vmm: &mut dyn Vmm,
    mut fb: Option<&mut dyn FbBytes>,
) -> Result<Vec<u8>, FwdFault> {
    let mut buf = vec![0u8; plan.len];
    for &(src, at, take) in &plan.runs {
        match src {
            PushSrc::Gpa(gpa) => guest_read(vmm, gpa, &mut buf[at..at + take])?,
            PushSrc::Fb { phys, va } => {
                // ★★★ THE GATE. Residency first, bytes second — and in that order,
                // because the bytes cannot answer the question.
                let fb = fb.as_deref_mut().ok_or(FwdFault::PushbufferAperture {
                    va,
                    aperture: Aperture::Vidmem,
                })?;
                if fb.page_written(phys) == Some(false) {
                    return Err(FwdFault::RingFbNeverWritten { va, phys });
                }
                if !fb.read(phys, &mut buf[at..at + take]) {
                    return Err(FwdFault::PushbufferAperture {
                        va,
                        aperture: Aperture::Vidmem,
                    });
                }
            }
        }
    }
    Ok(buf)
}

/// A ring's translated runs, computed under the core's locks and read outside them.
///
/// ⊘ Opaque on purpose: the runs name a **framebuffer offset or a GPA** depending on the
/// aperture, and exposing them as numbers is precisely the confusion [`PushSrc`] exists to
/// prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingPlan {
    runs: Vec<(PushSrc, usize, usize)>,
    len: usize,
}

/// [`plan_gpfifo_ring`]'s answer: a plan, or one of [`RingLook`]'s named absences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingPlanLook {
    /// The ring resolved; [`fetch_ring_bytes`] will produce its bytes.
    Planned(RingPlan),
    /// One of the six named absences. ⊘ Never [`RingLook::Ring`].
    Absent(RingLook),
}

/// Translate one [`PushRange`]'s first `len` bytes into the guest-physical runs they
/// occupy, through `table` — `(gpa, offset within the range, length)`, in ascending order
/// and covering `[0, len)` exactly.
///
/// ★ A VA range is contiguous; the memory behind it need not be. So this partitions
/// rather than resolving once: reading `len` bytes from the *first* binding's physical
/// address would run off the end of that binding and into whatever guest page follows —
/// which is the same "the read succeeded, the bytes are wrong" failure as the
/// untranslated read, one level down.
///
/// ⊘ [`AddressTable::binding_at`] rather than [`AddressTable::resolve`], and the choice is
/// forced rather than stylistic: `resolve` deliberately hides the binding's **extent**, and
/// without the extent there is no way to know how many bytes may legally be read before
/// the next lookup. The miss it would have raised is constructed here verbatim, so the
/// vocabulary a caller sees is the address plane's own.
fn push_range_gpas(
    table: &AddressTable,
    pdb: Pdb,
    r: &PushRange,
    len: usize,
    vidmem: VidmemRoute,
) -> Result<Vec<(PushSrc, usize, usize)>, FwdFault> {
    let mut out: Vec<(PushSrc, usize, usize)> = Vec::new();
    let mut at = 0usize;
    while at < len {
        // `r.va + at` cannot wrap into a *different* mapping silently: a wrap means the
        // guest's range ran off the top of the address space, which resolves to nothing
        // at the bottom because `bind` refuses a wrapping range in the first place.
        let va = GpuVa(r.va.0.wrapping_add(at as u64));
        if va.0 < r.va.0 {
            return Err(FwdFault::Address(AddressFault::Malformed { pdb, va: r.va }));
        }
        let (start, b_len, b) = table
            .binding_at(va)
            .ok_or(FwdFault::Address(AddressFault::Miss { pdb, va }))?;
        // ★★★ Which STORE owns these bytes. Sysmem `phys` is a GPA; vidmem `phys` is a
        // framebuffer offset into our own `SparseFb`. They are different address spaces
        // that share a number, so the aperture picks the reader — it is never a cast.
        let vid = match b.aperture() {
            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => false,
            // ⊘ **Default-off, and the default is this arm.** `VidmemRoute::Refuse` keeps
            // the pre-w235 behaviour byte for byte: vidmem `phys` is not a guest-physical
            // address, and handing it to `gpa_read` would read the guest RAM page that
            // happens to share the number.
            Aperture::Vidmem if vidmem == VidmemRoute::OwnFramebuffer => true,
            other => {
                return Err(FwdFault::PushbufferAperture {
                    va,
                    aperture: other,
                });
            }
        };
        let off = va.0 - start;
        let phys = b
            .phys()
            .checked_add(off)
            .ok_or(FwdFault::Address(AddressFault::Malformed { pdb, va }))?;
        let gpa = if vid {
            PushSrc::Fb { phys, va }
        } else {
            PushSrc::Gpa(phys)
        };
        // `binding_at` never returns a zero-length range (`bind` refuses one), so `take`
        // is always ≥ 1 and this loop always advances.
        let take = usize::try_from((b_len - off).min((len - at) as u64)).unwrap_or(len - at);
        debug_assert!(take > 0, "a translation step must consume bytes");
        if out.len() == MAX_PUSH_SPANS {
            return Err(FwdFault::PushTooFragmented {
                va: r.va,
                len: r.len,
            });
        }
        out.push((gpa, at, take));
        at += take;
    }
    Ok(out)
}

/// ★★★ **The operand split**, for one decoded copy-engine command.
///
/// Two steps, in this order and only this order:
///
/// 1. **RESOLVE** the destination to a physical address. A virtual destination is walked
///    through the *issuing channel's own* `Vas` — which is what makes the framebuffer-alias
///    case work: the kernel's copy-engine utility maps FB into its own address space, so
///    its 512 MiB alias resolves in ITS table to the physical page it is writing. MISS =
///    FAULT, no fallback walk, no cross-VAS guess (that fallback is the C's #12 collision
///    class, `eight_blockers_resolved.md` §2).
/// 2. **CLASSIFY** on that resolved physical, via the device-global ownership index.
///
/// Doing it in the other order — classify on the operand *form*, then resolve — is the
/// inverted gate this replaces.
fn classify_ce(
    spine: &Spine,
    proc: &Proc,
    cid: ChanId,
    chan_pdb: Option<Pdb>,
    cgpu: GpuId,
    dst: GpuVa,
    dst_is_virtual: bool,
) -> Result<CeOperands, FwdFault> {
    let (phys, aperture) = if dst_is_virtual {
        // The destination is an address in the issuing channel's VAS. Walk it there.
        let pdb = chan_pdb.ok_or(FwdFault::NoVas(cid))?;
        let vas = proc
            .vases
            .get(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
        match vas.table.resolve(pdb, dst) {
            Ok((b, off)) => (b.phys().wrapping_add(off), b.aperture()),
            // ★ A virtual destination we cannot resolve is NOT a fault here, and that is
            // deliberate. The overwhelming majority of virtual-destination copies are
            // ordinary data writes into a user address space we never had to model — the
            // forward path, whose whole premise is that we do not track those addresses.
            // Faulting would turn "we are not intercepting this" into "the guest did
            // something wrong". What must never happen is *guessing* it into a page-table
            // write, and an unresolved destination cannot be classified as one: it has no
            // physical address to look up.
            Err(_) => return Ok(CeOperands::VaOperand { phys: 0 }),
        }
    } else {
        // A physical destination names the framebuffer directly.
        (dst.0, Aperture::Vidmem)
    };
    match spine.pt_page_owner(cgpu, phys) {
        Some((owner, owner_pdb)) => Ok(CeOperands::PhysOperand {
            page: phys & !0xfffu64,
            aperture,
            owner,
            owner_pdb,
        }),
        None => Ok(CeOperands::VaOperand { phys }),
    }
}

// =================================================================================
// ★★★ THE REPRESENTABILITY SPLIT (`eight_blockers_resolved.md` §12) — #102 stage C2.
//
// The owner's ruling, restated as the four things it decides:
//
//   1. We perform a copy ONLY where it is UNREPRESENTABLE by a real copy engine — an
//      operand landing in *fabricated* space that no real engine can be pointed at.
//   2. Everything representable goes to real hardware. That is normally FASTER than a
//      CPU memcpy, not merely more faithful.
//   3. A single request may SPLIT. Its representable sub-ranges are issued to real CE;
//      only the unrepresentable remainder is ours. ⇒ the operand ranges must be
//      PARTITIONED, not classified whole.
//   4. The executor is the ISOLATE in both cases (`VerbPlan::CeSplit`).
//
// ★★★ THE CRITERION IS A PROPERTY OF THE ADDRESS, NOT OF OUR KNOWLEDGE ABOUT ITS ROLE.
// That is what dissolves the orphan-leaf problem (§12.1(i)): a fresh page-table leaf in
// fabricated space is performed-by-us — and therefore its content is in our hands —
// BEFORE any PDE points at it, so it does not need to be recognised as a page table
// first. There is deliberately NO "is this a page table?" test here; re-introducing one
// is precisely the bug the ruling removes.
// =================================================================================

/// ★★★ **Where a copy-engine operand's address LIVES** — the §12 criterion, and it is a
/// property of the address alone.
///
/// The C's own version of this is `nvkvm_dp_classify_fb` (`C: nvkvm_gpu_emul.c:1013`),
/// which answers `1 = fbback`, `2 = gpga with a real object`, `0 = still a fake
/// fb_page`. It is the same question — *does real device memory exist behind this
/// address?* — asked over guest-framebuffer-physical addresses instead of over the
/// address table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Representability {
    /// ★ **Representable.** The range is host-published in the owning `Vas` — real host
    /// memory, mapped into that `Vas`'s own host VAS **at the identical address**
    /// (address identity, `#102` stage A). A real engine can be pointed at the guest's
    /// own number, because that is the number the host MMU walks for.
    ///
    /// ⚠ **AND the object must be the range's ONLY memory** —
    /// [`kayfabe_mmu::BackingBytes::SoleBacking`]. A host object that merely *exists* at
    /// the address is not enough: `PublishVidmem` used to put one at a VA whose bytes the
    /// guest goes on reading and writing through the emulated framebuffer, and aiming an
    /// engine at that is the owner's forbidden #2. ⊘ That is now enforced **at the bind**
    /// rather than here — [`kayfabe_mmu::Binding::real_gpu_memory`] refuses it, so this arm
    /// reads a kind that could only have been declared truthfully. See
    /// [`FwdFault::RegionKindRefused`].
    HostBacked,
    /// ★★★ **Fabricated.** The range is *declared* in the address table and nothing
    /// host-side exists behind it: it lives in the emulated framebuffer, which is memory
    /// we invented. A real engine pointed here resolves nothing — `Xid 31 FAULT_PDE`.
    /// This is where the guest's page tables live, and it is where a
    /// declared-but-unpublished range lives, and the split does not distinguish them
    /// *because it must not* (§12.1(i)).
    Fabricated,
    /// **A physical operand** — the command named a guest-framebuffer-physical address,
    /// so no GPU VA denotes it at all. Unrepresentable by construction rather than by
    /// lookup: *"a CE physical copy bypasses the MMU, so the page-table walk can NEVER
    /// discover this dst (no PTE)"* (`C: :6244`). The C agrees on the answer for a
    /// different-looking reason — `dst_phys` is a negated conjunct of its execute
    /// predicate (`C: :6310`), so a physical operand is never the host engine's there
    /// either.
    PhysicalOperand,
    /// **Not tracked.** No binding covers the range. Forwarded — never guessed into a
    /// capture and never claimed as ours (MISS = FAULT is about *resolving*, and we are
    /// not resolving it; the overwhelming majority of these are ordinary data in a user
    /// address space we never had to model).
    ///
    /// ★ The safety net for this arm is **not** here: it is the #14 ring gate
    /// ([`VerbPlan::gated_doorbell`]), which refuses to ring a channel whose working set
    /// is not host-published. So "forward it" cannot degrade into "hardware dereferences
    /// something that was never mapped" — that submission does not reach a doorbell.
    ///
    /// # ★★★★★ AN ABSENT TABLE ROW IS NOT NEUTRAL — it routes guest work to the host GPU
    ///
    /// ⚠ Read the arm above and this one together, because the pairing is counter-intuitive
    /// and was measured rather than reasoned. A range **we know nothing about** lands here
    /// and goes to `HostCe`; the *same* range, once a binding exists for it with no host
    /// object, becomes [`Representability::Fabricated`] and goes to `CeExecutor::Ours`.
    ///
    /// ⇒ **Populating the address table moves work OFF the hardware arm, not onto it.**
    /// `[measured 2026-08-11, boots w234a/w234b]` the user proc's framebuffer ranges had no
    /// binding at all — this arm — until the executor-write witness
    /// (`KAYFABE_PT_WITNESS_EXEC`) gave them one; arming it took them to `Fabricated`, i.e.
    /// to the CPU executor addressing the emulated framebuffer the guest actually reads.
    ///
    /// ★ So *"the table is incomplete"* is never merely a missing diagnostic. Every VA the
    /// table does not bind is a VA this classifier will hand to a real engine on the
    /// strength of knowing nothing about it, and the only thing standing behind that is the
    /// #14 gate. That is the owner's **STEP 1 residency before STEP 2 executor**
    /// (`ce_executor_tree.md`) stated from the other side: an unanswerable residency
    /// question does not defer the executor choice, it *makes* it.
    Untracked,
}

impl Representability {
    /// Which engine may be pointed at an address of this kind.
    ///
    /// ★ Note what is NOT consulted: the work kind, and who submitted it. Those are
    /// [`ce_executor_c`]'s inputs — the C's predicate, kept as the baseline this is
    /// measured against — and replacing them with the address is the whole content of
    /// §12's ruling.
    #[must_use]
    pub fn executor(self) -> CeExecutor {
        match self {
            Representability::HostBacked | Representability::Untracked => CeExecutor::HostCe,
            Representability::Fabricated | Representability::PhysicalOperand => CeExecutor::Ours,
        }
    }
}

/// One sub-range of a partitioned copy-engine request: the instruction, plus the
/// evidence that chose its engine.
///
/// The evidence rides along rather than being recomputed, because "why did this range go
/// to that engine" is exactly what a test must be able to assert without re-implementing
/// the classifier and thereby asserting nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeSpan {
    /// The instruction handed to the isolate.
    pub sub: CeSubCopy,
    /// Where the destination address lives.
    pub dst_kind: Representability,
    /// Where the source address lives — `None` for a scrub or a fill, which have no
    /// source operand (`C: :6320` "No src is set.").
    pub src_kind: Option<Representability>,
    /// ★★★ **E10b/E10e — the destination operand AS THE CPU REACHES IT**: the plane its
    /// bytes live in, who guarantees they stay, and — since §14.14 — **the address they are
    /// at in that plane**. `Some` exactly when the destination is CPU-resident (the emulated
    /// framebuffer or guest RAM), `None` when it is real device memory the isolate reaches or
    /// untracked. The shell CPU executor reads it to choose which store to write *and where*;
    /// a `by == CeExecutor::Ours` sub-copy whose destination place is `None` is a straddle no
    /// single executor can run and the executor refuses it rather than guessing a store.
    ///
    /// ⊘⊘ **This is NOT [`CeSubCopy::dst`], and the difference is the whole of §14.14's
    /// REFUTED 4.** `sub.dst` stays a **GPU virtual address**, because the `HostCe` arm
    /// submits it to a host VAS where the host MMU walks for exactly that number. This is
    /// where the same byte lives in *our* store. For a physical operand the two coincide —
    /// which is why a whole test file could exercise the executor and never see that it wrote
    /// at the wrong one. [`kayfabe_arch::PlaneAddr`] is a distinct type so the mistake cannot
    /// be made again without failing to compile.
    pub dst_place: Option<CpuOperand>,
    /// ★★★ **E10b/E10e — the source operand as the CPU reaches it.** `None` for a scrub/fill
    /// (no source operand), for a real-device-memory source, or an untracked one. Distinct
    /// from [`Self::dst_place`] because the two ends can differ — `memmgrTestCeUtils`' `sys ←
    /// vid` copy reads the framebuffer and writes guest RAM in one instruction, at two
    /// unrelated addresses.
    pub src_place: Option<CpuOperand>,
}

/// Upper bound on the sub-copies ONE copy-engine request may partition into.
///
/// A guest controls both the request's length and its address table's fragmentation, so
/// the span count is guest-influenced and needs a bound (boundary-1). Exceeding it is a
/// **loud refusal** ([`FwdFault::CeTooFragmented`]) and never a truncation: a partition
/// that stops early is a partition that silently drops the tail of a copy, which is the
/// C's own `#13 CE-DROP` failure mode (`C: :6389`) reintroduced on purpose.
pub const MAX_CE_SPANS: usize = 4096;

/// Upper bound on the sub-copies ONE `parse_pushbuffer` call may produce across ALL of a
/// ring's copy-engine requests.
///
/// [`MAX_CE_SPANS`] bounds any single request; a hostile ring can declare *many*
/// maximally fragmented ones, and the same reasoning that gave the parser
/// `MAX_PUSH_TOTAL_BYTES` beside `MAX_PUSH_RANGE_BYTES` applies here. Loud, never a
/// truncation, for the same reason.
pub const MAX_CE_SPANS_PER_PARSE: usize = 1 << 16;

/// ★★★ **THE residency oracle this port has today** — the static mapping from what the
/// guest *declared* about an operand to where its bytes are.
///
/// # ⊘ What it can and cannot answer, said plainly
///
/// It reads the guest's own declaration and nothing else: a `_TARGET` beside a physical
/// operand (`clc7b5.h:66-80`), or the aperture of the leaf a resolved binding came from. It
/// **cannot** discover that an address is really in host device memory, and it **cannot**
/// answer for managed memory at all — where those bytes are is a fact the host driver owns
/// and may change (`mode2_uvm_residency.md`). Both of those arrive as *different
/// implementations of [`ResidencyOracle`]*, which is the whole reason the query is a trait
/// with one implementation rather than two free functions
/// (owner directive, 2026-08-08 — `execution_plane_increments.md` §14).
///
/// ★ **A unit struct, deliberately.** It holds no state because a static declaration-to-
/// place mapping *has* none; an implementation that needed state would be a different one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeclaredResidency;

impl ResidencyOracle for DeclaredResidency {
    /// `_LOCAL_FB` → the emulated framebuffer, `_{COHERENT,NONCOHERENT}_SYSMEM` → guest
    /// RAM, `_PEERMEM` → **not ours**: a second physical GPU's framebuffer, which this
    /// device does not back.
    ///
    /// ⊘ `addr` is unread **by this implementation** and that is a statement about this
    /// implementation, not about the seam: an oracle that tracked real residency would look
    /// it up, which is why the parameter is on the trait.
    fn residency_of_physical(&self, target: PhysTarget, _addr: u64) -> Option<Residency> {
        match target {
            PhysTarget::LocalFb => Some(Residency::stable(CpuPlane::Fb)),
            PhysTarget::CoherentSysmem | PhysTarget::NonCoherentSysmem => {
                Some(Residency::stable(CpuPlane::GuestRam))
            }
            PhysTarget::Peer => None,
        }
    }

    /// The virtual-operand analogue: a *declared-but-unpublished* binding (`host = None`)
    /// lives in the store its aperture names — a `Vidmem` binding is a page of the emulated
    /// framebuffer (`#13`'s page tables live there), a sysmem binding is guest RAM
    /// (`Binding::phys` for sysmem *is* a guest-physical address).
    ///
    /// `[measured 2026-08-08, boot `run_p35_84d857d`]` the CeUtils channel that walls has
    /// its ring, pushbuffer and finishPayload all under the sysmem arm and its page
    /// directories under the vidmem one, so both are live on the path that matters.
    fn residency_of_aperture(&self, aperture: Aperture, _addr: u64) -> Option<Residency> {
        match aperture {
            Aperture::Vidmem => Some(Residency::stable(CpuPlane::Fb)),
            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => {
                Some(Residency::stable(CpuPlane::GuestRam))
            }
            Aperture::Peer => None,
        }
    }
}

/// Ask [`DeclaredResidency`] about a physical operand; **no CPU plane** becomes this crate's
/// named peer refusal.
///
/// ⊘ The refusal is raised HERE and not inside the oracle: an oracle's job is to answer, and
/// *"this answer means the caller must stop"* is the caller's policy. That split is what lets
/// a later oracle report a [`kayfabe_arch::Backing::HostOwned`] answer without inventing a
/// fault, and lets the *executor* decide what to do about it.
fn residency_of_target(target: PhysTarget, addr: u64) -> Result<Residency, FwdFault> {
    DeclaredResidency
        .residency_of_physical(target, addr)
        .ok_or(FwdFault::CePeerOperand { addr })
}

/// Ask [`DeclaredResidency`] about a resolved binding's aperture. See
/// [`residency_of_target`].
fn residency_of_aperture(aperture: Aperture, addr: u64) -> Result<Residency, FwdFault> {
    DeclaredResidency
        .residency_of_aperture(aperture, addr)
        .ok_or(FwdFault::CePeerOperand { addr })
}

/// Classify ONE already-resolved sub-range of an operand — its representability, and (when
/// it is ours to execute) the CPU plane its bytes live in.
///
/// ★ E10b carries the plane beside the representability rather than deriving it later,
/// because the two `Ours` representabilities live in *different* stores (a fabricated
/// vidmem page in the framebuffer, a fabricated sysmem page in guest RAM) and the shell
/// executor must know which to touch. `HostBacked`/`Untracked` are not ours to run, so they
/// carry no plane.
///
/// ★★★ **E10e — and the ADDRESS in that plane is computed here too, from the binding.**
/// `binding.phys + within` is the only place both halves are in hand; the run's start is a
/// **virtual** address and the byte is not there. Deriving it downstream is what
/// §14.14 REFUTED 4 measured going wrong (`#12`'s where-mistake), and the two are different
/// types now so the derivation cannot be skipped silently.
///
/// `within` is the run's offset **inside** the binding, as [`AddressTable::spans`] reports
/// it — not the run's offset into the request. A copy that starts halfway through a mapping
/// resolves to halfway through its backing.
fn representability_of(
    binding: Option<&Binding>,
    within: u64,
    addr: u64,
) -> Result<(Representability, Option<CpuOperand>), FwdFault> {
    match binding {
        // ★★★★★ **THE KIND IS READ, NOT DERIVED.**
        //
        // ⊘ This match used to be the tree's ONLY answer to *"what kind of region is this"*,
        // and two of its arms were **unguarded fall-throughs pointing in opposite
        // directions**: `Some(b)` with no host object meant `Fabricated` (our CPU executor)
        // and `None` meant `Untracked` (**the real host GPU**). Neither was a decision
        // anyone took. `[measured 2026-08-11, gpga_region_kind.md §1.1]`
        //
        // ⇒ The kind is now decided at [`kayfabe_mmu::AddressTable::bind`], by the site that
        // knows, and this function **reads** it. Everything the old arms encoded survives as
        // a property of `RegionKind`, and the two states they encoded WRONGLY —
        // a `Peer` binding classified as fiction, and a shadowing backing classified as
        // pointable — are now unconstructible (`RegionKindFault`).
        Some(b) => match b.kind() {
            // ★ Kind 3. A host object, mapped at the identical address in the owning
            // `Vas`'s own host VAS, and it is the range's ONLY memory —
            // `kayfabe_mmu::Binding::real_gpu_memory` refuses to be built any other way. A
            // real engine may be pointed at the guest's own number.
            //
            // ⊘ **Forbidden #2 has not been dropped; it MOVED to the entrance.** This arm
            // used to be `b.host.is_some_and(is_sole_backing)`, with a second arm raising
            // [`FwdFault::BackingNotGuestVisible`] for a shadowing backing. The shadow can
            // no longer enter the address table at all — ruling 3, enforced at the
            // constructor — so the refusal happens before the host object is adopted rather
            // than after every reader has seen a published range.
            // `ce_executor_tree.md` (owner, 2026-08-07): *"Forbidden … 2. Landing the data
            // where the guest cannot see it."*
            kayfabe_mmu::RegionKind::RealGpuMemory => Ok((Representability::HostBacked, None)),
            // ★ Kinds 2 and 4 — the emulated framebuffer, and the guest's own physical
            // pages. Both are ours to execute and they live in **different stores**, which
            // is what the `CpuOperand`'s residency carries: `Vidmem` → `CpuPlane::Fb`,
            // sysmem → `CpuPlane::GuestRam`.
            //
            // ⊘ They are ONE arm here and TWO kinds in the table deliberately. The executor
            // choice does not distinguish them; the owner's taxonomy does, because kind 2 is
            // memory we invented and kind 4 is the guest's own — and *that* is the
            // distinction ruling 2 scopes (*"fake framebuffer exists ONLY for guest-KERNEL
            // channels we emulate"*). Collapsing them in the table is what made the question
            // unanswerable.
            kayfabe_mmu::RegionKind::FakeFramebuffer | kayfabe_mmu::RegionKind::GuestPhysDma => {
                let residency = residency_of_aperture(b.aperture(), addr)?;
                Ok((
                    Representability::Fabricated,
                    Some(CpuOperand {
                        residency,
                        addr: PlaneAddr(b.phys()).offset(within),
                    }),
                ))
            }
        },
        // ★★★ **Kind 1 — unallocated, i.e. NO ROW.** ⚠ Not neutral: it routes to the host
        // GPU. See [`Representability::Untracked`], and [`kayfabe_mmu::RegionKind`] for why
        // this is the absence of a row rather than a fourth variant.
        None => Ok((Representability::Untracked, None)),
    }
}

/// One resolved run of an operand's range: `(start, len, representability, cpu-operand)` —
/// the operand present exactly when the run's `representability.executor() ==
/// CeExecutor::Ours`. `start` is the run's address **in the operand's own space** (a VA for a
/// virtual operand); the plane address is inside the [`CpuOperand`].
pub type OperandRun = (u64, u64, Representability, Option<CpuOperand>);

/// ★★★ **THE OPERAND-RESOLVER SEAM** — how a *virtual* copy-engine operand becomes a place
/// (`execution_plane_increments.md` §14.15 obstacle 2; owner's ruling, 2026-08-08).
///
/// # Why this exists at all
///
/// [`partition_ce`] and `kayfabe_rt::cpu_ce::write_completion` both used to take an
/// [`AddressTable`] outright. That is the right answer for every channel that *has* a
/// `Vas` — and it is not available for the one channel the CE branch exists to serve: the
/// GSP-managed CeUtils channel walls `NoVas(ChanId(1))`, and its only route to an address
/// is `kayfabe_device::ceresolve`'s walk from the page-directory root the guest published
/// (§14.13, where all three of its addresses resolved).
///
/// ## ⊘ Why this and NOT a TLB fill at the demand
///
/// The alternative was to run the walk and *write its answers into an `AddressTable`*, so
/// the existing consumers were unchanged. **A table filled from a walk is a cache of the
/// walk**, and `gmmu_publication_discipline.md` §7 rule 6 is *"never cache the walk — the
/// result is valid for this fault only"*. Rule 7 (*"serialise against the observed
/// invalidate"*) is what would have bounded such a cache, and it is **vacuous on this
/// path**: §5 measured **both** invalidate transports at zero, so there is no event to
/// invalidate a cache with. A cache with no invalidation is stale for the life of the
/// device. The seam keeps every resolution on the guest's own demand, which is §6.1's
/// discipline unchanged and needs no new argument.
///
/// # ⊘ It answers for VIRTUAL operands only
///
/// A *physical*-mode operand bypassed the MMU by construction, so there is nothing to
/// resolve and [`partition_ce`] answers it without consulting a resolver at all. Handing a
/// physical address to a resolver would be asking the MMU about an address that never went
/// through it.
pub trait OperandResolver {
    /// Partition the **virtual** range `[addr, addr+len)` into the maximal runs over which
    /// this resolver's answer is constant, in ascending order, covering the effective range
    /// exactly (a wrapping range is clipped, never wrapped — see [`AddressTable::spans`]).
    ///
    /// # Errors
    /// [`FwdFault`], by variant. An implementation that resolves by *walking* faults by
    /// name at the level it failed; one that resolves by *table lookup* reports an
    /// uncovered run as [`Representability::Untracked`], which is a hole rather than a
    /// fault because an untracked VA is forwardable.
    fn resolve_runs(&mut self, addr: GpuVa, len: u64) -> Result<Vec<OperandRun>, FwdFault>;

    /// Resolve ONE aligned word's worth of virtual address — the completion-semaphore
    /// query, which is a point query and not a range one.
    ///
    /// ⊘ Deliberately **not** a default over [`OperandResolver::resolve_runs`]: the two
    /// have different refusal postures. A hole is a legitimate answer to a range query and
    /// is never a legitimate answer here — a completion written at an address that
    /// resolved to nothing is `#12`'s where-mistake — so each implementation states its own
    /// refusal instead of inheriting a lossy one.
    ///
    /// # Errors
    /// [`FwdFault`], by variant: the implementation's own miss/fault, or
    /// [`FwdFault::CePeerOperand`] for a peer aperture.
    fn resolve_word(&mut self, addr: GpuVa) -> Result<CpuOperand, FwdFault>;
}

/// ★★★ The [`OperandResolver`] this port has had all along: **one channel's own
/// [`AddressTable`]**, or the honest absence of one.
///
/// ⊘ `Untracked` is a variant rather than an `Option<&AddressTable>` field because the two
/// arms answer differently *and* refuse differently, and a `None` inside one arm reads as a
/// missing value rather than as a decision.
#[derive(Debug)]
pub enum TableOperands<'a> {
    /// The channel has a `Vas` and this is its table, keyed by the PDB a miss is reported
    /// against.
    Table {
        /// The address table to resolve in.
        table: &'a AddressTable,
        /// The PDB, carried so a miss names the address space it missed in.
        pdb: Pdb,
    },
    /// The channel has no address table at all — nothing is tracked, so nothing is ours.
    Untracked,
}

impl<'a> TableOperands<'a> {
    /// The resolver for a channel whose `Vas` was found (or not).
    #[must_use]
    pub fn new(table: Option<&'a AddressTable>, pdb: Option<Pdb>) -> TableOperands<'a> {
        match (table, pdb) {
            (Some(table), Some(pdb)) => TableOperands::Table { table, pdb },
            _ => TableOperands::Untracked,
        }
    }
}

impl OperandResolver for TableOperands<'_> {
    fn resolve_runs(&mut self, addr: GpuVa, len: u64) -> Result<Vec<OperandRun>, FwdFault> {
        match self {
            TableOperands::Table { table, .. } => table
                .spans(addr, len)
                .into_iter()
                .map(|(s, l, b)| {
                    let (kind, place) = representability_of(
                        b.as_ref().map(|(x, _)| x),
                        b.map_or(0, |(_, o)| o),
                        s,
                    )?;
                    Ok((s, l, kind, place))
                })
                .collect(),
            // No table for this channel's VAS at all: nothing is tracked, so nothing is
            // ours. The same clipping rule the table's own range query uses.
            TableOperands::Untracked => {
                let end = (u128::from(addr.0) + u128::from(len)).min(1u128 << 64);
                let eff = (end - u128::from(addr.0)) as u64;
                if eff == 0 {
                    Ok(Vec::new())
                } else {
                    Ok(vec![(addr.0, eff, Representability::Untracked, None)])
                }
            }
        }
    }

    fn resolve_word(&mut self, addr: GpuVa) -> Result<CpuOperand, FwdFault> {
        match self {
            TableOperands::Table { table, pdb } => {
                let (binding, off) = table.resolve(*pdb, addr).map_err(FwdFault::Address)?;
                let phys = PlaneAddr(binding.phys()).offset(off);
                let residency = residency_of_aperture(binding.aperture(), phys.0)?;
                Ok(CpuOperand {
                    residency,
                    addr: phys,
                })
            }
            TableOperands::Untracked => Err(FwdFault::CeNoTable { va: addr }),
        }
    }
}

/// The partition of one operand's range, as [`OperandRun`]s — the `plane`
/// present exactly when `kind.executor() == CeExecutor::Ours`.
///
/// A physical operand is ONE run of [`Representability::PhysicalOperand`]: there is
/// nothing to look up, and no sub-range of it could be anything else. ★ Its plane comes
/// from the operand's `_TARGET` ([`PhysTarget`]) — the E10b residency signal — so a
/// `_LOCAL_FB` physical copy and a `_SYSMEM` physical copy are told apart even when their
/// addresses collide numerically.
///
/// # Errors
/// [`FwdFault::CePeerOperand`] if any run resolves into peer memory, plus whatever the
/// resolver's own virtual arm refuses with.
fn operand_runs(
    ops: &mut dyn OperandResolver,
    addr: GpuVa,
    is_virtual: bool,
    target: PhysTarget,
    len: u64,
) -> Result<Vec<OperandRun>, FwdFault> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if !is_virtual {
        // ★ A physical operand's address IS its plane address — the command bypassed the
        // MMU, so there is nothing to resolve. ⊘ This coincidence is exactly what hid the
        // §14.14 defect: every executor test used this arm.
        let residency = residency_of_target(target, addr.0)?;
        return Ok(vec![(
            addr.0,
            len,
            Representability::PhysicalOperand,
            Some(CpuOperand {
                residency,
                addr: PlaneAddr(addr.0),
            }),
        )]);
    }
    ops.resolve_runs(addr, len)
}

/// Do two adjacent sub-copies' places join up — same store, same ownership, and the second's
/// bytes starting exactly `prev_len` after the first's?
///
/// ★★★ **This is a correctness condition the merge did not previously have to state**, and
/// carrying the plane address is what exposed it. Before §14.14 the merge compared
/// [`Residency`] alone, so two spans landing in *different, non-adjacent* bindings that
/// happened to share an aperture merged into ONE instruction — harmless while the executor
/// used the VA (the host MMU re-walked each page) and a **write past the end of the first
/// backing** the moment it uses the plane address. Contiguity, not equality.
fn plane_follows(prev: Option<CpuOperand>, next: Option<CpuOperand>, prev_len: u64) -> bool {
    match (prev, next) {
        (None, None) => true,
        (Some(a), Some(b)) => a.residency == b.residency && a.addr.offset(prev_len) == b.addr,
        _ => false,
    }
}

/// ★★★ **THE RANGE ALGEBRA** (§12.3): partition one copy-engine request into the
/// maximal sub-copies over which BOTH operands' representability is constant.
///
/// A copy has two ends, and a sub-copy may go to real hardware only if **both** of them
/// can be expressed to it. So the destination's partition and the source's partition are
/// **intersected** — at their common offsets, not at their addresses, because the two
/// operands sit at different addresses and advance together.
///
/// Guarantees, all pinned by the property test:
/// - the sub-copies are contiguous, ordered and non-overlapping;
/// - they cover the effective range **exactly** — a partition that is not total is a
///   silently dropped copy;
/// - none is zero-length;
/// - `dst`/`src` of the `i`-th sub-copy are the original operands advanced by the same
///   offset, which is what makes partition-then-execute byte-identical to
///   execute-whole.
///
/// A wrapping `addr + len` is CLIPPED, never wrapped (see [`AddressTable::spans`]); the
/// destination governs the effective length, and the source is clipped to match, so a
/// copy is never issued reading past where its destination stops.
///
/// ★ E10b threads each operand's `_TARGET` ([`PhysTarget`], meaningful only for a physical
/// operand) so a `by == CeExecutor::Ours` sub-copy carries the CPU plane each end lives in
/// — the shell executor's "which store" answer, computed here where the residency signal is.
///
/// # Errors
/// [`FwdFault::CeTooFragmented`] if the partition exceeds [`MAX_CE_SPANS`];
/// [`FwdFault::CePeerOperand`] if an operand resolves into peer memory.
#[allow(clippy::too_many_arguments)]
pub fn partition_ce(
    ops: &mut dyn OperandResolver,
    dst: GpuVa,
    dst_is_virtual: bool,
    dst_target: PhysTarget,
    src: GpuVa,
    src_is_virtual: bool,
    src_target: PhysTarget,
    len: u64,
    work: kayfabe_arch::CeWork,
) -> Result<Vec<CeSpan>, FwdFault> {
    let dst_runs = operand_runs(ops, dst, dst_is_virtual, dst_target, len)?;
    // The destination decides how much of the request exists at all (a clipped
    // destination clips the whole copy).
    let eff: u64 = dst_runs.iter().map(|(_, l, _, _)| *l).sum();
    if eff == 0 {
        return Ok(Vec::new());
    }
    // A scrub or a fill has NO source operand, so there is no second partition to
    // intersect — its representability is a property of its destination alone.
    let has_src = matches!(work, kayfabe_arch::CeWork::Copy);
    let src_runs = if has_src {
        operand_runs(ops, src, src_is_virtual, src_target, eff)?
    } else {
        Vec::new()
    };

    // Walk both partitions by OFFSET into the request, cutting at every boundary either
    // one introduces. `src_runs` may be shorter than `eff` only if the source range was
    // clipped at the top of the address space; the remainder then has no source, which
    // is a source that reads nothing — modelled as untracked (forwardable), never as a
    // silent shortening of the destination.
    let mut out: Vec<CeSpan> = Vec::new();
    let mut off: u64 = 0;
    let mut di = 0usize;
    let mut d_consumed: u64 = 0;
    let mut si = 0usize;
    let mut s_consumed: u64 = 0;
    while off < eff {
        let (_, d_len, d_kind, d_run_place) = dst_runs[di];
        let d_left = d_len - d_consumed;
        // ★★★ The plane address advances with the cut, exactly as `sub.dst` does — a
        // sub-copy starting `d_consumed` bytes into its run reaches bytes `d_consumed`
        // further into the backing. ⊘ Reusing the run's own address for every cut of it
        // would rewrite the first page of a mapping once per span.
        let d_place = d_run_place.map(|p| CpuOperand {
            addr: p.addr.offset(d_consumed),
            ..p
        });
        let (s_left, s_kind, s_place) = if !has_src {
            (u64::MAX, None, None)
        } else if si < src_runs.len() {
            let (_, s_len, s_kind, s_run_place) = src_runs[si];
            (
                s_len - s_consumed,
                Some(s_kind),
                s_run_place.map(|p| CpuOperand {
                    addr: p.addr.offset(s_consumed),
                    ..p
                }),
            )
        } else {
            (eff - off, Some(Representability::Untracked), None)
        };
        let take = d_left.min(s_left).min(eff - off);
        debug_assert!(take > 0, "a partition step must consume bytes");
        if out.len() == MAX_CE_SPANS {
            return Err(FwdFault::CeTooFragmented { dst, len });
        }
        let by = match s_kind {
            // Both ends must be expressible for hardware to run it. Combining by "the
            // stricter answer wins" rather than by a rule per operand: an unrepresentable
            // SOURCE is just as fatal to a real engine as an unrepresentable destination,
            // and the C says the same thing with `!src_phys && !dst_phys` (`C: :6310`).
            Some(s) => match (d_kind.executor(), s.executor()) {
                (CeExecutor::HostCe, CeExecutor::HostCe) => CeExecutor::HostCe,
                _ => CeExecutor::Ours,
            },
            None => d_kind.executor(),
        };
        out.push(CeSpan {
            sub: CeSubCopy {
                dst: dst.0.wrapping_add(off),
                src: match work {
                    kayfabe_arch::CeWork::Copy => CeSource::Address(src.0.wrapping_add(off)),
                    // A scrub zeroes; a fill writes its pattern. The C's scrub arm is a
                    // no-op only because ITS backing is sparse-zero — stating it as an
                    // explicit zero fill keeps the meaning where the backing cannot
                    // supply it.
                    kayfabe_arch::CeWork::Scrub => CeSource::Constant(0),
                    kayfabe_arch::CeWork::Fill { pattern } => CeSource::Constant(pattern),
                },
                // ⊘ **`None` here, ALWAYS, and never at this layer.** `partition_ce` splits
                // ONE launch into sub-copies and cannot know which of them is last — the
                // caller does, and attaching the release to any but the last would let the
                // guest's payload land before the guest's bytes. See the `completion` arm in
                // `parse_pushbuffer_inner`, which is the only writer of this field.
                guest_release: None,
                len: take,
                by,
            },
            dst_kind: d_kind,
            src_kind: s_kind,
            // ★ E10b/E10e: each operand's own plane **and its address in that plane** ride
            // along, so the shell knows which store to touch and where. A scrub/fill or a
            // HostCe/untracked end carries `None`.
            dst_place: d_place,
            src_place: s_place,
        });
        off += take;
        d_consumed += take;
        if d_consumed == d_len {
            di += 1;
            d_consumed = 0;
        }
        if has_src && si < src_runs.len() {
            s_consumed += take;
            if s_consumed == src_runs[si].1 {
                si += 1;
                s_consumed = 0;
            }
        }
    }
    // Adjacent sub-copies that ended up on the SAME engine are merged, so a boundary
    // that both partitions happen to agree across does not become two instructions. The
    // evidence is kept from the first of the run.
    let mut merged: Vec<CeSpan> = Vec::with_capacity(out.len());
    for s in out {
        match merged.last_mut() {
            Some(prev)
                if prev.sub.by == s.sub.by
                    && prev.dst_kind == s.dst_kind
                    && prev.src_kind == s.src_kind
                    // ★★★ The places must be CONTIGUOUS to merge, not equal. Two adjacent
                    // spans of one binding never have the same plane address — the second
                    // starts where the first ended — so an equality test here would silently
                    // stop merging, and (worse) a test that ignored the place entirely would
                    // merge two spans whose backings are NOT adjacent into one instruction
                    // that runs off the end of the first.
                    && plane_follows(prev.dst_place, s.dst_place, prev.sub.len)
                    && plane_follows(prev.src_place, s.src_place, prev.sub.len)
                    && prev.sub.dst.wrapping_add(prev.sub.len) == s.sub.dst =>
            {
                prev.sub.len += s.sub.len;
            }
            _ => merged.push(s),
        }
    }
    Ok(merged)
}

/// ★★★ Build the ISOLATE's instruction for a partitioned request (§12.4 — *"the
/// executor is the isolate in both cases"*).
///
/// The core decides *what*; the isolate holds bytes and does *it*. There is deliberately
/// no path by which the pure core moves a byte: this returns a plan, and a plan is
/// executed on a checked-out worker with **no lock held** (R1).
///
/// An empty partition yields no plan: a request that covers nothing is not a verb.
#[must_use]
pub fn plan_ce_split(host_vas: HostHandle, spans: &[CeSpan]) -> Option<VerbPlan> {
    if spans.is_empty() {
        return None;
    }
    Some(VerbPlan::CeSplit {
        vas: host_vas,
        subs: spans.iter().map(|s| s.sub).collect(),
    })
}

// =====================================================================================
// ★★★ E6 — THE JOIN. The three phases that carry a partitioned copy-engine request from
// the core to a real engine (`execution_plane_increments.md`, the E6 row).
// =====================================================================================

/// What one forwarded copy-engine request did — the value [`commit_ce`] reports.
///
/// ★ The two executor counts are carried **separately** rather than summed, because the
/// question a caller has is never "how many sub-copies" but *"did any byte of this reach a
/// real engine"*. [`kayfabe_isolate::CeExecutor`] is deliberately not a `bool` for the
/// same reason, one layer down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeForwarded {
    /// The proc whose channel submitted.
    pub proc: ProcId,
    /// The channel.
    pub chan: ChanId,
    /// Sub-copies a **real copy engine** ran ([`kayfabe_isolate::CeExecutor::HostCe`]).
    pub host_ce: usize,
    /// Sub-copies the **isolate** ran itself ([`kayfabe_isolate::CeExecutor::Ours`]).
    pub ours: usize,
}

impl CeForwarded {
    /// Did any sub-copy reach a real copy engine?
    ///
    /// ⊘ **Not an acceptance predicate.** It says the plan chose hardware and the verb
    /// returned `Ok`, which is a fact about *us*. Whether bytes moved is
    /// `kayfabe_isolate_host::rm::CeEvidence::copied()`'s question, and it is answered by
    /// reading the destination — never by counting our own decisions.
    #[must_use]
    pub fn reached_hardware(&self) -> bool {
        self.host_ce > 0
    }
}

/// The ID-shaped hints [`commit_ce`] re-validates against (R5). Identities only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CePlan {
    /// The submitting proc.
    pub proc: ProcId,
    /// The submitting channel.
    pub chan: ChanId,
    /// The channel's OWN target GPU — its isolate/arena key, never the doorbell's.
    pub gpu: GpuId,
    /// The channel's declared VAS, when it has one. `None` only for the empty
    /// partition, which needs no address space because it names no address.
    pub pdb: Option<Pdb>,
    /// The host VAS the sub-copies' addresses live in, as observed at plan time.
    pub host_vas: Option<HostHandle>,
    /// How many sub-copies the plan carried (so a reply that partitioned differently is
    /// visible rather than silently adopted).
    pub subs: usize,
}

/// PLAN (R1) for a partitioned copy-engine request. A pure `&Proc` read; nothing is
/// mutated until the commit, and no host verb exists until this has passed.
///
/// `spans` is what [`partition_ce`] produced for one guest `LAUNCH_DMA` — or what
/// [`apply_pushbuffer`] accumulated across a whole ring, in submission order. It is
/// **not** re-derived here: re-partitioning would be a second, competing answer to a
/// question `apply_pushbuffer` already answered against the same table.
///
/// ## The three refusals, and why each is a refusal rather than a repair
///
/// - [`FwdFault::UnknownChannel`] — nothing to submit *on*.
/// - [`FwdFault::NoTarget`] — the `(proc, gpu)` isolate does not exist, so there is no
///   executor. Checked **before** any host op, like [`plan_publish`]'s.
/// - [`FwdFault::NoHostVas`] — the channel's `Vas` has never been host-published, so the
///   addresses in `spans` denote nothing in any host address space. Materializing an
///   empty host VAS here would turn a refusal into `Xid 31 FAULT_PDE`.
///
/// ## An empty partition issues NO verb and checks out NO worker
///
/// [`plan_ce_split`]'s *"a request that covers nothing is not a verb"*, carried up: the
/// plan's `verbs` is `None`, `Staged::check_out` takes no worker, and the commit runs
/// straight through. That is the ordinary case for a ring that carried no copy at all,
/// and it must not cost a pool slot.
///
/// # Errors
/// [`FwdFault`], by variant, as above — plus [`FwdFault::RetiredProc`].
pub fn plan_ce(proc: &Proc, cid: ChanId, spans: &[CeSpan]) -> Result<Planned<CePlan>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    let chan: &Channel = proc.channels.get(&cid).ok_or(FwdFault::UnknownChannel {
        proc: pid,
        chan: cid,
    })?;
    let cgpu = chan.gpu;
    // An empty partition names no address, so it needs neither an executor nor an
    // address space — and asking for either would refuse a submission that is legal.
    if spans.is_empty() {
        return Ok(Planned {
            plan: CePlan {
                proc: pid,
                chan: cid,
                gpu: cgpu,
                pdb: chan.vas_pdb,
                host_vas: None,
                subs: 0,
            },
            verbs: None,
        });
    }
    // ★★★ R1's spawn deferral — see [`missing_isolate`].
    if !proc.isolates.contains_key(&cgpu) {
        return Err(missing_isolate(proc, cgpu));
    }
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let vas = proc
        .vases
        .get(&(cgpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
    let host_vas = vas.host_vas.ok_or(FwdFault::NoHostVas { chan: cid, pdb })?;
    let verbs = plan_ce_split(host_vas, spans);
    debug_assert!(verbs.is_some(), "a non-empty partition is a verb");
    Ok(Planned {
        plan: CePlan {
            proc: pid,
            chan: cid,
            gpu: cgpu,
            pdb: Some(pdb),
            host_vas: Some(host_vas),
            subs: spans.len(),
        },
        verbs,
    })
}

/// COMMIT (R5) for a partitioned copy-engine request: re-resolve the proc, the channel
/// and the `Vas` through their IDs and report what ran.
///
/// ## ★★ It adopts NOTHING, and that is a statement rather than an omission
///
/// Every other commit in this file exists to take host handles the execute phase minted
/// and put them into core state. [`VerbPlan::CeSplit`] mints none: it moves bytes into
/// memory the guest already owns, through a host VAS the `Vas` already held. So this
/// phase has no adoption to perform, no orphans to dispose of, and nothing to leak.
///
/// What it still owes is **attribution**. The bytes have already moved by the time this
/// runs — a post-hoc refusal cannot unmove them — so the guard's job is not to undo but
/// to refuse *reporting the result against a world that no longer exists*: a caller told
/// "channel C copied" about a channel torn down mid-flight would file the fact on
/// whatever inherits that slot. That is the same aliasing class `#102`'s latch skips a
/// retired owner for.
///
/// ⊘ The `host_vas` re-check is therefore an **equality**, not a re-read-and-adopt: a
/// `Vas`'s host VAS is write-once (`commit_publish` refuses [`Stale::Rebound`] when one
/// is already set), so a differing value means the `Vas` itself was replaced, and the
/// copy ran against an address space that is not this one's.
///
/// # Panics
/// If `reply` is not the [`VerbReply::CeSplit`] its plan asked for.
///
/// # Errors
/// [`Refusal`] carrying a [`Stale`] variant. Never retryable: re-running a copy that
/// already executed would perform it **twice**, and a copy is not idempotent.
pub fn commit_ce(
    proc: &mut Proc,
    plan: &CePlan,
    reply: Option<VerbReply>,
) -> Result<CeForwarded, Refusal> {
    let (host_ce, ours) = match reply {
        None => {
            // The no-verb arm: an empty partition. Nothing ran, and nothing may claim to.
            debug_assert_eq!(plan.subs, 0, "only an empty partition issues no verb");
            (0, 0)
        }
        Some(VerbReply::CeSplit { host_ce, ours }) => (host_ce, ours),
        Some(_) => return wrong_reply("ce split"),
    };
    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Proc(plan.proc))));
    }
    if !proc.channels.contains_key(&plan.chan) {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Channel(plan.chan))));
    }
    if let (Some(pdb), Some(host_vas)) = (plan.pdb, plan.host_vas) {
        match proc.vases.get(&(plan.gpu, pdb)) {
            Some(vas) if vas.host_vas == Some(host_vas) => {}
            Some(_) | None => {
                return Err(Refusal::bare(FwdFault::Stale(Stale::Vas {
                    gpu: plan.gpu,
                    pdb,
                })));
            }
        }
    }
    Ok(CeForwarded {
        proc: plan.proc,
        chan: plan.chan,
        host_ce,
        ours,
    })
}

/// The **single-threaded composition** of [`plan_ce`] / `Worker::execute` / [`commit_ce`]
/// (R1), for a caller holding a bare `&mut Proc`.
///
/// ★ L1 does **not** go through here: `kayfabe_rt::SharedDevice::forward_ce` drives the
/// three phases itself, interleaved with its lock acquire/release and its pool-full wait
/// — the same split [`exec_doorbell`]'s docs record.
///
/// # Errors
/// [`FwdFault`] from either the plan or the commit.
pub fn exec_ce(proc: &mut Proc, cid: ChanId, spans: &[CeSpan]) -> Result<CeForwarded, FwdFault> {
    let planned = plan_ce(proc, cid, spans)?;
    let gpu = planned.plan.gpu;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_ce(proc, &planned.plan, reply)
    })
}

/// ACT phase of the pushbuffer parse: apply the decoded `methods` of channel `cid`
/// to **its owning proc only** (`&mut Proc` + the read-only spine for the arch's
/// method decoder). Feeds: the operand split ([`classify_ce`]) → latched [`PtWrite`]s
/// for the caller to route to their owners; `SemRelease` → the proc's
/// `CompletionQueue`; honors `TlbInvalidate` membars; passes opaque methods through.
///
/// ★ It **observes** page-table writes and does not apply them: the owner of a written
/// page is routinely a different proc, and this phase holds only the issuing one.
/// [`latch_pt_writes`] is the applying half.
pub fn apply_pushbuffer(
    spine: &Spine,
    proc: &mut Proc,
    cid: ChanId,
    methods: Vec<(u32, Vec<u32>)>,
) -> Result<PushbufferOutcome, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get_mut(&cid).ok_or(FwdFault::NoVas(cid))?;
    let chan_pdb = chan.vas_pdb;
    let cgpu = chan.gpu;
    // ★★★ E5 — DECODE THE WHOLE RUN, against THIS CHANNEL'S engine method state
    // (`execution_plane_increments.md` §8.2.1). A copy-engine operation is five method
    // runs and `LAUNCH_DMA` carries none of its operands, so a per-method decode cannot
    // produce a `CeLaunchDma` whose `dst`/`src`/`len` are anything but invented — which
    // is exactly what `E4` measured and refused.
    //
    // ⊘ The state is the ARCH'S; this crate carries it and reads no field. It is
    // `&mut` because the engine's is: the accumulator is where the operands live between
    // the run that writes them and the run that fires them.
    //
    // ★★ Done here rather than in `read_pushbuffer` for the reason the phase split
    // already exists: the state is per-CHANNEL, and the read phase holds no proc and
    // therefore no channel. It is also why a run split across two GPFIFO entries of the
    // same ring works without a special case — `read_pushbuffer` returns one method list
    // for the whole ring, and the accumulator would carry it across in any event.
    let decoded = spine
        .arch()
        .pushbuffer()
        .decode_run(&mut chan.method_state, &methods);
    // The C's `is_user_ce(s->chan_client)` conjunct — a property of the SUBMITTER, read
    // once per parse because a channel belongs to exactly one proc.
    let origin = ChannelOrigin::of(proc.id);

    let mut out = PushbufferOutcome::default();
    for method in decoded {
        match method {
            kayfabe_arch::PushMethod::SetObject { .. } => {
                // Routing confirmation only — no address/completion state changes.
            }
            kayfabe_arch::PushMethod::CeLaunchDma {
                dst,
                src,
                len,
                dst_is_virtual,
                src_is_virtual,
                // ★ E10b consumes the phys-mode targets E10a decoded: for a physical
                // operand they are the residency signal `partition_ce` turns into a CPU
                // plane. For a virtual operand they are carried but unread (the MMU, not
                // this register, answers residency there).
                dst_target,
                src_target,
                work,
                completion,
            } => {
                // ★★★ DECISION 1 of 2 — EXECUTE.
                //
                // (a) The C's predicate (§11.5): work kind, BOTH operand forms, and the
                //     submitting channel's origin. No address. Recorded as the baseline.
                match ce_executor_c(work, origin, src_is_virtual, dst_is_virtual) {
                    CeExecutor::HostCe => out.c_execute_host_ce += 1,
                    CeExecutor::Ours => out.c_execute_ours += 1,
                }
                // (b) §12's ruling, which is what we ACT on: the ADDRESSES, partitioned.
                //     Resolved in the ISSUING channel's own Vas — the same table, and
                //     the same reason, as the capture decision below.
                let dst_table = chan_pdb
                    .and_then(|pdb| proc.vases.get(&(cgpu, pdb)))
                    .map(|v| &v.table);
                // ★ §14.15 obstacle 2 — the operand-resolver seam. Here it is the
                // channel's own address table, which is what every channel with a `Vas`
                // resolves through; the VAS-less CeUtils channel takes the same seam with
                // `kayfabe_rt::ceutils`'s published-root walk on the other side of it.
                let mut ops = TableOperands::new(dst_table, chan_pdb);
                let spans = partition_ce(
                    &mut ops,
                    dst,
                    dst_is_virtual,
                    dst_target,
                    src,
                    src_is_virtual,
                    src_target,
                    len,
                    work,
                )?;
                if out.ce_spans.len() + spans.len() > MAX_CE_SPANS_PER_PARSE {
                    return Err(FwdFault::CeTooFragmented { dst, len });
                }
                // ★★★★★ w283 — where THIS launch's spans begin, latched before the extend so
                // its own declared release can be attached to its OWN last span and to no
                // other launch's. ⊘ `ce_spans.last_mut()` alone would attach a release to
                // whatever the previous launch left there when this one partitioned to zero
                // spans — which is exactly the `PhysOperand` / release-only case.
                let spans_from = out.ce_spans.len();
                out.ce_spans.extend(spans);
                // ★★★ DECISION 2 of 2 — CAPTURE. Reads the RESOLVED PHYSICAL destination
                // and nothing else. Independent of the above by construction: it is not
                // in scope of that match and cannot see its answer.
                match classify_ce(spine, proc, cid, chan_pdb, cgpu, dst, dst_is_virtual)? {
                    // ★ VA-OPERAND — not a page-table write. The operands are addresses
                    // the host MMU resolves for itself once the address space is
                    // resident, so the address plane has nothing to extract. Counted, not
                    // acted on. Whether hardware or we execute it is DECISION 1's answer.
                    CeOperands::VaOperand { .. } => out.data_copies += 1,
                    // ★★★ PHYS-OPERAND — a page-table write. The payload is guest-physical
                    // PTE values, which cannot be handed to hardware. LATCH the page here
                    // (O(1), index only — decoding per write livelocked on the bench,
                    // `C: :8686-8690`) and let the caller route it to its OWNER.
                    CeOperands::PhysOperand {
                        page,
                        aperture,
                        owner,
                        owner_pdb,
                    } => out.pt_writes.push(PtWrite {
                        gpu: cgpu,
                        page,
                        aperture,
                        owner,
                        owner_pdb,
                        bytes: len,
                    }),
                }
                // ★★★ E10e — THE COMPLETION THIS LAUNCH RELEASES, appended **after** its
                // own spans and in the same iteration, so `sem_releases` is ordered
                // *behind* the bytes it certifies. `cpu_ce::write_completion` is the
                // consumer, and its contract — write every payload, then signal, and never
                // signal if a write refused — is what makes this the truthful half of
                // `ce_executor_tree.md`'s rule 1.
                //
                // ⊘ **This is a DIFFERENT WORD from the `SemRelease` arm below**, four
                // bytes apart on the channel that walls. RM releases the finishPayload
                // through the *engine* class and, at the bottom of the same block, a
                // *host*-class semaphore meaning only "HOST has read the methods"
                // (`ogkm-580: channel_utils.c:250, 698-746, 838-840`). Until this arm
                // existed only the host one was decodable — [`kayfabe_arch::CeCompletion`]
                // carries the citation and the boot log that printed both addresses.
                if let Some(c) = completion {
                    proc.completion.observe(OsEventRef(c.addr.0 ^ c.payload))?;
                    out.sem_releases.push((c.addr, c.payload));
                    // ★★★★★ **w283 — HAND THE GUEST'S OWN RELEASE TO THE ENGINE.**
                    //
                    // Attached to the LAST span of THIS launch, so the engine writes the
                    // guest's payload after it has moved the guest's bytes — submission
                    // order in one pushbuffer, not an ordering we impose afterwards.
                    //
                    // ⊘ **`HostCe` only, and that is not a policy choice.** A span running
                    // on `CeExecutor::Ours` is served by the shell's CPU executor, and this
                    // field is the HOST engine's instruction — attaching it to a CPU-executed
                    // span would name a writer that is not the one running the work.
                    //
                    // ⚠⚠ **AND THE `Ours` ARM HAS NO WRITER AT ALL TODAY — measured, not
                    // assumed.** `[measured 2026-08-13, `git grep write_completion` over the
                    // whole workspace]` `kayfabe_rt::cpu_ce::write_completion` — the documented
                    // `sem_releases` consumer — has **zero call sites**: every hit is its own
                    // definition or a doc reference. So `out.sem_releases` is populated on this
                    // path and **dropped**, which is exactly what the control arms measure
                    // (`semaphore 0x00000000`, every boot). ⊘ An earlier draft of this comment
                    // said the `Ours` completions were *"`write_completion`'s and are
                    // unchanged"* — true of the design and **false of the tree**, and left
                    // standing it would have read as *"the other arm is already handled"*.
                    // ⇒ There is therefore exactly ONE writer for a guest completion in this
                    // tree, it is the engine, and it is this field. A second one would be
                    // `a_second_source_of_truth_beside_a_complete_value`; today there is not
                    // even a first one on the CPU arm.
                    //
                    // ⊘ And nothing is attached when this launch produced no span of its
                    // own (`spans_from == len()`): a release with no bytes behind it is a
                    // `CeRelease`, handled by its own arm, and inventing a carrier for it
                    // here would put the guest's payload on a copy it never asked for.
                    //
                    // ⚠ **`u32::try_from`, and a payload that does not fit is NOT attached.**
                    // `LAUNCH_SEMAPHORE_RELEASE_ONE_WORD` releases ONE 32-bit word, so a
                    // wider payload is a four-word release this encoding cannot express.
                    // ⊘ Truncating would write a DIFFERENT value at the address the guest
                    // polls — a wrong completion, which is worse than none — so it is
                    // declined, and the guest then waits visibly instead of being lied to.
                    if let Some(last) = out.ce_spans.get_mut(spans_from..).and_then(<[_]>::last_mut)
                        && last.sub.by == CeExecutor::HostCe
                        && let Ok(payload) = u32::try_from(c.payload)
                    {
                        last.sub.guest_release = Some(kayfabe_isolate::CeGuestRelease {
                            va: c.addr.0,
                            payload,
                        });
                    }
                }
            }
            // ★★★ **A launch that moves no bytes and exists only to release** — see
            // [`kayfabe_arch::PushMethod::CeRelease`] for UVM's `channel_init` push, which
            // is nothing else.
            //
            // ⊘ It contributes to `sem_releases` and to **NOTHING** else: no `ce_spans` (it
            // names no operand), no `classify_ce` (there is no destination to attribute),
            // no `data_copies`. That asymmetry is the honesty: this method's whole content
            // is the release, so recording anything more would be reporting work the guest
            // did not ask for.
            //
            // ⚠ Appended in decode order, which keeps `sem_releases` behind any bytes an
            // EARLIER launch in the same ring owes — the ordering `cpu_ce::write_completion`
            // depends on. A release-only launch owes nothing itself, so its position in that
            // sequence is the only ordering fact it carries, and it is preserved rather than
            // hoisted.
            kayfabe_arch::PushMethod::CeRelease { completion, .. } => {
                proc.completion
                    .observe(OsEventRef(completion.addr.0 ^ completion.payload))?;
                out.sem_releases.push((completion.addr, completion.payload));
                out.ce_releases += 1;
            }
            kayfabe_arch::PushMethod::SemRelease { addr, payload } => {
                // Completion observe on the OWNING proc's queue (per-`Proc`, §2.4).
                // A hostile guest flooding sem-releases is loud-capped, not OOM.
                proc.completion.observe(OsEventRef(addr.0 ^ payload))?;
                out.sem_releases.push((addr, payload));
            }
            kayfabe_arch::PushMethod::TlbInvalidate { pdb, membar } => {
                out.invalidates.push((pdb, membar));
                // A membar is a hard barrier: the interpreter honors it before
                // advancing (recorded here; the real transport blocks on refresh).
            }
            // ★★★ §16.24's tripwire on the forwarding parse too, and it RETURNS rather
            // than counting — the same ordering property `kayfabe_rt::ceutils` states: a
            // submission carrying a fault cancel must not have its copies partitioned and
            // handed on while the cancel is dropped. ⊘ `out` is discarded with the `?`, so
            // no span from this parse can reach an executor.
            kayfabe_arch::PushMethod::UvmSwFaultMethod { method } => {
                return Err(FwdFault::UvmFaultMethodWithoutFaultDelivery { method });
            }
            kayfabe_arch::PushMethod::Opaque => out.opaque += 1,
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
    let ranges = pushbuffer_ranges(&gpu.spine, ring);
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let methods = read_pushbuffer(&gpu.spine, proc, cid, vmm, &ranges)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let out = apply_pushbuffer(&gpu.spine, proc, cid, methods)?;
    latch_pt_writes(gpu, &out.pt_writes);
    Ok(out)
}

/// ★★★ **E6 — THE JOIN, single-threaded**: a guest's ring becomes real work on a real
/// engine. [`parse_pushbuffer`] recovers what the guest asked for; [`exec_ce`] issues it.
///
/// This is the composition the E6 row names, with `Worker::execute` and
/// `RmBackend::ce_copy` reached through [`exec_ce`]'s round trip. L1's form is
/// `kayfabe_rt::SharedDevice::submit_ring`, which is the same two steps with each half
/// taking and releasing its own locks.
///
/// ⊘ **The page-table decode is NOT folded in**, deliberately. `parse_pushbuffer` latches
/// the witnessed page-table pages onto their owners; turning them into bindings is
/// `plan/run/commit_pt_decode`, which needs a `GmmuFmt` out of the spine and a worker of
/// its own, and which visits **other procs**. Folding it here would hide a cross-proc pass
/// inside a per-channel one.
///
/// # Errors
/// [`FwdFault`] from either half — the parse's address/read refusals, or the forward's.
pub fn submit_ring(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    pid: ProcId,
    cid: ChanId,
    ring: &[u8],
) -> Result<(PushbufferOutcome, CeForwarded), FwdFault> {
    let parsed = parse_pushbuffer(gpu, vmm, pid, cid, ring)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let forwarded = exec_ce(proc, cid, &parsed.ce_spans)?;
    Ok((parsed, forwarded))
}

/// ★★★ Route each latched page-table write to the `Vas` that **owns** the page.
///
/// This is a separate step from [`apply_pushbuffer`] for a structural reason, not a
/// stylistic one: the act phase holds the **issuing** proc, and the owner of a written
/// page-table page is routinely a **different** proc — the guest kernel is what writes a
/// user process's page tables. So the act phase can only *observe* the write; applying it
/// needs the device, which is exactly the plan/commit shape the rest of the plane uses.
///
/// A write whose owner has retired between the parse and here is **dropped, silently and
/// correctly**: its page tables are gone, and re-attaching a dirty page to a survivor is
/// how the C's never-pruned table aliased two processes (`eight_blockers_resolved.md` §2).
pub fn latch_pt_writes(gpu: &mut Gpu, writes: &[PtWrite]) {
    for w in writes {
        let Some(owner) = gpu.procs.get_mut(&w.owner) else {
            continue; // owner retired in the gap — the pages died with it
        };
        if owner.is_retired() {
            continue;
        }
        if let Some(vas) = owner.vases.get_mut(&(w.gpu, w.owner_pdb)) {
            vas.pt_pages.insert(w.page);
        }
    }
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
// The gate is STRUCTURAL: [`plan_doorbell`] is the ONE ring gate (★ corrected 2026-07-27:
// this said "[`handle_doorbell`] is the ONE ring path (nothing else in the workspace
// reaches `RmBackend::ring_doorbell`)" — false as stated; the sole `ring_doorbell` call
// site is in `kayfabe_isolate::Worker::execute`, and the L1 path goes through
// `kayfabe_rt::SharedDevice::doorbell`, not through `handle_doorbell`. Found by the
// whitepaper's verification pass). `plan_doorbell` is the sole constructor of
// `VerbPlan::Doorbell` in the production crates and it gates the caller-recovered
// working set against the channel's `Vas` table before returning one — so there is no
// ungated sibling to bypass and no un-gated plan any production path can hand a worker
// (the C's "one exec path" refactor-debt lesson, closed by construction).
//
// ★★ 2026-07-27: this used to carry a residual — "`VerbPlan` is a public enum, so this
// is a call-graph property, not a type-system one". It is now BOTH.
// `kayfabe_isolate::VerbPlan::Doorbell` is `#[non_exhaustive]` (no struct expression
// outside that crate: E0639, pinned by a trybuild row) and its only constructor,
// `VerbPlan::gated_doorbell`, RUNS this gate through the abstract `RingWorkingSet` view
// `VasGate` below implements. `gate_working_set` further down is the read-only QUERY
// form of the same predicate; it cannot ring anything.
// =================================================================================

/// Read-only query: would `working_set` pass channel `cid`'s ring-gate right now
/// (every VA published into that channel's Vas's own host VAS)? A VA with no host
/// publication (`Binding::host = None`) is a loud [`FwdFault`], never guessed.
///
/// This is the load-bearing per-`Vas` publication check: two procs' identical guest
/// VAs each resolve in their OWN Vas (keyed by PDB), so the gate passes for both only
/// because each published into its OWN host VAS (distinct `HostHandle`s). The
/// ENFORCING form lives inside [`plan_doorbell`] — this query cannot ring.
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
///
/// **Every miss here ⇒ FAULT**, and one of them is the taxonomy's clearest illustration:
/// `chan.vas_pdb == None` is the *same absence* that `Gpu::sync_proc_to_boundary`
/// deliberately DEFERS on. At ring time it is never knowable — this submission is being
/// gated now — so it is `FwdFault::NoVas`, by name. The category belongs to the site, not
/// to the absence (`kayfabe_core` crate docs).
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
kayfabe_util::assert_send_sync!(
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

// ── Task #111: from a REFUSED ring to a fault the guest can be told about ──────────

/// ★★★ **The derivation site.** Turn the working-set miss that just refused a doorbell
/// into the facts `kayfabe_core::fault::verdict` decides on.
///
/// `docs/design/simulated_gpu_fault.md` §4. It sits here, beside [`gate_vas`], because
/// this is where the miss is *produced*: the refusal and the facts about it are one
/// locked snapshot apart, and re-deriving them later would be re-reading a table the
/// guest may have changed underneath us.
///
/// ## ★★ What this does NOT do, and why the split matters
///
/// It does not decide. It collects — the channel's declared engine, the VAS it declared,
/// the address that missed — and hands them to the core's policy, which is the only
/// thing that may say *"this may be presented to the guest as hardware"*. The two halves
/// are deliberately in different crates: a derivation that also decided would make the
/// guest-kernel-vs-application rule a property of the forwarding plane, where it would
/// be re-implemented the next time another site wants to fault.
///
/// ★ The caller supplies `cause` and `access`. They are **not** derivable here: the
/// address table records bindings, not the direction of the access that missed, and this
/// function must not invent one. A caller with no honest access direction has no honest
/// fault to emit.
///
/// # Errors
///
/// [`FwdFault::UnknownVchid`] if the route names a channel this proc no longer has —
/// which is a *race*, not a fault to report: the guest freed the channel while we were
/// deciding to fault it, and the thing to tell it about is gone.
pub fn fault_facts(
    proc: &Proc,
    route: &DoorbellRoute,
    va: GpuVa,
    cause: kayfabe_arch::fault::MmuFaultCause,
    access: kayfabe_arch::fault::MmuFaultAccess,
) -> Result<kayfabe_core::fault::FaultFacts, FwdFault> {
    let chan = proc
        .channels
        .get(&route.chan)
        .ok_or(FwdFault::UnknownVchid {
            gpu: route.gpu,
            vchid: route.vchid,
        })?;
    Ok(kayfabe_core::fault::FaultFacts {
        gpu: route.gpu,
        proc: route.proc,
        chan: route.chan,
        vchid: chan.vchid,
        pdb: chan.vas_pdb,
        va,
        engine: chan.engine,
        cause,
        access,
        // ★ A held fact like every other field here: the channel declared it at alloc
        // time and the core carries it. Collected beside the refusal in the same locked
        // snapshot, because a notifier address re-read later could belong to a channel
        // the guest has since freed and re-allocated on the same slot.
        error_notifier: chan.error_notifier,
    })
}
