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

    /// Model the32-bit wire receipt of a PRIVATE GPU-written reference after an
    /// authenticated ordering tail. No production adapter calls this method.
    ///
    /// Requires: native class/tail semantics validated; private USERD lifetime;
    /// immutable inspection from birth removes ALL guest SET_REFERENCE writes;
    /// no command-fetch escape, CPU completion write or guest-writable alias.
    /// A raw reference from an unfiltered guest channel is NOT authenticated.
    ///
    /// The one-empty-slot bound gives <2^31 outstanding receipts even for the
    /// largest power-of-two u32 ring. Interpret only forward differences that
    /// fit that window; reject stale/out-of-flight readings without release.
    /// Native cursor-transfer/source-generation semantics remain independent.
    ///
    /// # Errors
    /// A reference beyond the issued window, or failure to allocate retirements.
    pub fn complete_reference(&mut self, reference: u32) -> Result<Vec<Retirement>, Refusal> {
        let delta = u64::from(reference.wrapping_sub(self.completed as u32));
        if delta > self.issued - self.completed {
            return Err(Refusal::Completion);
        }
        self.complete(self.completed + delta)
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

#[cfg(test)]
mod reference_wrap {
    use super::*;

    #[test]
    fn wire_wrap_does_not_reuse_a_live_receipt_or_skip_work() {
        // Test-only seed after a simulated completed prefix. No public API
        // resets native USERD/counters or assumes a migration resets them.
        let mut q = Ledger::new(4, 4).unwrap();
        let base = u64::from(u32::MAX) - 1;
        q.issued = base;
        q.completed = base;
        q.fetched = base;
        for get in 1..=3 {
            q.publish(Retirement {
                generation: 19,
                guest_get: get,
            })
            .unwrap();
        }
        assert_eq!(q.complete_reference(2), Err(Refusal::Completion));
        assert_eq!(q.pending(), 3);
        for (wire, get) in [(u32::MAX, 1), (0, 2), (1, 3)] {
            assert_eq!(
                q.complete_reference(wire).unwrap(),
                [Retirement {
                    generation: 19,
                    guest_get: get
                }]
            );
            assert_eq!(q.complete_reference(wire).unwrap(), []);
        }
        assert_eq!(q.completed(), base + 3);
        assert_eq!(q.available(), 3);
        assert_eq!(q.complete_reference(u32::MAX), Err(Refusal::Completion));
    }

    /// 2026-10-31 — the retired #12 guard's contract (`fence_backwards_jump_is_a_loud_refusal_never_a_completion`,
    /// `fence_arm_is_wrap_correct`) against the absolute-count model: a 32-bit wire reference
    /// completes exactly the work up to the one absolute count in `[completed, issued]` it names, and
    /// anything else (a stale value, a backwards write, a count nobody published) is
    /// `Refusal::Completion` with the ledger untouched. The window is the work in flight, not a typed
    /// constant. Seeded, with the counts straddling the 32-bit wrap. (No production adapter calls
    /// `complete_reference`; this pins the rule for the day one does.)
    #[test]
    fn a_reference_completes_only_the_work_it_names_and_a_stale_one_completes_nothing() {
        let mut rng = 0x7265_6631_u64;
        let mut next = move || {
            rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let u32_range = 1u64 << 32;
        for entries in [4u32, 8, 16] {
            for base in [
                0,
                1,
                u32_range - 9,
                u32_range - 3,
                u32_range - 1,
                u32_range,
                u32_range + 1,
                3 * u32_range - 2,
            ] {
                for inflight in 0..entries {
                    let mut q = Ledger::new(entries, entries).unwrap();
                    q.issued = base;
                    q.completed = base;
                    q.fetched = base;
                    for get in 0..inflight {
                        q.publish(Retirement {
                            generation: 1,
                            guest_get: get,
                        })
                        .unwrap();
                    }
                    let (done, issued) = (q.completed, q.issued);
                    assert_eq!(issued - done, u64::from(inflight));
                    // every reference around the window, and a seeded spread over the whole space
                    let mut refs: Vec<u32> = Vec::new();
                    for d in -6i64..=i64::from(inflight) + 6 {
                        refs.push((done as i64 + d) as u32);
                    }
                    for _ in 0..24 {
                        refs.push(next() as u32);
                    }
                    for r in refs {
                        let named = (done..=issued).rev().find(|&c| c as u32 == r);
                        let mut t = q.clone();
                        match (named, t.complete_reference(r)) {
                            (Some(c), Ok(got)) => {
                                assert_eq!(t.completed(), c, "base {base} ref {r:#x}");
                                assert_eq!(got.len() as u64, c - done);
                            }
                            (None, Err(Refusal::Completion)) => {
                                assert_eq!(t.completed(), done, "a refusal changed state");
                                assert_eq!(t.pending(), inflight as usize);
                            }
                            (m, g) => panic!("base {base} ref {r:#x}: model {m:?}, ledger {g:?}"),
                        }
                    }
                }
            }
        }
    }
}
