//! Unconnected HOST command-consumption accounting for the Windows user relay.
//!
//! Source proof: public NVIDIA dev_pbdma.ref GP_ENTRY SYNC_WAIT says a zero-
//! length CONTROL NOP prevents FURTHER GP entries being processed until the
//! preceding PB segment's HOST processing finishes. Place such a control after
//! a COMPLETE authored command packet, then a nonempty MAIN witness. GP_GET
//! past the witness commits that later entry, so the control wait finished.
//! This retires the PRECEDING command bytes; never engine operands/work.
//!
//! Native/model validation is separate from the public-source proof. No VM
//! adapter calls this module. No clock, timeout, guest fence or raw guest GP
//! value supplies a receipt. GP-slot credit is separate from PB-byte credit.

use std::collections::VecDeque;

/// Trusted private-ring policy; independent of family/die/guest ring growth.
pub const MAX_ENTRIES: u32 = 65536; // SCRATCH d3d11-gap: was 512

/// Opaque source descriptor progress, bound by the adapter to a pinned lifetime.
/// This is not engine completion or permission to unmap engine operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsumedSource {
    /// Host-assigned source backing generation, not a reused guest handle.
    pub generation: u64,
    /// Source GP cursor the adapter may publish to that generation only.
    pub guest_get: u32,
}

/// Three private descriptors to write completely BEFORE publishing PUT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publication {
    /// Direct MAIN entry for complete authored commands.
    pub command_slot: u32,
    /// Host-authored CONTROL NOP, LENGTH0, SYNC_WAIT.
    pub barrier_slot: u32,
    /// Direct MAIN entry to a nonempty, immutable HOST-NOP witness.
    pub witness_slot: u32,
    /// Native PUT after all three descriptors and commands are visible.
    pub put: u32,
    /// Absolute commitment position needed for preceding command-byte reuse.
    pub witness_sequence: u64,
}

/// Named refusal; none supplies a consumption receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Invalid/trusted-policy-exceeding capacity.
    Capacity,
    /// Zero or mismatched native channel/allocation lifetime.
    Lifetime,
    /// Insufficient descriptor credits, preserving one empty slot.
    Full,
    /// Out-of-ring or beyond-issued private hardware cursor.
    Fetch,
    /// Host absolute sequence exhausted; cannot wrap into a live receipt.
    Sequence,
    /// Bounded bookkeeping allocation failed.
    Allocation,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    witness: u64,
    source: ConsumedSource,
}

/// Worker-owned bounded ledger. The adapter must read fresh coherent hardware
/// GP_GET from THIS native USERD, never a cached/guest/reset/reused allocation.
/// Hardware cursor samples are ordered for the live channel; arbitrary old
/// samples cannot be disambiguated by a modulo index alone.
#[derive(Debug)]
pub struct Consumption {
    entries: u32,
    lifetime: u64,
    issued: u64,
    fetched: u64,
    pending: VecDeque<Pending>,
}

impl Consumption {
    /// Start a NEW native channel with its initial private cursors at zero.
    /// Never use this to reset an existing live channel or source migration.
    ///
    /// # Errors
    /// Unsupported capacity, invalid lifetime or allocation failure.
    pub fn new(entries: u32, lifetime: u64) -> Result<Self, Refusal> {
        if entries < 4 || !entries.is_power_of_two() || entries > MAX_ENTRIES {
            return Err(Refusal::Capacity);
        }
        if lifetime == 0 {
            return Err(Refusal::Lifetime);
        }
        let mut pending = VecDeque::new();
        // A partially consumed bundle can release descriptor credit before PB
        // credit. Bound by descriptor count, not floor(capacity/3).
        pending
            .try_reserve_exact((entries - 1) as usize)
            .map_err(|_| Refusal::Allocation)?;
        Ok(Self {
            entries,
            lifetime,
            issued: 0,
            fetched: 0,
            pending,
        })
    }

    /// Reusable descriptor slots, excluding one empty slot.
    #[must_use]
    pub fn available_gp(&self) -> u32 {
        self.entries - 1 - (self.issued - self.fetched) as u32
    }

    /// Source/command-byte receipts still awaiting their later witness commit.
    #[must_use]
    pub fn pending_sources(&self) -> usize {
        self.pending.len()
    }

    /// Reserve BEFORE I/O; no cancellation/reuse after a possibly visible write.
    /// An I/O failure must stop/quiesce the native channel. No abort API exists.
    /// No packet may straddle the inserted barrier/witness. The witness bytes
    /// are immutable for the channel's whole life; only preceding commands retire.
    ///
    /// # Errors
    /// Backpressure or host sequence exhaustion; state is unchanged.
    pub fn publish(&mut self, source: ConsumedSource) -> Result<Publication, Refusal> {
        if self.available_gp() < 3 {
            return Err(Refusal::Full);
        }
        let witness = self.issued.checked_add(3).ok_or(Refusal::Sequence)?;
        let mask = u64::from(self.entries - 1);
        let out = Publication {
            command_slot: (self.issued & mask) as u32,
            barrier_slot: ((self.issued + 1) & mask) as u32,
            witness_slot: ((self.issued + 2) & mask) as u32,
            put: (witness & mask) as u32,
            witness_sequence: witness,
        };
        self.pending.push_back(Pending { witness, source });
        self.issued = witness;
        Ok(out)
    }

    /// Observe ordered actual private GP_GET. A command or CONTROL entry's own
    /// commit releases descriptor credit ONLY. PB credit/source progress requires
    /// commitment of the entry AFTER the CONTROL NOP/SYNC_WAIT.
    ///
    /// # Errors
    /// Wrong lifetime, impossible/out-of-flight cursor or bounded allocation failure.
    /// Refusal grants no credit and leaves all accounting unchanged.
    pub fn observe_get(&mut self, lifetime: u64, get: u32) -> Result<Vec<ConsumedSource>, Refusal> {
        if lifetime != self.lifetime {
            return Err(Refusal::Lifetime);
        }
        if get >= self.entries {
            return Err(Refusal::Fetch);
        }
        let old = (self.fetched % u64::from(self.entries)) as u32;
        let delta = u64::from(get.wrapping_sub(old) & (self.entries - 1));
        if delta > self.issued - self.fetched {
            return Err(Refusal::Fetch);
        }
        let fetched = self.fetched + delta;
        let count = self
            .pending
            .iter()
            .take_while(|p| p.witness <= fetched)
            .count();
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| Refusal::Allocation)?;
        out.extend(self.pending.drain(..count).map(|p| p.source));
        self.fetched = fetched;
        Ok(out)
    }
}

#[cfg(test)]
mod boundary {
    use super::*;
    #[test]
    fn exhausted_absolute_sequence_is_refused_without_public_reset_or_reuse() {
        let mut q = Consumption::new(8, 91).unwrap();
        // Test-only state seed after a simulated consumed prefix.
        q.issued = u64::MAX - 3;
        q.fetched = q.issued;
        let source = ConsumedSource {
            generation: 23,
            guest_get: 1,
        };
        let p = q.publish(source).unwrap();
        assert_eq!(p.witness_sequence, u64::MAX);
        assert_eq!(q.observe_get(91, p.put).unwrap(), [source]);
        assert_eq!(q.available_gp(), 7);
        assert_eq!(q.publish(source), Err(Refusal::Sequence));
        assert_eq!(q.pending_sources(), 0);
        assert_eq!(q.available_gp(), 7);
    }
}
