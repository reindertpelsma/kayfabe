//! The correctness oracle: **a map that "succeeds" but points at the wrong page must fail.**
//!
//! Every page of the workload holds one 64-bit signature repeated across the whole page,
//! `sig = splitmix64(seed ^ epoch << 48 ^ page_index)`. After the invalidate the GPU copies a bounded
//! sample of whole pages through its mapping into a staging buffer the CPU can read with ordinary loads,
//! and the CPU compares **every word** of each copied page with the page's own signature. What can go
//! wrong is told apart:
//!
//! * a wrong page behind the PTE returns another page's signature -> `mismatch`;
//! * a stale translation from the previous refresh (same VA band, new backing) returns the previous
//!   epoch's signature -> `stale` (a mismatch, and named, because it is the invalidate that failed);
//! * a PTE that points at a stale *copy* of the page rather than the live page is caught by the live
//!   check: the CPU rewrites some sampled pages after the map and the GPU must see the new words.
//!
//! And the oracle must be able to fail on its own: for every sampled page the comparison is also run
//! against a deliberately wrong expectation (the signature of a different page) and **must mismatch**
//! (`decoy`). A comparison that cannot say "different" proves nothing, so a decoy that does not trip
//! fails the run (the arm's positive control, printed as `RUNGCTL_refresh_stress`).
//!
//! The decoy is run on the staged page when the page matched its signature, and on a synthetic page
//! holding the signature when it did not: that keeps the control independent of what the hardware
//! returned (a wrong page must not also look like a broken instrument), while still exercising the very
//! comparison that judges the real data.

use super::params::StressParams;
use super::rng::{SplitMix64, choose_distinct, mix};

/// The epoch tag of the live-page rewrite (above any real epoch, which is `< MAX_REFRESHES`).
pub const LIVE_EPOCH: u32 = 0xFFFF;

/// The signature word of `page` in refresh `epoch`.
#[must_use]
pub fn signature(seed: u64, epoch: u32, page: u32) -> u64 {
    mix(seed ^ (u64::from(epoch) << 48) ^ u64::from(page))
}

/// The deliberately wrong expectation for `page`: another page's signature, different from `want`.
///
/// # Panics
/// If `total < 2` (parameter validation guarantees at least two pages).
#[must_use]
pub fn decoy_word(seed: u64, epoch: u32, page: u32, total: u32, want: u64) -> u64 {
    assert!(total >= 2, "a decoy needs a second page");
    let step = 1 + (mix(seed ^ u64::from(page).rotate_left(17)) % u64::from(total - 1)) as u32;
    let other = (page + step) % total;
    debug_assert_ne!(other, page);
    let w = signature(seed, epoch, other);
    if w == want { want ^ 1 } else { w }
}

/// The pages the oracle samples in one refresh, grouped by batch: for each batch its first page, its last
/// page, and `sample_per_batch - 2` seeded random pages between them (distinct).
#[must_use]
pub fn plan_sample(p: &StressParams, seed: u64, epoch: u32) -> Vec<u32> {
    let mut rng = SplitMix64::new(mix(seed ^ 0x5A5A_0000_0000 ^ u64::from(epoch)));
    let per = p.sample_per_batch();
    let mut out = Vec::with_capacity(p.sample as usize);
    for b in 0..p.batches {
        let first = b * p.pages_per_batch;
        let last = first + p.pages_per_batch - 1;
        out.push(first);
        out.push(last);
        out.extend(choose_distinct(first + 1, last, per - 2, &mut rng));
    }
    out
}

/// One word that was not what the page holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    /// The page index within the refresh.
    pub page: u32,
    /// Index of the first wrong word within the page.
    pub word: u32,
    /// What the GPU copy returned.
    pub got: u64,
    /// What the page holds.
    pub want: u64,
}

/// Counts the oracle accumulates over a run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OracleTally {
    /// Pages compared.
    pub checked: u64,
    /// Pages with at least one wrong word.
    pub mismatches: u64,
    /// Of those, pages whose every word equals the previous epoch's signature (a stale translation).
    pub stale: u64,
    /// The first mismatch, for the report.
    pub first: Option<Mismatch>,
    /// Decoy comparisons made.
    pub decoy_checked: u64,
    /// Decoy comparisons that (correctly) found the data different from the decoy.
    pub decoy_tripped: u64,
    /// Live-page comparisons made.
    pub live_checked: u64,
    /// Live-page comparisons that failed.
    pub live_mismatches: u64,
}

impl OracleTally {
    /// Judge one staged page against its signature, the previous epoch's signature (if there was a
    /// previous epoch) and its decoy. `decoy` is never equal to `want` unless the caller planted a
    /// collision (the control must then fail).
    pub fn judge_page(
        &mut self,
        page: u32,
        words: &[u64],
        want: u64,
        previous: Option<u64>,
        decoy: u64,
    ) {
        self.checked += 1;
        if let Some(i) = words.iter().position(|&w| w != want) {
            self.mismatches += 1;
            if previous.is_some_and(|prev| words.iter().all(|&w| w == prev)) {
                self.stale += 1;
            }
            self.first.get_or_insert(Mismatch {
                page,
                word: u32::try_from(i).unwrap_or(u32::MAX),
                got: words[i],
                want,
            });
        }
        self.decoy_checked += 1;
        let matched = words.iter().all(|&w| w == want);
        let tripped = if matched {
            words.iter().any(|&w| w != decoy)
        } else {
            // The staged data is wrong; test the instrument on what the page should have held.
            decoy != want
        };
        if tripped {
            self.decoy_tripped += 1;
        }
    }

    /// Judge one live-page readback: every word must be the rewritten value.
    pub fn judge_live(&mut self, words: &[u64], want: u64) {
        self.live_checked += 1;
        if words.iter().any(|&w| w != want) {
            self.live_mismatches += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: usize = 512;

    #[test]
    fn signatures_differ_by_page_epoch_and_seed() {
        let s = signature(1, 0, 5);
        assert_ne!(s, signature(1, 0, 6));
        assert_ne!(s, signature(1, 1, 5));
        assert_ne!(s, signature(2, 0, 5));
        assert_eq!(s, signature(1, 0, 5));
        assert_ne!(signature(1, LIVE_EPOCH, 5), signature(1, 7, 5));
    }

    #[test]
    fn the_decoy_is_another_pages_signature_and_never_the_real_one() {
        for page in [0u32, 1, 2, 1000, 98_303] {
            for epoch in 0..3 {
                let want = signature(9, epoch, page);
                let d = decoy_word(9, epoch, page, 98_304, want);
                assert_ne!(d, want);
            }
        }
        // Two pages only: the decoy is the other one.
        let want = signature(3, 0, 0);
        assert_eq!(decoy_word(3, 0, 0, 2, want), signature(3, 0, 1));
        // Even a planted collision cannot make the decoy equal the expectation.
        assert_ne!(
            decoy_word(3, 0, 0, 2, signature(3, 0, 1)),
            signature(3, 0, 1)
        );
    }

    #[test]
    fn the_sample_has_the_designed_shape() {
        let p = StressParams::default();
        let s = plan_sample(&p, 0xfeed, 0);
        assert_eq!(s.len(), 1536, "1.56% of 98,304 pages");
        for b in 0..24usize {
            let chunk = &s[b * 64..(b + 1) * 64];
            let lo = (b as u32) * 4096;
            assert_eq!(chunk[0], lo, "first page of batch {b}");
            assert_eq!(chunk[1], lo + 4095, "last page of batch {b}");
            let mut d = chunk.to_vec();
            d.sort_unstable();
            d.dedup();
            assert_eq!(d.len(), 64, "distinct within batch {b}");
            assert!(chunk.iter().all(|&x| x >= lo && x < lo + 4096));
        }
        assert_eq!(s, plan_sample(&p, 0xfeed, 0), "deterministic");
        assert_ne!(
            s,
            plan_sample(&p, 0xfeed, 1),
            "a new epoch draws a new sample"
        );
        assert_ne!(
            s,
            plan_sample(&p, 0xbeef, 0),
            "a new seed draws a new sample"
        );
    }

    #[test]
    fn a_tiny_batch_is_sampled_whole() {
        let p = StressParams {
            batches: 2,
            pages_per_batch: 4,
            sample: 8,
            live: 1,
            ..StressParams::default()
        };
        let s = plan_sample(&p, 1, 0);
        let mut a = s[..4].to_vec();
        a.sort_unstable();
        assert_eq!(a, vec![0, 1, 2, 3]);
        assert_eq!(s.len(), 8);
    }

    fn page(w: u64) -> Vec<u64> {
        vec![w; WORDS]
    }

    #[test]
    fn a_correct_page_passes_and_its_decoy_trips() {
        let mut t = OracleTally::default();
        let want = signature(1, 0, 7);
        t.judge_page(7, &page(want), want, None, decoy_word(1, 0, 7, 100, want));
        assert_eq!(
            (t.checked, t.mismatches, t.decoy_checked, t.decoy_tripped),
            (1, 0, 1, 1)
        );
        assert!(t.first.is_none());
    }

    #[test]
    fn one_wrong_word_anywhere_in_the_page_fails_and_is_located() {
        for bad in [0usize, 1, 255, 511] {
            let mut t = OracleTally::default();
            let want = signature(1, 0, 7);
            let mut w = page(want);
            w[bad] ^= 1 << 40;
            t.judge_page(7, &w, want, None, decoy_word(1, 0, 7, 100, want));
            assert_eq!(t.mismatches, 1);
            assert_eq!(t.first.map(|m| (m.page, m.word)), Some((7, bad as u32)));
            assert_eq!(t.stale, 0, "a single flipped word is not a stale page");
        }
    }

    #[test]
    fn another_pages_content_fails_and_the_previous_epochs_content_is_named_stale() {
        let mut t = OracleTally::default();
        let want = signature(1, 1, 7);
        let prev = signature(1, 0, 7);
        t.judge_page(
            7,
            &page(signature(1, 1, 8)),
            want,
            Some(prev),
            decoy_word(1, 1, 7, 100, want),
        );
        assert_eq!((t.mismatches, t.stale), (1, 0), "wrong page");
        t.judge_page(
            7,
            &page(prev),
            want,
            Some(prev),
            decoy_word(1, 1, 7, 100, want),
        );
        assert_eq!((t.mismatches, t.stale), (2, 1), "stale translation");
        t.judge_page(
            7,
            &page(0),
            want,
            Some(prev),
            decoy_word(1, 1, 7, 100, want),
        );
        assert_eq!((t.mismatches, t.stale), (3, 1), "zeros are neither");
    }

    #[test]
    fn a_decoy_that_equals_the_data_does_not_trip_so_the_control_can_fail() {
        // Plant a collision: the "wrong" expectation equals the staged data. The comparison then cannot
        // distinguish them, and the tally must say so (decoy_tripped < decoy_checked).
        let mut t = OracleTally::default();
        let want = 0x1234;
        t.judge_page(0, &page(want), want, None, want);
        assert_eq!((t.decoy_checked, t.decoy_tripped), (1, 0));
    }

    #[test]
    fn a_wrong_page_does_not_masquerade_as_a_broken_instrument() {
        // The staged page is the NEXT page's content, which is also exactly what a decoy built from the
        // next page would hold. The primary comparison fails; the control still trips.
        let mut t = OracleTally::default();
        let want = signature(1, 0, 7);
        let wrong = signature(1, 0, 8);
        t.judge_page(7, &page(wrong), want, None, wrong);
        assert_eq!((t.mismatches, t.decoy_checked, t.decoy_tripped), (1, 1, 1));
    }

    #[test]
    fn the_live_check_sees_a_stale_copy() {
        let mut t = OracleTally::default();
        let new = signature(1, LIVE_EPOCH, 3);
        t.judge_live(&page(new), new);
        t.judge_live(&page(signature(1, 0, 3)), new);
        assert_eq!((t.live_checked, t.live_mismatches), (2, 1));
    }
}
