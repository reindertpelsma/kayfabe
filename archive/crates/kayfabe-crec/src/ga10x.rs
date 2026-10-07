//! The GA10x [`Arch`] the replay drives — and a **re-export** of the register model, which
//! now lives one crate over.
//!
//! ## ★★ The model MOVED on 2026-07-31, and this file is what is left
//!
//! Every offset and encoding that used to be here is now `kayfabe_device::ga10x`. The
//! reason is stage Q4: a guest's trapped register accesses have to reach a register model,
//! and a shipped archive cannot depend on this crate — `kayfabe-crec` pulls in
//! `kayfabe-mocks`, whose manifest says *"Test-only; never a production dependency"*. The
//! alternative was a second copy of the same offsets in a production crate, which is two
//! descriptions of one chip that can disagree.
//!
//! ★ The consequence is the good one: the 359 062-record `cap1` differential now runs
//! against **the same encoder the guest reads through**, so a divergence there is a
//! divergence in the shipped map rather than in a harness's copy of it.
//!
//! What is still here is the [`Arch`] wrapper, and it stays here on purpose: its non-GSP
//! halves are `MockArch`'s, which is exactly what a *replay harness* may do and exactly
//! what a production device may not.

use kayfabe_arch::gsp::GspModel;
use kayfabe_arch::ids::{ClassId, ControlCmd, VChid};
use kayfabe_arch::{Arch, DoorbellTarget, GmmuFmt, ObjectKind, PushbufferAbi, UserdModel};
use kayfabe_mocks::MockArch;

pub use kayfabe_device::ga10x::{FB_SIZE_MB, Ga10xGspModel, RMARGS_ID, USABLE_FB_SIZE_IN_MB_ADDR};

/// An [`Arch`] that is `MockArch` in every respect except that its GSP is [`Ga10xGspModel`].
///
/// Composition, not modification — the same shape the conformance suite's `GspArch` uses.
/// ★ The non-GSP halves are deliberately still the mock's: this harness replays the **GSP
/// plane only**, and a divergence that needed a real GMMU or a real USERD model would be a
/// finding about scope, not a silent wrong answer.
#[derive(Debug)]
pub struct Ga10xArch {
    inner: MockArch,
    gsp: Ga10xGspModel,
}

impl Default for Ga10xArch {
    fn default() -> Ga10xArch {
        Ga10xArch::new()
    }
}

impl Ga10xArch {
    /// The architecture.
    #[must_use]
    pub fn new() -> Ga10xArch {
        Ga10xArch {
            inner: MockArch::new(),
            gsp: Ga10xGspModel::new(),
        }
    }
}

impl Arch for Ga10xArch {
    fn name(&self) -> &'static str {
        "GA10x (GA106, replay)"
    }
    fn classify(&self, class: ClassId) -> ObjectKind {
        self.inner.classify(class)
    }
    fn vchid_from_userd_flags(&self, flags: u32) -> Option<VChid> {
        self.inner.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.inner.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.inner.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.inner.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.inner.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.inner.pushbuffer()
    }
    fn gsp(&self) -> Option<&dyn GspModel> {
        Some(&self.gsp)
    }
}
