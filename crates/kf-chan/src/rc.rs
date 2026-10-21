//! Host robust-channel (RC) observation for kayfabe's own Translated rings.
//!
//! ⊘ PRODUCT BUG B1 (owner, 2026-10-22): today NOTHING observes a host RC on a Translated ring (they are
//! born with `err_notifier: 0`), so this watcher is a deliberate no-op stub that pins the gap: every
//! `observe` returns `None`. `tests/translated_ring_rc.rs` is EXPECTED RED until the real rule (the one the
//! passthrough twins use in `kf-qemu` `rc_scan`) lands here and the ring birth arms a notifier.

/// What the host wrote: the `ROBUST_CHANNEL_*` code (`info32`, the Xid number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedRc {
    /// `info32` of the record.
    pub except_type: u32,
}

/// A ring's watcher over its 16-byte host error record.
#[derive(Debug)]
pub struct RcWatch {
    _at_arm: [u32; 4],
}

impl RcWatch {
    /// Arm over the record's contents at birth.
    #[must_use]
    pub fn armed(at_arm: [u32; 4]) -> Self {
        Self { _at_arm: at_arm }
    }

    /// The guest's own CPU-RM will write PREEMPTIVE_REMOVAL after an `NV_OK` STOP_CHANNEL.
    pub fn expect_guest_stop_write(&mut self) {}

    /// Look at the record now. STUB (B1): never reports.
    pub fn observe(&mut self, _words: [u32; 4]) -> Option<ObservedRc> {
        None
    }
}
