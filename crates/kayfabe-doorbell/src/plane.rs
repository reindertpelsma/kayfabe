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
use crate::caps::{Refusal, Twin, VmCaps};
use crate::channel::{Disposition, Owner, Submission};

/// The answer to *"can this submission's operands be translated?"* — ⊘ **three-valued on
/// purpose**. Collapsing `NotYet` into `No` is fable's CRITICAL S2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Translatable {
    /// Every operand resolves.
    Yes,
    /// ⊘ **Not yet** — the mirror has not caught up with an invalidate still in flight. This is a
    /// *timing* state, not a verdict about the guest's work.
    NotYet,
    /// The operand names something nothing binds, and no amount of waiting changes it.
    No,
}
use crate::completion::Completion;
use crate::leaf::{GuestRamLayout, HostSlice, LeafRefusal};
use crate::lifetime::{Step, Teardown, TeardownError, WalkerState};
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

    /// ⊘ §7/§5.6: can every operand of the work now queued on this channel be translated?
    ///
    /// ★★★ **Three answers, not two.** `[fable w823, CRITICAL S2]` the first version returned
    /// `bool`, and a `false` was treated as a terminal verdict — so a ring landing in the
    /// **invalidate window**, when the VA mirror has not yet caught up, went straight to
    /// refuse-and-poison on a kernel channel. That is a **guest-wide denial of service from a
    /// timing hint**, reachable by an unprivileged process (§47: no identity at the trap) and
    /// reachable by the guest kernel's own legitimate ring with no attacker at all.
    ///
    /// ⇒ §5.6's fence is *"the mirror must catch up"*, and §5.2 says what to do meanwhile:
    /// *"cannot act yet ⇒ CAS BUSY → RUNG, then re-publish."*
    fn operands_translatable(&self, host_token: u32, up_to_seq: u64) -> Translatable;

    /// §8: write a completion ourselves. ⊘ Called **only** where [`Completion::for_route`] says a
    /// forge is licensed — i.e. where no GPU work ran.
    fn forge_completion(&self, host_token: u32);

    /// §7: refuse the submission and poison the device. ⊘ A kernel channel is NEVER faulted.
    fn refuse_and_poison(&self, host_token: u32);

    /// §7: fault one user channel. Blast radius is the process that asked.
    fn fault_channel(&self, host_token: u32);

    /// §6.4: map a **bounded** slice of registered guest RAM. ⊘ Note the signature: it takes a
    /// [`HostSlice`], which can only be produced from a block **we** minted. There is
    /// deliberately no verb here that accepts a raw guest-physical address.
    fn map_guest_slice(&self, slice: HostSlice);

    /// §9: the ordered teardown steps, performed by the host side.
    fn teardown_step(&self, step: Step);
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
    /// The time-setting registers of this device's timer HAL, refused by name on the privileged
    /// write arm (`timer::TimerRegs`). Per family, never per die.
    pub timer: crate::timer::TimerRegs,
    /// ★ Where the doorbell is, which is what decides whether BAR1 may trap at all
    /// (`trappolicy::doorbell_for`). Per family, never per die.
    pub doorbell: crate::trappolicy::DoorbellPlacement,
    /// This device's `NV_ESC_CARD_INFO.reg_size`. ⊘ Not the GA106 constant — the region list the
    /// VMM registers is derived from it, so a wrong value under-traps or over-traps BAR0.
    pub bar0_bytes: u64,
    /// Which tokens are guest-KERNEL channels (§7's failure-policy selector).
    kernel_tokens: Vec<u32>,
}

impl<'v> Plane<'v> {
    /// The GA106 bench shape: Ampere timer HAL, 16 MiB BAR0. ⊘ A named default — see
    /// [`Plane::for_family`] for the general form and `timer::GA106_BAR0_BYTES` for where the
    /// real BAR0 size comes from.
    pub fn new(vmm: &'v Vmm, n_tokens: usize, token_mask: u32) -> Plane<'v> {
        Self::for_family(
            vmm,
            n_tokens,
            token_mask,
            crate::classgen::Family::Ampere,
            crate::timer::GA106_BAR0_BYTES,
        )
    }

    /// ★ The general constructor: the family selects the timer HAL (`timer::timer_regs_for`),
    /// and `bar0_bytes` is `NV_ESC_CARD_INFO.reg_size` for this device.
    pub fn for_family(
        vmm: &'v Vmm,
        n_tokens: usize,
        token_mask: u32,
        family: crate::classgen::Family,
        bar0_bytes: u32,
    ) -> Plane<'v> {
        Plane {
            tokens: (0..n_tokens).map(|_| TokenWord::new()).collect(),
            bits: RungBitmap::new(),
            vmm,
            drainer_wake: WakeWord::new(),
            ring: PrivRing::new(),
            token_mask,
            timer: crate::timer::timer_regs_for(family),
            doorbell: crate::trappolicy::doorbell_for(family),
            bar0_bytes: bar0_bytes as u64,
            kernel_tokens: Vec::new(),
        }
    }

    /// ★ The single wakeup word, reached through the VMM. ⊘ There is deliberately no per-plane
    /// `worker_wake` field to reach instead.
    #[inline]
    pub fn worker_wake(&self) -> &WakeWord {
        &self.vmm.worker_wake
    }

    /// ⊘ §7 needs to know whether a channel is the guest's KERNEL or a user channel, because that
    /// selects the failure policy. It is recorded at allocation, never inferred at submit time.
    fn owner_of(&self, tok: u32) -> Owner {
        if self.kernel_tokens.iter().any(|t| *t == tok) { Owner::Kernel } else { Owner::User }
    }

    /// ★ Twin allocation goes through §9.1's caps. ⊘ On a shared host GPU the driver enforces no
    /// per-client quota, so this is the only thing standing between one guest and its neighbours.
    pub fn allocate_channel(
        &mut self,
        caps: &mut VmCaps,
        tok: u32,
        route: Route,
        host_token: u32,
        owner: Owner,
    ) -> Result<(), Refusal> {
        // ⊘ `[fable S7]` a guest-root-derived index must not panic the VMM.
        let idx = (tok & self.token_mask) as usize;
        if idx >= self.tokens.len() {
            return Err(Refusal::OverDeclaredCap { twin: Twin::Channel, cap: self.tokens.len() as u32, asked: tok });
        }
        caps.acquire(Twin::Channel)?;
        // ⊘⊘ `[fable S3]` the return value is the point: a failed allocate leaves the OLD route
        // and host_token in place, and ignoring it serves the new channel's rings on the old
        // channel's twin.
        let ok = self.tokens[idx].allocate(route, host_token)
            || self.tokens[idx].allocate_fresh(route, host_token);
        if !ok {
            caps.release(Twin::Channel);
            return Err(Refusal::OverDeclaredCap { twin: Twin::Channel, cap: 0, asked: tok });
        }
        // ⊘⊘⊘ `[fable S5]` kernel_tokens was STICKY: never removed on free, duplicated on reuse,
        // cleared only at teardown. A kernel chid, once freed and recycled to a user process,
        // kept the KERNEL failure policy — so a user channel's translation miss poisoned the
        // device guest-wide, defeating §7's blast-radius argument entirely.
        // ⇒ Ownership is set per allocation, and cleared for the other case. It is also a set
        // rather than a growing Vec, so a guest allocating and freeing in a loop cannot grow it.
        self.kernel_tokens.retain(|t| *t != tok);
        if owner == Owner::Kernel {
            self.kernel_tokens.push(tok);
        }
        Ok(())
    }

    /// The free path. ⊘ `[fable H3]` there was no free at all: caps were acquired and never
    /// released, so a guest that allocated and freed more than its cap over its life was refused
    /// forever.
    pub fn free_channel(&mut self, caps: &mut VmCaps, tok: u32) -> bool {
        let idx = (tok & self.token_mask) as usize;
        let Some(w) = self.tokens.get(idx) else { return false };
        // §5.2: free waits out BUSY before the twin may be dropped.
        if !w.retire() {
            return false;
        }
        self.kernel_tokens.retain(|t| *t != tok);
        caps.release(Twin::Channel);
        true
    }

    fn path(&self) -> TrapPath<'_> {
        TrapPath {
            tokens: &self.tokens,
            bits: &self.bits,
            worker_wake: &self.vmm.worker_wake,
            drainer_wake: &self.drainer_wake,
            ring: &self.ring,
            token_mask: self.token_mask,
            timer: self.timer,
        }
    }

    /// ★★★ §6.4 ON THE LIVE PATH. The guest's page-table leaf names a guest-physical address;
    /// this is the **only** way that address reaches a host mapping call.
    ///
    /// ⊘ It **selects** a registered block and an offset within it — it can never name a base.
    /// A leaf resolving nowhere is refused by name and counted, because the addresses it would
    /// otherwise resolve include **our own memslots** (the read shadow, the doorbell bitmap, the
    /// boot pages), and mapping one hands the drainer's state to the GPU as a DMA target.
    pub fn resolve_and_map_leaf(
        &self,
        host: &dyn HostOps,
        layout: &GuestRamLayout,
        gpa: u64,
        len: u64,
    ) -> Result<HostSlice, LeafRefusal> {
        let slice = layout.leaf(gpa, len)?;
        host.map_guest_slice(slice);
        Ok(slice)
    }

    /// ★★★ §9 ON THE LIVE PATH. Runs the teardown in the order §9 requires and **refuses any
    /// other**, including reaching `close` without the explicit free.
    ///
    /// ⊘ The walker reset is not an afterthought inside the sequence: without it the next driver
    /// instance's first diff reports *"unchanged"* and maps nothing — a second boot that faults
    /// for reasons the first did not.
    pub fn teardown(&mut self, host: &dyn HostOps, walker: &mut WalkerState) -> Result<(), TeardownError> {
        let mut t = Teardown::new();
        for step in crate::lifetime::ORDER {
            t.run(step)?;
            if step == Step::ResetWalkerState {
                walker.reset();
            }
            if step == Step::ResetRing {
                self.kernel_tokens.clear();
            }
            host.teardown_step(step);
        }
        debug_assert!(t.complete());
        Ok(())
    }



    /// ★★★ **What the VMM registers as trapped** — the whole answer, in one place.
    ///
    /// ⊘ There is no companion `read_regions()`. `trappolicy::may_trap_read` is `false` for every
    /// `(bar, offset)` and [`trappolicy::TrapRegion`] has no read field, so "no read exits" is a
    /// property of the type, not of a caller remembering not to ask.
    pub fn trap_regions(&self) -> Vec<crate::trappolicy::TrapRegion> {
        crate::trappolicy::trap_regions(self.doorbell, self.bar0_bytes)
    }

    /// ★ THE ONLY vCPU WRITE ENTRY POINT.
    ///
    /// ⊘ The structural gate runs **above** the three-way classifier: a classifier bug cannot act
    /// on a write the design forbids, because such a write never reaches the classifier. In a
    /// correct VMM this is unreachable — [`Plane::trap_regions`] never registered that address —
    /// so reaching it means the VMM registered a region we did not ask for.
    #[inline]
    pub fn trap_write(&self, class: Class, bar: u8, off: u32, val: u64, width: u8) -> Action {
        if !crate::trappolicy::may_trap_write(crate::vmm::Bar(bar), off as u64, self.doorbell) {
            return Action::None;
        }
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
                // ★★★ §7 ON THE LIVE PATH. Not a component consulted somewhere else: the
                // submission decision happens HERE, before any work is handed to the host.
                // ⊘ The kernel arm is a security boundary — a translation miss on a kernel
                // channel must never become a fault, because the unified-memory driver treats any
                // channel error as globally fatal and one fault kills CUDA for every process in
                // the guest.
                // ★★★ §5.6's FENCE, on the live path. A stale mirror is a reason to WAIT, never
                // a reason to poison: put the token back as RUNG and re-publish so a later pass
                // retries once the VA manager has applied the diff.
                // ⊘ Bounded by the same K as the re-act loop — §5.2's livelock argument applies
                // identically, and an unbounded "try again" is a way for a guest to pin a worker.
                match host.operands_translatable(t.host_token, seq) {
                    Translatable::NotYet if round < REACT_ROUNDS => {
                        // ⚠ §5.2: "found but unactionable counts as NO WORK for the purpose of
                        // sleeping" — so this is NOT counted in `served`, or a worker spins on a
                        // token whose enabling register write also needs a worker.
                        w.put_back();
                        self.bits.publish(tok);
                        let _ = self.vmm.worker_wake.bump();
                        break;
                    }
                    _ => {}
                }
                let sub = Submission {
                    owner: self.owner_of(tok),
                    route: t.route,
                    all_operands_translatable: matches!(
                        host.operands_translatable(t.host_token, seq),
                        Translatable::Yes
                    ),
                };
                let mut submitted = false;
                let did_gpu_work = match sub.decide() {
                    Disposition::Submit => {
                        submitted = true;
                        match t.route {
                        Route::Translated => host.run_translated(t.host_token, seq),
                        Route::Emulated => {
                            host.run_emulated(t.host_token, seq);
                            false
                        }
                        // ⊘ A passthrough token is rung INLINE on the vCPU and must never be
                        // served here; reaching this arm means the classifier disagreed with the
                        // token word.
                        // ⊘ `[fable S4]` was `break` WITHOUT release(), wedging the token BUSY
                        // forever so `retire()` never succeeded and the free path spun. Release
                        // first, then leave.
                        Route::Passthrough | Route::Unknown => {
                            let _ = w.release(round, REACT_ROUNDS);
                            break;
                        }
                    }
                    }
                    Disposition::RefuseAndPoison => {
                        host.refuse_and_poison(t.host_token);
                        false
                    }
                    Disposition::FaultChannel => {
                        host.fault_channel(t.host_token);
                        false
                    }
                };
                // ★★★ §8 ON THE LIVE PATH, and ⊘ **only when the work was actually SUBMITTED.**
                // `[fable w823, S6]` the first version forged after a refusal or a fault, because
                // `did_gpu_work == false` looks the same in both cases. §8: *"A completion written
                // for work that did not happen is how a scrub becomes a leak"* — and work that was
                // REFUSED is the purest instance of work that did not happen.
                if submitted && Completion::for_route(t.route, did_gpu_work) == Completion::Forge {
                    host.forge_completion(t.host_token);
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
