//! GPU-free private-ring accounting prototype (Windows user-ring P2).
//!
//! NOT CONNECTED to channel birth, mappings, USERD, doorbells or migration.
//! No native cursor-transfer rule is assumed. A retirement token is opaque
//! metadata supplied by a future validated stream adapter, not guest completion.
//! Host-ring entries and staging remain held until a real engine-completion
//! proof. GP_GET alone never retires work or supplies such a proof (native P0
//! observed GP_GET=1 with a fault and completion fence=0).
//!
//! A permitted completion mechanism in a restricted user space is still a
//! design/measurement gate: do not map the Translated runner's writable private
//! fence there. OWNER_RULINGS §AI authorizes only read-only commands/staging.
//! The prototype is deliberately conservative and may not yet make progress
//! on a real migration; no timeout substitutes for a missing proof.

use std::collections::VecDeque;

/// Opaque validated source progress, preserved across host-ring wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retirement {
    /// A host-authored backing/stream lifetime, never inferred from a reused handle.
    pub generation: u64,
    /// The source cursor to publish after real completion, if its lifetime is live.
    pub guest_get: u32,
}

/// Where a future worker writes one immutable host GP entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Publication {
    /// Entry slot to fill before publishing the host PUT.
    pub slot: u32,
    /// Host PUT to publish only after writing the entry/command bytes and fencing.
    pub put: u32,
    /// A host-authored monotonically increasing completion receipt.
    pub sequence: u64,
}

/// Named model refusals; none changes queue state or completes work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Capacity is not a power of two >=2 within the trusted caller's budget.
    Capacity,
    /// One slot stays empty; the remaining slots have not actually completed.
    Full,
    /// Bounded metadata allocation failed.
    Allocation,
    /// Host-authored receipt sequence exhausted; never wrap into a live receipt.
    Sequence,
    /// Hardware GET is outside the ring or beyond work published so far.
    Fetch,
    /// A completion receipt names work never published.
    Completion,
}

/// Pure bounded accounting; the worker owns all mutation, with no I/O or locks.
#[derive(Debug, Clone)]
pub struct Ledger {
    entries: u32,
    issued: u64,
    completed: u64,
    fetched: u64,
    pending: VecDeque<Retirement>,
}

impl Ledger {
    /// Allocate within a TRUSTED per-channel/per-VM `maximum`, not a guest field.
    /// Hardware/class validation and the budget's provenance belong to the adapter.
    ///
    /// # Errors
    /// Invalid capacity or inability to allocate the bounded metadata.
    pub fn new(entries: u32, maximum: u32) -> Result<Self, Refusal> {
        if entries < 2 || !entries.is_power_of_two() || entries > maximum {
            return Err(Refusal::Capacity);
        }
        let count = usize::try_from(entries - 1).map_err(|_| Refusal::Capacity)?;
        let mut pending = VecDeque::new();
        pending
            .try_reserve_exact(count)
            .map_err(|_| Refusal::Allocation)?;
        Ok(Self {
            entries,
            issued: 0,
            completed: 0,
            fetched: 0,
            pending,
        })
    }

    /// Ring size, fixed at birth.
    #[must_use]
    pub const fn entries(&self) -> u32 {
        self.entries
    }

    /// Slots still protected from reuse by outstanding work.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Credits, leaving one empty slot to distinguish wrap from full.
    #[must_use]
    pub fn available(&self) -> u32 {
        self.entries - 1 - self.pending.len() as u32
    }

    /// Last actually completed host-authored receipt.
    #[must_use]
    pub const fn completed(&self) -> u64 {
        self.completed
    }

    /// Reserve accounting before the worker publishes immutable commands/GP entry.
    /// An I/O failure afterward must stop/quiesce the channel; do not roll back a
    /// possibly visible publication and reuse its slot or completion sequence.
    ///
    /// # Errors
    /// Backpressure or exhaustion of the host-authored receipt sequence.
    pub fn publish(&mut self, retires: Retirement) -> Result<Publication, Refusal> {
        if self.available() == 0 {
            return Err(Refusal::Full);
        }
        let sequence = self.issued.checked_add(1).ok_or(Refusal::Sequence)?;
        let p = Publication {
            slot: (self.issued % u64::from(self.entries)) as u32,
            put: (sequence % u64::from(self.entries)) as u32,
            sequence,
        };
        self.pending.push_back(retires);
        self.issued = sequence;
        Ok(p)
    }

    /// Observe a hardware GET, WITHOUT retiring source progress or staging.
    ///
    /// # Errors
    /// Invalid/out-of-flight index. Stale observations may be refused; they must
    /// never trigger a timer-based release or a completion synthesized from GET.
    pub fn observe_get(&mut self, get: u32) -> Result<(), Refusal> {
        if get >= self.entries {
            return Err(Refusal::Fetch);
        }
        let old = (self.fetched % u64::from(self.entries)) as u32;
        let delta = (get.wrapping_sub(old)) & (self.entries - 1);
        if u64::from(delta) > self.issued - self.fetched {
            return Err(Refusal::Fetch);
        }
        self.fetched += u64::from(delta);
        Ok(())
    }

    /// Retire only from a REAL ENGINE completion proof for this channel/lifetime.
    /// The model's simulation supplies it. No production proof adapter exists;
    /// guest-written memory, a wake, registration reply or GP_GET cannot supply it.
    /// Duplicate/older authenticated receipts are harmless and retire nothing.
    ///
    /// # Errors
    /// A future receipt or allocation failure; neither changes accounting.
    pub fn complete(&mut self, sequence: u64) -> Result<Vec<Retirement>, Refusal> {
        if sequence > self.issued {
            return Err(Refusal::Completion);
        }
        if sequence <= self.completed {
            return Ok(Vec::new());
        }
        let count = usize::try_from(sequence - self.completed).map_err(|_| Refusal::Completion)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| Refusal::Allocation)?;
        out.extend(self.pending.drain(..count));
        self.completed = sequence;
        // Actual completion also implies fetch; it does not prove a new source
        // lifetime is valid. The future adapter must pin/revalidate each receipt.
        self.fetched = self.fetched.max(sequence);
        Ok(out)
    }
}
