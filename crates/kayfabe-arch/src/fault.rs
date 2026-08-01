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
