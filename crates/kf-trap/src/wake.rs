//! The wakeup word — §5.3.
//!
//! ```text
//! worker_vcpu_poll : u64 = { work_seq : 56 (HIGH) , workers_polling : 8 (LOW) }
//! ```
//!
//! ★ **`work_seq` is the HIGH half, and that is the whole reason the split is 56/8.** §5.3: *"a
//! carry then falls off the top of the word instead of landing in the poller count as a phantom
//! poller that makes every later trap pay a syscall."* ⇒ If the sequence were low, its overflow
//! would increment `workers_polling`, and a phantom poller means every subsequent vCPU trap
//! writes the eventfd for a waiter that does not exist.
//!
//! ⊘ 56 bits is **not** about global wrap — it is about wrapping *inside one scan*. Every vCPU
//! CASes the same cacheline, so the aggregate rate is capped by ping-pong at ~10⁷/s ⇒ ~228 years.

use core::sync::atomic::{AtomicU64, Ordering};

const POLLER_BITS: u32 = 8;
const POLLER_MASK: u64 = (1 << POLLER_BITS) - 1;
const SEQ_ONE: u64 = 1 << POLLER_BITS;

/// §3 caps workers at 255 — which is exactly what 8 bits holds, and is why it is 8.
pub const MAX_WORKERS: u64 = POLLER_MASK;

#[derive(Debug, Default)]
pub struct WakeWord(AtomicU64);

/// What a publisher must do after bumping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Nobody was parked — the scan will find it. ★ The common case, and it costs no syscall.
    NoOne,
    /// At least one worker had registered as polling: write the eventfd **once**.
    ///
    /// ⚠ §5.3: the eventfd is a **sum, not a queue**, `EFD_SEMAPHORE | EFD_NONBLOCK`, and a wake
    /// must reach **ONE** waiter. *"A plain write wakes every non-exclusive waiter, so at 255
    /// workers a single doorbell wakes 255 threads and 254 of them find nothing."*
    SignalOne,
}

impl WakeWord {
    pub const fn new() -> WakeWord {
        WakeWord(AtomicU64::new(0))
    }

    /// vCPU: **publish first, then bump.** §5.3's rule: *"every write to a work source
    /// happens-before the bump."* `AcqRel` gives that on both sides.
    #[inline]
    pub fn bump(&self) -> Wake {
        let prev = self.0.fetch_add(SEQ_ONE, Ordering::AcqRel);
        if prev & POLLER_MASK != 0 { Wake::SignalOne } else { Wake::NoOne }
    }

    /// Worker: read the sequence **before** scanning. §5.3: *"every `seen` load happens-before
    /// the scan."*
    #[inline]
    pub fn seen(&self) -> u64 {
        self.0.load(Ordering::Acquire) >> POLLER_BITS
    }

    /// Worker: scanned and found nothing; try to park.
    ///
    /// ★ This is `prepare_to_wait`-then-`schedule` (§5.3), and it *"removes the question of who
    /// clears a flag and when"*: we only register as polling if the sequence has not moved since
    /// `seen`. If it has, work arrived during our scan and we must rescan instead of sleeping —
    /// the lost-wakeup race, closed without a flag.
    ///
    /// Returns `true` if registered (caller may block on the eventfd), `false` if it must rescan.
    pub fn try_park(&self, seen: u64) -> bool {
        let mut cur = self.0.load(Ordering::Acquire);
        loop {
            if (cur >> POLLER_BITS) != seen {
                return false; // work arrived during the scan
            }
            let n = cur & POLLER_MASK;
            if n >= MAX_WORKERS {
                return false; // 8 bits is the cap; never wrap into the sequence
            }
            match self.0.compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true,
                Err(seen_now) => cur = seen_now,
            }
        }
    }

    /// Worker: woke up (or gave up); deregister.
    #[inline]
    pub fn unpark(&self) {
        let prev = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prev & POLLER_MASK != 0, "unpark() without a matching try_park()");
    }

    #[inline]
    pub fn pollers(&self) -> u64 {
        self.0.load(Ordering::Acquire) & POLLER_MASK
    }
}
