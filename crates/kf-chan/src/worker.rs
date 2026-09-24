//! ★★★★★ **The worker plane** — `THE_ARCHITECTURE_v3.md` §2.2/§2.4: scan, claim, serve, release;
//! park only on the wake word's say-so.
//!
//! The vCPU side is `kf_trap::TrapPath` (stamp + RUNG in one CAS, bit then summary, bump); this is
//! the other side. ★ **A host completion is an internal RING of the channel's own token**: the
//! host event fd's readiness does not call the channel — it rings its token exactly as a doorbell
//! would. So doorbells and completions share ONE path, and a channel's pump is serialized by the
//! token's `BUSY` state, never by a lock.
//!
//! The only waits are the `epoll` below (§35); the serve callback must never block (§2 — it runs
//! the Translated runner's `pump`, which returns instead of waiting).

use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use kf_trap::{Claim, REACT_ROUNDS, Release, RungBitmap, TokenWord, Wake, WakeWord};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A poller tag with this bit set is a HOST EVENT fd; the low 32 bits are its channel's token.
pub const HOST_EVENT_TAG: u64 = 1 << 32;
/// The poller tag of the worker eventfd.
pub const WORKER_EFD_TAG: u64 = 0;
/// Tokens taken per scan.
const SCAN_LIMIT: usize = 64;
/// How long a parked worker sleeps before re-checking `stop` — never a correctness timeout.
const PARK_MS: u32 = 50;

/// Serve one claimed token: run its channel as far as it can go without waiting.
pub trait Serve: Sync {
    /// Called with the token `BUSY` and owned by this worker. Must not block.
    fn serve(&self, token: u32);
}

/// Counters, for the gate — never read on a hot path.
#[derive(Debug, Default)]
pub struct WorkerStats {
    /// `serve` calls.
    pub served: AtomicU64,
    /// Times a worker parked in `epoll`.
    pub parks: AtomicU64,
    /// Host events turned into internal rings.
    pub host_rings: AtomicU64,
    /// Timeslice expiries (`RepublishAndMoveOn`).
    pub timeslices: AtomicU64,
}

/// ★ The shared doorbell plane, as the workers see it.
pub struct WorkerPlane<'a> {
    /// The token words.
    pub tokens: &'a [TokenWord],
    /// The rung bitmap.
    pub bits: &'a RungBitmap,
    /// The worker wake word (NOT the drainer's — §5.3).
    pub wake: &'a WakeWord,
    /// The worker eventfd.
    pub efd: &'a Notifier,
    /// Counters.
    pub stats: &'a WorkerStats,
}

impl WorkerPlane<'_> {
    /// Publish `t` (bit, then summary) and wake one parked worker if any.
    fn publish(&self, t: u32) {
        self.bits.publish(t);
        if self.wake.bump() == Wake::SignalOne {
            let _ = self.efd.signal();
        }
    }

    /// ★ A host completion for `t`'s channel: ring its token from inside the plane. The stamp is
    /// the token's own (there is no guest ring position behind an internal ring).
    pub fn ring_internal(&self, t: u32) {
        let Some(w) = self.tokens.get(t as usize) else { return };
        if w.ring(w.load().applied_seq) {
            self.publish(t);
        }
        self.stats.host_rings.fetch_add(1, Ordering::Relaxed);
    }

    /// One worker's loop, until `stop`. `poller` must watch the worker eventfd at
    /// [`WORKER_EFD_TAG`] and each channel's host event fd at `HOST_EVENT_TAG | token`.
    pub fn run(&self, serve: &dyn Serve, poller: &Poller, stop: &AtomicBool) {
        let mut toks = Vec::with_capacity(SCAN_LIMIT);
        while !stop.load(Ordering::Acquire) {
            // §5.3: every `seen` load happens-before the scan.
            let seen = self.wake.seen();
            self.bits.scan(&mut toks, SCAN_LIMIT);
            let mut progress = false;
            for &t in &toks {
                let Some(w) = self.tokens.get(t as usize) else { continue };
                if let Claim::Won(_) = w.claim() {
                    progress = true;
                    let mut rounds = 0;
                    loop {
                        serve.serve(t);
                        self.stats.served.fetch_add(1, Ordering::Relaxed);
                        rounds += 1;
                        match w.release(rounds, REACT_ROUNDS) {
                            Release::Idled => break,
                            Release::ActAgain => {}
                            Release::RepublishAndMoveOn => {
                                self.stats.timeslices.fetch_add(1, Ordering::Relaxed);
                                self.publish(t);
                                break;
                            }
                        }
                    }
                }
            }
            if progress {
                continue;
            }
            // Park only if nothing arrived since `seen` — the lost-wakeup race, closed.
            if !self.wake.try_park(seen) {
                continue;
            }
            self.stats.parks.fetch_add(1, Ordering::Relaxed);
            let mut ready = ReadyTokens::new();
            let got = poller.wait(&mut ready, PollTimeout::Millis(PARK_MS));
            self.wake.unpark();
            if got.is_ok() {
                for tag in ready.iter() {
                    if tag & HOST_EVENT_TAG != 0 {
                        self.ring_internal((tag & 0xFFFF_FFFF) as u32);
                    } else {
                        let _ = self.efd.drain();
                    }
                }
            }
        }
    }
}
