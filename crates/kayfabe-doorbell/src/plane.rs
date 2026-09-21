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

/// ★★★ **PER-VMM state.** §9.3 is explicit that this is not per GPU:
///
/// > *"the **worker pool**; ★ **one** wakeup word and one eventfd — a worker registers on exactly
/// > one word, and **N words would need N atomics and reopen the lost-wakeup proof**; **one** VA
/// > manager … the object graph, since client handles are one namespace"*
///
/// ⊘⊘ **This type exists because the first composition got it wrong.** `Plane` originally held
/// `worker_wake` itself, so one `Plane` per GPU meant **N wakeup words** — which is exactly the
/// shape §9.3 refuses, and it would have reopened the lost-wakeup proof silently: each word is
/// individually correct, and a worker parked on GPU 0's word simply never learns about GPU 1's
/// work. ⚠ A bug that needs a second GPU to appear would not have shown up in any test here.
pub struct Vmm {
    /// One word, one eventfd, for every GPU.
    pub worker_wake: WakeWord,
}

impl Default for Vmm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vmm {
    pub const fn new() -> Vmm {
        Vmm { worker_wake: WakeWord::new() }
    }
}

/// **PER-GPU state.** §9.3's left column: the register shadow and classifier keyed by
/// `(gpu, bar, offset)`, the token words and rung bitmap (*"token spaces overlap across GPUs"*),
/// and ⊘ **the ring and its drainer — because ordering is per PCI function**, so a shared ring
/// would impose an order across devices that hardware does not have.
pub struct Plane<'v> {
    pub tokens: Vec<TokenWord>,
    pub bits: RungBitmap,
    /// Borrowed from the VMM: **one** word across all GPUs.
    pub vmm: &'v Vmm,
    /// Per GPU, like the ring it wakes.
    pub drainer_wake: WakeWord,
    pub ring: PrivRing,
    pub token_mask: u32,
}

impl<'v> Plane<'v> {
    pub fn new(vmm: &'v Vmm, n_tokens: usize, token_mask: u32) -> Plane<'v> {
        Plane {
            tokens: (0..n_tokens).map(|_| TokenWord::new()).collect(),
            bits: RungBitmap::new(),
            vmm,
            drainer_wake: WakeWord::new(),
            ring: PrivRing::new(),
            token_mask,
        }
    }

    /// ★ The single wakeup word, reached through the VMM. ⊘ There is deliberately no per-plane
    /// `worker_wake` field to reach instead.
    #[inline]
    pub fn worker_wake(&self) -> &WakeWord {
        &self.vmm.worker_wake
    }

    fn path(&self) -> TrapPath<'_> {
        TrapPath {
            tokens: &self.tokens,
            bits: &self.bits,
            worker_wake: &self.vmm.worker_wake,
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

    #[cfg(test)]
    pub(crate) fn occupancy_for_test(&self) -> usize {
        self.ring.occupancy()
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
                        let _ = self.vmm.worker_wake.bump();
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
