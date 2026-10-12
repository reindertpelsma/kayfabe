// Included with native or Loom AtomicU64; no alternate algorithm.

/// One entry's dispatch, publication exclusion and exact allocation lifetime.
pub struct DispatchWord {
    pub(super) word: AtomicU64,
}

/// Result of a bounded callback claim. WakeWorker must publish a durable token
/// bit using the existing wake protocol, not just signal an eventfd. Direct
/// contention wakes a direct-token drain, never command inspection.
pub enum Begin<'a> {
    /// Pre-mapped row is exclusively owned until Ticket::finish.
    Inline(Ticket<'a>),
    /// Worker owns dispatch, inline owner is busy, or a CAS lost.
    WakeWorker,
    /// Dead or retiring; no work or wake.
    Ignore,
}

/// Exclusive row lifetime. No Drop cleanup: abandoning a ticket retains BUSY
/// and therefore cannot license Free/reuse or pretend native work completed.
#[must_use = "finish the claim after bounded publication, or retain it on failure"]
pub struct Ticket<'a> {
    row: &'a DispatchWord,
    claimed: u64,
}

impl Default for DispatchWord {
    fn default() -> Self {
        Self::new()
    }
}
impl DispatchWord {
    /// Initially unallocated, clean and generation zero.
    pub fn new() -> Self {
        Self {
            word: AtomicU64::new(0),
        }
    }

    /// Publish already prepared metadata. Only an off-vCPU lifecycle controller
    /// calls this, after the previous native channel/storage cleanup succeeded.
    pub fn install(&self, target: Mode, host_token: u32) -> Result<u32, InstallError> {
        if target == Mode::Dead {
            return Err(InstallError::DeadMode);
        }
        let old = self.word.load(Ordering::Acquire);
        if mode(old) != Mode::Dead || old & (BUSY | RETIRING) != 0 {
            return Err(InstallError::NotClean);
        }
        let next = u64::from(generation(old)) + 1;
        if next > MAX_GENERATION {
            return Err(InstallError::GenerationExhausted);
        }
        let new =
            (next << GENERATION_SHIFT) | (u64::from(host_token) << HOST_SHIFT) | target as u64;
        self.word
            .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| next as u32)
            .map_err(|_| InstallError::Retry)
    }

    /// Fixed-cost MMIO/ioeventfd callback admission. The caller copies at most
    /// MAX_INLINE_GP descriptors, never PB words, and finishes the returned ticket.
    /// Resolve row metadata AFTER this claim against the ticket generation: a
    /// delayed callback can legally claim a replacement allocation, not the row
    /// it observed before calling this method.
    pub fn begin_inline(&self) -> Begin<'_> {
        self.begin_inline_after_observe(|| {})
    }

    // Test hook at the real initial-observation gap; production uses a no-op.
    pub(super) fn begin_inline_after_observe(&self, after_observe: impl FnOnce()) -> Begin<'_> {
        let seen = self.word.load(Ordering::Acquire);
        after_observe();
        if mode(seen) == Mode::Dead || seen & RETIRING != 0 {
            return Begin::Ignore;
        }
        // Record pending before attempting ownership. A concurrent owner that
        // consumes it starts AFTER this publication and reads the latest PUT.
        let old = self.word.fetch_or(PENDING, Ordering::AcqRel) | PENDING;
        if mode(old) == Mode::Dead || old & RETIRING != 0 {
            return Begin::Ignore;
        }
        if mode(old) == Mode::Worker || old & BUSY != 0 {
            return Begin::WakeWorker;
        }
        match self.word.compare_exchange(
            old,
            (old | BUSY) & !PENDING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Begin::Inline(Ticket {
                row: self,
                claimed: old,
            }),
            Err(_) => Begin::WakeWorker,
        }
    }

    /// Select inspection before accepting a software object/registration. This
    /// prevents NEW inline claims; an old claim must finish before worker admission.
    /// It deliberately does not claim the old native stream has drained.
    pub fn request_worker(&self, expected: u32) -> Change {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected
            || matches!(mode(old), Mode::Dead | Mode::Direct)
            || old & RETIRING != 0
        {
            return Change::Stale;
        }
        if mode(old) == Mode::Worker {
            return Change::Done;
        }
        let new = (old & !MODE) | Mode::Worker as u64 | HANDOFF | PENDING;
        self.cas(old, new)
    }

    /// Fresh-channel handoff ONLY. Empty GET/PUT is not the condition: this checks
    /// never-published history and excludes a claimant that might still publish.
    pub fn finish_fresh_handoff(&self, expected: u32) -> Change {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected || mode(old) != Mode::Worker || old & RETIRING != 0 {
            return Change::Stale;
        }
        if old & BUSY != 0 {
            return Change::Busy;
        }
        if old & HANDOFF == 0 {
            return Change::Done;
        }
        if old & PUBLISHED != 0 {
            return Change::NativeHandoffPending;
        }
        self.cas(old, old & !HANDOFF)
    }

    /// Worker claim uses the same exclusion as inline copying. Caller must still
    /// respect the existing token's ordering stamp, VA gate and completion kind.
    pub fn begin_worker(&self, expected: u32) -> Option<Ticket<'_>> {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected
            || matches!(mode(old), Mode::Dead | Mode::Direct)
            || old & (BUSY | RETIRING | HANDOFF) != 0
        {
            return None;
        }
        self.word
            .compare_exchange(
                old,
                (old | BUSY) & !PENDING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        Some(Ticket {
            row: self,
            claimed: old,
        })
    }

    /// Off-vCPU service for a contended Direct doorbell. This only forwards the
    /// opaque native token; it neither copies rings nor inspects commands, and
    /// cannot convert Linux Passthrough or introduce host-owned GPU mappings.
    pub fn begin_direct_drain(&self, expected: u32) -> Option<Ticket<'_>> {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected
            || mode(old) != Mode::Direct
            || old & (BUSY | RETIRING | HANDOFF) != 0
        {
            return None;
        }
        self.word
            .compare_exchange(
                old,
                (old | BUSY) & !PENDING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        Some(Ticket {
            row: self,
            claimed: old,
        })
    }

    /// Stop new claims. The existing claimant remains owned until it finishes.
    pub fn request_retire(&self, expected: u32) -> Change {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected || mode(old) == Mode::Dead {
            return Change::Stale;
        }
        self.cas(old, old | RETIRING)
    }

    /// Permission to proceed with native cleanup, not proof it happened.
    pub fn finish_retire(&self, expected: u32) -> Change {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected || old & RETIRING == 0 {
            return Change::Stale;
        }
        if old & BUSY != 0 {
            return Change::Busy;
        }
        let dead = (u64::from(expected) << GENERATION_SHIFT) | RETIRING;
        self.cas(old, dead)
    }

    /// Controller calls ONLY after successful native cleanup. Failure leaves
    /// RETIRING set; no timer, guest cursor or missing event grants reuse.
    pub fn confirm_native_cleanup(&self, expected: u32) -> Change {
        let old = self.word.load(Ordering::Acquire);
        if generation(old) != expected
            || mode(old) != Mode::Dead
            || old & (BUSY | RETIRING) != RETIRING
        {
            return Change::Stale;
        }
        self.cas(old, old & !RETIRING)
    }

    /// Acquire view used by the worker's durable-work check.
    pub fn pending(&self) -> bool {
        self.word.load(Ordering::Acquire) & PENDING != 0
    }

    fn cas(&self, old: u64, new: u64) -> Change {
        match self
            .word
            .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Change::Done,
            Err(_) => Change::Retry,
        }
    }
}

impl Ticket<'_> {
    /// Full opaque host token; no field bits are stripped or reinterpreted.
    pub fn host_token(&self) -> u32 {
        (self.claimed >> HOST_SHIFT) as u32
    }
    /// Exact allocation lifetime for the associated pre-created row.
    pub fn generation(&self) -> u32 {
        generation(self.claimed)
    }
    /// Claimed dispatch policy, not the channel's GPU permission/completion kind.
    pub fn mode(&self) -> Mode {
        mode(self.claimed)
    }

    /// Recheck before raw publication. A cutover racing AFTER this check is still
    /// safe for CPU exclusion: BUSY remains held, and mark_published records the
    /// native-handoff requirement BEFORE PUT can become visible.
    pub fn may_publish_raw(&self) -> bool {
        let now = self.row.word.load(Ordering::Acquire);
        matches!(self.mode(), Mode::Direct | Mode::CopyRing)
            && mode(now) == self.mode()
            && now & RETIRING == 0
    }

    /// Must precede ANY native PUT/doorbell publication, even a publication that
    /// later reports failure. This records history, not native completion.
    pub fn mark_published(&self) {
        self.row.word.fetch_or(PUBLISHED, Ordering::Release);
    }

    /// Release after the bounded memory/MMIO body. True requests a durable worker
    /// wake; no registered pending doorbell may be silently discarded.
    pub fn finish(self) -> bool {
        let old = self.row.word.fetch_and(!BUSY, Ordering::AcqRel);
        old & (PENDING | RETIRING) != 0
    }
}
