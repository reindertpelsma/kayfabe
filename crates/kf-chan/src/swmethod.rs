//! ★★ OWNER_RULINGS §U (2026-10-07) — **a software method on a Translated channel, as a stage of
//! the stream kayfabe writes.**
//!
//! On bare metal a method on a software subchannel stalls the PBDMA and the GSP runs it
//! (`deferred_api.c:694-718`: every deferred call but `FIFO_UPDATE_CHANNEL_INFO` is stalling). On
//! a Translated channel nothing the guest wrote reaches the engine, so the stall is kayfabe's:
//!
//! 1. **Drain.** Everything before the method is fenced; nothing after it is fetched into the host
//!    ring. (Stronger than the hardware, which only stops the PBDMA: the action then never runs
//!    against work the guest ordered before it and that is still in flight.)
//! 2. **Start**, once that fence completed. For a deferred `DMA_INVALIDATE_TLB` the runner first
//!    puts a GATE into the host ring — an equality acquire of a fresh payload on the ring's own
//!    gate word ([`crate::tspace_unsafe::gate_acquire_words`]) followed by a fence — and hands the
//!    VA-manager thread the gate with the walk of the channel's OWN space (§U.2: the guest's
//!    `hClientVA`/`hDeviceVA`/`hVASpace` are ignored). The VA thread commits the diff, the host
//!    invalidate lands, and only then does it store the payload ([`crate::host::Gate::release`]).
//!    Any other action runs on the plane's act thread.
//! 3. **Resume** when the action reports done: the stream continues; work after the method is
//!    bound only now, against the committed rows. A failed action kills the channel by name.
//!
//! ⊘ The doorbell is not a boundary (§U.2): the ordering lives only in the ring kayfabe writes —
//! the gate in it, and the runner not writing what follows until the action is done. Nothing
//! blocks on a vCPU; no completion is forged (the engine passes the gate only after the store).
//!
//! [`SwSuspend`] is the pure stage machine `crate::host::TranslatedChannel::pump` drives; the
//! model test below runs it against a simulated engine and VA thread over the REAL authored words.

use crate::host::reached;
use crate::tmode::SwCall;

/// What a software method needs, as the channel's software objects classify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwKind {
    /// Consumed with nothing to do (an object's `NO_OPERATION`).
    Nop,
    /// A host-authored action; `gate`: put the host gate in the stream first (a deferred TLB
    /// invalidate).
    Act {
        /// The action is a deferred TLB invalidate.
        gate: bool,
    },
}

/// What the runner does on one pump of a suspended channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwStep {
    /// The work before the method has not completed: wait for the engine.
    Wait,
    /// It has: put the gate (payload `Some`) behind it, then start the action.
    Start {
        /// The gate payload, for a gated action.
        gate: Option<u32>,
    },
    /// The action runs elsewhere: wait for its ring.
    Running,
    /// Done: continue the stream; author `retires` as a split's would be.
    Resume {
        /// The guest `GP_GET` the method's entry completes, if it ended one.
        retires: Option<u32>,
    },
}

/// ★ One software method in flight on a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwSuspend {
    /// The call.
    pub call: SwCall,
    retires: Option<u32>,
    /// The fence that covers every pushed piece before the method.
    seq: u32,
    gate: Option<u32>,
    started: bool,
}

impl SwSuspend {
    /// Suspended at `call`, whose prior work is covered by fence `seq`; `gate`: the payload when
    /// the action is gated.
    #[must_use]
    pub const fn new(call: SwCall, retires: Option<u32>, seq: u32, gate: Option<u32>) -> Self {
        Self {
            call,
            retires,
            seq,
            gate,
            started: false,
        }
    }

    /// ★ One pump. `done`: the fence the engine has released; `poll`: the started action's outcome
    /// (`None` = still running) — asked only once the action was started.
    ///
    /// # Errors
    /// The action's refusal: the channel dies.
    pub fn step(
        &mut self,
        done: u32,
        poll: impl FnOnce() -> Option<Result<(), String>>,
    ) -> Result<SwStep, String> {
        if !reached(done, self.seq) {
            return Ok(SwStep::Wait);
        }
        if !self.started {
            self.started = true;
            return Ok(SwStep::Start { gate: self.gate });
        }
        match poll() {
            None => Ok(SwStep::Running),
            Some(Ok(())) => Ok(SwStep::Resume {
                retires: self.retires,
            }),
            Some(Err(e)) => Err(e),
        }
    }
}

/// The next gate payload after `last`: never 0 (the word starts at 0, and an equality acquire of
/// 0 would pass before any release).
#[must_use]
pub const fn next_gate(last: u32) -> u32 {
    match last.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

#[cfg(test)]
mod model {
    //! ★ The model test the ruling asks for: the gate's ordering, over the real authored words.
    //! Three actors take steps in every order a seeded schedule produces —
    //! - the ENGINE executes the host ring in order: a work item, a fence release, or a host
    //!   semaphore ACQUIRE decoded from the words [`crate::tspace_unsafe::gate_acquire_words`]
    //!   wrote (it passes only when the word holds the payload);
    //! - the VA THREAD, once asked, walks, COMMITS, invalidates, and only then releases the gate;
    //! - the RUNNER pushes the guest's work, and at the trigger follows [`SwSuspend`].
    //!
    //! Invariants, checked after every step: the walk starts only after every pre-trigger item
    //! executed; the gate word is stored only after the commit; the engine passes the acquire
    //! only after the store; no post-trigger item executes before the commit; and the runner
    //! binds post-trigger items only after the commit (so they see committed rows).
    use super::*;
    use kf_abi::submit::{fifo, method_header_decode};

    #[derive(Debug, Clone, PartialEq)]
    enum Item {
        Work(u32),
        Fence(u32),
        Acquire { va: u64, payload: u32 },
    }

    /// Decode a run of normalized host words into the engine's items.
    fn decode(words: &[u32]) -> Vec<Item> {
        let (mut lo, mut hi, mut pay) = (0u32, 0u32, 0u32);
        let mut out = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let h = method_header_decode(words[i]).unwrap();
            for k in 0..h.arg_words {
                let m = h.method + 4 * k as u32;
                let v = words[i + 1 + k];
                match m {
                    fifo::SEM_ADDR_LO => lo = v,
                    fifo::SEM_ADDR_HI => hi = v,
                    fifo::SEM_PAYLOAD_LO => pay = v,
                    fifo::SEM_EXECUTE => {
                        let va = (u64::from(hi) << 32) | u64::from(lo);
                        if v & fifo::SEM_EXECUTE_OPERATION_MASK
                            == fifo::SEM_EXECUTE_OPERATION_ACQUIRE
                        {
                            out.push(Item::Acquire { va, payload: pay });
                        } else {
                            out.push(Item::Fence(pay));
                        }
                    }
                    _ => {}
                }
            }
            i += 1 + h.arg_words;
        }
        out
    }

    const GATE_VA: u64 = 0x1_0000_e040;
    const FENCE_VA: u64 = 0x1_0000_e000;

    #[derive(Default)]
    struct World {
        ring: std::collections::VecDeque<Item>,
        fence: u32,
        gate: u32,
        executed: Vec<u32>,
        // VA thread
        asked: Option<u32>,
        walked: bool,
        committed: bool,
        released: bool,
        // log of invariant-relevant facts
        bound_post_at_commit: Option<bool>,
    }

    /// Item ids: 1..=3 before the trigger, 10..=12 after.
    const PRE: [u32; 3] = [1, 2, 3];
    const POST: [u32; 3] = [10, 11, 12];

    fn run(seed: u64) {
        let mut w = World::default();
        let mut rng = seed;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        // The runner pushes the pre-trigger work and the fence that covers it (`Next::Sw`).
        let mut seq = 0u32;
        for id in PRE {
            w.ring.push_back(Item::Work(id));
        }
        seq += 1;
        for it in decode(&crate::tspace_unsafe::fence_words(FENCE_VA, seq).unwrap()) {
            w.ring.push_back(it);
        }
        let call = SwCall {
            sub: 5,
            value: 1,
            method: 0x200,
            data: 0x4000_0002,
        };
        let mut sw = Some(SwSuspend::new(call, Some(7), seq, Some(next_gate(0))));
        let mut started_gate: Option<u32> = None;
        let mut post_pushed = false;
        let mut steps = 0;
        while !(post_pushed && w.ring.is_empty()) {
            steps += 1;
            assert!(steps < 10_000, "seed {seed}: no progress");
            match next() % 3 {
                // ENGINE
                0 => match w.ring.front().cloned() {
                    Some(Item::Work(id)) => {
                        if POST.contains(&id) {
                            assert!(
                                w.committed,
                                "seed {seed}: post-trigger work ran before the commit"
                            );
                        }
                        w.executed.push(id);
                        w.ring.pop_front();
                    }
                    Some(Item::Fence(p)) => {
                        w.fence = p;
                        w.ring.pop_front();
                    }
                    Some(Item::Acquire { va, payload }) => {
                        assert_eq!(va, GATE_VA);
                        if w.gate == payload {
                            assert!(
                                w.released && w.committed,
                                "seed {seed}: acquire passed unreleased"
                            );
                            w.ring.pop_front();
                        }
                    }
                    None => {}
                },
                // VA THREAD: walk → commit (+ host invalidate) → release, one stage per step.
                1 => {
                    if let Some(p) = w.asked {
                        if !w.walked {
                            assert!(
                                PRE.iter().all(|id| w.executed.contains(id)),
                                "seed {seed}: walk before the work the guest ordered before it"
                            );
                            w.walked = true;
                        } else if !w.committed {
                            w.committed = true;
                        } else if !w.released {
                            w.gate = p;
                            w.released = true;
                        }
                    }
                }
                // RUNNER
                _ => {
                    let Some(s) = sw.as_mut() else {
                        if !post_pushed {
                            // Bound NOW — after the commit (the stage machine said Resume).
                            w.bound_post_at_commit = Some(w.committed);
                            for id in POST {
                                w.ring.push_back(Item::Work(id));
                            }
                            post_pushed = true;
                        }
                        continue;
                    };
                    let done_flag = w.released;
                    match s.step(w.fence, || done_flag.then_some(Ok(()))) {
                        Ok(SwStep::Wait | SwStep::Running) => {}
                        Ok(SwStep::Start { gate }) => {
                            let p = gate.expect("gated");
                            started_gate = Some(p);
                            for it in decode(
                                &crate::tspace_unsafe::gate_acquire_words(GATE_VA, p).unwrap(),
                            ) {
                                w.ring.push_back(it);
                            }
                            seq += 1;
                            for it in
                                decode(&crate::tspace_unsafe::fence_words(FENCE_VA, seq).unwrap())
                            {
                                w.ring.push_back(it);
                            }
                            w.asked = Some(p);
                        }
                        Ok(SwStep::Resume { retires }) => {
                            assert_eq!(retires, Some(7));
                            sw = None;
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            }
            if w.released {
                assert!(w.committed, "seed {seed}: released before commit");
            }
        }
        assert_eq!(started_gate, Some(1));
        assert_eq!(w.bound_post_at_commit, Some(true));
        assert_eq!(w.executed, [1, 2, 3, 10, 11, 12]);
    }

    #[test]
    fn the_gate_orders_the_stream_around_the_commit_in_every_schedule() {
        for seed in 1..=2000u64 {
            run(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        }
    }

    /// The acquire is an EQUALITY acquire with ACQUIRE_SWITCH_TSG, on our own word, 40-bit.
    #[test]
    fn the_gate_words_are_an_equality_acquire_on_our_word() {
        let w = crate::tspace_unsafe::gate_acquire_words(GATE_VA, 5).unwrap();
        assert_eq!(
            decode(&w),
            [Item::Acquire {
                va: GATE_VA,
                payload: 5
            }]
        );
        assert_eq!(
            *w.last().unwrap(),
            1 << 12,
            "ACQUIRE (0) | ACQUIRE_SWITCH_TSG"
        );
        assert!(crate::tspace_unsafe::gate_acquire_words(1 << 40, 5).is_none());
        assert!(crate::tspace_unsafe::gate_acquire_words(GATE_VA + 2, 5).is_none());
    }

    #[test]
    fn gate_payloads_never_reuse_zero() {
        assert_eq!(next_gate(0), 1);
        assert_eq!(next_gate(u32::MAX), 1);
        assert_eq!(next_gate(41), 42);
    }

    /// A refused action kills the channel; a not-yet-complete prior fence never starts it.
    #[test]
    fn stage_machine_refusal_and_wraparound() {
        let call = SwCall {
            sub: 5,
            value: 1,
            method: 0x200,
            data: 9,
        };
        let mut s = SwSuspend::new(call, None, u32::MAX, None);
        assert_eq!(s.step(u32::MAX - 1, || unreachable!()), Ok(SwStep::Wait));
        assert_eq!(
            s.step(0, || unreachable!()),
            Ok(SwStep::Start { gate: None }),
            "wrapping order"
        );
        assert_eq!(s.step(0, || None), Ok(SwStep::Running));
        assert_eq!(
            s.step(0, || Some(Err("named".into()))),
            Err("named".to_string())
        );
    }
}
