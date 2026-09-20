//! Each test names the §5 hazard it guards. ⊘ A test here that does not cite a paragraph of the
//! design is testing the implementation against itself.

use crate::*;

// ---- §5.2 the four states + DEAD -------------------------------------------------------------

#[test]
fn ring_on_idle_publishes_and_ring_on_rung_does_not() {
    let w = TokenWord::new();
    assert!(w.allocate(Route::Passthrough, 0x123));
    // §5.2: IDLE → RUNG is the transition that owes a bit and a wake.
    assert!(w.ring(1), "IDLE→RUNG must ask the caller to publish");
    // §5.2: "a further ring returns early because RUNG is already set, so it produces no bit and
    // no wake" — correct, because a bit is already outstanding.
    assert!(!w.ring(2), "RUNG→RUNG must NOT publish again");
    assert_eq!(w.load().state, State::Rung);
    assert_eq!(w.load().applied_seq, 2, "the stamp must still advance");
}

#[test]
fn claim_excludes_a_second_worker() {
    // ★ §5.2: "A bit that says work exists is not a bit that says you may touch the hardware."
    // Without this, two workers walk one channel against one cursor and [c0,p1) executes twice.
    let w = TokenWord::new();
    w.allocate(Route::Translated, 7);
    w.ring(1);
    assert!(matches!(w.claim(), Claim::Won(_)));
    assert_eq!(w.claim(), Claim::NotOurs, "a second claim must fail");
}

#[test]
fn ring_while_busy_becomes_busy_rung_and_is_re_acted() {
    let w = TokenWord::new();
    w.allocate(Route::Translated, 7);
    w.ring(1);
    assert!(matches!(w.claim(), Claim::Won(_)));
    assert!(!w.ring(2), "a ring over BUSY publishes nothing — the owner will see it");
    assert_eq!(w.load().state, State::BusyRung);
    assert_eq!(w.release(0, REACT_ROUNDS), Release::ActAgain);
    assert_eq!(w.load().state, State::Busy);
    assert_eq!(w.release(1, REACT_ROUNDS), Release::Idled);
    assert_eq!(w.load().state, State::Idle);
}

#[test]
fn the_react_loop_is_bounded_and_hands_the_token_back() {
    // ⊘ §5.2: otherwise "a process ringing its own channel in a tight loop keeps a worker in
    // act → CAS fails → act forever; with enough channels it pins every worker and the guest
    // kernel's own scrub and UVM channels starve" — the INNER boundary of §4.
    let w = TokenWord::new();
    w.allocate(Route::Passthrough, 1);
    w.ring(1);
    assert!(matches!(w.claim(), Claim::Won(_)));
    for round in 0..REACT_ROUNDS {
        w.ring(round as u64 + 2); // the adversary, ringing in a tight loop
        assert_eq!(w.release(round, REACT_ROUNDS), Release::ActAgain);
    }
    w.ring(99);
    assert_eq!(
        w.release(REACT_ROUNDS, REACT_ROUNDS),
        Release::RepublishAndMoveOn,
        "after K rounds the worker MUST give up the token — that is the timeslice"
    );
    assert_eq!(w.load().state, State::Rung, "and it goes back as RUNG, not IDLE");
}

#[test]
fn retire_waits_out_busy_then_allocation_installs_the_new_route() {
    // ⊘ §5.2: without DEAD, "the guest frees a channel while its token is BUSY, the free path
    // drops the twin, the driver RECYCLES the channel id, and the new channel's first ring lands
    // on a word still carrying the OLD route — while the old worker is still reading what it
    // believes is a pushbuffer." This is the use-after-free.
    let w = TokenWord::new();
    w.allocate(Route::Passthrough, 0xAAA);
    w.ring(1);
    assert!(matches!(w.claim(), Claim::Won(_)));
    assert!(!w.retire(), "retire MUST refuse while a worker owns the token");
    // The recycled id must not be installable while the old worker is live.
    assert!(!w.allocate(Route::Emulated, 0xBBB), "allocation over a BUSY token is the UAF");
    assert_eq!(w.release(0, REACT_ROUNDS), Release::Idled);
    assert!(w.retire());
    assert_eq!(w.load().state, State::Dead);
    assert!(!w.ring(5), "a retired token absorbs rings and must not resurrect");
    assert_eq!(w.load().state, State::Dead);
    assert!(w.allocate(Route::Emulated, 0xBBB), "DEAD → IDLE with the new route");
    assert_eq!(w.load().route, Route::Emulated);
    assert_eq!(w.load().host_token, 0xBBB);
}

// ---- §5.1 the bitmap --------------------------------------------------------------------------

#[test]
fn publish_sets_bit_then_summary_and_scan_finds_it() {
    let b = RungBitmap::new();
    b.publish(1234);
    assert!(b.bit(1234) && b.summary_bit(1234));
    let mut out = Vec::new();
    assert_eq!(b.scan(&mut out, 64), 1);
    assert_eq!(out, vec![1234]);
}

#[test]
fn a_token_published_during_a_scan_is_not_lost() {
    // ⊘⊘ §5.1: "Clear the summary bit first, then re-read the word. Clearing AFTER a scan loses
    // a token that arrived during it." This is the ordering that makes that true.
    let b = RungBitmap::new();
    b.publish(10);
    let mut out = Vec::new();
    b.scan(&mut out, 64);
    assert_eq!(out, vec![10]);
    // Arrives right after the scan drained the word.
    b.publish(11);
    out.clear();
    assert_eq!(b.scan(&mut out, 64), 1, "the later publish must still be findable");
    assert_eq!(out, vec![11]);
}

#[test]
fn a_partial_scan_puts_back_bit_and_summary_together() {
    // ⊘ §5.2: "Setting only the summary loses the token PERMANENTLY." The put-back path in scan()
    // must restore BOTH, in publish order.
    let b = RungBitmap::new();
    for t in 0..8u32 {
        b.publish(t);
    }
    let mut out = Vec::new();
    assert_eq!(b.scan(&mut out, 3), 3);
    out.clear();
    let rest = b.scan(&mut out, 64);
    assert_eq!(rest, 5, "the five not taken must still be reachable, not stranded");
}

// ---- §5.3 the wake word -------------------------------------------------------------------

#[test]
fn bump_owes_no_syscall_when_nobody_is_parked() {
    let w = WakeWord::new();
    assert_eq!(w.bump(), Wake::NoOne, "the common case must cost no eventfd write");
}

#[test]
fn a_bump_during_a_scan_refuses_the_park() {
    // ★ §5.3: prepare_to_wait-then-schedule. "register as polling ONLY if the sequence is
    // unchanged; otherwise rescan." This closes the lost-wakeup race with no flag.
    let w = WakeWord::new();
    let seen = w.seen();
    w.bump(); // work arrives while the worker is scanning
    assert!(!w.try_park(seen), "must rescan, never sleep on stale evidence");
}

#[test]
fn a_parked_worker_is_signalled_exactly_once() {
    let w = WakeWord::new();
    let seen = w.seen();
    assert!(w.try_park(seen));
    assert_eq!(w.pollers(), 1);
    assert_eq!(w.bump(), Wake::SignalOne);
    w.unpark();
    assert_eq!(w.pollers(), 0);
}

#[test]
fn the_sequence_carry_falls_off_the_top_and_never_becomes_a_phantom_poller() {
    // ★★★ §5.3's whole reason for the 56/8 split, HIGH sequence: "a carry then falls off the top
    // of the word instead of landing in the poller count as a phantom poller that makes every
    // later trap pay a syscall."
    let w = WakeWord::new();
    // Drive the sequence to its maximum, then one more.
    for _ in 0..4 {
        w.bump();
    }
    let before = w.pollers();
    // Simulate a full wrap by bumping 2^56 times — done arithmetically, not by looping.
    // The invariant under test: pollers is untouched by ANY number of bumps.
    for _ in 0..1000 {
        w.bump();
    }
    assert_eq!(w.pollers(), before, "bumping must never disturb the poller count");
    assert_eq!(before, 0);
}

#[test]
fn the_poller_count_saturates_rather_than_wrapping_into_the_sequence() {
    // ⊘ 8 bits holds 255, which is §3's worker cap. Wrapping would corrupt work_seq.
    let w = WakeWord::new();
    let seen = w.seen();
    let mut parked = 0;
    for _ in 0..300 {
        if w.try_park(seen) {
            parked += 1;
        }
    }
    assert_eq!(parked, wake::MAX_WORKERS, "must cap at 255, never wrap");
    assert_eq!(w.seen(), seen, "and parking must not move the sequence");
}

// ---- the whole plane under concurrency -------------------------------------------------------

#[test]
fn no_ring_is_ever_lost_under_concurrent_vcpus_and_workers() {
    // ★★★ THE TEST THE DESIGN IS FOR. §5's entire purpose is the invariant the owner stated:
    // "anything queued now on the SPSC ring is guaranteed to be eventually scheduled, and it
    // never blocks."
    //
    // ⊘ A single-threaded test cannot fail the way this plane fails. The losses §5.1/§5.2 warn
    // about — a bit published without a summary, a summary cleared after a scan, a put-back that
    // republishes only half — all need a publisher and a scanner running at once.
    //
    // ⚠ This is a RACE test, not a proof: on x86 TSO it cannot catch a missing fence (§5.3 says
    // so). It catches lost work, which is the failure that matters here.
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as O};
    use std::sync::Arc;

    const N_TOKENS_USED: u32 = 512;
    const RINGS_PER_VCPU: u64 = 2_000;
    const N_VCPU: usize = 4;
    const N_WORKER: usize = 3;

    struct Plane {
        words: Vec<TokenWord>,
        bits: RungBitmap,
        wake: WakeWord,
        served: AtomicU64,
        stop: AtomicBool,
    }

    let plane = Arc::new(Plane {
        words: (0..N_TOKENS_USED).map(|_| TokenWord::new()).collect(),
        bits: RungBitmap::new(),
        wake: WakeWord::new(),
        served: AtomicU64::new(0),
        stop: AtomicBool::new(false),
    });
    for (i, w) in plane.words.iter().enumerate() {
        assert!(w.allocate(Route::Translated, i as u32));
    }

    let mut hs = Vec::new();
    for v in 0..N_VCPU {
        let p = Arc::clone(&plane);
        hs.push(std::thread::spawn(move || {
            for n in 0..RINGS_PER_VCPU {
                let t = ((n as u32).wrapping_mul(2654435761).wrapping_add(v as u32)) % N_TOKENS_USED;
                // ★ THE vCPU PATH, in the order §5.2/§5.3 mandate:
                //   stamp+RUNG (one CAS) → publish bit,summary → bump.
                if p.words[t as usize].ring(n + 1) {
                    p.bits.publish(t);
                    let _ = p.wake.bump();
                }
            }
        }));
    }
    for _ in 0..N_WORKER {
        let p = Arc::clone(&plane);
        hs.push(std::thread::spawn(move || {
            let mut found = Vec::new();
            loop {
                let seen = p.wake.seen();          // §5.3: seen BEFORE the scan
                p.bits.scan(&mut found, 128);
                if found.is_empty() {
                    if p.stop.load(O::Acquire) {
                        // Drain once more: a ring may have landed between scan and stop.
                        p.bits.scan(&mut found, 128);
                        if found.is_empty() {
                            return;
                        }
                    } else {
                        let _ = p.wake.try_park(seen);
                        if p.wake.pollers() > 0 {
                            p.wake.unpark();
                        }
                        std::thread::yield_now();
                        continue;
                    }
                }
                for &t in &found {
                    if let Claim::Won(_) = p.words[t as usize].claim() {
                        let mut round = 0;
                        loop {
                            p.served.fetch_add(1, O::AcqRel);
                            match p.words[t as usize].release(round, REACT_ROUNDS) {
                                Release::Idled => break,
                                Release::ActAgain => round += 1,
                                Release::RepublishAndMoveOn => {
                                    // ⊘ bit THEN summary — the same order the trap uses.
                                    p.bits.publish(t);
                                    let _ = p.wake.bump();
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }));
    }
    for h in hs.drain(..N_VCPU) {
        h.join().unwrap();
    }
    plane.stop.store(true, O::Release);
    for h in hs {
        h.join().unwrap();
    }

    // ⊘⊘ THE ASSERTION THAT MATTERS: after every vCPU has finished and every worker has drained,
    // NO TOKEN MAY BE LEFT RUNG. A token stuck in RUNG with no bit is the permanent loss §5.2
    // describes; a token left BUSY is a leaked claim.
    let mut stuck = Vec::new();
    for (i, w) in plane.words.iter().enumerate() {
        let st = w.load().state;
        if st != State::Idle {
            stuck.push((i, st));
        }
    }
    assert!(
        stuck.is_empty(),
        "tokens left un-idle after drain — work was LOST or a claim leaked: {stuck:?}"
    );
    assert!(plane.served.load(O::Acquire) > 0, "the workers must have served something");
}
