//! Durable host robust-channel observation and bounded guest error publication.
//!
//! An OS event is only a wake. The private host notifier is the authority; neither
//! a guest notifier nor a completion fence can manufacture an RC error here.

/// What the host wrote: the `ROBUST_CHANNEL_*` code (`info32`, the Xid number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedRc {
    /// `info32` of the record.
    pub except_type: u32,
}

/// A ring's watcher over its 16-byte host error record.
#[derive(Debug)]
pub struct RcWatch {
    at_arm: [u32; 4],
    reported: bool,
    guest_stop_write: bool,
}

impl RcWatch {
    /// Arm over the record's contents at birth.
    #[must_use]
    pub fn armed(at_arm: [u32; 4]) -> Self {
        Self {
            at_arm,
            reported: false,
            guest_stop_write: false,
        }
    }

    /// Rebaseline the next complete PREEMPTIVE_REMOVAL after an accepted guest stop.
    /// Private Translated notifiers do not need this: guest CPU-RM cannot write them.
    pub fn expect_guest_stop_write(&mut self) {
        self.guest_stop_write = true;
    }

    /// Accept one complete, changed record per channel life; status is published last.
    pub fn observe(&mut self, words: [u32; 4]) -> Option<ObservedRc> {
        if self.reported || words == self.at_arm || words[3] >> 16 == 0 {
            return None;
        }
        // ogkm-580: nverror.h, ROBUST_CHANNEL_PREEMPTIVE_REMOVAL.
        if self.guest_stop_write && words[2] == 45 {
            self.at_arm = words;
            self.guest_stop_write = false;
            return None;
        }
        self.reported = true;
        Some(ObservedRc {
            except_type: words[2],
        })
    }
}

/// Sample the completion/status word before reading metadata, then verify it again.
///
/// ogkm-580 `method_notification.c:138-155`: the context-DMA record publishes
/// info16/status as its final 32-bit write. A metadata-first read could pair old
/// info32 with a newly arrived status. No wait, allocation, or guest-controlled loop.
pub fn snapshot(mut load: impl FnMut(u64) -> Option<u32>) -> Option<[u32; 4]> {
    let status = load(12)?;
    if status >> 16 == 0 {
        return None;
    }
    std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    let words = [load(0)?, load(4)?, load(8)?, status];
    (load(12)? == status).then_some(words)
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

/// One observed record, retained across guest-write failures until publication.
pub struct RcDelivery {
    route: RcRoute,
    watch: RcWatch,
    pending: Option<[u32; 4]>,
    failed: bool,
}

impl RcDelivery {
    /// Arm a new allocation over its private host record.
    #[must_use]
    pub fn armed(route: RcRoute, at_arm: [u32; 4]) -> Self {
        Self {
            route,
            watch: RcWatch::armed(at_arm),
            pending: None,
            failed: false,
        }
    }

    /// Whether a host error has made this channel unusable (also while publication retries).
    #[must_use]
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Observe host words, then publish four guest words with the status word last.
    /// A failed store retains the error; only successful publication returns an event.
    ///
    /// # Errors
    /// The guest write port's refusal.
    pub fn poll<E>(
        &mut self,
        words: [u32; 4],
        mut store: impl FnMut(u64, u32) -> Result<(), E>,
    ) -> Result<Option<RoutedRc>, E> {
        if self.pending.is_none() && self.watch.observe(words).is_some() {
            self.failed = true;
            self.pending = Some(words);
        }
        let Some(record) = self.pending else {
            return Ok(None);
        };
        // Reject invalid routes rather than silently truncating a hostile engine id.
        let Ok(engine) = u16::try_from(self.route.engine) else {
            return Ok(None);
        };
        if engine == 0 {
            return Ok(None);
        }
        let timestamp = u64::from(record[0]) | (u64::from(record[1]) << 32);
        let bytes = kf_abi::notifier::ErrorNotification::rc(record[2], engine, timestamp).encode();
        for (i, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let word = u32::from_le_bytes(*chunk);
            if i == 3 {
                kf_linux_raw::release_fence();
            }
            store((i as u64) * 4, word)?;
        }
        // The record, including its status, precedes the event queue publication as well.
        kf_linux_raw::release_fence();
        self.pending = None;
        Ok(Some(RoutedRc {
            route: self.route,
            except_type: record[2],
        }))
    }
}
