//! The plane, composed — §3's shape, with the host side behind a trait.
//!
//! ## Why this exists
//!
//! The modules below it are each correct in isolation; §3 is a claim about how they **fit**:
//!
//! | thread | count | job |
//! |---|---|---|
//! | **vCPU** | guest's | traps only. Never blocks, never waits on us |
//! | **worker** | N ≤ 255 | scan, claim a token, run the channel's work |
//! | **register drainer** | 1 | drains the privileged ring **in order** |
//! | **VA manager** | 1 | **all** `mmap`/`munmap` |
//!
//! ★ *"The VA manager is a thread rather than a lock so that 'who may map' is a **structural**
//! answer."* ⊘ And the drainer is one thread because *"an ordered ring drained by many is not
//! ordered"* — the property §5.4 exists to provide.
//!
//! ## The seam
//!
//! [`HostOps`] is where the real host RM lives. Today `kayfabe-isolate-host::rm` implements that
//! surface behind an IPC hop; **step 2 of the rewrite lifts it in-process and it implements this
//! trait directly.** ⇒ Everything above the trait is already written, and the deletion does not
//! have to invent a new shape to land in.
//!
//! ⚠ **Nothing in this file may block a vCPU.** The only vCPU entry point is
//! [`Plane::trap_write`], which is [`crate::trap::TrapPath`] and nothing else.

use crate::bitmap::RungBitmap;
use crate::ring::PrivRing;
use crate::token::{Claim, Release, Route, TokenWord};
use crate::trap::{Action, Class, TrapPath};
use crate::wake::WakeWord;
use crate::REACT_ROUNDS;

/// The host side. ⊘ Deliberately tiny: every method is a thing §9 says we **author**, never a
/// guest value forwarded. *"We author every host call; our host-verb signatures do not accept a
/// guest flag word."*
pub trait HostOps: Send + Sync {
    /// Ring a real host doorbell for an already-born host channel.
    fn ring_host(&self, host_token: u32);
    /// Run one unit of a translated channel's work. Returns whether anything reached the GPU —
    /// ⊘ which §8 needs in order to license (or refuse) a forge.
    fn run_translated(&self, host_token: u32, up_to_seq: u64) -> bool;
    /// Run an emulated function. There is no GPU counterpart.
    fn run_emulated(&self, host_token: u32, up_to_seq: u64);
    /// Apply one privileged register write, in ring order.
    fn apply_register(&self, bar: u8, offset: u32, value: u64, width: u8);
}

/// Everything the trap path and the workers share.
pub struct Plane {
    pub tokens: Vec<TokenWord>,
    pub bits: RungBitmap,
    pub worker_wake: WakeWord,
    pub drainer_wake: WakeWord,
    pub ring: PrivRing,
    pub token_mask: u32,
}

impl Plane {
    pub fn new(n_tokens: usize, token_mask: u32) -> Plane {
        Plane {
            tokens: (0..n_tokens).map(|_| TokenWord::new()).collect(),
            bits: RungBitmap::new(),
            worker_wake: WakeWord::new(),
            drainer_wake: WakeWord::new(),
            ring: PrivRing::new(),
            token_mask,
        }
    }

    fn path(&self) -> TrapPath<'_> {
        TrapPath {
            tokens: &self.tokens,
            bits: &self.bits,
            worker_wake: &self.worker_wake,
            drainer_wake: &self.drainer_wake,
            ring: &self.ring,
            token_mask: self.token_mask,
        }
    }

    /// ★ THE ONLY vCPU ENTRY POINT.
    #[inline]
    pub fn trap_write(&self, class: Class, bar: u8, off: u32, val: u64, width: u8) -> Action {
        self.path().write(class, bar, off, val, width)
    }

    /// One worker pass. Returns how many tokens it served.
    ///
    /// ⚠ §5.2: *"'Found but unactionable' counts as NO WORK for the purpose of sleeping"* — so a
    /// put-back is not counted here, or a worker spins on a token whose enabling register write
    /// also needs a worker.
    pub fn worker_pass(&self, host: &dyn HostOps, scratch: &mut Vec<u32>, limit: usize) -> usize {
        self.bits.scan(scratch, limit);
        let mut served = 0;
        for &tok in scratch.iter() {
            let Some(w) = self.tokens.get(tok as usize) else { continue };
            let Claim::Won(t) = w.claim() else { continue };
            let mut round = 0;
            loop {
                // ⊘ Re-read the stamp each round: a ring that arrived while we held the token
                // updated it in place, and that is how BUSY_RUNG work is picked up without a
                // second bit.
                let seq = w.load().applied_seq;
                match t.route {
                    Route::Translated => {
                        host.run_translated(t.host_token, seq);
                    }
                    Route::Emulated => host.run_emulated(t.host_token, seq),
                    // ⊘ A passthrough token is rung INLINE on the vCPU and must never be served
                    // here; reaching this arm means the classifier disagreed with the token word.
                    Route::Passthrough | Route::Unknown => break,
                }
                served += 1;
                match w.release(round, REACT_ROUNDS) {
                    Release::Idled => break,
                    Release::ActAgain => round += 1,
                    Release::RepublishAndMoveOn => {
                        // ⊘ bit then summary, then a bump — the same order the trap uses, because
                        // it is the same function.
                        self.bits.publish(tok);
                        let _ = self.worker_wake.bump();
                        break;
                    }
                }
            }
        }
        served
    }

    /// One drainer pass. §5.4: peek → apply → commit, **in order**, on **one** thread.
    pub fn drainer_pass(&self, host: &dyn HostOps, budget: usize) -> usize {
        let mut n = 0;
        while n < budget {
            let Some((_seq, w)) = self.ring.peek() else { break };
            host.apply_register(w.bar, w.offset, w.value, w.width);
            self.ring.commit();
            n += 1;
        }
        n
    }
}
