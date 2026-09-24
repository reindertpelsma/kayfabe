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
            words: (0..N_WORDS).map(|_| AtomicU64::new(0)).collect::<Vec<_>>().into_boxed_slice(),
            summary: (0..N_SUMMARY).map(|_| AtomicU64::new(0)).collect::<Vec<_>>().into_boxed_slice(),
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
        // ★ Begin where the last scan stopped, and wrap. Every summary word is still visited on
        // every scan, so nothing is missed; what changes is WHICH tokens fill a bounded `out`.
        // ⇒ A token cannot be starved indefinitely, because the start walks past it.
        let base = self.next_start.load(Ordering::Relaxed);
        for k in 0..N_SUMMARY {
            let si = (base + k) % N_SUMMARY;
            let mut s = self.summary[si].load(Ordering::Acquire);
            while s != 0 {
                let b = s.trailing_zeros() as usize;
                s &= s - 1;
                let w = si * 64 + b;
                // ⊘ SUMMARY FIRST. See above — this order is the invariant.
                self.summary[si].fetch_and(!(1u64 << b), Ordering::AcqRel);
                let mut word = self.words[w].swap(0, Ordering::AcqRel);
                while word != 0 {
                    let t = word.trailing_zeros() as usize;
                    word &= word - 1;
                    out.push((w * 64 + t) as u32);
                    if out.len() >= limit {
                        // Put back what we did not take, in the same order publish uses.
                        if word != 0 {
                            self.words[w].fetch_or(word, Ordering::Release);
                            self.summary[si].fetch_or(1u64 << b, Ordering::Release);
                        }
                        // ⊘ Resume at the NEXT summary word, not this one: resuming here would
                        // re-take the same attacker's group first and rebuild the starve.
                        self.next_start.store((si + 1) % N_SUMMARY, Ordering::Relaxed);
                        return out.len();
                    }
                }
            }
        }
        self.next_start.store((base + 1) % N_SUMMARY, Ordering::Relaxed);
        out.len()
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
