//! ★★★ **A bad guest pointer, made into a fault the guest's own driver handles.**
//!
//! `docs/design/simulated_gpu_fault.md` (task #111). This module owns the **decision**:
//! given that some access could not be translated, *is this the guest application's
//! mistake, and may we therefore tell the guest its hardware faulted?*
//!
//! It owns nothing else. The address plane detects (`kayfabe_mmu::AddressFault::Miss`),
//! an arch-impl encodes the numbers (`kayfabe_arch::fault::MmuFaultCodes`), the ABI
//! quarantine lays out the bytes (`kayfabe_abi::rc`) and the GSP transport posts them
//! (`kayfabe_gsp::GspFsm::post_event`). Four layers, one decision, and the decision is
//! here because it is the only one of the five that is *policy*.
//!
//! # ★★★ The two failure modes, and why conflating them would hide our own bugs
//!
//! An untranslatable address has two completely different meanings and exactly one of
//! them may become a simulated fault:
//!
//! | | who chose the address | what it means | what we do |
//! |---|---|---|---|
//! | **A** | a guest **application** | the app dereferenced memory it never mapped — its bug, and real hardware's answer is an MMU fault | [`FaultVerdict::Emit`] |
//! | **B** | the guest **kernel** (its RM/UVM clients), or our own derivation | a driver we are impersonating hardware for cannot reach an address it set up. Either the guest driver is broken or **we** are | [`FaultVerdict::Escalate`] |
//!
//! Emitting a fault for B would be the worst possible outcome, and worse than the hang
//! it replaces: it converts *our* defect into a plausible, well-formed message blaming
//! the guest, the guest kills a channel and reports a clean error, and the run looks
//! like a correctly-diagnosed application bug. Every measurement taken afterwards is
//! measuring a lie. This project has the shape of that failure written down twice
//! already — the C's "log 256 times and then stop, but release the completion anyway"
//! (`C: src/qemu/nvkvm_gpu_emul.c:6376-6395` and `:6503`), and the GA10x 512M-leaf gap
//! that silently dropped page-table writes for weeks. A fault emitter with no
//! attribution rule is the third.
//!
//! ## ★★ The discriminator is a fact the guest DECLARED, not a heuristic
//!
//! It is [`crate::project::SYSTEM_ANCHOR`]: every client that declared itself
//! [`kayfabe_arch::ClientKind::Kernel`] on its own `NV01_ROOT` belongs to the one
//! reserved system component, by rule and never by inference
//! (`crate::project`'s module docs). So:
//!
//! - a channel owned by [`crate::gpu::Gpu::SYSTEM_PROC`] is the guest **kernel**'s ⇒ B;
//! - a channel owned by any other `Proc` is a guest **application**'s ⇒ A.
//!
//! That is the same key the whole `Proc` grouping already turns on, so this rule cannot
//! drift away from the one the data plane uses — there is one classification, not two.
//!
//! ⚠ What the rule does **not** claim: it does not say the *application* is at fault
//! rather than us, only that the address was chosen inside an application's context. A
//! kayfabe bug that loses an application's binding produces an A-classified fault and
//! blames the app. [`FaultReport::census`]-style counting is the mitigation named in
//! the design note §7, and it is not built; the honest statement is that A is
//! *attributable to an application's context*, which is strictly weaker than *caused by
//! the application*, and the emitter must never be described as the stronger thing.

use kayfabe_arch::fault::{MmuFaultAccess, MmuFaultCause};
use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa, Pdb, VChid};

use crate::{ChanId, ProcId};

/// One simulated GPU fault, ready to be encoded and posted.
///
/// Every field is a fact this port already held when it refused — the channel it
/// declined to ring, the address that did not resolve, the engine the channel was
/// declared for. Nothing here is derived from silicon, because no silicon was consulted;
/// see [`FaultReport::attribution_note`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultReport {
    /// The target the doorbell addressed.
    pub gpu: GpuId,
    /// The proc whose context the faulting address belongs to.
    pub proc: ProcId,
    /// The channel we refused to ring — the *attribution*, and the field the guest's
    /// receiver looks the channel up by.
    pub chan: ChanId,
    /// That channel's runlist index, which is what the guest's `chid` means.
    pub vchid: VChid,
    /// The address space the address was resolved in, when the channel declared one. A
    /// GSP-managed channel has no declared VAS (`Channel::vas_pdb = None`) and this is
    /// `None` — recorded rather than defaulted, because "no VAS" is itself a reason a
    /// fault may not be application-attributable.
    pub pdb: Option<Pdb>,
    /// The virtual address that did not translate.
    pub va: GpuVa,
    /// The engine the channel is declared for — what the RC event routes on.
    pub engine: EngineKind,
    /// Why it did not translate.
    pub cause: MmuFaultCause,
    /// What kind of access it was.
    pub access: MmuFaultAccess,
}

impl FaultReport {
    /// ★ **What this fault honestly carries, and what it cannot.**
    ///
    /// Returned as data rather than written in prose so that a consumer which logs a
    /// fault logs its own limits beside it. Real silicon fills a 32-byte fault-buffer
    /// entry with a GPC id, a uTLB client id and a timestamp taken by the MMU
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc369.h:36-67`). We ran no shader
    /// and read no MMU: the channel, the address, the access direction and the engine
    /// class are ours to state, and the rest would be invented.
    #[must_use]
    pub fn attribution_note(&self) -> &'static str {
        "channel + address + access direction are held facts; GPC/client/timestamp are \
         not observed and are not reported"
    }
}

/// Why a fault may **not** be presented to the guest as hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAttributable {
    /// The channel belongs to the guest **kernel**'s system component (failure mode B).
    ///
    /// The guest driver set this address up through an RM verb we serviced. If it does
    /// not resolve, either it never bound or we lost it, and both are defects on this
    /// side of the boundary. A simulated fault here would tell the guest kernel that its
    /// own correct request faulted in hardware.
    GuestKernelContext {
        /// The channel, so the escalation names the same thing an emit would have.
        chan: ChanId,
        /// Its runlist index.
        vchid: VChid,
    },
    /// The channel declared no address space, so there is no application VAS the address
    /// could have come from — a system-routed, GSP-managed channel.
    ///
    /// ★ Deliberately its own variant rather than folded into
    /// [`NotAttributable::GuestKernelContext`]. They are different mistakes: one is
    /// "the guest kernel's channel", the other is "a channel with no VAS at all", and a
    /// caller told only "not attributable" for both cannot tell an ownership question
    /// from a modelling gap.
    NoAddressSpace {
        /// The channel.
        chan: ChanId,
    },
}

/// What to do about an untranslatable address.
///
/// ★ There is no third arm and there must not be one. "Log it and carry on" is exactly
/// what the C did at seven separate sites, and the measured guest-visible result was a
/// timeout every time (`docs/design/simulated_gpu_fault.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultVerdict {
    /// Tell the guest its hardware faulted.
    Emit(FaultReport),
    /// Do **not** tell the guest anything. Raise this to the operator instead: it is a
    /// defect on one side or the other of our own boundary.
    Escalate(NotAttributable),
}

/// The facts a caller must have in hand to ask for a verdict.
///
/// A struct rather than eight positional arguments because six of the eight are ids and
/// two of those are `u32` newtypes over the same shape — a positional call is one
/// transposition away from attributing a fault to the wrong channel, which is the one
/// error this whole module exists to make impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultFacts {
    /// The target the doorbell addressed.
    pub gpu: GpuId,
    /// The owning proc, as the exec-plane route resolved it.
    pub proc: ProcId,
    /// The channel.
    pub chan: ChanId,
    /// Its runlist index.
    pub vchid: VChid,
    /// The channel's declared VAS, if it declared one.
    pub pdb: Option<Pdb>,
    /// The address that did not translate.
    pub va: GpuVa,
    /// The channel's declared engine.
    pub engine: EngineKind,
    /// Why it did not translate.
    pub cause: MmuFaultCause,
    /// What kind of access it was.
    pub access: MmuFaultAccess,
}

/// ★★★ **The rule.** Decide whether `facts` may be presented to the guest as a hardware
/// fault.
///
/// `system_proc` is passed in rather than read from a constant so that the caller is the
/// one holding the device's own notion of its system component — a free function that
/// reached for [`crate::gpu::Gpu::SYSTEM_PROC`] itself would be a second definition of
/// "the guest kernel" sitting next to the projection's.
#[must_use]
pub fn verdict(facts: FaultFacts, system_proc: ProcId) -> FaultVerdict {
    // Ownership FIRST. A system channel with a VAS and a system channel without one are
    // both failure mode B, and reporting the VAS-shaped reason for the first would send
    // the reader looking at the address plane for an ownership fact.
    if facts.proc == system_proc {
        return FaultVerdict::Escalate(NotAttributable::GuestKernelContext {
            chan: facts.chan,
            vchid: facts.vchid,
        });
    }
    let Some(pdb) = facts.pdb else {
        return FaultVerdict::Escalate(NotAttributable::NoAddressSpace { chan: facts.chan });
    };
    FaultVerdict::Emit(FaultReport {
        gpu: facts.gpu,
        proc: facts.proc,
        chan: facts.chan,
        vchid: facts.vchid,
        pdb: Some(pdb),
        va: facts.va,
        engine: facts.engine,
        cause: facts.cause,
        access: facts.access,
    })
}

kayfabe_util::assert_send_sync!(FaultReport, FaultVerdict, NotAttributable, FaultFacts);
