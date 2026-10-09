// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **How many messages the GSP model posted to the guest's status queue** — replies and events
//! apart (`docs/design/V3_IRQ_SOURCE_TRACE.md`, 2026-10-09).
//!
//! A reply answers a command (an RPC function below `NV_VGPU_MSG_EVENT_FIRST_EVENT`); an event is
//! unsolicited (`GSP_INIT_DONE`, `POST_EVENT`, `RC_TRIGGERED`, `UCODE_LIBOS_PRINT`, … — `0x1000` and
//! up). The device shell raises the GSP's stall vector for some events and for no reply, and the
//! counts here are what that shell's raise counters are read against.
//!
//! The cell is shared (`Arc` of atomics): the device keeps a handle, the state machine bumps it from
//! whatever thread services the queue, and a device reset keeps the handle ([`crate::GspFsm`]'s reset
//! carries it across, like the system-info cell), so the counts are for the process, not for one
//! guest life. It is statistics, not state: two machines are equal whatever their counts.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
struct Inner {
    replies: AtomicU64,
    events: AtomicU64,
}

/// Shared counters of the messages posted to the guest's status queue.
#[derive(Debug, Clone, Default)]
pub struct PostStats(Arc<Inner>);

impl PostStats {
    /// A fresh cell.
    #[must_use]
    pub fn new() -> PostStats {
        PostStats::default()
    }

    /// Count one posted message of RPC function `function`.
    pub fn note(&self, function: u32) {
        let c = if function >= kf_abi::generated::rpc::NV_VGPU_MSG_EVENT_FIRST_EVENT {
            &self.0.events
        } else {
            &self.0.replies
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// `(replies, events)` posted so far.
    #[must_use]
    pub fn get(&self) -> (u64, u64) {
        (
            self.0.replies.load(Ordering::Relaxed),
            self.0.events.load(Ordering::Relaxed),
        )
    }
}

/// Statistics are not state: a machine's identity ignores them.
impl PartialEq for PostStats {
    fn eq(&self, _: &PostStats) -> bool {
        true
    }
}

impl Eq for PostStats {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_and_events_are_counted_apart_and_equality_ignores_them() {
        let s = PostStats::new();
        s.note(76);
        s.note(103);
        s.note(0x1003);
        s.note(0x1001);
        s.note(0x1000);
        assert_eq!(s.get(), (2, 3));
        let t = s.clone();
        t.note(10);
        assert_eq!(s.get(), (3, 3), "a clone is the same cell");
        assert_eq!(s, PostStats::new());
    }
}
