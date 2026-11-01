//! ★★★★★ **One completion fd for the session, and the set of channels waiting on it.**
//!
//! The host non-stall notifier is GPU-wide and carries no identity (`intr.c:1195-1205`), so a
//! completion can only ever say *"someone finished"*. `[measured w826 gate 4]` with one event fd per
//! host ring, every completion flagged EVERY ring's fd and rang every channel — `host_rings=180` for
//! 72 entries, idle channels included. ⇒ **One fd; on its readiness ring only the tokens with a fence
//! IN FLIGHT.** The wake cannot be narrower than "someone finished"; the ring can be narrower than
//! "everyone".
//!
//! ⊘ **Mark BEFORE the doorbell.** A fence can complete — and its interrupt fire and be consumed —
//! before the ringing thread returns from the doorbell store. Marking after would let that wake find
//! the channel not yet in flight, and nothing would ever ring it again. Marking before makes the
//! worst case a spurious ring, which costs one fence read.

use std::sync::atomic::{AtomicU64, Ordering};

/// 2026-10-31: the in-flight token set, split out of [`Completions`] so its contract is testable
/// without a host (the event fd is a `/dev/nvidia*` descriptor). Behaviour is unchanged: one bit per
/// token in a boxed array of atomic words, `Release` on write, `Acquire` on read, and a token past the
/// table is ignored rather than growing anything (the table is sized once, from the token table).
pub struct InflightSet {
    words: Box<[AtomicU64]>,
}

impl InflightSet {
    /// A set with room for `tokens` tokens (rounded up to a whole word).
    #[must_use]
    pub fn new(tokens: usize) -> InflightSet {
        InflightSet {
            words: (0..tokens.div_ceil(64))
                .map(|_| AtomicU64::new(0))
                .collect(),
        }
    }

    /// `token` has (or is about to have) a fence in flight. A token past the table is ignored.
    pub fn mark(&self, token: u32) {
        if let Some(w) = self.words.get(token as usize / 64) {
            w.fetch_or(1 << (token % 64), Ordering::Release);
        }
    }

    /// `token` has nothing in flight. A token past the table is ignored.
    pub fn clear(&self, token: u32) {
        if let Some(w) = self.words.get(token as usize / 64) {
            w.fetch_and(!(1 << (token % 64)), Ordering::Release);
        }
    }

    /// Every token currently in flight, ascending.
    pub fn for_each(&self, mut f: impl FnMut(u32)) {
        for (i, w) in self.words.iter().enumerate() {
            let mut bits = w.load(Ordering::Acquire);
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                f(i as u32 * 64 + b);
            }
        }
    }
}

/// The session's completion edge + the in-flight token set.
pub struct Completions {
    ev: kf_host::EventFd,
    inflight: InflightSet,
    /// Diagnostic (`KF3_COMPLETION_PROBE`): when the fd last became ready, and how often.
    t0: std::time::Instant,
    last_wake_ns: AtomicU64,
    wakes: AtomicU64,
}

impl Completions {
    /// Open the session's completion fd and arm the host NSI edge (`FIFO_EVENT_MTHD`) once.
    /// `tokens` bounds the token table.
    ///
    /// # Errors
    /// The host's refusal, by name.
    pub fn open(rm: &kf_host::HostRm, tokens: usize) -> Result<Completions, String> {
        let ev = rm.open_event_fd().map_err(|e| format!("event fd: {e:?}"))?;
        rm.alloc_os_event(rm.subdevice(), crate::host::FIFO_EVENT_MTHD, true, &ev)
            .map_err(|e| format!("os event: {e:?}"))?;
        rm.arm_repeat(crate::host::FIFO_EVENT_MTHD)
            .map_err(|e| format!("notify: {e:?}"))?;
        Ok(Completions {
            ev,
            inflight: InflightSet::new(tokens),
            t0: std::time::Instant::now(),
            last_wake_ns: AtomicU64::new(0),
            wakes: AtomicU64::new(0),
        })
    }

    /// ★ Also wake on engine `notify_index`'s non-stall notifier (e.g. `kf_host::notifier_ce(n)` for
    /// the async copy engine our rings run on) — the same fd, dataless, REPEAT once per session.
    ///
    /// # Errors
    /// The host's refusal, by name.
    pub fn also(&self, rm: &kf_host::HostRm, notify_index: u32) -> Result<(), String> {
        rm.alloc_os_event(rm.subdevice(), notify_index, true, &self.ev)
            .map_err(|e| format!("os event {notify_index}: {e:?}"))?;
        rm.arm_repeat(notify_index)
            .map_err(|e| format!("notify {notify_index}: {e:?}"))
    }

    /// The fd every worker's poller watches (level-triggered; a coalesced WAKE).
    #[must_use]
    pub fn event_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.ev.as_fd()
    }

    /// The worker saw the completion fd ready. Diagnostic bookkeeping only; nothing reads it to
    /// decide anything.
    pub fn note_wake(&self) {
        let ns = u64::try_from(self.t0.elapsed().as_nanos()).unwrap_or(u64::MAX - 1);
        self.last_wake_ns.store(ns + 1, Ordering::Relaxed);
        self.wakes.fetch_add(1, Ordering::Relaxed);
    }

    /// Milliseconds since the last [`Completions::note_wake`], and the wakes so far.
    #[must_use]
    pub fn last_wake(&self) -> (Option<u64>, u64) {
        let ns = self.last_wake_ns.load(Ordering::Relaxed);
        let ago = (ns != 0).then(|| {
            let now = u64::try_from(self.t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
            now.saturating_sub(ns - 1) / 1_000_000
        });
        (ago, self.wakes.load(Ordering::Relaxed))
    }

    /// `token` has (or is about to have) a fence in flight. Call BEFORE the doorbell.
    pub fn mark(&self, token: u32) {
        self.inflight.mark(token);
    }

    /// `token` has nothing in flight. Only its owner (the worker holding it `BUSY`) calls this.
    pub fn clear(&self, token: u32) {
        self.inflight.clear(token);
    }

    /// Every token currently in flight — what a wake must ring.
    pub fn for_each_inflight(&self, f: impl FnMut(u32)) {
        self.inflight.for_each(f);
    }
}

#[cfg(test)]
mod tests {
    //! 2026-10-31 -- the contracts of the retired `kayfabe-completion` per-process completion
    //! plane (11 tests, `archive/crates/kayfabe-completion`) that survive in v3, restated on the one
    //! live structure that carries them. What did NOT survive is listed in the porting PR: v3 posts
    //! no completion batch through a faked GSP queue and arms no mapped-fence jump guard.
    use super::InflightSet;

    fn inflight(s: &InflightSet) -> Vec<u32> {
        let mut v = Vec::new();
        s.for_each(|t| v.push(t));
        v
    }

    /// `t14_per_proc_completion_no_starve`: the round-8 starvation was a completion that was
    /// observed and then NEVER delivered once the other process went quiet. The v3 shape of
    /// "delivered" is "rung on a wake": a token with a fence in flight is rung on EVERY wake until its
    /// own owner clears it, whichever token completed or submitted last, and a token that completed
    /// is not rung again.
    #[test]
    fn a_wake_rings_every_token_still_in_flight_whoever_finished_last() {
        let s = InflightSet::new(256);
        let (a, b) = (3u32, 200u32); // different words: no shared state between them
        s.mark(a);
        s.mark(b);
        // B finishes and its owner clears it; A's fence has not completed and A submits nothing.
        s.clear(b);
        for wake in 0..3 {
            assert_eq!(inflight(&s), vec![a], "wake {wake}: A is rung off any wake");
        }
        s.clear(a);
        assert!(
            inflight(&s).is_empty(),
            "and a completed token is not rung again"
        );
    }

    /// `ack_removes_an_in_flight_event_and_spares_its_siblings` and
    /// `outstanding_accounting_sums_all_three_queues_and_predicate_is_any`: clearing removes exactly
    /// its own token (a `!=`/`==` slip would delete every sibling and keep the cleared one), the set
    /// is the union over every word (a `||`/`&&` or `+`/`*` slip would lose a word), and clearing
    /// twice or clearing a token never marked changes nothing.
    #[test]
    fn clear_removes_only_its_own_token_and_the_set_is_the_union_of_every_word() {
        let s = InflightSet::new(320);
        let all = [0u32, 63, 64, 127, 128, 255, 256, 319];
        for t in all {
            s.mark(t);
        }
        assert_eq!(
            inflight(&s),
            all.to_vec(),
            "ascending, across every word boundary"
        );
        s.clear(64);
        s.clear(64);
        s.clear(5); // never marked
        let want: Vec<u32> = all.iter().copied().filter(|t| *t != 64).collect();
        assert_eq!(inflight(&s), want);
        // Only ONE word populated: the other words being empty must not hide it.
        for t in want {
            s.clear(t);
        }
        s.mark(300);
        assert_eq!(inflight(&s), vec![300]);
    }

    /// `successive_batches_carry_distinct_monotonic_ids` (the drain key must not collide) restated:
    /// two tokens 64 apart address the same bit of different words, and marking is idempotent.
    #[test]
    fn distinct_tokens_never_alias_and_marking_twice_is_one_mark() {
        let s = InflightSet::new(192);
        s.mark(5);
        s.mark(5);
        s.mark(5 + 64);
        s.mark(5 + 128);
        assert_eq!(inflight(&s), vec![5, 69, 133]);
        s.clear(69);
        assert_eq!(inflight(&s), vec![5, 133]);
    }

    /// Boundary 1 (`MAX_OUTSTANDING_COMPLETIONS`, `MAX_ARMED_FENCES`): a hostile guest cannot grow
    /// this state. The table is sized once; a token past it is ignored, on both calls, and never
    /// panics (a guest chooses its token).
    #[test]
    fn a_token_past_the_table_is_ignored_and_nothing_grows() {
        let s = InflightSet::new(100); // two words
        for t in [128u32, 1_000, u32::MAX - 63, u32::MAX] {
            s.mark(t);
            s.clear(t);
        }
        assert!(inflight(&s).is_empty());
        s.mark(99);
        assert_eq!(inflight(&s), vec![99]);
        let empty = InflightSet::new(0);
        empty.mark(0);
        assert!(
            inflight(&empty).is_empty(),
            "a zero-token table holds nothing"
        );
    }
}
