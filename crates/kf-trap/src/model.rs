//! An **exhaustive interleaving check** for the doorbell protocol — P1's last gate item.
//!
//! ## ⊘⊘⊘ WHAT THIS DOES AND DOES NOT ESTABLISH. Read this before trusting a green run.
//!
//! **Does:** enumerate **every** interleaving of the modelled threads' steps, under *sequential
//! consistency*, and check an invariant at every reachable state. That is exhaustive where
//! thread-based testing is sampling — the `claim()` race found at w823 reproduced **2 times in 40**
//! under real threads; this finds it **deterministically, in the first run**, and names the
//! interleaving.
//!
//! ⊘ **Does NOT:** establish the memory orderings. §5.3 says *"orderings are load-bearing and x86
//! TSO hides their absence"* — and an SC interleaving model hides their absence just as
//! thoroughly, because SC is *stronger* than anything the hardware gives. ⇒ **A green run here
//! means the ALGORITHM has no lost-wakeup or lost-token interleaving. It says nothing about
//! whether `Release`/`Acquire` are strong enough on a weakly-ordered target.** That remains
//! argued from the spec, and it is recorded as such in `THE_V3_PLAN.md`.
//!
//! ⚠ It is also **bounded**: the modelled programs are short by construction. A bug needing more
//! steps than are modelled is out of reach. ⇒ The model is a *falsifier*, not a proof.

use std::collections::HashSet;

// ---- the modelled protocol -------------------------------------------------------------------
//
// One token, abstracted to exactly what the invariant needs: the state machine, the stamp, and
// whether a bitmap bit is outstanding.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S {
    Idle,
    Rung,
    Busy,
    BusyRung,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct World {
    state: S,
    /// The stamp. A ring bumps it **even when the state does not change** — which is precisely
    /// what made the single-CAS `claim()` fail spuriously.
    stamp: u8,
    /// Is a bitmap bit outstanding for this token?
    bit: bool,
    /// How much work the guest has queued, and how much a worker has accounted for.
    queued: u8,
    served: u8,
}

/// One step of one thread. ⊘ Each is a single atomic operation, so an interleaving of these IS an
/// interleaving of the real protocol's atomics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    /// vCPU: `ring()` — one CAS that stamps and sets RUNG.
    Ring,
    /// vCPU: publish the bit, iff the ring said IDLE→RUNG.
    PublishIfOwed,
    /// worker: `scan()` consumes the bit.
    TakeBit,
    /// worker: `claim()`'s **load**. ⊘⊘⊘ Split from the CAS deliberately: the w823 defect IS the
    /// gap between them. A model that collapses `claim()` into one atomic step **cannot contain
    /// the race** — and the first version of this model did exactly that, explored 35 states, and
    /// reported the known bug as absent. ⇒ **Model the operations a thread can be preempted
    /// between, not the functions you wrote.**
    ClaimLoad,
    /// worker: `claim()`'s compare-and-swap.
    ClaimCas,
    /// worker: act + `release()`.
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Thread {
    prog: [Option<Step>; 4],
    pc: u8,
    /// vCPU-local: did our `ring()` transition IDLE→RUNG (so we owe a publish)?
    owes_publish: bool,
    /// worker-local: did we take a bit, and did we win the claim?
    holds_bit: bool,
    holds_claim: bool,
    /// worker-local: the stamp observed by `ClaimLoad`, which `ClaimCas` compares against.
    observed_stamp: u8,
    observed_state: S,
}

/// Whether `claim()` retries on CAS failure. ★ The whole point of the model: run it **both ways**
/// and show the checker distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimShape {
    /// The w823 bug: a single CAS, `NotOurs` on any failure.
    SingleCas,
    /// The fix: retry; give up only on `state != RUNG`.
    Retrying,
}

fn step(w: &mut World, t: &mut Thread, s: Step, shape: ClaimShape) {
    match s {
        Step::Ring => {
            w.queued = w.queued.saturating_add(1);
            w.stamp = w.stamp.wrapping_add(1); // the stamp moves even on RUNG→RUNG
            t.owes_publish = w.state == S::Idle;
            w.state = match w.state {
                S::Idle | S::Rung => S::Rung,
                S::Busy | S::BusyRung => S::BusyRung,
            };
        }
        Step::PublishIfOwed => {
            if t.owes_publish {
                w.bit = true;
                t.owes_publish = false;
            }
        }
        Step::TakeBit => {
            if w.bit {
                w.bit = false;
                t.holds_bit = true;
            }
        }
        Step::ClaimLoad => {
            t.observed_stamp = w.stamp;
            t.observed_state = w.state;
        }
        Step::ClaimCas => {
            if !t.holds_bit {
                return;
            }
            // The CAS compares the WHOLE WORD — state and stamp are packed together, so a
            // concurrent ring that only restamped still fails it.
            let word_unchanged = w.stamp == t.observed_stamp && w.state == t.observed_state;
            match shape {
                // ⊘ THE BUG: give up on any CAS failure. A concurrent `Ring` moved the stamp, so
                // the worker concludes someone else owns the token — while holding its only bit.
                ClaimShape::SingleCas => {
                    if t.observed_state == S::Rung && word_unchanged {
                        w.state = S::Busy;
                        t.holds_claim = true;
                    }
                    // ⊘⊘⊘ **AND THIS LINE IS THE WHOLE DEFECT.** On a failed claim the worker
                    // MOVES ON to the next token in its scan result — it does not retry and it
                    // does not put the bit back. The bit was consumed by `scan()` and is simply
                    // GONE. The second version of this model kept `holds_bit` set here, so the
                    // invariant's "is anyone still holding it?" clause answered *yes* and the bug
                    // read as absent. ⇒ Model what the worker DOES, not what you wish it did.
                    t.holds_bit = false;
                }
                // The fix: re-read and retry; give up only on `state != RUNG`.
                ClaimShape::Retrying => {
                    // Retries until the state itself says another owner: the loop cannot be
                    // defeated by a concurrent restamp, so it either wins or the state moved on.
                    if w.state == S::Rung {
                        w.state = S::Busy;
                        t.holds_claim = true;
                    }
                    t.holds_bit = false;
                }
            }
        }
        Step::Release => {
            if !t.holds_claim {
                return;
            }
            w.served = w.queued; // acting covers everything stamped so far
            w.state = match w.state {
                S::Busy => S::Idle,
                S::BusyRung => S::Rung, // put back; the worker republishes
                other => other,
            };
            if w.state == S::Rung {
                w.bit = true; // republish, bit-then-summary
            }
            t.holds_claim = false;
        }
    }
}

/// ★★★ THE INVARIANT, checked at every **quiescent** state (all threads finished).
///
/// A token that still has work outstanding must be **reachable**: either a bit points at it, or a
/// worker still holds it. ⊘ `Rung` with **no bit and nobody holding it** is the permanent loss
/// `THE_DESIGN.md` §5.2 describes — the guest rang, and nothing will ever look again.
fn quiescent_invariant(w: &World, threads: &[Thread]) -> Result<(), String> {
    let anyone_holds = threads.iter().any(|t| t.holds_claim || t.holds_bit);
    let unserved = w.served < w.queued;
    if unserved && !w.bit && !anyone_holds {
        return Err(format!(
            "LOST: state={:?} stamp={} queued={} served={} bit=false, nobody holding",
            w.state, w.stamp, w.queued, w.served
        ));
    }
    Ok(())
}

/// Exhaustively explore every interleaving. Returns the first counterexample trace, if any.
pub fn check(shape: ClaimShape) -> Result<usize, String> {
    // A vCPU that rings twice (the second ring is what perturbs the stamp under a claim), and a
    // worker that scans, claims and releases.
    let vcpu = Thread {
        prog: [Some(Step::Ring), Some(Step::PublishIfOwed), Some(Step::Ring), Some(Step::PublishIfOwed)],
        pc: 0,
        owes_publish: false,
        holds_bit: false,
        holds_claim: false,
        observed_stamp: 0,
        observed_state: S::Idle,
    };
    let worker = Thread {
        prog: [Some(Step::TakeBit), Some(Step::ClaimLoad), Some(Step::ClaimCas), Some(Step::Release)],
        pc: 0,
        owes_publish: false,
        holds_bit: false,
        holds_claim: false,
        observed_stamp: 0,
        observed_state: S::Idle,
    };
    let w0 = World { state: S::Idle, stamp: 0, bit: false, queued: 0, served: 0 };

    let mut seen: HashSet<(World, Vec<Thread>)> = HashSet::new();
    let mut explored = 0usize;
    let mut stack = vec![(w0, vec![vcpu, worker], Vec::<(usize, Step)>::new())];

    while let Some((w, ts, trace)) = stack.pop() {
        if !seen.insert((w, ts.clone())) {
            continue;
        }
        explored += 1;
        let done = ts.iter().all(|t| t.pc as usize >= t.prog.len() || t.prog[t.pc as usize].is_none());
        if done {
            if let Err(why) = quiescent_invariant(&w, &ts) {
                let t: Vec<String> = trace.iter().map(|(i, s)| format!("T{i}:{s:?}")).collect();
                return Err(format!("{why}\n  interleaving: {}", t.join(" → ")));
            }
            continue;
        }
        for i in 0..ts.len() {
            let pc = ts[i].pc as usize;
            let Some(s) = ts[i].prog.get(pc).copied().flatten() else { continue };
            let (mut w2, mut ts2) = (w, ts.clone());
            step(&mut w2, &mut ts2[i], s, shape);
            ts2[i].pc += 1;
            let mut tr = trace.clone();
            tr.push((i, s));
            stack.push((w2, ts2, tr));
        }
    }
    Ok(explored)
}
