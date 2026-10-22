//! Optional host-authored bench fault; no guest command bytes are submitted here.
use std::sync::atomic::AtomicBool;

/// Bench-only guest doorbell trigger, not a GP entry count.
pub const TRIGGER_PUT: u32 = 0xb1c0_0001;
/// Public C56F SEC_OP_RESERVED6 at bits 31:29 (ogkm-580: clc56f.h:300-308).
pub const BAD_WORD: u32 = 6 << 29;
/// One candidate slot's allocation facts and current bench trigger.
#[derive(Clone, Copy)]
pub struct Candidate {
    /// Last GP_PUT from the existing worker read.
    pub put: Option<u32>,
    /// Allocation-declared engine.
    pub engine: u32,
    /// Allocation-declared GPFIFO class.
    pub class: u32,
    /// This is a Translated slot.
    pub translated: bool,
    /// Native birth asserted USER privilege.
    pub host_user: bool,
}

/// One intentional fault per explicitly armed host process.
pub struct Arm {
    run: String,
    claimed: AtomicBool,
}

impl Arm {
    /// Absent/invalid run names leave the bench disabled.
    #[must_use]
    pub fn configured(_run: Option<&str>) -> Option<Self> {
        None
    }
    /// Host-authored run identifier for the fault marker.
    #[must_use]
    pub fn run(&self) -> &str {
        &self.run
    }
    /// Claim only a matching Translated C56F COPY0 USER twin, once.
    #[must_use]
    pub fn claim(&self, _candidate: Candidate) -> bool {
        let _ = &self.claimed;
        false
    }
}

/// Ordered native operations, separated for GPU-free fault-arming tests.
pub trait FaultPort {
    /// The native channel was born USER.
    fn user(&self) -> bool;
    /// Publish words/descriptor, without ringing.
    fn push(&mut self, words: &[u32]) -> Result<(), String>;
    /// Publish the real fence tail and ring the native doorbell.
    fn ring(&mut self) -> Result<(), String>;
}

/// Submit the fixed invalid word to this owned USER ring only.
///
/// # Errors
/// Any native privilege, storage, capacity or doorbell refusal; never retry a partial injection.
pub fn inject(_port: &mut impl FaultPort) -> Result<(), String> {
    Err("negative-control interface not implemented".into())
}
