//! The compiler-checked bridges from this crate's vocabulary to `kayfabe-trace`'s.
//!
//! `kayfabe-trace` sits **below** this crate in the lattice (so that every plane can
//! emit into it without a cycle), which means it cannot name [`RmEvent`], [`ProcId`] or
//! [`RmGraphError`]. The bridge is therefore written here — and it is written as an
//! **exhaustive `match`**, not a `Display` impl or a `_ =>` arm, so that adding a variant
//! to any of those three types fails the build until the trace vocabulary is told about
//! it. `testing_doctrine.md` §6 rule 1: prefer a mechanism to a sentence.
//!
//! Nothing in this module emits. It converts. The decision about *where* the core emits
//! is the adapter's, and threading a `Trace` argument through the plane entry points is
//! a separate, deliberate step (see `kayfabe-trace`'s crate docs).

use kayfabe_trace::{AsRmVerb, FaultTag, Faulted, ProcRef, RmVerb};

use crate::ProcId;
use crate::gpu::GpuError;
use crate::rmgraph::{RmEvent, RmGraphError};

impl From<ProcId> for ProcRef {
    fn from(p: ProcId) -> ProcRef {
        ProcRef(p.0)
    }
}

impl AsRmVerb for RmEvent {
    fn as_rm_verb(&self) -> RmVerb {
        match *self {
            RmEvent::Alloc {
                parent,
                class,
                client: _,
                handle: _,
                facts: _,
            } => RmVerb::Alloc { class, parent },
            RmEvent::Dup { src, dst: _ } => RmVerb::Dup {
                src_client: src.client,
                src: src.handle,
            },
            RmEvent::SetPageDir { pdb, .. } => RmVerb::SetPageDir { pdb },
            RmEvent::MapMemoryDma {
                memory,
                va,
                len,
                client: _,
                vaspace: _,
                offset: _,
            } => RmVerb::MapMemoryDma { memory, va, len },
            RmEvent::Unmap { va, .. } => RmVerb::Unmap { va },
            RmEvent::Free { .. } => RmVerb::Free,
        }
    }
}

impl Faulted for RmGraphError {
    fn fault_tag(&self) -> FaultTag {
        match self {
            RmGraphError::ConflictingAlloc(_) => FaultTag("RmGraphError::ConflictingAlloc"),
            RmGraphError::ConflictingDup(_) => FaultTag("RmGraphError::ConflictingDup"),
            RmGraphError::InvalidDeviceInstance { .. } => {
                FaultTag("RmGraphError::InvalidDeviceInstance")
            }
            RmGraphError::ConflictingMap { .. } => FaultTag("RmGraphError::ConflictingMap"),
            RmGraphError::UndeclaredClientKind(_) => FaultTag("RmGraphError::UndeclaredClientKind"),
            RmGraphError::ReservedClient(_) => FaultTag("RmGraphError::ReservedClient"),
            RmGraphError::DuplicateClientRoot { .. } => {
                FaultTag("RmGraphError::DuplicateClientRoot")
            }
            RmGraphError::UndeclaredClient(_) => FaultTag("RmGraphError::UndeclaredClient"),
            RmGraphError::FreeUnknown(_) => FaultTag("RmGraphError::FreeUnknown"),
            RmGraphError::CapacityExceeded(_) => FaultTag("RmGraphError::CapacityExceeded"),
        }
    }
}

impl Faulted for GpuError {
    fn fault_tag(&self) -> FaultTag {
        match self {
            // ★ Delegated, so an apply refused by the graph reports WHICH protocol rule
            // it broke — `UndeclaredClient` and `ConflictingAlloc` are different findings,
            // and a test that could not tell them apart is §2's canary-passing-for-the-
            // wrong-reason all over again.
            GpuError::Graph(e) => e.fault_tag(),
            GpuError::Projection(_) => FaultTag("GpuError::Projection"),
            GpuError::LateMerge { .. } => FaultTag("GpuError::LateMerge"),
            GpuError::Gpa(_) => FaultTag("GpuError::Gpa"),
            GpuError::Address(f) => f.fault_tag(),
            GpuError::UnbackedMapping { .. } => FaultTag("GpuError::UnbackedMapping"),
            GpuError::HeterogeneousArch { .. } => FaultTag("GpuError::HeterogeneousArch"),
            GpuError::SpineCapacity { .. } => FaultTag("GpuError::SpineCapacity"),
        }
    }
}

impl Faulted for crate::promote::PromoteFault {
    /// Flat, per variant — and exhaustive, so a new refusal cannot reach the census
    /// unnamed. Each of these is a distinct finding: "the guest named an object we
    /// cannot see" and "the guest tried to write another process's address space" must
    /// never be counted as one number.
    fn fault_tag(&self) -> FaultTag {
        use crate::promote::PromoteFault as F;
        match self {
            F::UnknownContextObject { .. } => FaultTag("PromoteFault::UnknownContextObject"),
            F::NotAContextObject { .. } => FaultTag("PromoteFault::NotAContextObject"),
            F::ContextVasUndeclared { .. } => FaultTag("PromoteFault::ContextVasUndeclared"),
            F::ContextVasNoOwner { .. } => FaultTag("PromoteFault::ContextVasNoOwner"),
            F::RetiredProc(_) => FaultTag("PromoteFault::RetiredProc"),
            F::UnknownVas { .. } => FaultTag("PromoteFault::UnknownVas"),
            F::ForeignContextObject { .. } => FaultTag("PromoteFault::ForeignContextObject"),
            F::TooManyRanges { .. } => FaultTag("PromoteFault::TooManyRanges"),
            F::Malformed { .. } => FaultTag("PromoteFault::Malformed"),
            F::UndecidableKind { .. } => FaultTag("PromoteFault::UndecidableKind"),
            F::SelfOverlap { .. } => FaultTag("PromoteFault::SelfOverlap"),
            F::Collides { .. } => FaultTag("PromoteFault::Collides"),
            F::HalfConflict { .. } => FaultTag("PromoteFault::HalfConflict"),
        }
    }
}
