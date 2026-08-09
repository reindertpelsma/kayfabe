//! ★★★ **E10c — the SHELL's CPU copy-engine executor** (`execution_plane_increments.md`
//! §14, the CPU branch of the CE decision tree).
//!
//! A [`CeExecutor::Ours`] sub-copy names *fabricated* space no real engine can be pointed
//! at — the emulated framebuffer, or a physical-mode operand in guest RAM. The isolate
//! **cannot** run such a copy and refuses it (`kayfabe_isolate_host::rm::ce_copy` →
//! `NOT_ON_THIS_RUNG`): it is a separate sandboxed process that deliberately holds neither
//! the emulated framebuffer nor guest RAM. That is the security posture working, not a gap.
//! So the copy is executed **here, in the shell**, against the two memory planes the shell
//! owns — the [`FbStore`] and the [`Vmm`] — with the store chosen by the E10b residency
//! answer ([`CpuPlane`]) each operand carries, never by the address value.
//!
//! ⊘ **Why this is SAFE code, not a `*_unsafe.rs` file** (`l1_os_shell.md` §4.2.1.1). This
//! module computes guest-physical and framebuffer-physical addresses by offset arithmetic,
//! but it never dereferences a raw pointer and never holds a `&[u8]` over live guest memory:
//! every access is a **bounded copy into an owned buffer through the [`Vmm`]/[`FbStore`]
//! trait, which re-validates the address against bounds the audited raw layer owns.** That is
//! precisely §4.2.1.1's sanctioned pattern — *"the first response to soundness-critical safe
//! code is to make the raw side re-validate, not to rename the file"* — so the unsound
//! surface stays in the audited crates (`kayfabe-linux-raw`/`kayfabe-qemu-raw`, the real
//! `Vmm`), and this executor is ordinary safe code on top of it (the §4.1 containment gate).
//!
//! ★ **This is forwarding, not forgery** (`mode2_forwarding_model.md`: *correctness =
//! observable end-states only*): a different executor producing the TRUE end-state is
//! forwarding. What is forbidden — and what this module never does — is signalling a copy
//! that did not move the bytes, or landing them where the guest cannot read them. The bytes
//! move first; E10d signals only after.

use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{Backing, CpuOperand, CpuPlane, PlaneAddr};
use kayfabe_device::{FbRefused, FbStore, FbWriter};
use kayfabe_fwd::{COMPLETION_VECTOR, CeSpan, FwdFault};
use kayfabe_isolate::{CeExecutor, CeSource};
use kayfabe_vmm::{Vmm, VmmError};

/// The bounded staging-buffer size for one copy step. A guest controls a copy's length, so
/// the executor never allocates it whole — it streams the copy through a fixed buffer, so a
/// hostile multi-gigabyte request costs a fixed 64 KiB of host memory, not its own size
/// (boundary-1 posture, the same reason [`kayfabe_fwd::read_pushbuffer`] clamps).
const CHUNK: usize = 64 * 1024;

/// Map a framebuffer-store refusal to the named CPU-CE fault.
fn fb_fault(e: FbRefused) -> FwdFault {
    FwdFault::CpuCeFb {
        phys: e.phys,
        why: e.why,
    }
}

/// Map a guest-RAM refusal to the named fault — the SAME split
/// [`kayfabe_fwd`]'s own `guest_read` makes, so a device-aimed operand and an unbacked one
/// stay distinguishable one layer over (`testing_doctrine.md` §2 rule 3).
fn ram_fault(e: VmmError) -> FwdFault {
    match e {
        VmmError::NonRamGpa { gpa } => FwdFault::NonRamGpa { gpa },
        VmmError::BadGpa { gpa } => FwdFault::GpaRead { gpa },
        _ => FwdFault::GpaRead {
            gpa: u64::MAX, // no address the port named; the variant carries none
        },
    }
}

/// ★★★ **The plane a residency answer permits us to touch — or a named refusal.**
///
/// # ⚠ The backing check is the point, and it is load-bearing today
///
/// A CPU copy assumes its operands hold still for its duration. [`Backing::Stable`] says
/// they do — the emulated framebuffer is ours outright and guest RAM is held by the
/// hypervisor for the life of the machine. [`Backing::HostOwned`] says they do **not**: the
/// managed-memory case, where the backing is a host `cudaMallocManaged` allocation and host
/// UVM may migrate it below the guest's addressing (`mode2_uvm_residency.md`, DECIDED
/// 2026-06-04, measured at host parity in the C artifact).
///
/// ⊘ **Refusing it is not a claim the case is impossible** — the C ran it. It is a claim
/// that *this port has built no interlock*, and copying from a page that may move mid-copy
/// is a silent wrong-bytes generator. When the interlock lands, the fix is here and the
/// executor's shape is unchanged, which is the whole reason the fact is a **field** rather
/// than a plane variant.
///
/// ★ Note the plane itself is unchanged for a host-owned backing: managed memory is still
/// guest RAM read through the `Vmm`. Place and ownership are independent, which is why they
/// are two fields.
fn touchable(op: CpuOperand) -> Result<CpuPlane, FwdFault> {
    match op.residency.backing {
        Backing::Stable => Ok(op.residency.plane),
        Backing::HostOwned => Err(FwdFault::CeUnstableBacking { addr: op.addr.0 }),
    }
}

/// Read `buf.len()` bytes from `plane` at [`PlaneAddr`] `addr` into `buf`.
///
/// ⊘ **`addr` is a [`PlaneAddr`], never a `u64`,** and that signature is load-bearing rather
/// than decorative: the whole of §14.14's REFUTED 4 is this function having been reachable
/// with a GPU virtual address, which for a physical operand is the same number and for a
/// virtual one is a different page.
fn read_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: PlaneAddr,
    buf: &mut [u8],
) -> Result<(), FwdFault> {
    match plane {
        CpuPlane::Fb => fb.read(addr.0, buf).map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_read(addr.0, buf).map_err(ram_fault),
    }
}

/// Write `bytes` to `plane` at [`PlaneAddr`] `addr`. See [`read_plane`] for why the type of
/// `addr` is the point.
fn write_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: PlaneAddr,
    bytes: &[u8],
) -> Result<(), FwdFault> {
    match plane {
        // ★ §16.16 — the shell's CPU copy executor NAMES ITSELF. Without this the executor's
        // pages were indistinguishable from a window's in the first-writer census, and
        // "which window put the ring's neighbours there" is the whole question the census
        // is for. See `kayfabe_device::FbWriter`.
        CpuPlane::Fb => fb
            .write_tagged(addr.0, bytes, FbWriter::Executor)
            .map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_write(addr.0, bytes).map_err(ram_fault),
    }
}

/// ★★★ **Execute ONE `CeExecutor::Ours` sub-copy in the shell.**
///
/// The three cases, all address-driven:
/// - **fill / scrub** ([`CeSource::Constant`]) — writes a byte-pattern into the destination
///   plane, its phase taken from the **absolute destination address** (`pattern_le[a % 4]`),
///   which is what makes a split fill byte-identical to a whole one (the same rule
///   `kayfabe_mocks`'s reference `ce_apply` and the C's remap component follow). A scrub is
///   the `pattern == 0` case.
/// - **copy** ([`CeSource::Address`]) — streams bytes from the source plane to the
///   destination plane through a bounded buffer.
///
/// # Errors
/// - [`FwdFault::CpuCeStraddle`] if a plane this sub-copy must touch is `None` — a straddle
///   the shell cannot span (one end is real device memory or untracked). Refused, never
///   guessed into a store.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] if a store
///   refuses an access.
///
/// # Panics
/// Debug-asserts the sub-copy is `Ours`; a `HostCe` sub-copy is the isolate's and must never
/// reach here (the divert in `SharedDevice::forward_ce` is what keeps that true).
pub fn execute_ours(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    span: &CeSpan,
) -> Result<(), FwdFault> {
    debug_assert_eq!(
        span.sub.by,
        CeExecutor::Ours,
        "the CPU executor runs only Ours sub-copies; HostCe is the isolate's"
    );
    let dst = span.sub.dst;
    let len = span.sub.len;
    // ★★★ The destination **place** must exist for anything to be written — and it is the
    // place, not `sub.dst`, that says where. `sub.dst` is a GPU virtual address (the `HostCe`
    // arm submits it to a host VAS); this executor reaches bytes in *our* stores and has no
    // MMU, so it uses the address the partition resolved (§14.14 REFUTED 4).
    let dst_op = span
        .dst_place
        .ok_or(FwdFault::CpuCeStraddle { dst, dst_end: true })?;
    let dst_plane = touchable(dst_op)?;

    match span.sub.src {
        CeSource::Constant(pattern) => {
            let p = pattern.to_le_bytes();
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                // Phase every byte by its ABSOLUTE address in the destination PLANE, not by
                // its offset within this chunk — a fill cut at an unaligned address must
                // still land the same bytes, which is what makes a split fill byte-identical
                // to a whole one.
                //
                // ★ Phasing by the plane address rather than by the VA is the same answer
                // wherever the engine's own is defined: a mapping is page-granular, so a VA
                // and its backing agree on the low bits, and `Ga10xPushbuffer::remap_fill`
                // refuses a fill whose destination is not element-aligned in the first place
                // (§14.14's sixth refusal). Where they could differ — a test binding with an
                // arbitrary `phys` — the plane address is the one the guest will read back.
                for (i, b) in chunk[..take].iter_mut().enumerate() {
                    let a = dst_op.addr.offset(off + i as u64);
                    *b = p[(a.0 % 4) as usize];
                }
                write_plane(fb, vmm, dst_plane, dst_op.addr.offset(off), &chunk[..take])?;
                off += take as u64;
            }
            Ok(())
        }
        CeSource::Address(_) => {
            // A real copy needs BOTH ends reachable; a missing source place is the same
            // straddle, named on the source end.
            let src_op = span.src_place.ok_or(FwdFault::CpuCeStraddle {
                dst,
                dst_end: false,
            })?;
            let src_plane = touchable(src_op)?;
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                read_plane(
                    fb,
                    vmm,
                    src_plane,
                    src_op.addr.offset(off),
                    &mut chunk[..take],
                )?;
                write_plane(fb, vmm, dst_plane, dst_op.addr.offset(off), &chunk[..take])?;
                off += take as u64;
            }
            Ok(())
        }
    }
}

/// Execute every `Ours` sub-copy of `spans` in submission order, skipping `HostCe` ones (the
/// isolate's). Returns the count actually run.
///
/// ★ **Submission order is preserved** because a copy engine's within-request ordering is
/// what the guest's own semaphore release depends on. This runs the shell's share; the
/// caller ([`crate::device::SharedDevice::forward_ce`]) interleaves the isolate's `HostCe`
/// share and places the E10d completion signal only after **all** of a request's bytes —
/// both shares — are in place.
///
/// # Errors
/// The first sub-copy that refuses stops the run and propagates, exactly as the isolate's
/// [`kayfabe_isolate::VerbPlan::CeSplit`] loop stops on the first refusal: a later sub-copy
/// must not assume an earlier one landed.
pub fn execute_ours_spans(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    spans: &[CeSpan],
) -> Result<usize, FwdFault> {
    let mut ran = 0usize;
    for span in spans {
        if span.sub.by == CeExecutor::Ours {
            execute_ours(fb, vmm, span)?;
            ran += 1;
        }
    }
    Ok(ran)
}

// =====================================================================================
// ★★★ E10d — THE COMPLETION WRITE-BACK TAIL. There is NONE today: the finishPayload
// semaphore the guest polls is written nowhere (`execution_plane_increments.md` §14.4(3),
// §14.5 E10d). This is the `sem_releases` consumer.
// =====================================================================================

/// ★★★ **One finishPayload release, with its address already RESOLVED to a plane
/// address** — the output of [`resolve_releases`] and the input of
/// [`write_resolved_completion`].
///
/// # ⊘ Why the split exists, and it is a borrow fact rather than a style one
///
/// `write_completion` used to resolve and write in one pass, which is right when the
/// resolver and the destination store are different objects. On the CeUtils path they are
/// **the same object**: the resolver walks the guest's page tables out of the emulated
/// framebuffer, and the executor writes the guest's bytes into that same framebuffer
/// (`execution_plane_increments.md` §14.13 — the walling channel's page directories are
/// vidmem). One `&mut` cannot be held twice, so the resolution is a **phase** and its
/// result is a value.
///
/// ★ The ordering the split must not lose is *resolve → write → signal*, and it does not:
/// [`write_resolved_completion`] still writes every payload before raising anything, and
/// raises nothing at all on a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedRelease {
    /// The virtual address the guest's pushbuffer named, kept for the report.
    pub va: GpuVa,
    /// Where that address lives, and at what address in that plane.
    pub op: CpuOperand,
    /// The payload the release carries. Written as its low 32 bits (a one-word release).
    pub payload: u64,
}

/// ★★★ **Phase 1 of the completion tail — resolve every release's VA through the operand
/// resolver, refusing by name rather than writing anywhere.**
///
/// # Errors
/// - Whatever the resolver refuses with — [`FwdFault::Address`] for a table miss (MISS =
///   FAULT: a completion aimed at nothing is not written and does not signal),
///   [`FwdFault::CeNoTable`] where there is no address space to miss in, or the walk's own
///   named fault.
/// - [`FwdFault::CePeerOperand`] if it resolves into peer memory.
pub fn resolve_releases(
    ops: &mut dyn kayfabe_fwd::OperandResolver,
    releases: &[(GpuVa, u64)],
) -> Result<Vec<ResolvedRelease>, FwdFault> {
    releases
        .iter()
        .map(|&(va, payload)| {
            Ok(ResolvedRelease {
                va,
                op: ops.resolve_word(va)?,
                payload,
            })
        })
        .collect()
}

/// ★★★ **Phase 2 of the completion tail — write every resolved payload, and only then
/// raise the completion interrupt.**
///
/// See [`write_completion`] for the `#12` argument this ordering is.
///
/// # Errors
/// - [`FwdFault::CeUnstableBacking`] if a release's backing is not ours to hold still.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] on a store
///   refusal. In every error case **no interrupt is raised**.
pub fn write_resolved_completion(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    releases: &[ResolvedRelease],
) -> Result<usize, FwdFault> {
    // 1. WRITE every payload first. A refusal here returns before any signal.
    for r in releases {
        let plane = touchable(r.op)?;
        // One-word (4-byte) release: the low 32 bits, little-endian.
        let bytes = (r.payload as u32).to_le_bytes();
        write_plane(fb, vmm, plane, r.op.addr, &bytes)?;
    }
    // 2. SIGNAL — only now, and only because every write above returned Ok. The guest's
    //    poll of `pbCpuVA` already sees the values; the interrupt wakes a blocking waiter.
    if !releases.is_empty() {
        vmm.raise_irq(COMPLETION_VECTOR).map_err(ram_fault)?;
    }
    Ok(releases.len())
}

/// ★★★ **Write a copy-engine request's finishPayload semaphores where the guest polls
/// them, then — and only then — raise the completion interrupt.**
///
/// Each `(addr, payload)` is a `SEM_RELEASE` the guest's own pushbuffer asked for. `addr`
/// is a GPU **virtual** address in the *channel's own VAS* — `pChannel->pbGpuVA +
/// finishPayloadOffset` (`ogkm-580: channel_utils.c:838-840`) — and the guest reads the
/// same physical allocation back through its CPU mapping (`pbCpuVA`). So the payload is
/// written to whatever that VA resolves to **in this channel's table**, in the channel's own
/// aperture — never to a scratch framebuffer page the guest does not look at.
///
/// # ⊘ #12, the bug this ordering exists to avoid
///
/// The C artifact's `#12` wrote completion **data to the framebuffer while the guest polled
/// `pbCpuVA`** — a semaphore that advanced somewhere the guest never read — and it cost
/// weeks. Two disciplines here are that lesson, made structural:
/// 1. **Where** — the payload lands at the resolved physical of the guest's own semaphore
///    VA, so `pbCpuVA` sees it.
/// 2. **When** — the interrupt is raised **after every byte is in place**, and **not at
///    all** if any write refuses. A completion signal for work whose result is not yet
///    visible is exactly the forgery `mode2_forwarding_model.md` forbids.
///
/// The finishPayload is a **one-word** (4-byte) release
/// (`ogkm-580: channel_utils.c:732` `_RELEASE_SIZE_4BYTE`, `:836` `_RELEASE_ONE_WORD`), so
/// the low 32 bits are written and the adjacent `authTagBufSema` (`:` `finishPayloadOffset +
/// CHANNEL_ENGINE_SEMAPHORE_SIZE`) is never clobbered by an over-wide store.
///
/// # Errors
/// - [`FwdFault::Address`] if a semaphore VA does not resolve in the channel's table
///   (MISS = FAULT — a completion aimed at nothing is not written and does not signal).
/// - [`FwdFault::CePeerOperand`] if it resolves into peer memory.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] on a store
///   refusal. In every error case **no interrupt is raised** — the writes that DID land are
///   the guest's own memory and are harmless, but no completion is claimed.
///
/// Returns the number of semaphores written on success.
pub fn write_completion(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    ops: &mut dyn kayfabe_fwd::OperandResolver,
    releases: &[(GpuVa, u64)],
) -> Result<usize, FwdFault> {
    let resolved = resolve_releases(ops, releases)?;
    write_resolved_completion(fb, vmm, &resolved)
}
