//! Diagnostic frontiers; packet-specific guest delivery remains external evidence.
/// Last published cohort and its successful marker coverage, fixed storage.
#[derive(Debug, Default)]
pub struct Packet {
    put: u64,
    covered: u64,
    marker: u64,
    observed: u64,
}
impl Packet {
    /// An actual guest-entry publication, not a GET observation.
    pub fn published(&mut self) {
        self.put = self.put.saturating_add(1);
    }
    /// A successful marker covers earlier publications only.
    pub fn marker(&mut self, seq: u64) {
        if seq > self.marker {
            self.marker = seq;
            self.covered = self.put;
        }
    }
    /// Validated reference; an unissued count establishes nothing.
    pub fn seen(&mut self, seq: u64) {
        if seq <= self.marker {
            self.observed = self.observed.max(seq);
        }
    }
    /// Latest cohort, never a claim to know which packet the guest timed out on.
    pub fn class(&self, unpublished_held: bool) -> &'static str {
        if unpublished_held {
            "i_unpublished"
        } else if self.put == 0 {
            "unknown"
        } else if self.covered < self.put || self.marker == 0 {
            "unknown_no_covering_marker"
        } else if self.observed < self.marker {
            "iii_host_completion_unobserved"
        } else {
            "ii_candidate_guest_delivery_unknown"
        }
    }
}
/// Cheap sequence invariant monitor, no clock or allocation.
#[derive(Debug, Default)]
pub struct Watch {
    issued: u64,
    seen: u64,
}
impl Watch {
    /// Bits: 1 observed unissued count, 2 seen regressed, 4 publication regressed.
    /// Passive diagnostics only: a guest can forge its own marker bytes.
    pub fn check(&mut self, issued: u64, seen: u64) -> u8 {
        let bits = u8::from(seen > issued)
            | (u8::from(seen < self.seen) << 1)
            | (u8::from(issued < self.issued) << 2);
        self.issued = issued;
        self.seen = seen;
        bits
    }
}
