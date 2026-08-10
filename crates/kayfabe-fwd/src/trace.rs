//! The compiler-checked bridge from this plane's faults to `kayfabe-trace`'s [`FaultTag`].
//!
//! `kayfabe-trace` sits **below** this crate in the lattice, so it cannot name
//! [`FwdFault`] or [`Stale`]. The bridge lives here instead, as an **exhaustive `match`**
//! rather than a `Display` impl: adding a fault variant fails the build until the trace
//! vocabulary is told about it, which is what stops a refusal quietly becoming
//! untraceable (`testing_doctrine.md` §6 rule 1).
//!
//! ★ `Stale` is tagged **through** `FwdFault::Stale`, so the tag of a staleness refusal
//! names its `Stale` variant rather than collapsing to `"FwdFault::Stale"`. §12.10's
//! lesson is exactly that: a canary that could not tell `Stale::Proc` from an anonymous
//! RM error passed for the wrong reason for a whole milestone.
//!
//! In its own file rather than in `lib.rs` on purpose: the mutation campaign's file scope
//! is `crates/kayfabe-fwd/src/lib.rs`, and a naming table with one arm per variant is
//! `core_mutation_gate.md` decision #15's "hollow test to move a number" in gate form —
//! it belongs outside the denominator, not inside it with a test written to chase it.

use kayfabe_trace::{FaultTag, Faulted};

use crate::{FwdFault, Stale};

impl Faulted for Stale {
    fn fault_tag(&self) -> FaultTag {
        match self {
            Stale::Proc(_) => FaultTag("Stale::Proc"),
            Stale::Channel(_) => FaultTag("Stale::Channel"),
            Stale::Vas { .. } => FaultTag("Stale::Vas"),
            Stale::Route { .. } => FaultTag("Stale::Route"),
            Stale::Rebound => FaultTag("Stale::Rebound"),
            Stale::Target { .. } => FaultTag("Stale::Target"),
        }
    }
}

impl Faulted for FwdFault {
    fn fault_tag(&self) -> FaultTag {
        match self {
            FwdFault::MalformedToken { .. } => FaultTag("FwdFault::MalformedToken"),
            FwdFault::UnknownVchid { .. } => FaultTag("FwdFault::UnknownVchid"),
            FwdFault::RetiredProc(_) => FaultTag("FwdFault::RetiredProc"),
            FwdFault::Condemned { .. } => FaultTag("FwdFault::Condemned"),
            FwdFault::NoVas(_) => FaultTag("FwdFault::NoVas"),
            FwdFault::NotScheduled { .. } => FaultTag("FwdFault::NotScheduled"),
            FwdFault::UnknownChannel { .. } => FaultTag("FwdFault::UnknownChannel"),
            FwdFault::IsolateRetired { .. } => FaultTag("FwdFault::IsolateRetired"),
            FwdFault::IsolatePending { .. } => FaultTag("FwdFault::IsolatePending"),
            FwdFault::NoHostVas { .. } => FaultTag("FwdFault::NoHostVas"),
            FwdFault::CeTooFragmented { .. } => FaultTag("FwdFault::CeTooFragmented"),
            FwdFault::UnknownPdb { .. } => FaultTag("FwdFault::UnknownPdb"),
            FwdFault::NoTarget { .. } => FaultTag("FwdFault::NoTarget"),
            FwdFault::CePeerOperand { .. } => FaultTag("FwdFault::CePeerOperand"),
            FwdFault::CeUnstableBacking { .. } => FaultTag("FwdFault::CeUnstableBacking"),
            FwdFault::CeNoTable { .. } => FaultTag("FwdFault::CeNoTable"),
            FwdFault::CeWalk { .. } => FaultTag("FwdFault::CeWalk"),
            FwdFault::CpuCeStraddle { .. } => FaultTag("FwdFault::CpuCeStraddle"),
            FwdFault::CpuCeFb { .. } => FaultTag("FwdFault::CpuCeFb"),
            // ★ Delegated, so an address fault's tag names WHICH address fault. A miss
            // and an overlap are different findings, and `mode2_address_table.md`'s whole
            // discipline is that a miss is loud and specific.
            FwdFault::Address(f) => f.fault_tag(),
            FwdFault::Arena => FaultTag("FwdFault::Arena"),
            FwdFault::GpaRead { .. } => FaultTag("FwdFault::GpaRead"),
            FwdFault::NonRamGpa { .. } => FaultTag("FwdFault::NonRamGpa"),
            FwdFault::PushbufferAperture { .. } => FaultTag("FwdFault::PushbufferAperture"),
            FwdFault::RingBroughtNoEntry { .. } => FaultTag("FwdFault::RingBroughtNoEntry"),
            FwdFault::SubmissionDecodedNoWork { .. } => {
                FaultTag("FwdFault::SubmissionDecodedNoWork")
            }
            FwdFault::CeReleaseNoClock => FaultTag("FwdFault::CeReleaseNoClock"),
            FwdFault::UvmFaultMethodWithoutFaultDelivery { .. } => {
                FaultTag("FwdFault::UvmFaultMethodWithoutFaultDelivery")
            }
            FwdFault::PushTooFragmented { .. } => FaultTag("FwdFault::PushTooFragmented"),
            FwdFault::Rm(e) => e.fault_tag(),
            FwdFault::NotAnEngine(_) => FaultTag("FwdFault::NotAnEngine"),
            FwdFault::WrongArm { .. } => FaultTag("FwdFault::WrongArm"),
            FwdFault::Present(_) => FaultTag("FwdFault::Present"),
            FwdFault::Completion(_) => FaultTag("FwdFault::Completion"),
            FwdFault::PoolSaturated { .. } => FaultTag("FwdFault::PoolSaturated"),
            FwdFault::Cancelled { .. } => FaultTag("FwdFault::Cancelled"),
            FwdFault::Wedged { .. } => FaultTag("FwdFault::Wedged"),
            FwdFault::Stale(s) => s.fault_tag(),
            FwdFault::SystemDataPlane => FaultTag("FwdFault::SystemDataPlane"),
            FwdFault::ForeignBacking { .. } => FaultTag("FwdFault::ForeignBacking"),
        }
    }
}
