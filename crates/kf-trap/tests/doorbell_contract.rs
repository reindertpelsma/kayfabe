//! ★ 2026-10-17 — **The doorbell contract, pinned on the REAL token word** (`kf_trap::TokenWord`),
//! GPU-free. The model (`kf-chan/tests/user_loop_model.rs`) proves the same rules over every
//! interleaving of a transliterated worker; these tests make the rules a regression suite on the
//! production atomic itself, one test per rule, each named after the rule it pins.
//!
//! THE CONTRACT (owner, `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` §1, restated 2026-10-17):
//!  1. the guest writes entries, then rings; the LAST doorbell covers every `GP_PUT` write before it;
//!  2. nothing else announces `GP_PUT` (no poll, no timer); the relay loops until its cursor equals
//!     the `GP_PUT` it loaded, then stops;
//!  3. a doorbell that fires during that loop forces one more pass, repeated until a pass sees no
//!     ring; none may be missed;
//!  4. doorbells coalesce: any number of rings before the next pass starts make exactly ONE pass;
//!  5. a doorbell with no work is safe: any process may ring at any time.
//!
//! `Relay` below is the smallest faithful use of the word: claim, load `GP_PUT` AFTER the claim,
//! consume up to it, release (`ActAgain` = another pass). It is the shape of
//! `ChanPlane::user_ring_serve`, not a copy of it.

use kf_trap::{Claim, REACT_ROUNDS, Release, Route, State, TokenWord};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

struct Relay {
    tok: TokenWord,
    /// The guest's `GP_PUT` (what a doorbell makes visible).
    put: AtomicU32,
    /// The relay's cursor: how far it has consumed.
    cursor: AtomicU32,
    /// Passes the relay made (a pass = one claim-load-consume).
    passes: AtomicU64,
}

impl Relay {
    fn new() -> Self {
        let tok = TokenWord::new();
        assert!(tok.allocate_fresh(Route::Passthrough, 1));
        Self {
            tok,
            put: AtomicU32::new(0),
            cursor: AtomicU32::new(0),
            passes: AtomicU64::new(0),
        }
    }
    /// The guest: write entries (GP_PUT), THEN ring (rule 1).
    fn guest_put(&self, put: u32) {
        self.put.store(put, Ordering::Release);
    }
    fn ring(&self) -> bool {
        self.tok.ring(1)
    }
    /// One worker visit: claim; then passes until the token says no ring arrived during the pass.
    /// Returns the number of passes of this visit and how the visit ended.
    fn visit(&self) -> Option<(u32, Release)> {
        let Claim::Won(_) = self.tok.claim() else {
            return None;
        };
        let mut rounds = 0u32;
        loop {
            // the claim cleared the flag BEFORE this load (rule 3's construction)
            let put = self.put.load(Ordering::Acquire);
            self.cursor.store(put, Ordering::Release);
            self.passes.fetch_add(1, Ordering::Relaxed);
            rounds += 1;
            match self.tok.release(rounds, REACT_ROUNDS) {
                Release::ActAgain => {}
                other => return Some((rounds, other)),
            }
        }
    }
}

/// RULE 4: any number of rings before the pass starts make exactly one pass.
#[test]
fn doorbells_before_the_pass_coalesce_into_one_pass() {
    for n in [1usize, 2, 5, 100, 10_000] {
        let r = Relay::new();
        r.guest_put(7);
        let mut publishes = 0;
        for _ in 0..n {
            publishes += usize::from(r.ring());
        }
        assert_eq!(
            publishes, 1,
            "{n} rings: only the IDLE->RUNG ring asks for a wake"
        );
        assert_eq!(
            r.visit(),
            Some((1, Release::Idled)),
            "{n} rings -> exactly one pass"
        );
        assert_eq!(r.cursor.load(Ordering::Acquire), 7);
        assert!(r.visit().is_none(), "nothing left rung after the pass");
        assert_eq!(r.tok.load().state, State::Idle);
    }
}

/// RULE 3: a ring that lands during the pass (after the claim) forces exactly one more pass, and the
/// entries published behind it are consumed by that pass.
#[test]
fn a_doorbell_during_a_pass_forces_one_more_pass() {
    let r = Relay::new();
    r.guest_put(3);
    r.ring();
    let Claim::Won(_) = r.tok.claim() else {
        panic!("the ring is ours")
    };
    // the pass loads GP_PUT=3 ... the guest writes PUT=9 and rings before the release
    let loaded = r.put.load(Ordering::Acquire);
    r.cursor.store(loaded, Ordering::Release);
    r.guest_put(9);
    assert!(
        !r.ring(),
        "a ring over BUSY publishes nothing: the owner will see it"
    );
    assert_eq!(r.tok.release(1, REACT_ROUNDS), Release::ActAgain);
    // the extra pass sees the new PUT
    r.cursor
        .store(r.put.load(Ordering::Acquire), Ordering::Release);
    assert_eq!(r.cursor.load(Ordering::Acquire), 9);
    assert_eq!(
        r.tok.release(2, REACT_ROUNDS),
        Release::Idled,
        "no ring in the extra pass: stop"
    );
    assert_eq!(r.tok.load().state, State::Idle);
}

/// RULE 3, repeated: a guest that rings during every pass gets a pass after every ring; the loop
/// ends only when a pass sees no ring, and the relay's cursor ends at the last PUT. The 8-round
/// bound (`REACT_ROUNDS`, the fairness timeslice) republishes the token instead of dropping it: a
/// ring is never lost, only deferred to the next worker visit.
#[test]
fn rings_during_every_pass_are_never_lost_even_across_the_fairness_bound() {
    let r = Relay::new();
    let total = 100u32;
    r.guest_put(1);
    r.ring();
    let mut put = 1;
    let mut visits = 0;
    while let Claim::Won(_) = r.tok.claim() {
        visits += 1;
        let mut rounds = 0;
        loop {
            r.cursor
                .store(r.put.load(Ordering::Acquire), Ordering::Release);
            rounds += 1;
            if put < total {
                put += 1;
                r.guest_put(put);
                r.ring(); // during the pass
            }
            match r.tok.release(rounds, REACT_ROUNDS) {
                Release::ActAgain => {}
                Release::Idled => break,
                // the timeslice: state is RUNG again, the caller republishes and moves on
                Release::RepublishAndMoveOn => {
                    assert_eq!(r.tok.load().state, State::Rung);
                    break;
                }
            }
        }
        assert!(visits < 1000, "the relay must make progress");
    }
    assert_eq!(
        r.cursor.load(Ordering::Acquire),
        total,
        "the last ring's PUT was consumed"
    );
    assert_eq!(r.tok.load().state, State::Idle);
    assert!(
        visits > 1,
        "the bound forced at least one republish ({visits} visits)"
    );
}

/// RULE 5: spurious doorbells (empty ring, idle relay, repeated, interleaved with real work, on a
/// freed channel) are safe: no fault, no state corruption, no lost work behind them.
#[test]
fn a_doorbell_with_no_work_is_safe_and_real_work_behind_it_is_still_relayed() {
    let r = Relay::new();
    // empty ring, idle relay
    assert!(r.ring());
    assert_eq!(r.visit(), Some((1, Release::Idled)));
    assert_eq!(r.cursor.load(Ordering::Acquire), 0, "nothing invented");
    // repeated, back to back, no PUT change
    for _ in 0..1000 {
        r.ring();
        assert_eq!(r.visit(), Some((1, Release::Idled)));
    }
    assert_eq!(r.tok.load().state, State::Idle);
    // real work afterwards is still relayed
    r.guest_put(5);
    r.ring();
    assert_eq!(r.visit(), Some((1, Release::Idled)));
    assert_eq!(r.cursor.load(Ordering::Acquire), 5);
    // a ring after the channel was freed is absorbed and does not resurrect the token
    assert!(r.tok.retire());
    assert!(!r.ring());
    assert_eq!(r.tok.load().state, State::Dead);
    assert!(r.visit().is_none());
}

/// RULES 1-3 under real concurrency: a guest thread writes PUT then rings, many times, with random
/// pauses; one worker thread claims whenever it can. After the guest's LAST ring the relay must end
/// with cursor == the last PUT, with NO poll and NO timer anywhere: the worker only visits when the
/// ring was published (the `bool` from `ring`) or republished by the release.
#[test]
fn after_the_last_doorbell_the_cursor_reaches_the_last_put_with_no_poll() {
    for seed in 0..40u32 {
        let r = Arc::new(Relay::new());
        let rungs = Arc::new(AtomicU64::new(0)); // wakes the guest owes the worker
        let done = Arc::new(AtomicU32::new(0));
        let n = 2000u32;
        let guest = {
            let (r, rungs, done) = (r.clone(), rungs.clone(), done.clone());
            std::thread::spawn(move || {
                let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
                for i in 1..=n {
                    r.guest_put(i);
                    if r.ring() {
                        rungs.fetch_add(1, Ordering::AcqRel);
                    }
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    if x % 7 == 0 {
                        std::thread::yield_now();
                    }
                }
                done.store(1, Ordering::Release);
            })
        };
        let worker = {
            let (r, rungs, done) = (r.clone(), rungs.clone(), done.clone());
            std::thread::spawn(move || {
                // "parked" = waiting for a published wake (rungs) -- never for time, never reading PUT
                loop {
                    let owed = rungs.load(Ordering::Acquire);
                    if owed > 0 {
                        rungs.fetch_sub(1, Ordering::AcqRel);
                        if let Some((_, Release::RepublishAndMoveOn)) = r.visit() {
                            // the release asks the caller to republish: that is a wake we owe ourselves
                            rungs.fetch_add(1, Ordering::AcqRel);
                        }
                    } else if done.load(Ordering::Acquire) == 1
                        && rungs.load(Ordering::Acquire) == 0
                    {
                        break;
                    } else {
                        std::thread::yield_now();
                    }
                }
            })
        };
        guest.join().unwrap();
        worker.join().unwrap();
        assert_eq!(
            r.cursor.load(Ordering::Acquire),
            n,
            "seed {seed}: the last doorbell's PUT was not consumed"
        );
        assert_eq!(r.tok.load().state, State::Idle, "seed {seed}");
    }
}
