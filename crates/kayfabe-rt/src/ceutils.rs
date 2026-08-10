//! ★★★ **E10e item (c) — THE SHELL DRIVER for a VAS-less CeUtils submission**
//! (`execution_plane_increments.md` §14.15, the four obstacles).
//!
//! One guest doorbell on the GSP-managed CeUtils channel becomes: walk the ring, read the
//! GPFIFO entry, walk and read the pushbuffer it names, decode the methods against the
//! chip's own codec, partition each `LAUNCH_DMA`'s operands, run the `Ours` sub-copies on
//! the CPU, and only then release the finishPayload the guest is spinning on.
//!
//! # ⊘ Why this glue is here and not in `kayfabe_fwd::apply_pushbuffer`
//!
//! `apply_pushbuffer` runs inside `SharedDevice::route_act` — under the device read lock
//! **and** the issuing proc's mutex. The resolution this channel needs comes from
//! [`kayfabe_device::RegPlane`], and `[src]` the plane's command-policy chain already calls
//! into the core *under the plane's own mutex* (`kayfabe_rmrpc::policy::Bridge` is boxed
//! into `PlaneState::policy`). So **plane→core is the established lock order**, and
//! resolving from inside the core's act phase would be its inversion — a guest-buildable
//! ABBA (`l1_os_shell.md` §6.3), constructed on purpose.
//!
//! ⇒ The driver runs off `DoorbellPort::ring`, which `RegPlane::write` documents as being
//! called with **no plane lock held**, and takes the two in the sanctioned order: the core
//! first (routing the token to its channel's declared facts, then releasing), the plane
//! second (one [`kayfabe_device::RegPlane::ce_session`] for the whole submission).
//!
//! # The four obstacles, and where each is closed
//!
//! 1. *"Nothing in the adapter calls the parse/forward path"* — [`run_submission`] is that
//!    caller, and the adapter hands it a `&mut dyn Vmm` it now holds.
//! 2. *"The two consumers want an `AddressTable`; this channel has no `Vas`"* —
//!    [`WalkOperands`], the second implementation of `kayfabe_fwd::OperandResolver`. ⊘ Not
//!    a TLB fill: nothing here is stored, so §7 rule 6 is satisfied rather than argued
//!    against.
//! 3. *"`RegPlane` holds both stores under one lock … `cpu_ce` takes `&mut dyn Vmm`"* —
//!    taken the owner's second way: **the driver runs where the `Vmm` is and the plane
//!    hands out its store**. `cpu_ce`'s signature is unchanged, so its `raise_irq` works
//!    through the real `Vmm` and needs no new port.
//! 4. *"The decode needs a per-channel `MethodState`"* — and it is **per-channel**, held by
//!    the adapter beside the ring cursor and handed back in [`CeUtilsRun::state`]. ⚠ It was
//!    submission-local until `s21_dbf853a_cup2`, on a claim that was true of RM's CeUtils
//!    pushbuffer and false of UVM's; see [`run_submission`] for the measurement.
//!
//! # ⊘ What this must never do
//!
//! Report a served doorbell over work that did not happen (§14.8). Every arm that cannot
//! do the work refuses **by name** and writes no completion: an operand run that is not
//! ours to execute, a walk that misses, a store that refuses, a decode that produced no
//! launch. A submission is served only when its bytes moved.

use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{
    CpuOperand, CpuPlane, PlaneAddr, PushMethod, PushbufferAbi, Residency, ResidencyOracle,
};
use kayfabe_device::CePlane;
use kayfabe_device::ceresolve::CeResolve;
use kayfabe_fwd::{
    CeSpan, DeclaredResidency, FwdFault, MAX_PUSH_RANGE_BYTES, MAX_PUSH_TOTAL_BYTES,
    OperandResolver, OperandRun, Representability, partition_ce,
};
use kayfabe_isolate::CeExecutor;
use kayfabe_vmm::{Vmm, VmmError};

/// ★ Re-exported so the adapter that must **keep** a channel's accumulator between
/// doorbells names the same type this driver decodes against, without taking a dependency
/// on Axis-B to hold one value. ⊘ One description of the engine's method state, two
/// holders — the shape `CeUtilsChannel` already uses for the channel's declared facts.
pub use kayfabe_arch::MethodState;

/// ★★ **How many GPFIFO entries one doorbell may consume.**
///
/// `[src]` `ogkm-580: ce_utils_sizes.h:27` — a CeUtils channel's ring is `NUM_COPY_BLOCKS`
/// = 4096 entries, and `[src] channel_utils.c:403-443` RM submits **one** block per
/// `channelFillGpFifo`. So a doorbell that appeared to bring thousands of new entries would
/// be a guest doing something this channel's own driver never does; the cap keeps a hostile
/// ring from turning one MMIO write into 4096 copies before anything can refuse.
const MAX_ENTRIES_PER_DOORBELL: u32 = 8;

/// The GPFIFO ring size a CeUtils channel declares, in entries — used only to wrap the
/// cursor. `[src]` `ogkm-580: ce_utils_sizes.h:27` (`NUM_COPY_BLOCKS`).
const RING_ENTRIES_FALLBACK: u32 = 4096;

/// One GPFIFO entry's stride in bytes, from the crate that owns NVIDIA's wire facts.
/// ⊘ Only the STRIDE — the entry's *meaning* is decoded by the arch's own
/// [`PushbufferAbi::gpfifo_entries`], never by a second decoder here.
const GP_ENTRY_SIZE: usize = kayfabe_abi::submit::GP_ENTRY_SIZE as usize;

/// ★★★ **How far this channel's ring has been consumed** — the per-channel GPFIFO read
/// cursor, owned by the adapter, passed in **by value** and handed back only on success
/// (see [`CeUtilsRun::cursor`]).
///
/// # ⊘ Why a cursor rather than a read of `GP_PUT`
///
/// The honest source of *"how many entries are new"* is the channel's USERD `GPPut`
/// (`[src] ogkm-580: channel_utils.c:523`, `MEM_WR32(&pControlGPFifo->GPPut, putIndex)`),
/// and this port does not know where this channel's USERD lives. So the cursor answers the
/// **other** half of the question — *"which entries have we already run"* — and the ring's
/// own encoding answers the first: an unwritten entry is zero, and
/// `kayfabe_abi::submit::gp_entry_decode` returns `None` for a zero-length entry
/// (`submit.rs:1257`), because RM zero-initialises the channel buffer
/// (`TRANSFER_FLAGS_SHADOW_INIT_MEM`, `[src] channel_utils.c:471-476`).
///
/// ⊘ **The cursor is what makes re-execution impossible**, and that matters more than it
/// looks: a memset re-run is idempotent, but a *completion* re-released for a copy that
/// already retired is a signal for work that did not happen on this doorbell.
///
/// ★ `[measured 2026-08-08, boot run_p35_84d857d]` corroborates the arithmetic from the
/// other side: RM writes the entry at `lastSubmittedEntry` (0 on a fresh channel) pointing
/// at method block `putIndex = lastSubmittedEntry + 1`, so the first submission's entry
/// names `pbGpuVA + 1 × CE_METHOD_SIZE_PER_BLOCK` = `pbGpuVA + 0x64` — and the device
/// printed exactly `gp0=0x420000064+0x60`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpCursor {
    /// The next ring entry index to read.
    pub next: u32,
}

/// The declared facts one CeUtils submission is addressed by — the `(hClient, hVASpace)`
/// pair the guest's publication is keyed on, plus the ring it declared.
///
/// ★ All four come off the **one** graph node that declared them (the channel's own
/// `RM_ALLOC`), via `SharedDevice::ce_channel_facts`, so they cannot be two projections
/// that disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeUtilsChannel {
    /// The owning `hClient`.
    pub client: u32,
    /// The `hVASpace` handle the channel named — `[measured]` equal to the `hObject` of
    /// the publication carrying this channel's page-directory root (§14.11's two-sided
    /// join).
    pub vaspace: u32,
    /// The GPFIFO ring's GPU virtual address (`pbGpuVA + channelPbSize`).
    pub ring_va: u64,
    /// How many entries the ring declares.
    pub ring_entries: u32,
}

/// What one served submission did. Every number is something that **happened**, so a report
/// built from it cannot say more than the work did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CeUtilsRun {
    /// GPFIFO entries consumed.
    pub entries: usize,
    /// Method `(header, args)` pairs decoded across those entries.
    pub methods: usize,
    /// `LAUNCH_DMA`s the codec fired.
    pub launches: usize,
    /// ★★★ How many of [`CeUtilsRun::launches`] moved **no bytes** — a
    /// [`PushMethod::CeRelease`], i.e. `DATA_TRANSFER_TYPE == NONE`.
    ///
    /// ⊘ Reported separately so `describe()` cannot let a submission that only released
    /// read as one that copied. `launches == releases` with `bytes == 0` is an honest and
    /// complete serving of UVM's `channel_init` push — and it must be distinguishable from
    /// a copy whose bytes we lost.
    pub releases: usize,
    /// Sub-copies the partition produced.
    pub spans: usize,
    /// Bytes actually moved by the CPU executor.
    pub bytes: u64,
    /// finishPayload semaphores written, after the bytes.
    pub completions: usize,
    /// ★ Where the **last** completion landed — its VA, its plane and its plane address.
    /// Carried so a boot can state the `#12` question's answer instead of implying it.
    pub completion_at: Option<(GpuVa, CpuPlane, u64)>,
    /// ★★★ **The ring cursor AFTER this submission** — the caller's next `next`.
    ///
    /// ⊘ Returned in the success value rather than advanced through a `&mut`, and that is
    /// the discipline rather than the style: a cursor advanced by a submission that then
    /// refused would skip the entry it could not run, turning one loud failure into a
    /// **silently dropped copy** (`#13`'s `CE-DROP` by another route). Here a refusal
    /// carries no cursor at all, so there is nothing for a caller to commit by accident.
    pub cursor: GpCursor,
    /// ★★★★ **The channel's method accumulator AFTER this submission** — the engine state
    /// the guest's own `SET_OBJECT` and operand writes left behind.
    ///
    /// ⊘ Handed back in the success value for exactly [`CeUtilsRun::cursor`]'s reason, and
    /// it is the same reason twice: a refusal must leave the channel where it was, so the
    /// guest's retry re-latches from the same point rather than running against half of a
    /// submission we did not finish. See [`run_submission`] for the boot in which rebuilding
    /// this per doorbell made every UVM push decode to nothing.
    pub state: MethodState,
}

impl CeUtilsRun {
    /// ★★★ One line naming **what happened**, for the boot report.
    ///
    /// ⊘ It is built here rather than in the adapter for a reason that has bitten this
    /// project: the adapter would have to name the plane a completion landed in, and
    /// `#12`'s whole lesson is about that word. Formatting it beside the fields keeps the
    /// aperture letter and the address that produced it in one place — `V` = this device's
    /// emulated framebuffer, `S` = guest RAM — matching
    /// `kayfabe_device::ceresolve::CeResolve::tag`'s vocabulary exactly, so a doorbell line
    /// and an addressing probe in the same boot can be read against each other.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "cpu-ce: {} gp, {} methods, {} launch ({} release-only), {} span, {} B, {} sem{}",
            self.entries,
            self.methods,
            self.launches,
            self.releases,
            self.spans,
            self.bytes,
            self.completions,
            self.completion_at
                .map_or_else(String::new, |(va, plane, phys)| format!(
                    " fin va=0x{:x} -> {}:0x{phys:x}",
                    va.0,
                    match plane {
                        CpuPlane::Fb => "V",
                        CpuPlane::GuestRam => "S",
                    }
                ))
        )
    }
}

/// A submission this driver would not serve, with the walk's own finding when it has one.
///
/// ⊘ Two fields rather than a flattened sentence: `fault` is the stable, matchable name a
/// test asserts on, and `detail` is the resolver's whole answer — the level an `Unmapped`
/// failed at, the aperture a root claimed — which `FwdFault` is `Copy` and cannot hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeUtilsRefusal {
    /// The named fault.
    pub fault: FwdFault,
    /// The walk's finding and the address it was asked about, when the refusal came from a
    /// resolution.
    pub detail: Option<(GpuVa, CeResolve)>,
}

impl CeUtilsRefusal {
    /// A bare fault with no resolution behind it.
    fn plain(fault: FwdFault) -> CeUtilsRefusal {
        CeUtilsRefusal {
            fault,
            detail: None,
        }
    }

    /// One line for a boot report: the fault, and the walk's own sentence when there is one.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.detail {
            None => format!("{:?}", self.fault),
            Some((va, r)) => format!("{:?} at va=0x{:x}: {}", self.fault, va.0, r.describe()),
        }
    }
}

/// Map a guest-RAM refusal to the named fault — the same split `kayfabe_fwd`'s own
/// `guest_read` makes, so a device-aimed address and an unbacked one stay distinguishable.
fn ram_fault(e: VmmError) -> FwdFault {
    match e {
        VmmError::NonRamGpa { gpa } => FwdFault::NonRamGpa { gpa },
        VmmError::BadGpa { gpa } => FwdFault::GpaRead { gpa },
        _ => FwdFault::GpaRead { gpa: u64::MAX },
    }
}

/// ★★★ **The second implementation of [`OperandResolver`]: the guest's own page-table
/// walk**, from the root the guest published, on the demand the session was opened with.
///
/// # ⊘ Nothing is stored, and that is the whole of the owner's ruling
///
/// `gmmu_publication_discipline.md` §7 rule 6 is *"never cache the walk — the result is
/// valid for this fault only"*, and rule 7 (*"serialise against the observed invalidate"*)
/// is **vacuous here**: §5 measured both invalidate transports at zero on this path, so a
/// cache would have nothing to clear it. Every question this resolver is asked descends the
/// tree again. That is what a TLB does and what the address plane's blessed staleness
/// already permits.
///
/// # Every resolution is `Fabricated`, and that is a measurement not a default
///
/// `[measured 2026-08-08, boot run_p35_84d857d]` nothing on this channel is host-published
/// — there is no `Binding::host`, because there is no `Vas` and never was. So every run is
/// `Representability::Fabricated` ⇒ `CeExecutor::Ours` ⇒ the shell CPU executor, which is
/// exactly `ce_executor_tree.md`'s STEP 2 answer for two CPU-reachable operands.
pub struct WalkOperands<'a, 'p> {
    ce: &'a mut CePlane<'p>,
    /// Where the last refusing resolution is recorded, so the driver can report the walk's
    /// whole finding beside the `Copy` fault. ⊘ Written **only** on a refusal.
    last: &'a mut Option<(GpuVa, CeResolve)>,
}

impl core::fmt::Debug for WalkOperands<'_, '_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkOperands").finish_non_exhaustive()
    }
}

impl<'a, 'p> WalkOperands<'a, 'p> {
    /// A resolver over one open CE session.
    pub fn new(
        ce: &'a mut CePlane<'p>,
        last: &'a mut Option<(GpuVa, CeResolve)>,
    ) -> WalkOperands<'a, 'p> {
        WalkOperands { ce, last }
    }

    /// Resolve one VA to `(plane address, residency, bytes left in its leaf page)`, or
    /// refuse by name having recorded the finding.
    fn resolve_one(&mut self, va: u64) -> Result<(CpuOperand, u64), FwdFault> {
        let r = self.ce.resolve(va);
        let CeResolve::Resolved {
            phys,
            aperture,
            page_size,
            ..
        } = r
        else {
            *self.last = Some((GpuVa(va), r));
            return Err(FwdFault::CeWalk {
                va: GpuVa(va),
                kind: r.kind(),
            });
        };
        // ⊘ ASKED, not computed here — the same oracle the table-backed resolver consults,
        // so a walked operand and a bound one can never come to disagree about where one
        // aperture's bytes live. A peer leaf has no CPU plane and is refused by name.
        let residency: Residency = DeclaredResidency
            .residency_of_aperture(aperture, phys)
            .ok_or(FwdFault::CePeerOperand { addr: phys })?;
        // How much of this leaf remains. A page's low bits are shared by the VA and its
        // backing (a mapping is page-granular), so the offset within the page is the VA's.
        let left = page_size - (va & (page_size - 1));
        Ok((
            CpuOperand {
                residency,
                addr: PlaneAddr(phys),
            },
            left,
        ))
    }
}

impl OperandResolver for WalkOperands<'_, '_> {
    fn resolve_runs(&mut self, addr: GpuVa, len: u64) -> Result<Vec<OperandRun>, FwdFault> {
        // Clip rather than wrap, exactly as `AddressTable::spans` does: a copy that ran off
        // the top of the address space must not reach a mapping at the bottom.
        let end = (u128::from(addr.0) + u128::from(len)).min(1u128 << 64);
        let eff = (end - u128::from(addr.0)) as u64;
        let mut out: Vec<OperandRun> = Vec::new();
        let mut off = 0u64;
        while off < eff {
            let va = addr.0 + off;
            let (op, left) = self.resolve_one(va)?;
            let take = left.min(eff - off);
            debug_assert!(take > 0, "a walk step must consume bytes");
            // Merge with the previous run when the backing is genuinely contiguous — same
            // store, same ownership, and the plane address continuing exactly. ⊘ Equality
            // of `Residency` alone is NOT enough (§14.15's second correction): two leaves
            // sharing an aperture but not adjacent would merge into one instruction and the
            // executor would write past the end of the first.
            if let Some(prev) = out.last_mut()
                && prev.2 == Representability::Fabricated
                && prev.3.is_some_and(|p| {
                    p.residency == op.residency && p.addr.offset(prev.1) == op.addr
                })
            {
                prev.1 += take;
            } else {
                out.push((va, take, Representability::Fabricated, Some(op)));
            }
            off += take;
        }
        Ok(out)
    }

    fn resolve_word(&mut self, addr: GpuVa) -> Result<CpuOperand, FwdFault> {
        // ⊘ A four-byte release must not straddle a page: if it did, one half would land in
        // a different backing and the guest would poll a torn word forever. The leaf's
        // remaining length is checked rather than assumed.
        let (op, left) = self.resolve_one(addr.0)?;
        if left < 4 {
            return Err(FwdFault::CeWalk {
                va: addr,
                kind: "SemaphoreStraddlesPage",
            });
        }
        Ok(op)
    }
}

/// Read `buf.len()` bytes at GPU virtual address `va`, re-resolving at every leaf boundary
/// and routing each piece to the store its **own leaf's aperture** names.
///
/// ⊘ The aperture is the router and it is not optional: a vidmem leaf is an offset into
/// this device's framebuffer and a sysmem leaf is a guest-physical address, two number
/// spaces that collide freely (`§8.2.3`'s measured warning: at 8 GiB a GPU VA is itself a
/// legal GPA). Serving one out of the other's store is the silent-wrong-bytes failure the
/// whole address plane exists to refuse.
fn read_va(
    ce: &mut CePlane<'_>,
    vmm: &mut dyn Vmm,
    last: &mut Option<(GpuVa, CeResolve)>,
    va: u64,
    buf: &mut [u8],
) -> Result<(), FwdFault> {
    let mut at = 0usize;
    while at < buf.len() {
        let here = va + at as u64;
        let (op, left) = WalkOperands::new(ce, last).resolve_one(here)?;
        let take = (left as usize).min(buf.len() - at);
        match op.residency.plane {
            CpuPlane::Fb => ce
                .fb()
                .read(op.addr.0, &mut buf[at..at + take])
                .map_err(|e| FwdFault::CpuCeFb {
                    phys: e.phys,
                    why: e.why,
                })?,
            CpuPlane::GuestRam => vmm
                .gpa_read(op.addr.0, &mut buf[at..at + take])
                .map_err(ram_fault)?,
        }
        at += take;
    }
    Ok(())
}

/// ★★★ **Serve ONE guest doorbell on a VAS-less CeUtils channel.**
///
/// `ce` is an open [`kayfabe_device::RegPlane::ce_session`] for this channel's
/// `(hClient, hVASpace)`; `pb` is the chip's own pushbuffer codec — the same one the core
/// decodes every other channel with, not a second copy; `vmm` is the shell's guest-memory
/// port, which is where every sysmem operand of this channel lives
/// (`[measured 2026-08-08, boot run_p35_84d857d]`: ring, pushbuffer and finishPayload all
/// resolved to guest RAM).
///
/// # ★★★★ The `MethodState` is PER-CHANNEL, and it was submission-local until it walled
///
/// The engine's method accumulator is per-**channel** in hardware. This driver used to
/// start a fresh one per doorbell, on a reason that was true and insufficient: `[src]
/// ogkm-580: channel_utils.c:806-990` builds each **CeUtils** block whole —
/// `channelPushMemoryProperties`, the remap registers, `LINE_LENGTH_IN`, the semaphore
/// registers and `LAUNCH_DMA` — into one method block, every time, inheriting no operand.
/// So a per-doorbell reset is equivalent *for that one driver*, and the old comment here
/// named the exception itself: *"a channel whose driver latches once and fires many times
/// would need the per-channel state."*
///
/// ⊘ **UVM is that driver, and it is the one at the wall.** `[measured 2026-08-09, boot
/// s21_dbf853a_cup2]` the refused submission framed cleanly into seven methods —
/// `sub4/m0x400/n4` (the operand quad), `sub4/m0x418=0x20` (`LINE_LENGTH_IN`, 32 bytes),
/// `sub4/m0x300` (`LAUNCH_DMA`), `sub4/m0x240/n3` (the semaphore triple), `sub4/m0x300`
/// again — and carried **no `SET_OBJECT`**. UVM binds the CE class once, in
/// `channel_init`'s first push, and fires on `NVA06F_SUBCHANNEL_COPY_ENGINE = 4` forever
/// after. `MethodState::subchannel_speaks`' unbound arm requires the class to be bound
/// **somewhere in this state** (`kayfabe-arch/src/lib.rs:1234`), so a state rebuilt per
/// doorbell answers `false` for every push after the first: `ce_launch` returned `None`,
/// all seven methods decoded `Opaque`, and the submission reported no launch at all.
///
/// ⇒ the state is passed **in** and handed back in [`CeUtilsRun::state`], on exactly the
/// same commit-on-success discipline as [`CeUtilsRun::cursor`]: a refused submission
/// mutates nothing, so the guest's own retry re-reads and re-latches from where it was.
/// ⊘ This is not a relaxation — nothing is inferred that the guest did not write. It is
/// the *removal* of an amnesia our own decoder invented; the codec's un-latched refusals
/// (`§14.14`) still refuse every operand the guest genuinely never wrote.
///
/// # Errors
/// [`CeUtilsRefusal`], always by name. ⊘ No completion is written on any error path, and no
/// interrupt is raised — a doorbell that could not do the work must not look like one that
/// did (§14.8).
pub fn run_submission(
    ce: &mut CePlane<'_>,
    pb: &dyn PushbufferAbi,
    vmm: &mut dyn Vmm,
    chan: CeUtilsChannel,
    cursor: GpCursor,
    state: MethodState,
) -> Result<CeUtilsRun, CeUtilsRefusal> {
    let mut last: Option<(GpuVa, CeResolve)> = None;
    let mut run = CeUtilsRun {
        cursor,
        state,
        ..CeUtilsRun::default()
    };
    let entries = if chan.ring_entries == 0 {
        RING_ENTRIES_FALLBACK
    } else {
        chan.ring_entries
    };
    // ★ WHICH submission this doorbell is about, latched before the loop advances the
    // cursor. Carried into every refusal below so a reader is never left to assume entry 0.
    let start_index = cursor.next % entries;

    // ---- 1. THE RING. Read forward from our cursor while the entries decode. -----------
    let mut ranges: Vec<kayfabe_arch::PushRange> = Vec::new();
    for _ in 0..MAX_ENTRIES_PER_DOORBELL {
        let idx = run.cursor.next % entries;
        let at = chan
            .ring_va
            .wrapping_add(u64::from(idx) * GP_ENTRY_SIZE as u64);
        let mut raw = [0u8; GP_ENTRY_SIZE];
        read_va(ce, vmm, &mut last, at, &mut raw).map_err(|f| CeUtilsRefusal {
            fault: f,
            detail: last,
        })?;
        // ⊘ The ENTRY IS DECODED BY THE ARCH, not here: `PushbufferAbi::gpfifo_entries` is
        // the same codec `kayfabe_fwd::pushbuffer_ranges` uses for every other channel, and
        // a second decoder in the shell would be two descriptions of one wire format. Only
        // the entry's *stride* comes from `kayfabe_abi`, because indexing the ring needs a
        // byte offset and no `PushbufferAbi` method hands one out.
        //
        // An unwritten entry is zero and decodes to nothing — that is the ring saying "no
        // more work", not a malformed entry, because RM zero-initialises this buffer
        // (`TRANSFER_FLAGS_SHADOW_INIT_MEM`).
        let Some(r) = pb.gpfifo_entries(&raw).into_iter().next() else {
            break;
        };
        ranges.push(r);
        run.cursor.next = run.cursor.next.wrapping_add(1) % entries;
        run.entries += 1;
    }
    // ⊘ A doorbell that brought no readable entry is NOT served. The guest rang for work;
    // if we found none, saying "served" is exactly §14.8's silent no-op.
    //
    // ★★★ And it is named for WHAT HAPPENED. This used to raise
    // `FwdFault::PushTooFragmented { len: 0 }` — a bound of ours on how many address-table
    // spans one range may cut into — for a case in which there is no range to cut. `[measured
    // 2026-08-09, boots `uvm2_d0fbac0` / `scan1_00865a7` / `vaspan_994bbdc`]` four boots read
    // their wall as a fragmentation limit that was never reached. A wrong name is a false
    // diagnosis with a fix attached, which is worse than none.
    if ranges.is_empty() {
        return Err(CeUtilsRefusal::plain(FwdFault::RingBroughtNoEntry {
            ring_va: GpuVa(chan.ring_va),
            index: start_index,
            entries,
        }));
    }

    // ---- 2. THE METHOD WORDS, bounded per range and in total (boundary-1). -------------
    let mut methods: Vec<(u32, Vec<u32>)> = Vec::new();
    let mut total = 0usize;
    for r in &ranges {
        if total >= MAX_PUSH_TOTAL_BYTES {
            break;
        }
        let len = (r.len as usize)
            .min(MAX_PUSH_RANGE_BYTES)
            .min(MAX_PUSH_TOTAL_BYTES - total);
        let mut buf = vec![0u8; len];
        read_va(ce, vmm, &mut last, r.va.0, &mut buf).map_err(|f| CeUtilsRefusal {
            fault: f,
            detail: last,
        })?;
        methods.extend(decode_methods(pb, &buf));
        total += len;
    }
    run.methods = methods.len();

    // ---- 3. DECODE the run against the chip's own codec. -------------------------------
    // ⊘ Against the CHANNEL's accumulator, carried in and handed back — see this function's
    // docs for the boot in which rebuilding it per doorbell made every UVM push decode to
    // nothing.
    let decoded = pb.decode_run(&mut run.state, &methods);

    // ★★★★ THE CENSUS OF WHAT DECODED, taken BEFORE the execute loop consumes `decoded`.
    //
    // ⊘ Its only consumer is the `launches == 0` refusal below, and it exists because that
    // refusal used to throw every one of these facts away and report the literal
    // `ClassId(0)` instead (`FwdFault::SubmissionDecodedNoWork` carries the full account).
    // A submission that read no method words, one that read words the codec recognized
    // nothing in, and one that decoded a `SET_OBJECT` for some other engine are three
    // different findings that produced one identical line in the boot report.
    let opaque = u32::try_from(
        decoded
            .iter()
            .filter(|m| matches!(m, PushMethod::Opaque))
            .count(),
    )
    .unwrap_or(u32::MAX);
    // ⊘ The LAST `SET_OBJECT` in the block, because that is the one in force when the
    // methods after it are interpreted — the same order the engine applies them in.
    let set_object = decoded.iter().rev().find_map(|m| match m {
        PushMethod::SetObject { class } => Some(*class),
        _ => None,
    });

    // ★★★ §16.24's TRIPWIRE, and it bites BEFORE the execute loop below — deliberately.
    //
    // The admission of `GP100_UVM_SW` was bounded by *"this port raises no fault for UVM to
    // cancel"*. A fault method in these bytes is that bound expiring, and the one thing
    // that must NOT happen then is for the copies beside it to run while the cancel is
    // walked past: that would leave UVM believing a fault it tracks was cancelled, which is
    // a wrong answer rather than a missing one. So the whole submission is refused, by a
    // name that says which assumption failed, before a single byte moves.
    //
    // ⚠ Placed here rather than inside the loop so it cannot be reached *after* a partial
    // execution — the ordering property, not just the refusal, is what makes it safe.
    if let Some(method) = decoded.iter().find_map(|m| match m {
        PushMethod::UvmSwFaultMethod { method } => Some(*method),
        _ => None,
    }) {
        return Err(CeUtilsRefusal {
            fault: FwdFault::UvmFaultMethodWithoutFaultDelivery { method },
            detail: last,
        });
    }

    // ---- 4. EXECUTE each launch, then release its completion. ---------------------------
    for m in decoded {
        // ★★★ **A launch that transfers nothing and exists only to release** — and it is
        // EXECUTED here, in decode order, alongside the copies.
        //
        // # ⊘ Why this is not the forged completion this whole file refuses
        //
        // A forgery advances a payload for work that **did not run**. `DATA_TRANSFER_TYPE ==
        // NONE` is the guest saying there is no work: the engine moves zero bytes, so
        // writing the payload is not a claim *about* a copy — it **is** the entire act the
        // guest asked the engine to perform. ⊘ Contrast the `SemRelease` arm below, which
        // stays unacted-on for the opposite reason: that word is the *host*-FIFO semaphore
        // beside a real copy, and advancing it would report on a transfer.
        //
        // ⚠ It is inside the same loop, in the same order, so a release that follows a copy
        // in one ring is still written **after** that copy's bytes. Hoisting these would
        // reorder a fence the guest set (`ogkm-580: uvm_channel.c:1055-1060`).
        //
        // `[measured 2026-08-09, boots fmb1/msr1 at 319d29a]` `cuInit` hangs in
        // `uvm_push_end_and_wait`; the push it waits on is `channel_init`'s and contains
        // exactly one of these and nothing else.
        if let PushMethod::CeRelease { completion, .. } = m {
            run.launches += 1;
            run.releases += 1;
            let resolved = {
                let mut ops = WalkOperands::new(ce, &mut last);
                crate::cpu_ce::resolve_releases(&mut ops, &[completion]).map_err(|f| {
                    CeUtilsRefusal {
                        fault: f,
                        detail: last,
                    }
                })?
            };
            if let Some(r) = resolved.first() {
                run.completion_at = Some((r.va, r.op.residency.plane, r.op.addr.0));
            }
            // ★★★ §16.66 — the timestamp source, sampled HERE, once, at the moment this
            // release retires. ⊘ Not hoisted above the loop: a run may carry several
            // releases and hardware stamps each with the time IT completed, so one sample
            // shared across a run would report a single instant for a sequence of events —
            // the same shape as an end-of-boot census answering a lifetime question.
            let now_ns = ce.now_ns();
            run.completions +=
                crate::cpu_ce::write_resolved_completion(ce.fb(), vmm, &resolved, Some(now_ns))
                    .map_err(CeUtilsRefusal::plain)?;
            continue;
        }
        let PushMethod::CeLaunchDma {
            dst,
            src,
            len,
            dst_is_virtual,
            src_is_virtual,
            dst_target,
            src_target,
            work,
            completion,
        } = m
        else {
            // Everything else on this channel is state the codec already consumed
            // (`SetObject`, the latching runs) or something we do not model. ⊘ A
            // `SemRelease` is deliberately NOT acted on here: it is the *host* semaphore
            // four bytes below the finishPayload (§14.15), and advancing it would satisfy
            // our own counters while the guest spins on the word above it.
            continue;
        };
        run.launches += 1;
        let spans: Vec<CeSpan> = {
            let mut ops = WalkOperands::new(ce, &mut last);
            partition_ce(
                &mut ops,
                dst,
                dst_is_virtual,
                dst_target,
                src,
                src_is_virtual,
                src_target,
                len,
                work,
            )
            .map_err(|f| CeUtilsRefusal {
                fault: f,
                detail: last,
            })?
        };
        // ⊘ THE §14.8 GUARD. `execute_ours_spans` SKIPS a `HostCe` sub-copy — it is the
        // isolate's — and this driver has no isolate path. A silent skip here would move
        // some bytes, release the semaphore and report a completion over a copy that was
        // only partly done, which is the exact shape this increment exists not to produce.
        if let Some(bad) = spans.iter().find(|s| s.sub.by != CeExecutor::Ours) {
            return Err(CeUtilsRefusal::plain(FwdFault::CpuCeStraddle {
                dst: bad.sub.dst,
                dst_end: true,
            }));
        }
        run.spans += spans.len();
        crate::cpu_ce::execute_ours_spans(ce.fb(), vmm, &spans).map_err(CeUtilsRefusal::plain)?;
        run.bytes += spans.iter().map(|s| s.sub.len).sum::<u64>();

        // ---- 5. SIGNAL, and only now. --------------------------------------------------
        //
        // ⊘ `completion` is the **finishPayload** — the engine-class `SET_SEMAPHORE_A/B/
        // PAYLOAD` release carried INSIDE the launch (§14.15), at `pbGpuVA +
        // finishPayloadOffset`. `[measured 2026-08-08]` that is guest RAM `0x2f2c_b004` on
        // the walling channel, four bytes ABOVE the host-FIFO semaphore at `…b000` that
        // `PushMethod::SemRelease` decodes. `channelWaitForFinishPayload` spins on the
        // former. Advancing the latter would log a completion, satisfy our counters and
        // leave the guest spinning forever.
        let Some(c) = completion else {
            continue;
        };
        let resolved = {
            let mut ops = WalkOperands::new(ce, &mut last);
            crate::cpu_ce::resolve_releases(&mut ops, &[c]).map_err(|f| CeUtilsRefusal {
                fault: f,
                detail: last,
            })?
        };
        if let Some(r) = resolved.first() {
            run.completion_at = Some((r.va, r.op.residency.plane, r.op.addr.0));
        }
        let now_ns = ce.now_ns();
        run.completions +=
            crate::cpu_ce::write_resolved_completion(ce.fb(), vmm, &resolved, Some(now_ns))
                .map_err(CeUtilsRefusal::plain)?;
    }

    // ⊘ A submission that decoded no launch moved no byte. It is not served.
    //
    // ★★★★ AND IT IS NAMED FOR WHAT HAPPENED. This used to raise
    // `FwdFault::NotAnEngine(kayfabe_arch::ids::ClassId(0))` — an engine-class-lookup
    // failure, with the class written **as a literal on this line**, for a path that never
    // looks a class up. `[measured 2026-08-09, boot s19_1dfde1b_cup2]` the boot report read
    // `NotAnEngine(ClassId(0))` and sent its reader to find who was supposed to supply the
    // class. Nobody was: `route_engine_object` is the only site that resolves one, and it
    // is not on the doorbell path. ⊘ Same defect as the `PushTooFragmented { len: 0 }` that
    // `RingBroughtNoEntry` replaced, one layer later — a borrowed variant whose argument
    // then had to be invented, and an invented argument reads exactly like a measurement.
    if run.launches == 0 {
        return Err(CeUtilsRefusal::plain(FwdFault::SubmissionDecodedNoWork {
            entries: u32::try_from(run.entries).unwrap_or(u32::MAX),
            index: start_index,
            methods: u32::try_from(run.methods).unwrap_or(u32::MAX),
            opaque,
            set_object,
        }));
    }
    Ok(run)
}

/// Decode a byte range of method words into `(header, args)` pairs against `pb`. Total on
/// any input — a truncated or hostile range yields fewer methods, never a panic and never
/// an unbounded read.
///
/// ⊘ The same shape as `kayfabe_fwd`'s own private `decode_methods`, over the codec
/// directly rather than over an `Arch`, because this driver holds the codec and not the
/// spine. The framing rule — advance past at least the header, so a bogus count cannot
/// stall — is the property both must have and is asserted here in its own test.
fn decode_methods(pb: &dyn PushbufferAbi, bytes: &[u8]) -> Vec<(u32, Vec<u32>)> {
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")))
        .collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let header = words[i];
        let nargs = pb.method_len(header);
        let start = i + 1;
        let end = start.saturating_add(nargs).min(words.len());
        out.push((header, words[start..end].to_vec()));
        i = end.max(i + 1);
    }
    out
}

kayfabe_util::assert_send_sync!(GpCursor, CeUtilsChannel, CeUtilsRun, CeUtilsRefusal);

// =====================================================================================
// ★★★★★ §16.79 — THE METHOD STREAM, READ AND NOT INTERPRETED
// =====================================================================================

/// ★★★★★ **Dump the raw method headers of the submission this doorbell is about, WITHOUT
/// deciding what any of them mean.**
///
/// # Why this exists, and why it is not [`run_submission`]
///
/// `[measured 2026-08-10, boots `w215_79ed443_ctl` / `w216_f5f55ad_mcbudget`]` `cuCtxCreate`
/// waits on GrCompute channel token `0x00000007` and rings it 86 times, while `cup2`
/// performs **zero kernel launches** — no `cuModuleLoad`, no PTX, nothing to execute. Two
/// readings fit: the traffic is user compute, or it is **golden-context initialisation**
/// (context-buffer setup and a report semaphore). They ask for completely different rungs,
/// and ⊘ **the channel's class cannot tell them apart** — `AMPERE_COMPUTE_B` is allocated
/// during context creation whether or not anything is ever launched.
///
/// Only the method stream can. [`run_submission`] cannot be asked: it is the CE executor's
/// reader, it decodes against the CE codec, and on a GR ring every method comes back
/// [`kayfabe_arch::PushMethod::Opaque`] by class gating — *"no launch found"* out of a
/// decoder that could not have found one is not evidence.
///
/// # ⊘ What this function refuses to do
///
/// It does **not** decode, classify, or name a single method. It reports the header word,
/// the method address the header carries, and the argument words — as numbers. A reader
/// joins them to `clc7c0.h` offline. ⊘ That is the whole discipline: a dump that said
/// "SET_OBJECT" would be this port's opinion about a GR class it has no codec for, and the
/// question being asked is precisely whether our opinion of this stream is right.
///
/// It advances nothing: `cursor` is taken by value and no state is written back, so calling
/// this on a doorbell the port then refuses leaves the channel exactly as it was.
///
/// # Errors
/// [`CeUtilsRefusal`] when the ring or a pushbuffer range will not resolve — the same walk,
/// the same names, as [`run_submission`]'s.
pub fn dump_submission_methods(
    ce: &mut CePlane<'_>,
    pb: &dyn PushbufferAbi,
    vmm: &mut dyn Vmm,
    chan: CeUtilsChannel,
    cursor: GpCursor,
    max_methods: usize,
) -> Result<String, CeUtilsRefusal> {
    let mut last: Option<(GpuVa, CeResolve)> = None;
    let entries = if chan.ring_entries == 0 {
        RING_ENTRIES_FALLBACK
    } else {
        chan.ring_entries
    };
    let start_index = cursor.next % entries;
    let mut next = cursor.next;

    // ---- 1. THE RING — the same read, the same arch decoder, as `run_submission`. -------
    let mut ranges: Vec<kayfabe_arch::PushRange> = Vec::new();
    for _ in 0..MAX_ENTRIES_PER_DOORBELL {
        let idx = next % entries;
        let at = chan
            .ring_va
            .wrapping_add(u64::from(idx) * GP_ENTRY_SIZE as u64);
        let mut raw = [0u8; GP_ENTRY_SIZE];
        read_va(ce, vmm, &mut last, at, &mut raw).map_err(|f| CeUtilsRefusal {
            fault: f,
            detail: last,
        })?;
        let Some(r) = pb.gpfifo_entries(&raw).into_iter().next() else {
            break;
        };
        ranges.push(r);
        next = next.wrapping_add(1) % entries;
    }
    if ranges.is_empty() {
        return Err(CeUtilsRefusal::plain(FwdFault::RingBroughtNoEntry {
            ring_va: GpuVa(chan.ring_va),
            index: start_index,
            entries,
        }));
    }

    // ---- 2. THE METHOD WORDS — bounded exactly as the executor's are. -------------------
    let mut methods: Vec<(u32, Vec<u32>)> = Vec::new();
    let mut total = 0usize;
    for r in &ranges {
        if total >= MAX_PUSH_TOTAL_BYTES {
            break;
        }
        let len = (r.len as usize)
            .min(MAX_PUSH_RANGE_BYTES)
            .min(MAX_PUSH_TOTAL_BYTES - total);
        let mut buf = vec![0u8; len];
        read_va(ce, vmm, &mut last, r.va.0, &mut buf).map_err(|f| CeUtilsRefusal {
            fault: f,
            detail: last,
        })?;
        methods.extend(decode_methods(pb, &buf));
        total += len;
    }

    // ---- 3. THE REPORT — numbers only. -------------------------------------------------
    let mut out = format!(
        "ring=0x{:x} idx={start_index} entries={entries} ranges={} methods={} bytes={total}",
        chan.ring_va,
        ranges.len(),
        methods.len(),
    );
    for r in &ranges {
        out.push_str(&format!(" | range va=0x{:x} len={}", r.va.0, r.len));
    }
    for (n, (header, args)) in methods.iter().take(max_methods).enumerate() {
        // ★ The method ADDRESS as the class headers write it: the header's low 13 bits are
        // the address in dwords, so `<< 2` is the byte offset a `clc7c0.h` `#define` states.
        // ⊘ Arithmetic on the header, not an interpretation of it — the only claim here is
        // that this arch packs the address in those bits, which `PushbufferAbi::method_len`
        // already relies on to walk the stream at all.
        let addr = (header & 0x1fff) << 2;
        // ★★★★★ §16.79.2 — THE SUBCHANNEL, and omitting it was a real defect in this
        // dump's first run. A method address means nothing on its own: the address space is
        // **per subchannel**, and the subchannel is bound by whichever `SET_OBJECT` came
        // before. `[measured 2026-08-10, boot w218_cb6adcc_grfull]` this very pushbuffer
        // binds `AMPERE_COMPUTE_B` to subchannel 1 and `AMPERE_DMA_COPY_B` to subchannel 4
        // in one stream, and the two class headers COLLIDE on the addresses it uses most:
        // `NVC7C0_SET_CWD_REF_COUNTER = 0x0248` is `NVC7B5_SET_SEMAPHORE_PAYLOAD`, and
        // `NVC7C0_INVALIDATE_TEXTURE_HEADER_CACHE_NO_WFI = 0x0244` is
        // `NVC7B5_SET_SEMAPHORE_B`. ⊘ A reader handed `addr=0x0248` and no subchannel cannot
        // tell a compute counter from a copy-engine semaphore payload — which is the single
        // most dangerous confusion available on this wire.
        let subch = (header >> 13) & 0x7;
        let count = (header >> 16) & 0x1fff;
        let secop = header >> 29;
        out.push_str(&format!(
            "\n      m[{n:03}] hdr=0x{header:08x} sub={subch} addr=0x{addr:04x} count={count} secop={secop} args=["
        ));
        for (i, a) in args.iter().take(8).enumerate() {
            out.push_str(&format!("{}0x{a:08x}", if i == 0 { "" } else { "," }));
        }
        if args.len() > 8 {
            out.push_str(&format!(",..+{}", args.len() - 8));
        }
        out.push(']');
    }
    if methods.len() > max_methods {
        out.push_str(&format!(
            "\n      ⊘ BOUNDED-DUMP: {} further method(s) not shown",
            methods.len() - max_methods
        ));
    }
    Ok(out)
}
