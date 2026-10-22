//! The rung bitmap — **scanning only, never CAS'd** (§5.1).
//!
//! §5.1 sizes it: token words are 4–16 MiB and are *"never scanned"*; the bitmap is **64 KiB**
//! with a **1 KiB summary** and is *"what a worker walks"*. ⇒ A worker touches 65 KiB to find
//! work among 2²¹ tokens, instead of 16 MiB.
//!
//! ⚠ **The bitmap is a HINT; the word is the TRUTH** (§5.1). A bit set over an IDLE word costs
//! one look. The bitmap may over-report; it must never under-report.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// 2²¹ tokens ⇒ 2²¹ bits = 256 KiB of bits... §5.1 quotes 64 KiB for the *expressible* range in
/// the common (Ampere, 19-bit) case. We size from the constant so the two cannot drift.
/// ⊘⊘ **This is a TABLE INDEX, not the host token.** `[fable w823]` they were the same constant,
/// so widening the host token to 32 bits would have allocated a 4-billion-entry bitmap, and
/// narrowing the index would have silently aliased guest tokens.
///
/// ★ The index is bounded by what the guest's doorbell register can *address*:
/// `VECTOR 11:0` + `RUNLIST_ID 22:16` ⇒ 12 + 7 = **19 bits** of addressable channel on Ampere,
/// and the per-die doorbell-type bits (GB202 bit 30, GB100 bits 22/31) are **not** part of the
/// index — they are flags on the value, which is exactly why the two must not share a constant.
pub const TOKEN_BITS: u32 = 19;
pub const N_TOKENS: usize = 1 << TOKEN_BITS;
pub const N_WORDS: usize = N_TOKENS / 64;
pub const N_SUMMARY: usize = N_WORDS / 64;

pub struct RungBitmap {
    words: Box<[AtomicU64]>,
    summary: Box<[AtomicU64]>,
    /// ⊘⊘⊘ **THE SCAN START ROTATES, AND WITHOUT THAT THIS PLANE HAS A STARVATION HOLE THE
    /// THREAT MODEL EXISTS TO PREVENT.**
    ///
    /// `[fable review, w823 — CRITICAL]` the first version always began at summary word 0 and
    /// took the first `limit` set bits in **ascending token order**. The guest's chid allocator
    /// hands out ascending ids, so an early-starting unprivileged process owns the low tokens.
    /// Ringing ≥`limit` of them in a loop fills every scan, and a higher-numbered token —
    /// **the kernel's scrubber and UVM channels** — is never reached.
    /// `[reproduced]` 10 000 worker passes, attacker on tokens 0..63 ⇒ kernel token 100 served
    /// **0 times**, still `Rung`.
    ///
    /// ⚠ **The §5.2 timeslice does not help**, and believing it did is how this shipped: `K`
    /// bounds how long one worker holds ONE claim. It says nothing about which tokens a scan
    /// looks at. Fairness of *holding* is not fairness of *finding*.
    next_start: AtomicUsize,
}

impl Default for RungBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl RungBitmap {
    pub fn new() -> RungBitmap {
        RungBitmap {
            words: (0..N_WORDS)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            summary: (0..N_SUMMARY)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            next_start: AtomicUsize::new(0),
        }
    }

    /// ★★★ PUBLISH. §5.2, stated as an absolute:
    ///
    /// > *"Re-publish means the bitmap bit AND the summary bit, **in that order**. Setting only
    /// > the summary loses the token PERMANENTLY: the next worker clears the summary, finds the
    /// > group word zero, and moves on — and a further ring from the guest returns early because
    /// > RUNG is already set, so it produces no bit and no wake."*
    ///
    /// ⇒ **bit, then summary. The trap and the put-back use the same order.** This is the only
    /// function that sets either, so the order cannot be got wrong at a call site.
    #[inline]
    pub fn publish(&self, token: u32) {
        let t = (token as usize) & (N_TOKENS - 1);
        let w = t / 64;
        // Release: everything the vCPU wrote to the work source happens-before this bit is
        // visible (§5.3's rule).
        self.words[w].fetch_or(1u64 << (t % 64), Ordering::Release);
        self.summary[w / 64].fetch_or(1u64 << (w % 64), Ordering::Release);
    }

    /// Scan for a set bit, clearing as it goes.
    ///
    /// ⊘⊘ §5.1: *"**Clear the summary bit first, then re-read the word.** Clearing after a scan
    /// loses a token that arrived during it."* ⇒ We clear the summary BEFORE reading the group
    /// word. If a publisher sets a bit between our clear and our read, we still see the bit. If
    /// it sets the bit after our read, it re-sets the summary and the next scan finds it.
    /// Clearing after would open a window where the bit is set and no summary points at it.
    pub fn scan(&self, out: &mut Vec<u32>, limit: usize) -> usize {
        out.clear();
        if limit == 0 {
            return 0;
        }
        // Resume after the last TOKEN, including a partially consumed word.
        // A word-only cursor can wrap back to a hot prefix in two words and
        // starve the last word's unreturned bits even at the production limit.
        let base = self.next_start.load(Ordering::Relaxed);
        for (lo, hi) in [(base, N_TOKENS), (0, base)] {
            if lo == hi {
                continue;
            }
            let first_word = lo / 64;
            let last_word = (hi - 1) / 64;
            for si in first_word / 64..=last_word / 64 {
                let mut s = self.summary[si].load(Ordering::Acquire);
                if si == first_word / 64 {
                    s &= u64::MAX << (first_word % 64);
                }
                if si == last_word / 64 {
                    s &= u64::MAX >> (63 - last_word % 64);
                }
                while s != 0 {
                    let b = s.trailing_zeros() as usize;
                    s &= s - 1;
                    let w = si * 64 + b;
                    // Clear summary BEFORE taking the word. A racing publisher
                    // is observed here or leaves its summary for a later scan.
                    self.summary[si].fetch_and(!(1u64 << b), Ordering::AcqRel);
                    let mut word = self.words[w].swap(0, Ordering::AcqRel);
                    let mut mask = u64::MAX;
                    if w == first_word {
                        mask &= u64::MAX << (lo % 64);
                    }
                    if w == last_word {
                        mask &= u64::MAX >> (63 - (hi - 1) % 64);
                    }
                    let excluded = word & !mask;
                    word &= mask;
                    // Bits outside this wrap range are still owed. Re-publish
                    // word BEFORE summary, exactly as publish does.
                    if excluded != 0 {
                        self.words[w].fetch_or(excluded, Ordering::Release);
                        self.summary[si].fetch_or(1u64 << b, Ordering::Release);
                    }
                    while word != 0 {
                        let t = word.trailing_zeros() as usize;
                        word &= word - 1;
                        let token = w * 64 + t;
                        out.push(token as u32);
                        if out.len() >= limit {
                            if word != 0 {
                                self.words[w].fetch_or(word, Ordering::Release);
                                self.summary[si].fetch_or(1u64 << b, Ordering::Release);
                            }
                            self.next_start
                                .store((token + 1) % N_TOKENS, Ordering::Relaxed);
                            return out.len();
                        }
                    }
                }
            }
        }
        self.next_start
            .store((base + 1) % N_TOKENS, Ordering::Relaxed);
        out.len()
    }

    /// ★ Device reset (`docs/design/V3_DEVICE_RESET.md`): no bit, no summary, the scan starting at
    /// 0 — a fresh bitmap. ⊘ Only once no token can be published (every channel retired, the vCPU
    /// gate closed); a stale bit left behind would at worst make a worker claim a fresh token and
    /// find it not rung, which is why this is hygiene rather than the safety mechanism.
    pub fn clear(&self) {
        for w in self.summary.iter().chain(self.words.iter()) {
            w.store(0, Ordering::Release);
        }
        self.next_start.store(0, Ordering::Relaxed);
    }

    /// Read-only inspection of a summary bit (tests and census; never a decision input).
    #[must_use]
    pub fn summary_bit(&self, token: u32) -> bool {
        let w = (token as usize & (N_TOKENS - 1)) / 64;
        self.summary[w / 64].load(Ordering::Acquire) & (1u64 << (w % 64)) != 0
    }
    /// Read-only inspection of a token's bit (tests and census; never a decision input).
    #[must_use]
    pub fn bit(&self, token: u32) -> bool {
        let t = token as usize & (N_TOKENS - 1);
        self.words[t / 64].load(Ordering::Acquire) & (1u64 << (t % 64)) != 0
    }
}
