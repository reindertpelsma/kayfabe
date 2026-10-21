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

/// Allocation-time routing facts, never decoded from the host's notifier payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RcRoute {
    /// Guest chid, including its runlist bits where applicable.
    pub chid: u32,
    /// Guest-declared engine (the host execution engine may differ).
    pub engine: u32,
    /// Our host token, for attribution only.
    pub host_token: u32,
}

/// One host error, routed to the guest whose channel owns the private notifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutedRc {
    /// Allocation-time identity.
    pub route: RcRoute,
    /// Host-observed Xid.
    pub except_type: u32,
}

/// Bounded observation/publication state for one Translated allocation.
/// DESIGN STUB: extended EXPECTED RED tests precede implementation.
pub struct RcDelivery;

impl RcDelivery {
    /// Arm a new allocation over its private host record.
    #[must_use]
    pub fn armed(_route: RcRoute, _at_arm: [u32; 4]) -> Self {
        Self
    }

    /// Whether a host error has made this channel unusable.
    #[must_use]
    pub fn failed(&self) -> bool {
        false
    }

    /// Observe host words, then publish four guest words with the status word last.
    /// A failed store retains the error for a later call; an event follows publication only.
    ///
    /// # Errors
    /// The guest write port's refusal.
    pub fn poll<E>(
        &mut self,
        _words: [u32; 4],
        _store: impl FnMut(u64, u32) -> Result<(), E>,
    ) -> Result<Option<RoutedRc>, E> {
        Ok(None)
    }
}
