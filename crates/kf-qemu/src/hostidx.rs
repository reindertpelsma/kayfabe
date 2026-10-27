// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Host token to guest token, outside the channel slot's lock.
//!
//! A worker serving a Translated channel `try_lock`s its slot and, until now, dropped the ring when
//! another thread held that lock for a moment (the act thread's scans of every slot, the probe,
//! the relay refresh). A dropped completion ring is a fence the host finished and no pump ever
//! reads: TUR1 (lease 57882297 TU106, lease 57908255 GA106, Windows 580.88) showed it in every
//! `HOST-FENCE-OVERDUE` dump: token word Idle, fence word equal to the in-flight sequence, last
//! wake 1 ms after the last pump, and `chan[contended>=1]`. A dropped doorbell ring is the same
//! loss for new guest work. The ring is owed back, and ringing needs the GUEST token while the
//! contended worker only has the host token and cannot read the slot to find it.

use std::collections::HashMap;
use std::sync::RwLock;

/// Host token to guest token for the live Translated channels.
#[derive(Debug, Default)]
pub struct HostIdx(RwLock<HashMap<u32, u32>>);

impl HostIdx {
    /// A Translated channel was born: `host` is its twin's token, `guest` the plane's token.
    pub fn insert(&self, host: u32, guest: u32) {
        if let Ok(mut m) = self.0.write() {
            m.insert(host, guest);
        }
    }

    /// The guest token behind a host token, if the channel is live.
    #[must_use]
    pub fn get(&self, host: u32) -> Option<u32> {
        self.0.read().ok()?.get(&host).copied()
    }

    /// The channel is gone.
    pub fn remove(&self, host: u32) {
        if let Ok(mut m) = self.0.write() {
            m.remove(&host);
        }
    }
}

/// Give back a consumed ring using its resolved guest identity. The callback only rings a token.
/// Shared by the native serve path and its GPU-free behavioral harness.
pub fn reowe(guest: Option<u32>, ring: impl FnOnce(u32)) {
    if let Some(guest) = guest {
        ring(guest);
    }
}

#[cfg(test)]
#[path = "hostidx_model.rs"]
mod model;

#[cfg(test)]
mod tests {
    use super::HostIdx;

    #[test]
    fn a_birth_makes_the_guest_token_findable_from_the_host_token() {
        let m = HostIdx::default();
        m.insert(0x8003b, 0x809);
        assert_eq!(m.get(0x8003b), Some(0x809));
        assert_eq!(m.get(0x8003c), None);
    }

    #[test]
    fn a_retire_forgets_it_and_a_reused_host_token_follows_the_new_birth() {
        let m = HostIdx::default();
        m.insert(7, 1);
        m.remove(7);
        assert_eq!(m.get(7), None);
        m.insert(7, 2);
        assert_eq!(m.get(7), Some(2));
    }

    /// The contended branch of `serve` must give the ring back, not drop it. Source guards in the
    /// repository's style: the branch calls `owe_ring`, and `owe_ring` rings the plane by the
    /// guest token found through this map.
    #[test]
    fn serve_gives_a_contended_ring_back() {
        let src = include_str!("chan.rs");
        let serve = src
            .split("pub fn serve(&self, ht: u32) -> bool {")
            .nth(1)
            .and_then(|s| s.split("g.serves = g.serves.saturating_add(1);").next())
            .expect("serve");
        assert!(serve.contains("self.owe_ring(ht)"), "{serve}");
        let old = "self.contended.fetch_add(1, Ordering::Relaxed);\n            return false;";
        assert!(
            !serve.contains(old),
            "the contended branch still drops the ring"
        );
    }

    #[test]
    fn owe_ring_rings_by_the_guest_token_and_counts_it() {
        let src = include_str!("chan.rs");
        let body = src
            .split("fn owe_ring(&self, ht: u32) -> bool {")
            .nth(1)
            .and_then(|s| s.split("\n    }\n").next())
            .expect("owe_ring");
        assert!(body.contains("self.host_idx.get(ht)"), "{body}");
        assert!(body.contains("self.plane.ring_internal("), "{body}");
        assert!(body.contains("self.contended.fetch_add(1"), "{body}");
    }
}
