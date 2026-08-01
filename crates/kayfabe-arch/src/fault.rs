//! Axis-B seam: **what a fault IS**, abstractly, and who encodes it as numbers.
//!
//! `docs/design/simulated_gpu_fault.md`. The core decides *that* an access faulted and
//! *why*, in the vocabulary below. Turning "why" into the integer a driver reads is a
//! fact about one GPU generation's MMU, so it is a trait here and a table in an
//! arch-impl crate — rule 2, the same shape as [`crate::GmmuFmt`].
//!
//! ## ★ Why this is a separate trait and not two more methods on [`crate::Arch`]
//!
//! [`crate::Arch`] is reached through a `Box<dyn Arch>` that the device owns, and the
//! fault emitter does not have one: it runs off a refused *plan*, which carries an
//! engine and an address and no architecture. Threading an `Arch` into it to read two
//! integers would widen the emitter's inputs to something it does not otherwise need,
//! and every existing `Arch` impl would grow two methods for one caller. A narrow seam
//! that one adapter implements is the smaller true statement.

/// **Why** an access could not be translated — the abstract cause.
///
/// Deliberately a small, closed set: the emitter can only honestly report a cause it
/// actually distinguished, and today it distinguishes one. Every variant here is
/// reachable from a *fact this port holds*, never from a guess about silicon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MmuFaultCause {
    /// **Nothing is mapped at this virtual address.** The address plane's
    /// `AddressFault::Miss` — the guest's own page tables would have missed too, because
    /// the address table IS the guest's TLB (`kayfabe_mmu`'s crate docs).
    ///
    /// ★ Reported as the *page-directory* miss rather than the page-table one. The
    /// distinction the hardware draws is "the walk ran out at a directory" vs "it reached
    /// a leaf and the leaf was invalid", and this port does not have that fact: the
    /// address table records bindings, not the level a hypothetical walk would have died
    /// at. Claiming the leaf variant would be inventing a walk that never happened, and
    /// both are fatal to the same channel in the same way.
    NothingMapped,
    /// The mapping exists but forbids this access's direction (a write to a read-only
    /// binding). Not produced by any site today — declared because the encoding table
    /// must be total over the vocabulary, not over the vocabulary's current callers.
    PermissionViolation,
}

/// **What kind** of access faulted.
///
/// All three are *virtual* accesses. A physical-access fault is deliberately not in this
/// vocabulary: this port never hands the guest a physical operand it did not itself
/// derive, so a physical fault would be our bug and must not be dressed as the guest's
/// (`docs/design/simulated_gpu_fault.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MmuFaultAccess {
    /// A read.
    Read,
    /// A write.
    Write,
    /// An atomic.
    Atomic,
}

/// ★★★ **Where the guest asked to be TOLD that its channel died.**
///
/// A channel declares an *error notifier* at allocation time (`hObjectError` on
/// `NV_CHANNEL_ALLOC_PARAMS`), and the guest's CPU-RM forwards its resolved backing to
/// the GSP in the same message — `errorNotifierMem`, a physical address, plus the
/// notifier's kind in `internalFlags[3:2]`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:548-570`,
/// `ogkm-580: src/nvidia/generated/g_kernel_channel_nvoc.h:185-189`). **In Mode 2 we are
/// the GSP, so that message is addressed to us and this is the fact it carries.**
///
/// # ★★ Why this is not optional decoration on an RC event
///
/// With Confidential Compute **off** — our target — CPU-RM does **not** write the
/// notifier when it receives `RC_TRIGGERED`; the conditional is explicit
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:657-668`):
///
/// ```text
///     bIsCcEnabled = gpuIsCCFeatureEnabled(pGpu);
///     // With CC enabled, CPU-RM needs to write error notifiers
///     if (bIsCcEnabled && pKernelChannel != NULL) { krcErrorSetNotifier(...); }
/// ```
///
/// and RM says so in prose on the very function it calls instead: *"except in the
/// GSP_CLIENT path where GSP has already written to the notifiers"*
/// (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:85-90`). The
/// consumer this port can read then spins forever on the zero: `uvm_channel_get_status`
/// returns `NV_OK` while `error_notifier->status == 0`
/// (`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:2058-2082`), and its callers are
/// `while (1)` loops that **ignore** `UVM_SPIN_LOOP`'s timeout return
/// (`uvm_channel.c:603-627`, `:660-676`; `kernel-open/nvidia-uvm/uvm_common.h:288-298`).
///
/// ⇒ Sending the event without writing this is a **hang generator** — the exact failure
/// mode task #111 exists to remove. So a fault this port cannot write a notifier for is
/// not emitted at all; see `kayfabe_core::fault::NotifierGap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorNotifier {
    /// The notifier lives in **guest RAM** at this guest-physical address, and this port
    /// has a write port for it.
    ///
    /// The address is `errorNotifierMem.base`, which CPU-RM already added the notifier's
    /// own offset into (`kernel_channel.c:557-560`), so index 0 of the
    /// `NV_CHANNELGPFIFO_NOTIFICATION_TYPE_ERROR` array is exactly here — no further
    /// arithmetic, and none invented.
    Sysmem {
        /// Guest-physical address of the 16-byte notification record.
        gpa: u64,
    },
    /// The channel declared one, and it is somewhere this port has **no write port
    /// for** — device memory, or an aperture the ABI seam does not model.
    ///
    /// ★ Kept as its own variant rather than collapsed into "no notifier". They are
    /// different findings: *"the guest waived error reporting"* and *"the guest asked to
    /// be told and we cannot reach the place"* lead a reader to different files, and the
    /// second is a gap in **us**.
    Unreachable,
}

/// Axis-B: the per-generation encoding of [`MmuFaultCause`] / [`MmuFaultAccess`].
///
/// **Total by construction.** Both methods return a `u32` and not an `Option`, because
/// every generation this port can target has an encoding for every variant above — the
/// vocabulary was chosen from the intersection, not from one chip's list. A generation
/// that genuinely lacks one would have to drop a variant from the vocabulary, which is a
/// change a reviewer sees; silently returning "no code" would produce an event the
/// receiver rejects, and the receiver rejects it *quietly*
/// (`docs/design/simulated_gpu_fault.md` §5).
///
/// `Send + Sync`: an encoding table is immutable shared data (crate docs, decision #17).
pub trait MmuFaultCodes: Send + Sync {
    /// The `NV_PFAULT_FAULT_TYPE_*` code for `cause`.
    fn fault_type(&self, cause: MmuFaultCause) -> u32;
    /// The `NV_PFAULT_ACCESS_TYPE_*` code for `access`.
    fn access_type(&self, access: MmuFaultAccess) -> u32;
}
