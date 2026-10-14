//! ★ 2026-10-14 — **when the USER-ring relay may READ a guest GP entry's pushbuffer**
//! (`docs/design/V3_USER_RING_SCAN_RACE.md`).
//!
//! Public `dev_pbdma.ref` (`NV_PPBDMA_GP_ENTRY1_SYNC`): with `SYNC_WAIT`, Host does not begin
//! fetching the entry's pushbuffer segment until the PRECEDING segment has been processed — and
//! for a `LENGTH == 0` entry, "no further GP entries will be processed until the wait finishes".
//! A guest may therefore publish entries behind a `SYNC_WAIT` whose pushbuffers it writes only
//! later, gated by a semaphore acquire in the preceding segment. Measured on Win11 + 580.88
//! (`traces/user_ring_scan_20261014/`): every scan refusal and every stale read sat behind a
//! `SYNC_WAIT` NOP the GPU had not yet reached; the relay had scanned bytes the GPU never fetches
//! (all-zero 88-word segments, filled milliseconds later), refused them, or sized a stale data word
//! as a packet header and lost its place for every later segment.
//!
//! [`SyncGate`] is the hardware's own rule, authored on the relay's private ring: before reading
//! anything at or after a guest `SYNC_WAIT` entry, publish a barrier (`SYNC_WAIT` NOP) and a
//! witness, and read on only once the witness has BEGUN processing (`GP_GET` past it) — the
//! barrier's wait, i.e. the preceding segment's processing, is then complete. That is the moment
//! the hardware itself would fetch.
//!
//! [`Grace`] is defence in depth: a scan refusal of a segment that was never published is retried
//! (re-read, rescanned) for [`REFUSE_AFTER`] before it is final. Measured after the gate: no
//! stale reads at all; the grace exists so a visibility window nobody has measured yet costs a
//! delay, not a dead channel and an adapter reset. Neither grants anything; a refusal after the
//! grace is exactly the refusal it was before, and hardware USER privilege enforces isolation.

use std::time::{Duration, Instant};

/// How long the same unpublished segment must keep refusing before the refusal is final.
pub const REFUSE_AFTER: Duration = Duration::from_millis(50);

/// What the relay does with the guest entry at its cursor ([`SyncGate::entry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Read and relay it now.
    Proceed,
    /// Author a barrier + witness on the private ring first, then call [`SyncGate::published`].
    PublishBarrier,
    /// The witness has not begun processing yet: read nothing, come back later.
    Wait,
}

/// Per-channel `SYNC_WAIT` state ([`Gate`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncGate {
    /// Private-ring sequence the witness must reach (fetched ≥ it) before reading on.
    witness: Option<u64>,
    /// The guest entry `(ring generation, cursor)` whose wait is already proven.
    cleared: Option<(u64, u32)>,
}

impl SyncGate {
    /// The guest entry `raw` sits at `(generation, cursor)`; `fetched` is the private ring's
    /// current fetch count. Entries without `SYNC_WAIT` (bit 63, `GP_ENTRY1_SYNC`) always proceed.
    pub fn entry(&mut self, raw: u64, generation: u64, cursor: u32, fetched: u64) -> Gate {
        if raw >> 63 & 1 == 0 || self.cleared == Some((generation, cursor)) {
            return Gate::Proceed;
        }
        match self.witness {
            None => Gate::PublishBarrier,
            Some(w) if fetched >= w => {
                self.witness = None;
                self.cleared = Some((generation, cursor));
                Gate::Proceed
            }
            Some(_) => Gate::Wait,
        }
    }

    /// The barrier and witness are on the private ring; `issued` is its issue count after them.
    pub fn published(&mut self, issued: u64) {
        self.witness = Some(issued);
    }

    /// Whether a witness is outstanding (the caller keeps its worker ticking).
    pub fn waiting(&self) -> bool {
        self.witness.is_some()
    }
}

/// Bounded retry of a scan refusal ([`REFUSE_AFTER`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Grace {
    first: Option<(u64, u32, Instant)>,
}

impl Grace {
    /// A scan of the unpublished entry `(generation, cursor)` refused at `now`: `true` = retry
    /// later (re-read and rescan), `false` = the refusal is final.
    pub fn refused(&mut self, generation: u64, cursor: u32, now: Instant) -> bool {
        match self.first {
            Some((g, c, at)) if g == generation && c == cursor => {
                now.saturating_duration_since(at) < REFUSE_AFTER
            }
            _ => {
                self.first = Some((generation, cursor, now));
                true
            }
        }
    }
    /// The entry scanned clean: forget the refusal.
    pub fn clear(&mut self) {
        self.first = None;
    }
    /// Whether a refusal is being retried.
    pub fn retrying(&self) -> bool {
        self.first.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_scan::{Fetch, Refusal, Scan};

    const SYNC_NOP: u64 = 1 << 63 | 1 << 41;
    fn seg(va: u64, words: u64, sync: bool) -> u64 {
        kf_abi::submit::gp_entry(va, words * 4).unwrap() | if sync { 1 << 63 } else { 0 }
    }

    /// ★ The measured failure, as an engine replay. Guest ring (Win11, run 2990 `ringtrace5`,
    /// entries 447-453): `[SYNC NOP, seg A, seg B, SYNC NOP, seg C, seg D]`; segment D's bytes are
    /// written by the guest only after the GPU passes the semaphore acquire in segment C. A relay
    /// that reads D when the guest's PUT names it scans zeros-then-tail and refuses `Legacy`; the
    /// gate makes it wait until the hardware would fetch D.
    #[test]
    fn a_segment_behind_an_unreached_sync_wait_is_not_read_until_the_witness_runs() {
        // D as the GPU will fetch it, and D as the relay read it early (front lines not yet written).
        let filled = {
            let mut w = vec![
                0x8000_0800u32,
                0x8011_0810,
                0xa002_0815,
                0,
                0x0425_2900,
                0x800e_0817,
                0x8016_0813,
                0xa007_0045,
            ];
            w.extend([
                0x340,
                3,
                0x58c0_0000,
                0x71cc_0301,
                3,
                0x18c0_0000,
                0x318c_0300,
            ]);
            w.extend([0x8000_0e6a, 0xe000_0000]);
            w
        };
        let mut early = filled.clone();
        early[..13].fill(0);
        // The old relay read D at once: its scan refuses the tail's data word as a header.
        assert_eq!(
            Scan::default().segment(&early, |_| false),
            Err(Refusal::Legacy)
        );
        assert_eq!(Scan::default().segment(&filled, |_| false), Ok(None));

        // The gated relay over the same ring.
        let ring = [
            SYNC_NOP,
            seg(0x1000, 8, false),
            seg(0x2000, 8, false),
            SYNC_NOP,
        ];
        let mut fetch = Fetch::new(64).unwrap();
        let mut gate = SyncGate::default();
        let mut cursor = 0u32;
        let mut reads_of_d_before_witness = 0;
        let mut witness_fetched = false;
        let mut step = 0;
        while cursor < ring.len() as u32 + 1 && step < 100 {
            step += 1;
            let raw = ring
                .get(cursor as usize)
                .copied()
                .unwrap_or(seg(0x4000, 16, false));
            match gate.entry(raw, 1, cursor, fetch.fetched()) {
                Gate::PublishBarrier => {
                    witness_fetched = false;
                    fetch.publish(None).unwrap();
                    fetch.publish(None).unwrap();
                    gate.published(fetch.issued());
                }
                Gate::Wait => {
                    // the GPU is still in the segment before the barrier (its acquire); release
                    // it after a few polls, as the guest's semaphore release would
                    if step % 3 == 0 {
                        let get = (fetch.issued() & 63) as u32;
                        fetch.observe(get).unwrap();
                        witness_fetched = true;
                    }
                }
                Gate::Proceed => {
                    if cursor == ring.len() as u32 && !witness_fetched {
                        reads_of_d_before_witness += 1;
                    }
                    fetch.publish(Some((1, cursor + 1))).unwrap();
                    cursor += 1;
                }
            }
        }
        assert_eq!(cursor, ring.len() as u32 + 1, "every entry relayed");
        assert_eq!(
            reads_of_d_before_witness, 0,
            "D read before the hardware would fetch it"
        );
    }

    #[test]
    fn entries_without_sync_wait_always_proceed() {
        let mut g = SyncGate::default();
        for c in 0..8 {
            assert_eq!(g.entry(seg(0x1000, 4, false), 1, c, 0), Gate::Proceed);
        }
        assert!(!g.waiting());
    }

    #[test]
    fn sync_wait_publishes_one_barrier_then_waits_for_its_witness_then_proceeds_once() {
        let mut g = SyncGate::default();
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 10), Gate::PublishBarrier);
        g.published(12);
        assert!(g.waiting());
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 11), Gate::Wait);
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 12), Gate::Proceed);
        // the same entry again (a retried turn) needs no second barrier
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 12), Gate::Proceed);
        // a SYNC_WAIT segment at the next cursor is gated afresh
        assert_eq!(
            g.entry(seg(0x1000, 4, true), 1, 6, 12),
            Gate::PublishBarrier
        );
        // and a new ring generation at the same cursor too
        g.published(14);
        assert_eq!(g.entry(seg(0x1000, 4, true), 1, 6, 14), Gate::Proceed);
        assert_eq!(g.entry(SYNC_NOP, 2, 5, 14), Gate::PublishBarrier);
    }

    #[test]
    fn grace_retries_the_same_entry_then_is_final() {
        let t = Instant::now();
        let ms = |n: u64| t + Duration::from_millis(n);
        let mut g = Grace::default();
        assert!(g.refused(1, 5, t));
        assert!(g.refused(1, 5, ms(49)));
        assert!(!g.refused(1, 5, ms(50)));
        // another entry, or another ring generation, starts its own grace
        assert!(g.refused(1, 6, ms(60)));
        assert!(g.refused(2, 6, ms(61)));
        g.clear();
        assert!(!g.retrying());
    }
}
