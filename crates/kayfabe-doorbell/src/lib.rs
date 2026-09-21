//! The doorbell plane — `THE_DESIGN.md` §5, and nothing else.
//!
//! ## Why this crate exists at all
//!
//! §4's **inner** privilege boundary: the doorbell is written by **unprivileged guest userspace**,
//! so it is adversarial to the guest's own root (§47). ⇒ Everything here is lock-free, allocation-
//! free and wait-free on the vCPU path: §48 (*"spend the latency budget behind the trap, never in
//! it"*), §48.2 (*"no unbounded spin, anywhere"*), §41 (what a vCPU MMIO write may do).
//!
//! ⊘ **There is no `unsafe` in this crate and no OS call.** The eventfd lives above it — this
//! layer only ever *says* whether one is owed ([`wake::Wake::SignalOne`]). That keeps the whole
//! plane testable with no GPU, no guest and no hypervisor, which is lane 1 of the ladder.
//!
//! ## The two structures, and why they cannot be one (§5.1)
//!
//! | structure | job |
//! |---|---|
//! | [`token::TokenWord`] | every CAS. One `u64`, so **no neighbour can fail it** |
//! | [`bitmap::RungBitmap`] | **scanning only, never CAS'd** |
//!
//! ## The orderings are load-bearing and x86 TSO hides their absence (§5.3)
//!
//! `AcqRel` on the poll word both sides, `Acquire` on `seen`, `Release`/`Acquire` on indices and
//! claims. The rule: **every write to a work source happens-before the bump, and every `seen`
//! load happens-before the scan.** ⚠ A test on x86 cannot catch a missing fence here; the
//! orderings are argued from the spec, not measured.

pub mod accessmap;
pub mod bitmap;
pub mod caps;
pub mod channel;
pub mod classgen;
pub mod element;
pub mod hostverb;
pub mod leaf;
pub mod lifetime;
pub mod model;
pub mod plane;
pub mod timer;
pub mod rmgraph;
pub mod rpc;
pub mod ring;
pub mod completion;
pub mod shadow;
pub mod swref;
pub mod token;
pub mod trap;
pub mod trappolicy;
pub mod vmm;
pub mod wake;

pub use bitmap::RungBitmap;
pub use caps::{Aperture, Backing, Refusal, Twin, VmCaps};
pub use channel::{size_is_total, Birth, Decoded, Disposition, Owner, Submission};
pub use leaf::{GuestRamBlock, GuestRamLayout, HostSlice, LeafRefusal};
pub use lifetime::{Step, Teardown, TeardownError, WalkerState};
pub use plane::{HostOps, Plane, Vmm};
pub use ring::{PrivRing, Push, RegWrite};
pub use completion::{Completion, EngineIrq, Raise};
pub use shadow::{Cell, ClearOutcome, Trigger, WriteSemantics};
pub use token::{Claim, Release, Route, State, Token, TokenWord};
pub use trap::{Action, Class, TrapPath};
pub use wake::{Wake, WakeWord};

/// §5.2's bound on the re-act loop. ⊘ Not tuning: it is the inner boundary. A guest process
/// ringing its own channel in a tight loop would otherwise pin a worker forever.
pub const REACT_ROUNDS: u32 = 8;

#[cfg(test)]
mod tests;
