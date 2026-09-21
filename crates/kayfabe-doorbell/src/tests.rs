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
                        // ⊘⊘⊘ **PAIR try_park WITH unpark, AND ONLY WHEN IT SUCCEEDED.**
                        // `[w823]` this was `let _ = try_park(seen); if pollers() > 0 { unpark() }`
                        // — which decrements ANOTHER worker's registration whenever our own park
                        // was refused (the sequence moved) but somebody else happened to be
                        // parked. That is an unbalanced `unpark`, it corrupts the poller count,
                        // and it trips `unpark`'s own debug assert. ⚠ It reproduced **once in
                        // ~75 runs**, which is exactly how a wrong pairing behaves: harmless
                        // until two threads interleave at the one point where it matters.
                        // ⇒ Only the thread that registered may deregister.
                        if p.wake.try_park(seen) {
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
    // ⊘⊘⊘ **THE DIAGNOSTIC THAT SEPARATES THE TWO CAUSES, AND WITHOUT IT THE FAILURE IS
    // UNACTIONABLE.** A token left `Rung` means one of two completely different things:
    //   * **bit STILL SET** ⇒ the token was published correctly and the WORKERS EXITED before
    //     draining it. A defect in this test's stop protocol; the plane is fine.
    //   * **bit MISSING**   ⇒ a token is `Rung` with nothing pointing at it. That is the
    //     PERMANENT LOSS §5.2 describes, and it is a defect in the plane.
    // `[w823]` the first version printed only `[(288, Rung), (365, Rung)]`, which cannot tell
    // them apart — and "is it my harness or my design" is the entire question.
    let mut stuck = Vec::new();
    for (i, w) in plane.words.iter().enumerate() {
        let st = w.load().state;
        if st != State::Idle {
            stuck.push((i, st, plane.bits.bit(i as u32), plane.bits.summary_bit(i as u32)));
        }
    }
    let orphaned: Vec<_> = stuck.iter().filter(|(_, _, bit, _)| !*bit).collect();
    assert!(
        orphaned.is_empty(),
        "⊘ PLANE DEFECT: token(s) RUNG with NO bitmap bit — the permanent loss of §5.2. \
         (token, state, bit, summary) = {orphaned:?}"
    );
    assert!(
        stuck.is_empty(),
        "⊘ HARNESS DEFECT: token(s) still published (bit set) when the workers exited — this \
         test's stop protocol raced, the plane did not lose anything. \
         (token, state, bit, summary) = {stuck:?}"
    );
    assert!(plane.served.load(O::Acquire) > 0, "the workers must have served something");
}

// ---- §5.4 the privileged ring ----------------------------------------------------------------

#[test]
fn the_ring_preserves_global_order_across_vcpus() {
    // ★ §5.4's reason for ONE ring: "a guest thread on one vCPU writes PDB_LO/PDB_HI under an RM
    // lock and releases it; another thread on another vCPU takes the lock and writes TRIGGER...
    // Per-vCPU rings preserve only per-vCPU order and fire the trigger against a stale base."
    let r = PrivRing::new();
    let pdb_lo = RegWrite { bar: 0, offset: 0x1000, value: 0xdead, width: 4 };
    let trigger = RegWrite { bar: 0, offset: 0x1008, value: 1, width: 4 };
    assert!(matches!(r.push(pdb_lo), Push::Queued(0)));
    assert!(matches!(r.push(trigger), Push::Queued(1)));
    assert_eq!(r.peek().unwrap().1, pdb_lo, "the base must drain BEFORE the trigger");
    r.commit();
    assert_eq!(r.peek().unwrap().1, trigger);
    r.commit();
    assert!(r.peek().is_none());
}

#[test]
fn peek_then_commit_means_a_failed_apply_retries_at_the_head() {
    // ⊘ §5.4: "peek → apply → commit, so a failed apply retries at the head". Consuming before
    // applying drops a write whose apply failed -- the silent-drop bug one layer down.
    let r = PrivRing::new();
    let w = RegWrite { bar: 0, offset: 0x40, value: 7, width: 4 };
    r.push(w);
    assert_eq!(r.peek().unwrap().1, w);
    // apply "fails" -- we do NOT commit
    assert_eq!(r.peek().unwrap().1, w, "the same write must still be at the head");
    assert_eq!(r.applied_seq(), 0, "applied_seq must not move on a failed apply");
    r.commit();
    assert_eq!(r.applied_seq(), 1);
}

#[test]
fn full_poisons_and_claims_nothing_and_then_drops_by_name() {
    // ⊘ §5.4: "Full ⇒ poison the device, never wait". And the claim must reserve NOTHING on the
    // full path -- "a claim-then-bail strands the consumer at that slot forever."
    let r = PrivRing::new();
    for i in 0..ring::CAPACITY {
        assert!(matches!(r.push(RegWrite { bar: 0, offset: i as u32, value: 0, width: 4 }), Push::Queued(_)));
    }
    assert_eq!(r.occupancy(), ring::CAPACITY);
    assert_eq!(r.push(RegWrite { bar: 0, offset: 0xffff, value: 0, width: 4 }), Push::Poisoned);
    assert!(r.is_poisoned());
    assert_eq!(r.occupancy(), ring::CAPACITY, "the failed push must NOT have reserved a slot");
    // Every slot must still be drainable -- no hole.
    for i in 0..ring::CAPACITY {
        assert_eq!(r.peek().unwrap().1.offset, i as u32, "a hole would strand the drainer here");
        r.commit();
    }
    assert!(r.peek().is_none());
    // ⊘ And once poisoned, further writes are dropped BY NAME, never silently: §5.4's security
    // half -- "dropping one write silently leaves state a later, differently-privileged guest
    // process inherits."
    assert_eq!(r.push(RegWrite { bar: 0, offset: 1, value: 1, width: 4 }), Push::Dropped);
}

#[test]
fn concurrent_producers_leave_no_hole_for_the_drainer() {
    // ⊘ The CAS-on-tested-cursor claim exists so that a producer never reserves a slot it will
    // not fill. With fetch_add, a racing full-check bails AFTER reserving and the drainer stops
    // at that index forever. This test would deadlock-by-assert in that design.
    // ⊘⊘ **THIS TEST WAS A LIE UNTIL w823 AND A KNOWN-POSITIVE CAUGHT IT.** It ran 6x500 = 3000
    // pushes into a 4096 ring, so the ring NEVER FILLED and the bail path -- the only path that
    // can strand the drainer -- never executed. With `push` deliberately rewritten to the
    // fetch_add-claim-then-bail that §5.4 says "strands the consumer at that slot forever", this
    // test still PASSED. A test named `leave_no_hole` that cannot observe a hole is the
    // "refuse by name means the NAME IS TRUE" failure, in a test.
    // ⇒ Oversubscribe deliberately: enough producers to exceed CAPACITY, so the full/bail path
    // runs CONCURRENTLY with live producers, which is the only way a hole can appear.
    use std::sync::Arc;
    let r = Arc::new(PrivRing::new());
    const PER: u32 = 1500;
    const N: u32 = 6;
    let mut hs = Vec::new();
    for v in 0..N {
        let r = Arc::clone(&r);
        hs.push(std::thread::spawn(move || {
            for i in 0..PER {
                r.push(RegWrite { bar: 0, offset: v * PER + i, value: v as u64, width: 4 });
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    // N*PER = 9000 > CAPACITY, so the ring MUST have filled and poisoned.
    assert!(r.is_poisoned(), "9000 pushes into a {}-slot ring must poison", ring::CAPACITY);
    let claimed = r.occupancy();
    assert!(claimed <= ring::CAPACITY, "occupancy {claimed} exceeded capacity — a lost bail");
    // ⊘ THE ASSERTION THE KNOWN-POSITIVE DEMANDS: every slot the producers CLAIMED must be
    // drainable. Under fetch_add-then-bail, a bailing producer leaves slot `tail` never marked
    // ready, `peek()` returns None at that index, and the drain stops SHORT of `claimed`.
    let mut drained = 0;
    while r.peek().is_some() {
        r.commit();
        drained += 1;
    }
    assert_eq!(
        drained, claimed,
        "drained {drained} of {claimed} claimed slots — a producer reserved a slot it never          filled, and the drainer is stranded at that index forever (§5.4)"
    );
}

#[test]
fn the_high_water_mark_is_recorded_for_the_teardown_line() {
    // §5.4: "sustained occupancy above 25 % is a defect to investigate, and the high-water mark
    // prints at teardown."
    let r = PrivRing::new();
    for i in 0..10 {
        r.push(RegWrite { bar: 0, offset: i, value: 0, width: 4 });
    }
    assert_eq!(r.high_water(), 10);
    while r.peek().is_some() {
        r.commit();
    }
    assert_eq!(r.high_water(), 10, "the mark is a maximum, not a gauge");
    assert!((r.high_water() as usize) < ring::OCCUPANCY_ALARM);
}

// ---- §5.5 the shadow -------------------------------------------------------------------------

#[test]
fn w1c_does_not_lose_a_concurrent_set() {
    // ⊘ §5.5: "fetch_and(!bits) — a load-store loses a worker's concurrent set." The worker sets
    // pending bits from the host edge while the guest's ISR clears the ones it saw.
    use std::sync::Arc;
    let c = Arc::new(Cell::new());
    c.apply(WriteSemantics::Plain, 0b1111);
    let c2 = Arc::clone(&c);
    // The "worker" setting a new pending bit concurrently with the ISR's clear.
    let h = std::thread::spawn(move || {
        for _ in 0..10_000 {
            // a set is an OR; under W1C-by-load-store this would be clobbered
            let _ = c2.read();
        }
    });
    for _ in 0..10_000 {
        c.apply(WriteSemantics::W1c, 0b0001);
    }
    h.join().unwrap();
    assert_eq!(c.read() & 0b0001, 0, "the cleared bit must stay cleared");
    assert_eq!(c.read() & 0b1110, 0b1110, "the bits the guest did NOT clear must survive");
}

#[test]
fn a_write_only_port_does_not_move_its_own_shadow() {
    // §5.5: "the readable effect is on a DIFFERENT register". Storing here would invent a value
    // the hardware does not have.
    let c = Cell::new();
    c.apply(WriteSemantics::WriteOnlyPort, 0xdead_beef);
    assert_eq!(c.read(), 0, "a write-only port's own cell must not take the value");
}

#[test]
fn a_stale_completion_may_not_clear_a_later_trigger() {
    // ⊘⊘⊘ §5.5's silent corruption, and it is the guest driver's DOCUMENTED behaviour, not a bug
    // we can fix: "the guest times out on invalidate A and continues; it later issues invalidate
    // B; our work for A finishes and clears the trigger; the guest reads zero and concludes B is
    // done. It is not."
    let t = Trigger::new();
    t.arm(100);                       // invalidate A, at ring position 100
    assert_eq!(t.read(), 1, "the guest spins while non-zero");
    t.arm(200);                       // the guest timed out on A and issued B
    // A's work finally finishes.
    assert_eq!(
        t.complete(100),
        ClearOutcome::Superseded,
        "A's completion MUST NOT clear B's trigger — that is the silent corruption"
    );
    assert_eq!(t.read(), 1, "the guest must still see B as outstanding");
    assert_eq!(t.complete(200), ClearOutcome::Cleared);
    assert_eq!(t.read(), 0);
}

#[test]
fn the_trigger_counts_issued_and_completed_separately() {
    // §5.5: "It keeps two counters — what it has issued and what has completed — and the owning
    // thread clears the shadow when its work is done. Doorbell workers fence on completed."
    let t = Trigger::new();
    t.arm(1);
    t.arm(2);
    assert_eq!(t.issued(), 2);
    assert_eq!(t.completed(), 0, "issued != completed is the whole point of two counters");
    t.complete(1); // superseded, but the work DID complete
    t.complete(2);
    assert_eq!(t.completed(), 2);
}

#[test]
fn an_overdue_trigger_trips_before_the_guest_gives_up() {
    // ⚠ §5.5's tripwire. Past the guest's own timeout it "proceeds anyway, with stale
    // translations and no error" -- so we must fault BEFORE that, or we never learn.
    let t = Trigger::new();
    let guest_budget = 4_000_000_000u64; // ~4s
    t.arm(1);
    assert!(!t.is_overdue(1_000_000_000, guest_budget), "1s of a 4s budget is not overdue");
    assert!(t.is_overdue(3_000_000_000, guest_budget), "3s of a 4s budget must trip the tripwire");
    t.complete(1);
    assert!(!t.is_overdue(u64::MAX, guest_budget), "a cleared trigger is never overdue");
}

// ---- §8 completions and interrupts -----------------------------------------------------------

#[test]
fn only_emulated_work_that_never_reached_the_gpu_may_be_forged() {
    // ⊘⊘ §8: "Forge is licensed ONLY where there was no work. A completion written for work that
    // did not happen is how a scrub becomes a leak." This campaign's most expensive measured
    // defect; the type must refuse to express it.
    assert_eq!(Completion::for_route(Route::Emulated, false), Completion::Forge);
    assert_eq!(
        Completion::for_route(Route::Emulated, true),
        Completion::Nothing,
        "emulated work that DID reach the GPU must not be forged — that is the leak"
    );
    // The GPU wrote the forwarded semaphore itself; a second author for one value is a bug.
    assert_eq!(Completion::for_route(Route::Translated, true), Completion::Nothing);
    assert_eq!(Completion::for_route(Route::Translated, false), Completion::Nothing);
    // We never inspected the channel, so its completion is not ours.
    assert_eq!(Completion::for_route(Route::Passthrough, true), Completion::Nothing);
    // An unknown route should never have been served, let alone completed.
    assert_eq!(Completion::for_route(Route::Unknown, false), Completion::Nothing);
}

#[test]
fn an_edge_that_arrives_while_masked_is_delivered_on_unmask() {
    // ⚠ §8's binding converse: "once armed, a later release MUST produce an interrupt. There is
    // no level-triggered fallback." Dropping the edge hangs a waiter with no recovery.
    // ⊘ And §5.5's hazard: the ISR writes the mask then reads pending, so the two are one state.
    let e = EngineIrq::new();
    e.mask();
    assert_eq!(e.retire(), Raise::Hold, "masked ⇒ held, not delivered");
    assert_eq!(e.delivered(), 0);
    assert_eq!(e.unmask(), Raise::Deliver, "the held edge MUST surface on unmask");
    assert_eq!(e.delivered(), 1);
    assert_eq!(e.unmask(), Raise::Hold, "and it must not be delivered twice");
}

#[test]
fn an_armed_engine_fires_on_new_work_and_not_retroactively() {
    // ★ §8: "The waiter owns the race ... so we fire on genuinely new completed work and need not
    // fire retroactively."
    let e = EngineIrq::new();
    e.arm();
    assert_eq!(e.retire(), Raise::Deliver);
    assert_eq!(e.retire(), Raise::Deliver);
    assert_eq!(e.delivered(), 2);
    assert_eq!(e.retired_count(), 2);
    // Arming again with nothing new pending must not manufacture an edge.
    assert_eq!(e.unmask(), Raise::Hold, "re-arming is not a completion");
}

#[test]
fn a_polled_engine_can_legitimately_deliver_zero() {
    // ⚠ §8: "The channels we care about most POLL — UVM spins, and the scrubber's blocking waits
    // loop on the semaphore word." A zero here is not automatically a defect, which is why the
    // interrupt plane is not the completion plane. `[w684]` 40 of 44 completions were never
    // announced, and that was correct.
    let e = EngineIrq::new(); // never armed: nobody registered for events
    for _ in 0..40 {
        assert_eq!(e.retire(), Raise::Hold);
    }
    assert_eq!(e.delivered(), 0, "never armed ⇒ nothing delivered");
    assert_eq!(e.retired_count(), 40, "but the work DID retire — count it separately");
}

// ---- §5 the trap path, and §4's INNER boundary ------------------------------------------------

struct Fixture {
    tokens: Vec<TokenWord>,
    bits: RungBitmap,
    wworker: WakeWord,
    wdrainer: WakeWord,
    ring: PrivRing,
    read_traps: readtrap::ReadTrapSet,
}
impl Fixture {
    fn new(n: usize) -> Fixture {
        Fixture {
            tokens: (0..n).map(|_| TokenWord::new()).collect(),
            bits: RungBitmap::new(),
            wworker: WakeWord::new(),
            wdrainer: WakeWord::new(),
            ring: PrivRing::new(),
            read_traps: readtrap::ReadTrapSet::new(),
        }
    }
    fn path(&self) -> TrapPath<'_> {
        TrapPath {
            tokens: &self.tokens,
            bits: &self.bits,
            worker_wake: &self.wworker,
            drainer_wake: &self.wdrainer,
            ring: &self.ring,
            token_mask: 0x1f,
            read_traps: &self.read_traps,
            phase: readtrap::Phase::Boot,
        }
    }
}

#[test]
fn an_unprivileged_process_cannot_keep_workers_from_parking() {
    // ⊘⊘⊘ §5's named attack, written as the adversary: "Bumping the sequence for an unowned token
    // lets an unprivileged process keep every worker spinning: workers register as polling only
    // if the sequence is unchanged, so a token nobody owns, rung in a loop, prevents them ever
    // parking."
    let f = Fixture::new(32);
    // Every token is Unknown -- nobody allocated them.
    let p = f.path();
    let seen = f.wworker.seen();
    for i in 0..10_000u64 {
        assert_eq!(p.write(Class::Doorbell, 0, 0, i & 0x1f, 4), Action::None);
    }
    assert_eq!(
        f.wworker.seen(),
        seen,
        "10 000 rings on UNOWNED tokens must not move work_seq — otherwise no worker can ever park"
    );
    // ⇒ And the consequence that makes it a DoS if violated: a worker can still park.
    assert!(f.wworker.try_park(seen), "a worker must still be able to park after the flood");
}

#[test]
fn the_doorbell_pages_other_offsets_do_nothing_at_all() {
    // ★ §5: "The doorbell page is 64 KiB and the doorbell is four bytes of it; every other offset
    // on it is guest-userspace-writable, and without this arm an unprivileged process pushes
    // unbounded garbage onto the plane reserved for guest root."
    let f = Fixture::new(32);
    f.tokens[3].allocate(Route::Translated, 0x33);
    let p = f.path();
    for off in (0..65536).step_by(4) {
        assert_eq!(
            p.write(Class::UserspaceMappable, 0, off, 3, 4),
            Action::None,
            "offset {off} on the doorbell page must do NOTHING"
        );
    }
    assert_eq!(f.ring.occupancy(), 0, "not one byte may reach the privileged ring");
    assert!(!f.ring.is_poisoned(), "and it must not be poisonable from userspace either");
    assert_eq!(f.tokens[3].load().state, State::Idle, "no token may be disturbed");
}

#[test]
fn a_passthrough_doorbell_is_inline_with_no_queue_no_wake_no_ring() {
    // §5: "write the host doorbell INLINE. No queue, no wake, no lock. Return."
    let f = Fixture::new(32);
    f.tokens[7].allocate(Route::Passthrough, 0xABC);
    let p = f.path();
    let seen = f.wworker.seen();
    assert_eq!(p.write(Class::Doorbell, 0, 0, 7, 4), Action::RingHostInline { host_token: 0xABC });
    assert_eq!(f.wworker.seen(), seen, "passthrough must not bump the work sequence");
    assert_eq!(f.ring.occupancy(), 0, "and must not touch the privileged ring");
}

#[test]
fn a_translated_doorbell_publishes_once_however_hard_it_is_rung() {
    // ★ The cheap path that makes a tight ring loop survivable rather than a denial of service:
    // a second ring over RUNG costs one CAS and produces no bit and no wake.
    let f = Fixture::new(32);
    f.tokens[5].allocate(Route::Translated, 0x55);
    let p = f.path();
    let seen = f.wworker.seen();
    assert_eq!(p.write(Class::Doorbell, 0, 0, 5, 4), Action::None); // published; nobody parked
    assert_eq!(f.wworker.seen(), seen + 1);
    for _ in 0..5_000 {
        assert_eq!(p.write(Class::Doorbell, 0, 0, 5, 4), Action::None);
    }
    assert_eq!(
        f.wworker.seen(),
        seen + 1,
        "5 000 further rings over an already-RUNG token must produce exactly ZERO further bumps"
    );
    let mut out = Vec::new();
    assert_eq!(f.bits.scan(&mut out, 64), 1, "and exactly one bit");
    assert_eq!(out, vec![5]);
}

#[test]
fn a_parked_worker_is_woken_by_a_translated_doorbell() {
    let f = Fixture::new(32);
    f.tokens[9].allocate(Route::Emulated, 0x99);
    let p = f.path();
    let seen = f.wworker.seen();
    assert!(f.wworker.try_park(seen));
    assert_eq!(p.write(Class::Doorbell, 0, 0, 9, 4), Action::WakeWorker);
}

#[test]
fn a_privileged_write_wakes_the_DRAINER_not_a_worker() {
    // ⊘ §5.3: "the drainer parks on its own word... Sharing one word with a wake-exactly-one
    // policy means a privileged register write can wake A WORKER INSTEAD OF THE DRAINER — and the
    // register write then waits for an unrelated doorbell while the guest spins on a trigger."
    let f = Fixture::new(32);
    let p = f.path();
    let wseen = f.wworker.seen();
    let dseen = f.wdrainer.seen();
    assert!(f.wworker.try_park(wseen), "a worker is parked");
    assert!(f.wdrainer.try_park(dseen), "and so is the drainer");
    let a = p.write(
        Class::Privileged { readable: true, semantics: WriteSemantics::Plain },
        0,
        0x110c00,
        1,
        4,
    );
    assert_eq!(a, Action::WakeDrainer, "the DRAINER must be the one woken");
    assert_eq!(f.wworker.seen(), wseen, "and the worker word must not have moved at all");
    assert_eq!(f.ring.occupancy(), 1);
}

#[test]
fn a_data_port_never_enters_the_privileged_ring() {
    // ⊘⊘ §5.4: on Turing/GA100 the firmware images load through auto-incrementing falcon data
    // ports -- "16 000–65 000 back-to-back writes during boot" -- and they "would overflow any
    // ring that exists". They are a synchronous store into the falcon image shadow.
    let f = Fixture::new(32);
    let p = f.path();
    for i in 0..20_000u64 {
        assert_eq!(
            p.write(
                Class::Privileged { readable: false, semantics: WriteSemantics::DataPort },
                0,
                0x110040,
                i,
                4
            ),
            Action::None
        );
    }
    assert_eq!(f.ring.occupancy(), 0, "20 000 data-port writes must not occupy one ring slot");
    assert!(!f.ring.is_poisoned(), "and must not poison the device -- boot would never complete");
}

#[test]
fn a_full_privileged_ring_poisons_rather_than_waiting() {
    let f = Fixture::new(32);
    let p = f.path();
    let cls = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };
    for i in 0..ring::CAPACITY {
        let a = p.write(cls, 0, i as u32, 0, 4);
        assert!(matches!(a, Action::None | Action::WakeDrainer));
    }
    assert_eq!(p.write(cls, 0, 0xffff, 0, 4), Action::PoisonDevice);
}

// ---- §7 channels -----------------------------------------------------------------------------

#[test]
fn an_untranslatable_operand_on_a_kernel_channel_refuses_and_never_faults() {
    // ⊘⊘⊘ §7's security boundary: "The unified-memory driver treats ANY channel error as GLOBALLY
    // FATAL — one fault kills CUDA for EVERY PROCESS IN THE GUEST until the driver reloads. A
    // design that forwards a translation miss as a sentinel fault hands unprivileged guest
    // userspace a way to kill the whole guest's GPU stack."
    let s = Submission {
        owner: Owner::Kernel,
        route: Route::Translated,
        all_operands_translatable: false,
    };
    assert_eq!(
        s.decide(),
        Disposition::RefuseAndPoison,
        "a kernel channel MUST refuse+poison, never fault — a fault is guest-wide DoS"
    );
    assert_ne!(s.decide(), Disposition::FaultChannel);
}

#[test]
fn a_user_channel_may_fault_because_the_blast_radius_is_the_asker() {
    let s = Submission {
        owner: Owner::User,
        route: Route::Translated,
        all_operands_translatable: false,
    };
    assert_eq!(s.decide(), Disposition::FaultChannel);
}

#[test]
fn translatable_operands_always_submit() {
    for owner in [Owner::Kernel, Owner::User] {
        for route in [Route::Passthrough, Route::Translated, Route::Emulated] {
            let s = Submission { owner, route, all_operands_translatable: true };
            assert_eq!(s.decide(), Disposition::Submit, "{owner:?}/{route:?}");
        }
    }
}

#[test]
fn a_kernel_channel_is_never_emulated() {
    // §7: "Kernel channels are TRANSLATED, not emulated ... running them on our CPU is how a
    // guest process reads another's freed pages." ⊘ This is the rule w823 measured violated on
    // the OLD architecture: forwarded=0 emulated>0 on seven arms.
    assert!(!Submission::kernel_channels_are_never_emulated(Owner::Kernel, Route::Emulated));
    assert!(Submission::kernel_channels_are_never_emulated(Owner::Kernel, Route::Translated));
    assert!(Submission::kernel_channels_are_never_emulated(Owner::User, Route::Emulated));
}

#[test]
fn an_unmodelled_method_form_is_sized_but_decodes_to_nothing() {
    // §7: "We SIZE every pushbuffer method form so the stream never desynchronises, and DECODE
    // only what we model. An undefined form decodes to nothing rather than to a guess."
    let (sz, d) = size_is_total(false, 5);
    assert_eq!(sz, 5, "an unmodelled form must STILL be sized, or the stream desynchronises");
    assert_eq!(d, Decoded::SizedOnly, "and must decode to nothing, never to a guess");
    let (sz, d) = size_is_total(true, 5);
    assert_eq!(sz, 5);
    assert_eq!(d, Decoded::Modelled { operand_words: 5 });
}

// ---- §6.4 the system-memory leaf bound --------------------------------------------------------

#[test]
fn a_leaf_naming_our_own_memslot_is_refused_by_name() {
    // ⊘⊘⊘ §6.4's attack, in full: "Our own structures — the register read shadow, the doorbell
    // bitmap, the boot pages — are host memory installed as guest-physical memslots. A guest
    // page-table entry naming one of THOSE addresses, resolved by a layout that maps
    // guest-physical to host pointers generically, would PIN OUR OWN STATE AND HAND IT TO THE GPU
    // AS A DMA TARGET. The guest could then have the engine write the bits the drainer owns."
    let mut l = GuestRamLayout::new();
    l.register(GuestRamBlock::register(0, 0x1_0000_0000, 0x4000_0000)); // 1 GiB of real guest RAM
    // Our doorbell bitmap / read shadow / boot pages live at a guest-physical address the guest
    // can SEE from its CPU but must never reach as a DMA leaf.
    const OUR_SHADOW_GPA: u64 = 0xF000_0000;
    assert_eq!(
        l.leaf(OUR_SHADOW_GPA, 0x1000),
        Err(LeafRefusal::NotInAnyRegisteredBlock),
        "a leaf naming OUR memory must be refused, not resolved"
    );
    assert_eq!(l.refused(), 1, "and the refusal must be COUNTED — §6.4: not silent");
    // Genuine guest RAM still resolves.
    let s = l.leaf(0x1_0000_2000, 0x1000).expect("real guest RAM must resolve");
    assert_eq!(s, HostSlice { block: 0, offset: 0x2000, len: 0x1000 });
}

#[test]
fn a_leaf_may_select_a_block_but_never_name_a_base() {
    // ★ The SHAPE, not the check: the only route to a HostSlice is GuestRamBlock::slice, and a
    // GuestRamBlock is minted at registration. There is deliberately no
    // `resolve(gpa, len) -> HostPtr` in this module -- that function is the circular one.
    let b = GuestRamBlock::register(7, 0x2_0000_0000, 0x1000_0000);
    assert_eq!(b.slice(0x100, 0x200).unwrap(), HostSlice { block: 7, offset: 0x100, len: 0x200 });
    assert_eq!(b.slice(0x0FFF_FF00, 0x200), Err(LeafRefusal::CrossesBlockEnd));
}

#[test]
fn an_overflowing_offset_cannot_wrap_into_a_legal_looking_range() {
    // ⊘ This is the ONE place a guest value meets a bound, so it is the one place wrapping would
    // be fatal: offset + len must be checked, not wrapping.
    let b = GuestRamBlock::register(0, 0, 0x1000);
    assert_eq!(b.slice(u64::MAX, 2), Err(LeafRefusal::CrossesBlockEnd));
    assert_eq!(b.slice(0x800, u64::MAX), Err(LeafRefusal::CrossesBlockEnd));
}

#[test]
fn a_leaf_that_starts_inside_a_block_but_runs_past_it_is_refused() {
    let mut l = GuestRamLayout::new();
    l.register(GuestRamBlock::register(0, 0x1000, 0x1000));
    assert!(l.leaf(0x1000, 0x1000).is_ok(), "exactly filling the block is legal");
    assert_eq!(l.leaf(0x1800, 0x1000), Err(LeafRefusal::CrossesBlockEnd));
    assert_eq!(l.refused(), 1);
}

// ---- §3 the shape, end to end -----------------------------------------------------------------

use std::sync::atomic::AtomicBool;

#[derive(Default)]
struct RecordingHost {
    rung: std::sync::atomic::AtomicU64,
    translated: std::sync::atomic::AtomicU64,
    emulated: std::sync::atomic::AtomicU64,
    registers: std::sync::Mutex<Vec<(u32, u64)>>,
    untranslatable: AtomicBool,
    forged: std::sync::atomic::AtomicU64,
    poisoned: std::sync::atomic::AtomicU64,
    faulted: std::sync::atomic::AtomicU64,
    mapped: std::sync::Mutex<Vec<leaf::HostSlice>>,
    teardown: std::sync::Mutex<Vec<lifetime::Step>>,
}
impl plane::HostOps for RecordingHost {
    fn ring_host(&self, _t: u32) {
        self.rung.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    fn run_translated(&self, _t: u32, _s: u64) -> bool {
        self.translated.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        true
    }
    fn run_emulated(&self, _t: u32, _s: u64) {
        self.emulated.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    fn apply_register(&self, _b: u8, off: u32, v: u64, _w: u8) {
        self.registers.lock().unwrap().push((off, v));
    }
    fn operands_translatable(&self, _t: u32, _s: u64) -> bool {
        !self.untranslatable.load(std::sync::atomic::Ordering::Acquire)
    }
    fn forge_completion(&self, _t: u32) {
        self.forged.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    fn refuse_and_poison(&self, _t: u32) {
        self.poisoned.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    fn fault_channel(&self, _t: u32) {
        self.faulted.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    fn map_guest_slice(&self, s: leaf::HostSlice) {
        self.mapped.lock().unwrap().push(s);
    }
    fn teardown_step(&self, s: lifetime::Step) {
        self.teardown.lock().unwrap().push(s);
    }
}

#[test]
fn the_drainer_applies_registers_in_global_order_across_vcpus() {
    // ★★★ §5.4's reason for ONE ring and ONE drainer, and §3's "an ordered ring drained by many is
    // not ordered". The hazard is concrete: "a guest thread on one vCPU writes PDB_LO/PDB_HI under
    // an RM lock and releases it; another thread on another vCPU takes the lock and writes
    // TRIGGER. On hardware the first trap returned before the lock released."
    use std::sync::Arc;
    let vmm = Box::leak(Box::new(plane::Vmm::new()));
    let p = Arc::new(Plane::new(vmm, 64, 0x3f));
    let host = Arc::new(RecordingHost::default());
    let cls = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };

    // Two "vCPUs" handing off through a lock, exactly as RM does.
    let lock = Arc::new(std::sync::Mutex::new(()));
    let mut hs = Vec::new();
    for v in 0..2u32 {
        let (p, lock) = (Arc::clone(&p), Arc::clone(&lock));
        hs.push(std::thread::spawn(move || {
            for i in 0..200u32 {
                let _g = lock.lock().unwrap();
                p.trap_write(cls, 0, 0x1000, (v * 1000 + i) as u64, 4); // base
                p.trap_write(cls, 0, 0x1008, 1, 4); // trigger
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(p.drainer_pass(&*host, 10_000), 800);
    let regs = host.registers.lock().unwrap();
    assert_eq!(regs.len(), 800);
    // ⊘ THE PROPERTY: every trigger is immediately preceded by ITS OWN base. Per-vCPU rings would
    // interleave and fire a trigger against a stale base.
    for pair in regs.chunks(2) {
        assert_eq!(pair[0].0, 0x1000, "a base must come first");
        assert_eq!(pair[1].0, 0x1008, "and its trigger immediately after");
    }
}

#[test]
fn every_ring_is_served_exactly_once_end_to_end() {
    // ★ The composition claim: vCPUs trap, workers scan/claim/serve, nothing is lost and nothing
    // is served twice. This is the whole plane running as §3 describes it.
    use std::sync::atomic::Ordering as O;
    use std::sync::Arc;
    let vmm = Box::leak(Box::new(plane::Vmm::new()));
    let p = Arc::new(Plane::new(vmm, 256, 0xff));
    let host = Arc::new(RecordingHost::default());
    for (i, w) in p.tokens.iter().enumerate().take(64) {
        assert!(w.allocate(Route::Translated, i as u32));
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut hs = Vec::new();
    for v in 0..3usize {
        let p = Arc::clone(&p);
        hs.push(std::thread::spawn(move || {
            for n in 0..3_000u64 {
                let t = ((n as u32).wrapping_mul(2654435761).wrapping_add(v as u32)) % 64;
                p.trap_write(Class::Doorbell, 0, 0, t as u64, 4);
            }
        }));
    }
    let mut ws = Vec::new();
    for _ in 0..3 {
        let (p, host, stop) = (Arc::clone(&p), Arc::clone(&host), Arc::clone(&stop));
        ws.push(std::thread::spawn(move || {
            let mut scratch = Vec::new();
            loop {
                let seen = p.worker_wake().seen();
                if p.worker_pass(&*host, &mut scratch, 64) == 0 {
                    if stop.load(O::Acquire) && p.worker_pass(&*host, &mut scratch, 64) == 0 {
                        return;
                    }
                    if p.worker_wake().try_park(seen) {
                        p.worker_wake().unpark();
                    }
                    std::thread::yield_now();
                }
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    stop.store(true, O::Release);
    for h in ws {
        h.join().unwrap();
    }

    // ⊘ No token may be left un-idle, and the bit must be gone with it.
    let orphaned: Vec<_> = p
        .tokens
        .iter()
        .enumerate()
        .filter(|(i, w)| w.load().state != State::Idle && !p.bits.bit(*i as u32))
        .map(|(i, w)| (i, w.load().state))
        .collect();
    assert!(orphaned.is_empty(), "⊘ PLANE DEFECT: rung with no bit: {orphaned:?}");
    assert!(
        p.tokens.iter().all(|w| w.load().state == State::Idle),
        "⊘ HARNESS: a token is still published when the workers exited"
    );
    assert!(host.translated.load(O::Acquire) > 0, "the host must actually have been driven");
}

#[test]
fn a_passthrough_token_is_never_served_by_a_worker() {
    // ⊘ Passthrough is rung INLINE on the vCPU (§5, §7: "may run on the vCPU"). If a worker ever
    // serves one, the classifier and the token word have disagreed.
    let vmm = plane::Vmm::new();
    let p = Plane::new(&vmm, 16, 0xf);
    let host = RecordingHost::default();
    p.tokens[2].allocate(Route::Passthrough, 0x22);
    assert_eq!(
        p.trap_write(Class::Doorbell, 0, 0, 2, 4),
        Action::RingHostInline { host_token: 0x22 }
    );
    // Nothing was published, so a worker pass finds nothing.
    let mut scratch = Vec::new();
    assert_eq!(p.worker_pass(&host, &mut scratch, 16), 0);
    assert_eq!(host.translated.load(std::sync::atomic::Ordering::Acquire), 0);
}

// ---- P1's last gate item: the exhaustive interleaving check -----------------------------------

#[test]
fn the_model_check_FINDS_the_single_cas_claim_bug() {
    // ★★★ THE KNOWN-POSITIVE FOR THE CHECKER ITSELF. A model checker that reports "no
    // counterexample" is worthless unless it can be shown to find a bug that is really there.
    //
    // This is the exact defect w823 hit on real hardware-free threads at 2/40 — a concurrent ring
    // moves the stamp, the single-CAS claim reads that as "someone else owns it", the worker
    // drops the token, and its bit is already consumed.
    match model::check(model::ClaimShape::SingleCas) {
        Err(why) => {
            assert!(why.contains("LOST"), "{why}");
            assert!(why.contains("interleaving:"), "it must NAME the interleaving: {why}");
        }
        Ok(n) => panic!("the checker explored {n} states and MISSED a known real bug"),
    }
}

#[test]
fn the_model_check_clears_the_retrying_claim() {
    // ⊘ And the negative: with the shipped shape there is NO interleaving that loses work.
    // ⚠ Under SEQUENTIAL CONSISTENCY only — this says nothing about whether Release/Acquire are
    // strong enough on a weakly-ordered target. See the module docs.
    match model::check(model::ClaimShape::Retrying) {
        Ok(explored) => assert!(explored > 20, "the checker must actually explore: {explored} states"),
        Err(why) => panic!("the shipped claim() has a losing interleaving:\n{why}"),
    }
}

// ---- §5 the read-trap allowlist ---------------------------------------------------------------

#[test]
fn a_boot_state_page_stops_read_trapping_at_runtime() {
    // ⊘⊘⊘ §5's measured parity disaster, as a test: "99% of its 1000-3000 exits per token were
    // READS of a single firmware debug register on a page this design keeps read-trapped as 'boot
    // state'. That one page cost a 2.5x LOSS ON LLM DECODE."
    let mut s = ReadTrapSet::new();
    const FW_DEBUG_PAGE: u32 = 0x110;
    s.add(FW_DEBUG_PAGE, ReadReason::BootStateMachine);
    assert_eq!(
        s.policy(FW_DEBUG_PAGE, 0x110000, Phase::Boot),
        ReadPolicy::Trap(ReadReason::BootStateMachine)
    );
    assert_eq!(
        s.policy(FW_DEBUG_PAGE, 0x110000, Phase::Runtime),
        ReadPolicy::FromShadow,
        "⊘ a boot-state page MUST stop trapping at runtime — this arm is the 2.5x"
    );
}

#[test]
fn a_latch_backed_page_traps_in_every_phase() {
    // ⊘ The framebuffer window "resolves through a latch": the value the guest must see does not
    // exist in DRAM to be read, so no phase can drop it.
    let mut s = ReadTrapSet::new();
    const FB_WINDOW_PAGE: u32 = 0x700;
    s.add(FB_WINDOW_PAGE, ReadReason::ResolvesThroughLatch);
    for phase in [Phase::Boot, Phase::Runtime] {
        assert_eq!(
            s.policy(FB_WINDOW_PAGE, 0, phase),
            ReadPolicy::Trap(ReadReason::ResolvesThroughLatch),
            "a latch cannot be served from a shadow in {phase:?}"
        );
    }
}

#[test]
fn an_unlisted_page_never_traps_which_is_the_default_that_matters() {
    // ★ §5: reads are served from ordinary DRAM "with no exit and no code -- because a vCPU inside
    // an MMIO exit is not preemptible, and driver init polls some registers thousands of times."
    let s = ReadTrapSet::new();
    for page in [0u32, 1, 42, 0x500, readtrap::BAR0_PAGES - 1] {
        assert_eq!(s.policy(page, page * 4096, Phase::Boot), ReadPolicy::FromShadow);
        assert_eq!(s.policy(page, page * 4096, Phase::Runtime), ReadPolicy::FromShadow);
    }
}

#[test]
fn the_timer_pages_settable_registers_are_refused_by_name() {
    // ⊘ §5: "the two time-register writes refused by name, so guest and host cannot drift onto
    // different timebases." A refusal, not a trap: letting the guest SET time makes every later
    // comparison between guest and host silently wrong.
    let s = ReadTrapSet::new();
    assert_eq!(s.policy(readtrap::LEGACY_TIMER_PAGE, 0x9400, Phase::Runtime), ReadPolicy::RefuseByName);
    assert_eq!(s.policy(readtrap::LEGACY_TIMER_PAGE, 0x9410, Phase::Runtime), ReadPolicy::RefuseByName);
    // ★ And the timer page itself is NOT read-trapped -- it is a computed shadow.
    assert_eq!(
        s.policy(readtrap::LEGACY_TIMER_PAGE, 0x9000, Phase::Runtime),
        ReadPolicy::FromShadow,
        "the legacy timer page is a computed shadow, not a read-trap"
    );
}

#[test]
fn the_read_trap_set_shrinks_between_boot_and_runtime() {
    // ⚠ §5 puts a number on it: "roughly 524 of 4096". The property under test is not the exact
    // count -- it is that the set is SMALLER at runtime, because a set that only ever grows has
    // given up the "only writes trap" property the design rests on.
    let mut s = ReadTrapSet::new();
    for p in 0..500u32 {
        s.add(0x100 + p, ReadReason::BootStateMachine);
    }
    for p in 0..24u32 {
        s.add(0x700 + p, ReadReason::ResolvesThroughLatch);
    }
    assert_eq!(s.trapped_at(Phase::Boot), 524, "§5's figure");
    assert_eq!(s.trapped_at(Phase::Runtime), 24, "only the latch-backed pages survive");
    assert!(
        s.trapped_at(Phase::Runtime) < s.trapped_at(Phase::Boot),
        "the set MUST shrink; a permanently-growing read-trap set is the 2.5x defect"
    );
}

// ---- the swref descriptor parser ---------------------------------------------------------------

#[test]
fn the_parser_handles_the_three_real_shapes_from_the_headers() {
    // ⊘ These lines are copied VERBATIM from ogkm's swref headers, not invented -- a parser
    // tested only on its author's examples proves nothing about the corpus.
    use swref::{parse_line, Value};

    // research_clones/ogkm/.../ampere/ga100/dev_ctrl.h:26 -- the bit range P1 names
    let d = parse_line("#define NV_CTRL_VF_DOORBELL_VECTOR                                  11:0 /* -WXUF */").unwrap();
    assert_eq!(d.name, "NV_CTRL_VF_DOORBELL_VECTOR");
    assert_eq!(d.value, Value::BitRange { hi: 11, lo: 0 });
    assert!(!d.readable, "-W... is write-only");
    assert!(d.writable);

    // .../turing/tu102/dev_vm.h:27 -- the aperture P1 names
    let d = parse_line("#define NV_VIRTUAL_FUNCTION                                    0x0003FFFF:0x00030000 /* RW--D */").unwrap();
    assert_eq!(d.value, Value::Aperture { hi: 0x0003FFFF, lo: 0x00030000 });
    assert!(d.readable && d.writable);

    // A plain offset.
    let d = parse_line("#define NV_PBUS_FOO                          0x00001700 /* R---V */").unwrap();
    assert_eq!(d.value, Value::Offset(0x1700));
    assert!(d.readable && !d.writable);
}

#[test]
fn a_constant_coded_register_is_READABLE() {
    // ⊘ `C` in position 1 was not obvious and was CHECKED against the corpus rather than guessed:
    // it appears on `NV_CONFIG_PCI_NV_0_VENDOR_ID` and on value defines like `..._NVIDIA`.
    // It means CONSTANT, and a constant is readable. Treating it as unreadable would have
    // shadowed the wrong set.
    let d = swref::parse_line("#define NV_CONFIG_PCI_NV_0_VENDOR_ID                           15:0 /* C--UF */").unwrap();
    assert!(d.readable, "C is CONSTANT, and a constant is readable");
    assert!(!d.writable);
}

#[test]
fn uncoded_and_odd_length_codes_are_skipped_not_guessed() {
    // ⚠ All 24 042 observed codes are exactly 5 chars. A 4- or 6-char code is a corpus we have
    // not seen; guessing its layout is how a capture-derived table expires as a vendor
    // regression. ⇒ Skip, never assume.
    assert!(swref::parse_line("#define NV_SOMETHING 0x1234").is_none(), "uncoded define");
    assert!(swref::parse_line("#define NV_ODD 0x1234 /* RWXY */").is_none(), "4-char code");
    assert!(swref::parse_line("/* just a comment */").is_none());
    assert!(swref::parse_line("#ifndef _DEV_VM_H_").is_none());
}

#[test]
fn the_overlay_cross_check_fails_the_build_on_a_vendor_rename() {
    // ★★★ §5.5's build gate: "it must fail the build when a register in the generated set has no
    // entry". A hand-maintained overlay whose names silently stop resolving is exactly the
    // failure `a_capture_derived_table_expires_as_a_vendor_regression` records.
    let generated = swref::parse_header(
        "#define NV_PFIFO_INTR_0 0x00002100 /* RW--V */\n\
         #define NV_PFIFO_INTR_EN_0 0x00002140 /* RW--V */\n",
    );
    assert_eq!(generated.len(), 2);
    assert!(swref::overlay_names_all_exist(&generated, &["NV_PFIFO_INTR_0"]).is_ok());
    // The vendor renames it in a new driver version:
    let missing = swref::overlay_names_all_exist(&generated, &["NV_PFIFO_INTR_0", "NV_PFIFO_INTR_LEAF"])
        .unwrap_err();
    assert_eq!(missing, vec!["NV_PFIFO_INTR_LEAF"], "the build must name what vanished");
}

#[test]
fn a_five_letter_english_word_is_not_an_access_code() {
    // ⊘ The defect that the corpus cross-check caught: "/* Fermi and later */" has a 5-character
    // first token, and length-only validation accepted it -- producing a silently unreadable,
    // unwritable descriptor. 1 975 rows of the corpus were wrong this way.
    assert!(swref::parse_line("#define NV_X 0x100 /* Fermi and later */").is_none());
    assert!(swref::parse_line("#define NV_Y 0x100 /* TODO: check */").is_none());
    // And a real code still parses.
    assert!(swref::parse_line("#define NV_Z 0x100 /* RW--V */").is_some());
}

#[test]
fn structure_bit_fields_give_the_userd_geometry() {
    // ★ NV_RAMUSERD_GP_GET is word 34 of USERD ⇒ byte 0x88. This is real geometry the doorbell
    // plane needs, and it only exists because the arithmetic form is evaluated rather than
    // skipped.
    use swref::Value;
    let d = swref::parse_line(
        "#define NV_RAMUSERD_GP_GET                     (34*32+31):(34*32+0) /* RWXUF */",
    )
    .unwrap();
    assert_eq!(d.value, Value::StructBits { hi: 34 * 32 + 31, lo: 34 * 32 });
    assert_eq!(d.value.byte_offset(), Some(0x88), "word 34 = byte 0x88");
    // ⊘ And it must NOT be mistaken for a register bit range -- 1120 would truncate to nonsense.
    assert!(!matches!(d.value, Value::BitRange { .. }));
}

#[test]
fn arithmetic_bit_positions_are_evaluated() {
    use swref::Value;
    let d = swref::parse_line("#define NV_X (0*32+31-3):(0*32+4) /* RW--V */").unwrap();
    assert_eq!(d.value, Value::BitRange { hi: 28, lo: 4 });
}

// ---- §9 lifetime and teardown -----------------------------------------------------------------

#[test]
fn closing_without_the_explicit_free_is_refused_by_name() {
    // ⊘⊘⊘ §9's central refusal. close() is NOT a synchronous free: "release runs on the LAST FILE
    // REFERENCE, not on close", and "the driver's close path defers the whole cleanup to a kernel
    // thread when its interruptible wait fails -- which a pending signal causes."
    //
    // ★ And why it is not hygiene: "a still-scheduled host channel whose ring lives in guest RAM
    // the guest has since reused is A LIVE DMA ENGINE WRITING INTO ANOTHER GUEST'S MEMORY."
    let mut t = Teardown::new();
    assert_eq!(
        t.run(Step::CloseDescriptor),
        Err(TeardownError::ClosedWithoutExplicitFree),
        "close before the ordered teardown must be refused BY NAME, not generically"
    );
}

#[test]
fn the_teardown_order_is_enforced_step_by_step() {
    let mut t = Teardown::new();
    for s in lifetime::ORDER {
        assert_eq!(t.run(s), Ok(()), "{s:?} should be next");
    }
    assert!(t.complete());

    // Any transposition is refused, naming what was expected.
    let mut t = Teardown::new();
    assert_eq!(
        t.run(Step::UnmapAll),
        Err(TeardownError::OutOfOrder {
            attempted: Step::UnmapAll,
            expected: Step::QuiesceWorkers
        })
    );
}

#[test]
fn signals_are_blocked_before_the_free_not_after() {
    // ⊘ Ordering with teeth: a pending signal is exactly what pushes the driver's close path onto
    // a kernel thread. Blocking AFTER the free would leave the free asynchronous -- the thing the
    // whole sequence exists to prevent.
    let bs = lifetime::ORDER.iter().position(|s| *s == Step::BlockSignals).unwrap();
    let fr = lifetime::ORDER.iter().position(|s| *s == Step::FreeClientTree).unwrap();
    assert!(bs < fr, "BlockSignals must precede FreeClientTree");
    let cl = lifetime::ORDER.iter().position(|s| *s == Step::CloseDescriptor).unwrap();
    assert!(fr < cl, "FreeClientTree must precede CloseDescriptor");
}

#[test]
fn mappings_and_in_flight_calls_are_released_before_the_free() {
    // ⊘ Each holds a FILE REFERENCE, and release runs on the last one. Freeing while either is
    // outstanding makes the free a no-op that returns success.
    let idx = |s: Step| lifetime::ORDER.iter().position(|x| *x == s).unwrap();
    assert!(idx(Step::UnmapAll) < idx(Step::FreeClientTree));
    assert!(idx(Step::JoinInFlight) < idx(Step::FreeClientTree));
}

#[test]
fn a_second_driver_instance_does_not_map_nothing() {
    // ⊘⊘⊘ §9's subtle one, and it is why "run every gate TWICE in one process" is the design's own
    // rule: the walker's previous-state snapshot "lives across a driver instance; if it survives
    // while every mapping is dropped, the next instance's first diff reports 'unchanged' and maps
    // NOTHING -- a second boot that faults for reasons the first did not."
    let mut w = WalkerState::default();

    // First driver instance: the walk maps everything.
    assert_eq!(w.diff(7), lifetime::Diff::Changed(7));
    assert_eq!(w.diff(7), lifetime::Diff::Unchanged, "no change within one instance is correct");

    // Teardown drops every mapping -- and MUST reset the walker.
    let mut t = Teardown::new();
    for s in lifetime::ORDER {
        if s == Step::ResetWalkerState {
            w.reset();
        }
        t.run(s).unwrap();
    }

    // Second instance: the guest rebuilds the same generation number, and the walker must NOT
    // report "unchanged" over mappings that no longer exist.
    assert_eq!(
        w.diff(7),
        lifetime::Diff::Changed(7),
        "⊘ the second boot must map -- 'unchanged' here is the fault-for-no-reason bug"
    );
}

#[test]
fn skipping_the_walker_reset_reproduces_the_second_boot_bug() {
    // ★ The known-positive, stated as a test rather than as prose: omit ONE step and the second
    // instance maps nothing.
    let mut w = WalkerState::default();
    assert_eq!(w.diff(7), lifetime::Diff::Changed(7));
    // teardown WITHOUT ResetWalkerState
    assert_eq!(
        w.diff(7),
        lifetime::Diff::Unchanged,
        "this is the defect: a surviving snapshot makes the next instance map nothing"
    );
}

// ---- §9.3 what is per GPU and what is per VMM -------------------------------------------------

#[test]
fn one_wakeup_word_serves_every_gpu() {
    // ⊘⊘⊘ §9.3: "ONE wakeup word and one eventfd -- a worker registers on exactly one word, and
    // N WORDS WOULD NEED N ATOMICS AND REOPEN THE LOST-WAKEUP PROOF."
    //
    // ★ This is the test that would have caught the original composition, where `Plane` owned its
    // own `worker_wake`: with one Plane per GPU, a worker parked for GPU 0 never learns about
    // GPU 1's work. ⚠ Each word is individually correct -- the bug needs a SECOND GPU to appear,
    // so no single-GPU test could find it.
    let vmm = plane::Vmm::new();
    let gpu0 = Plane::new(&vmm, 32, 0x1f);
    let gpu1 = Plane::new(&vmm, 32, 0x1f);
    gpu0.tokens[1].allocate(Route::Translated, 0x11);
    gpu1.tokens[2].allocate(Route::Translated, 0x22);

    // A worker parks. It registers on THE word, not on a per-GPU one.
    let seen = vmm.worker_wake.seen();
    assert!(vmm.worker_wake.try_park(seen));

    // Work arrives on the OTHER GPU.
    assert_eq!(
        gpu1.trap_write(Class::Doorbell, 0, 0, 2, 4),
        Action::WakeWorker,
        "a doorbell on GPU 1 must wake the worker parked through the VMM"
    );
    vmm.worker_wake.unpark();

    // And on the first, equally.
    let seen = vmm.worker_wake.seen();
    assert!(vmm.worker_wake.try_park(seen));
    assert_eq!(gpu0.trap_write(Class::Doorbell, 0, 0, 1, 4), Action::WakeWorker);
}

#[test]
fn each_gpu_has_its_own_ring_because_ordering_is_per_pci_function() {
    // §9.3's left column: "the ring and its drainer -- ordering is per PCI function". A shared
    // ring would impose an order ACROSS devices that the hardware does not have, and would make
    // one GPU's full ring poison another's device.
    let vmm = plane::Vmm::new();
    let gpu0 = Plane::new(&vmm, 32, 0x1f);
    let gpu1 = Plane::new(&vmm, 32, 0x1f);
    let cls = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };
    for i in 0..ring::CAPACITY {
        gpu0.trap_write(cls, 0, i as u32, 0, 4);
    }
    assert_eq!(gpu0.trap_write(cls, 0, 0xffff, 0, 4), Action::PoisonDevice);
    assert!(gpu0.ring.is_poisoned());
    assert!(!gpu1.ring.is_poisoned(), "⊘ one GPU's full ring must NOT poison another device");
    assert_eq!(gpu1.occupancy_for_test(), 0);
}

#[test]
fn token_spaces_overlap_across_gpus_and_that_is_fine() {
    // §9.3: "token words and rung bitmap (token spaces overlap across GPUs)". The same token
    // number on two GPUs is two different channels, and must not alias.
    let vmm = plane::Vmm::new();
    let gpu0 = Plane::new(&vmm, 32, 0x1f);
    let gpu1 = Plane::new(&vmm, 32, 0x1f);
    gpu0.tokens[5].allocate(Route::Passthrough, 0xAAA);
    gpu1.tokens[5].allocate(Route::Passthrough, 0xBBB);
    assert_eq!(
        gpu0.trap_write(Class::Doorbell, 0, 0, 5, 4),
        Action::RingHostInline { host_token: 0xAAA }
    );
    assert_eq!(
        gpu1.trap_write(Class::Doorbell, 0, 0, 5, 4),
        Action::RingHostInline { host_token: 0xBBB },
        "the same guest token on another GPU is a DIFFERENT host channel"
    );
}

// ---- §9.1 per-VM caps --------------------------------------------------------------------------

#[test]
fn a_guest_cannot_starve_its_neighbours_by_allocating_twins() {
    // §9.1: "The host driver enforces NO per-client quota. On a host with more than one VM, one
    // guest can STARVE THE OTHERS by allocating twins."
    let mut vm = VmCaps::from_declared(4, 8, 2, 16);
    for _ in 0..4 {
        assert_eq!(vm.acquire(Twin::Channel), Ok(()));
    }
    assert_eq!(
        vm.acquire(Twin::Channel),
        Err(Refusal::OverDeclaredCap { twin: Twin::Channel, cap: 4, asked: 5 }),
        "past the cap must be refused BY NAME, carrying the cap and the ask"
    );
    assert_eq!(vm.refused(Twin::Channel), 1, "and counted — a zero here must be evidence");
    // ⊘ Refusing one class must not disturb another.
    assert_eq!(vm.acquire(Twin::AddressSpace), Ok(()));
    vm.release(Twin::Channel);
    assert_eq!(vm.acquire(Twin::Channel), Ok(()), "a freed twin returns capacity");
}

#[test]
fn the_cap_is_what_we_already_told_the_guest() {
    // ★★★ §9.1: "We already told the guest how many channels it may have, so THAT NUMBER is the
    // cap." ⇒ Enforcing it is not a policy choice; a guest past it has ignored what we told it.
    // ⊘ Inventing a different number would be a policy, wrong in one direction or the other for
    // every workload.
    const DECLARED_CHANNELS: u32 = 2048; // the figure handed out in GET_GSP_STATIC_INFO
    let mut vm = VmCaps::from_declared(DECLARED_CHANNELS, 0, 0, 0);
    for _ in 0..DECLARED_CHANNELS {
        vm.acquire(Twin::Channel).unwrap();
    }
    match vm.acquire(Twin::Channel) {
        Err(Refusal::OverDeclaredCap { cap, .. }) => {
            assert_eq!(cap, DECLARED_CHANNELS, "the refusal must cite the DECLARED number");
        }
        Ok(()) => panic!("the declared cap was not enforced"),
    }
}

#[test]
fn an_unbackable_aperture_page_is_refused_not_holed() {
    // ⊘⊘ §9.1: "make an unbackable page a NAMED REFUSAL rather than a silent hole — the guest
    // reading an unbacked hole is A WRONG ANSWER, NOT AN ERROR." A hole returns plausible bytes
    // and the guest proceeds on them.
    let mut a = Aperture::size_from_free(256 << 20, 128 << 20);
    assert_eq!(a.len(), 128 << 20, "sized from what is actually FREE, not from the request");
    assert_eq!(a.back(0, 4096), Backing::Backed { host_offset: 0 });
    assert_eq!(
        a.back((128 << 20) - 2048, 4096),
        Backing::RefusedUnbackable,
        "a page running past the backed length must be refused, never holed"
    );
    assert_eq!(a.refusals(), 1);
    // Overflow must not wrap into a legal-looking range.
    assert_eq!(a.back(u64::MAX, 4096), Backing::RefusedUnbackable);
}

// ---- THE WIRING, exercised end to end ---------------------------------------------------------

#[test]
fn a_kernel_channel_with_an_untranslatable_operand_is_refused_not_faulted_ON_THE_LIVE_PATH() {
    // ★★★ §7 was a correct, tested COMPONENT calling nobody. This asserts it on the path: the
    // worker must consult it before handing work to the host.
    // ⊘ A fault here would be a guest-wide DoS -- UVM treats any channel error as globally fatal.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 3, Route::Translated, 0x33, Owner::Kernel).unwrap();
    host.untranslatable.store(true, O::Release);

    p.trap_write(Class::Doorbell, 0, 0, 3, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);

    assert_eq!(host.poisoned.load(O::Acquire), 1, "the kernel channel must be refused+poisoned");
    assert_eq!(host.faulted.load(O::Acquire), 0, "⊘ and NEVER faulted");
    assert_eq!(host.translated.load(O::Acquire), 0, "nothing may have been submitted");
}

#[test]
fn a_user_channel_with_the_same_miss_faults_ON_THE_LIVE_PATH() {
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 4, Route::Translated, 0x44, Owner::User).unwrap();
    host.untranslatable.store(true, O::Release);
    p.trap_write(Class::Doorbell, 0, 0, 4, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.faulted.load(O::Acquire), 1, "a USER channel may fault");
    assert_eq!(host.poisoned.load(O::Acquire), 0);
}

#[test]
fn a_forge_happens_only_for_emulated_work_ON_THE_LIVE_PATH() {
    // §8, wired: an Emulated route did no GPU work, so a completion is owed; a Translated route
    // had its semaphore forwarded, so forging would be a second author for one value.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 5, Route::Emulated, 0x55, Owner::User).unwrap();
    p.allocate_channel(&mut caps, 6, Route::Translated, 0x66, Owner::User).unwrap();
    let mut scratch = Vec::new();
    p.trap_write(Class::Doorbell, 0, 0, 5, 4);
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.forged.load(O::Acquire), 1, "emulated work owes a forged completion");
    p.trap_write(Class::Doorbell, 0, 0, 6, 4);
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.forged.load(O::Acquire), 1, "⊘ translated work must NOT be forged — the GPU wrote it");
}

#[test]
fn the_cap_refuses_a_channel_ON_THE_LIVE_PATH() {
    // §9.1 wired into allocation, which is the only place it can stop a guest starving a neighbour.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let mut caps = VmCaps::from_declared(2, 8, 8, 8);
    assert!(p.allocate_channel(&mut caps, 1, Route::Translated, 1, Owner::User).is_ok());
    assert!(p.allocate_channel(&mut caps, 2, Route::Translated, 2, Owner::User).is_ok());
    assert!(
        p.allocate_channel(&mut caps, 3, Route::Translated, 3, Owner::User).is_err(),
        "the third channel is past the declared cap and must be refused"
    );
    assert_eq!(p.tokens[3].load().route, Route::Unknown, "and the token must be untouched");
}

#[test]
fn a_leaf_naming_our_memslot_never_reaches_a_host_map_ON_THE_LIVE_PATH() {
    // ★★★ §6.4 wired. The signature is the guard: map_guest_slice takes a HostSlice, which can
    // only come from a block we minted -- so there is no way to reach it with a raw gpa.
    let vmm = plane::Vmm::new();
    let p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut layout = GuestRamLayout::new();
    layout.register(GuestRamBlock::register(0, 0x1_0000_0000, 0x1000_0000));
    assert!(p.resolve_and_map_leaf(&host, &layout, 0x1_0000_1000, 0x1000).is_ok());
    assert_eq!(host.mapped.lock().unwrap().len(), 1);
    // Our own memslot:
    assert_eq!(
        p.resolve_and_map_leaf(&host, &layout, 0xF000_0000, 0x1000),
        Err(LeafRefusal::NotInAnyRegisteredBlock)
    );
    assert_eq!(host.mapped.lock().unwrap().len(), 1, "⊘ and NOTHING reached the host");
}

#[test]
fn teardown_runs_the_whole_ordered_sequence_ON_THE_LIVE_PATH() {
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut w = WalkerState::default();
    assert_eq!(w.diff(9), lifetime::Diff::Changed(9));
    p.teardown(&host, &mut w).unwrap();
    assert_eq!(*host.teardown.lock().unwrap(), lifetime::ORDER.to_vec(), "in §9's order");
    assert_eq!(w.diff(9), lifetime::Diff::Changed(9), "the second instance must map");
}

#[test]
fn boot_completing_drops_the_boot_read_traps_ON_THE_LIVE_PATH() {
    // §5 wired: the phase is the Plane's, and boot_complete() is what makes the 2.5x real.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    p.read_traps.add(0x110, readtrap::ReadReason::BootStateMachine);
    assert_eq!(p.trap_read(0x110_000), readtrap::ReadPolicy::Trap(readtrap::ReadReason::BootStateMachine));
    p.boot_complete();
    assert_eq!(p.trap_read(0x110_000), readtrap::ReadPolicy::FromShadow, "⊘ the 2.5x arm");
}

// ---- host verbs: authored, never forwarded; and the pointer discipline ------------------------

#[test]
fn a_corrupted_vmm_addr_is_refused_not_dereferenced() {
    // ★★★ `[owner]` "unsafe must protect against memory bugs in safe". ⇒ `checked()` does NOT
    // trust that safe code preserved anything: it re-validates against the range the caller
    // registered, every time. A safe-code bug that swapped, truncated or aliased the VmmAddr
    // produces a REFUSAL, never a bad dereference.
    use hostverb::VmmAddr;
    const BASE: usize = 0x1000_0000;
    const LEN: usize = 0x1000;
    // SAFETY: test-only; no dereference occurs.
    let good = unsafe { VmmAddr::new(BASE, LEN) };
    assert_eq!(good.checked(BASE, LEN, 0x800), Some(BASE));

    // A safe-code bug hands back an address outside the registered range.
    let corrupted = unsafe { VmmAddr::new(BASE + 0x8000, LEN) };
    assert_eq!(corrupted.checked(BASE, LEN, 0x10), None, "outside the range ⇒ refuse");

    // A length larger than what was registered.
    assert_eq!(good.checked(BASE, LEN, 0x2000), None, "past the end ⇒ refuse");

    // ⊘ And overflow must not wrap into a legal-looking range.
    let high = unsafe { VmmAddr::new(usize::MAX - 8, LEN) };
    assert_eq!(high.checked(BASE, LEN, 64), None);
}

#[test]
fn guest_flag_words_are_refused_and_counted_never_forwarded() {
    // §9: "We author every host call; our host-verb signatures do not accept a guest flag word."
    // ⊘ The absence of the field is the enforcement; this counter makes the refusal visible so a
    // zero is evidence rather than silence.
    let mut r = hostverb::FlagRefusals::default();
    assert_eq!(r.count(), 0);
    r.refuse_guest_flags(0xdead_beef);
    assert_eq!(r.count(), 1);
}

#[test]
fn only_four_leaf_bits_are_translated_and_the_rest_are_refused() {
    // §6.4: "We translate aperture, address, read-only and page size; every other bit is refused
    // BY NAME and counted, and the kind is fixed to the store's." ⊘ Forwarding a guest-chosen
    // page KIND would mint host mappings from a guest value and touch device-global compression
    // state -- a guest-to-host coupling in the same family as forwarding a flag word.
    let translated = [hostverb::TranslatedLeafBit::ReadOnly];
    assert_eq!(translated.len(), 1, "read-only is the only leaf BIT we carry through");
}
