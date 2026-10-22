//! Optional host-authored bench fault; no guest command bytes are submitted here.
use std::sync::atomic::{AtomicBool, Ordering};

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
    pub fn configured(run: Option<&str>) -> Option<Self> {
        let run = run.filter(|v| {
            !v.is_empty()
                && v.len() <= 32
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })?;
        Some(Self {
            run: run.into(),
            claimed: AtomicBool::new(false),
        })
    }
    /// Host-authored run identifier for the fault marker.
    #[must_use]
    pub fn run(&self) -> &str {
        &self.run
    }
    /// Claim only a matching Translated C56F COPY0..9 USER twin, once.
    #[must_use]
    pub fn claim(&self, candidate: Candidate) -> bool {
        candidate.put == Some(TRIGGER_PUT)
            && candidate.translated
            && candidate.host_user
            && candidate.class == 0xc56f
            && kf_abi::submit::copy_index_of_engine_type(candidate.engine).is_some_and(|n| n < 10)
            && self
                .claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
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
pub fn inject(port: &mut impl FaultPort) -> Result<(), String> {
    if !port.user() {
        return Err("negative control requires a C56F channel born USER".into());
    }
    port.push(&[BAD_WORD])?;
    port.ring()
}

/// Adapter over a private native ring; guest bytes never supply its stream.
pub struct NativePort<'a> {
    ring: &'a mut crate::host::HostRing,
    rm: &'a kf_host::HostRm,
}

impl<'a> NativePort<'a> {
    /// Borrow this owned ring and its host session for one injection.
    pub fn new(ring: &'a mut crate::host::HostRing, rm: &'a kf_host::HostRm) -> Self {
        Self { ring, rm }
    }
}

impl FaultPort for NativePort<'_> {
    fn user(&self) -> bool {
        let (arch, implementation, _) = self.rm.arch_info();
        self.ring.channel().assert_user().is_ok()
            && kf_chip::choose_host_classes(arch, implementation, self.rm.supported_class_ids())
                .is_ok_and(|c| c.gpfifo_channel().channel_id().0 == 0xc56f)
    }
    fn push(&mut self, words: &[u32]) -> Result<(), String> {
        self.ring
            .push(words)?
            .map_err(|_| "negative-control ring busy".into())
    }
    fn ring(&mut self) -> Result<(), String> {
        self.ring
            .fence(self.rm)?
            .map(|_| ())
            .map_err(|_| "negative-control tail busy".into())
    }
}
