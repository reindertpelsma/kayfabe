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
//! ⊘ **Why `_unsafe.rs` with no `unsafe` keyword** (`l1_os_shell.md` §4.2.1.1, third
//! construct): this file does raw *address arithmetic over guest memory* — it forms
//! guest-physical and framebuffer-physical addresses by offset and moves bytes between two
//! stores the guest can write concurrently. No raw pointer is dereferenced (every access is
//! a bounded copy through the [`Vmm`]/[`FbStore`] trait, which validates the address it is
//! given), so there is no `unsafe {}`; but the *reasoning* is the raw-surface kind the
//! naming rule names, so the file is named for it.
//!
//! ★ **This is forwarding, not forgery** (`mode2_forwarding_model.md`: *correctness =
//! observable end-states only*): a different executor producing the TRUE end-state is
//! forwarding. What is forbidden — and what this module never does — is signalling a copy
//! that did not move the bytes, or landing them where the guest cannot read them. The bytes
//! move first; E10d signals only after.

use kayfabe_arch::CpuPlane;
use kayfabe_device::{FbRefused, FbStore};
use kayfabe_fwd::{CeSpan, FwdFault};
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

/// Read `buf.len()` bytes from `plane` at physical/guest address `addr` into `buf`.
fn read_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: u64,
    buf: &mut [u8],
) -> Result<(), FwdFault> {
    match plane {
        CpuPlane::Fb => fb.read(addr, buf).map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_read(addr, buf).map_err(ram_fault),
    }
}

/// Write `bytes` to `plane` at physical/guest address `addr`.
fn write_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: u64,
    bytes: &[u8],
) -> Result<(), FwdFault> {
    match plane {
        CpuPlane::Fb => fb.write(addr, bytes).map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_write(addr, bytes).map_err(ram_fault),
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
    // The destination plane must exist for anything to be written.
    let dst_plane = span.dst_plane.ok_or(FwdFault::CpuCeStraddle {
        dst,
        dst_end: true,
    })?;

    match span.sub.src {
        CeSource::Constant(pattern) => {
            let p = pattern.to_le_bytes();
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                // Phase every byte by its ABSOLUTE destination address, not by its offset
                // within this chunk — a fill cut at an unaligned address must still land
                // the same bytes.
                for (i, b) in chunk[..take].iter_mut().enumerate() {
                    let a = dst.wrapping_add(off + i as u64);
                    *b = p[(a % 4) as usize];
                }
                write_plane(fb, vmm, dst_plane, dst.wrapping_add(off), &chunk[..take])?;
                off += take as u64;
            }
            Ok(())
        }
        CeSource::Address(src) => {
            // A real copy needs BOTH ends reachable; a missing source plane is the same
            // straddle, named on the source end.
            let src_plane = span.src_plane.ok_or(FwdFault::CpuCeStraddle {
                dst,
                dst_end: false,
            })?;
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                read_plane(
                    fb,
                    vmm,
                    src_plane,
                    src.wrapping_add(off),
                    &mut chunk[..take],
                )?;
                write_plane(fb, vmm, dst_plane, dst.wrapping_add(off), &chunk[..take])?;
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
