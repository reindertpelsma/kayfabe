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

/// The session's completion edge + the in-flight token set.
pub struct Completions {
    ev: kf_host::EventFd,
    inflight: Box<[AtomicU64]>,
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
        rm.arm_repeat(crate::host::FIFO_EVENT_MTHD).map_err(|e| format!("notify: {e:?}"))?;
        let words = tokens.div_ceil(64);
        Ok(Completions { ev, inflight: (0..words).map(|_| AtomicU64::new(0)).collect() })
    }

    /// The fd every worker's poller watches (level-triggered; a coalesced WAKE).
    #[must_use]
    pub fn event_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.ev.as_fd()
    }

    /// `token` has (or is about to have) a fence in flight. Call BEFORE the doorbell.
    pub fn mark(&self, token: u32) {
        if let Some(w) = self.inflight.get(token as usize / 64) {
            w.fetch_or(1 << (token % 64), Ordering::Release);
        }
    }

    /// `token` has nothing in flight. Only its owner (the worker holding it `BUSY`) calls this.
    pub fn clear(&self, token: u32) {
        if let Some(w) = self.inflight.get(token as usize / 64) {
            w.fetch_and(!(1 << (token % 64)), Ordering::Release);
        }
    }

    /// Every token currently in flight — what a wake must ring.
    pub fn for_each_inflight(&self, mut f: impl FnMut(u32)) {
        for (i, w) in self.inflight.iter().enumerate() {
            let mut bits = w.load(Ordering::Acquire);
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                f(i as u32 * 64 + b);
            }
        }
    }
}
