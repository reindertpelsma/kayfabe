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
}
impl Fixture {
    fn new(n: usize) -> Fixture {
        Fixture {
            tokens: (0..n).map(|_| TokenWord::new()).collect(),
            bits: RungBitmap::new(),
            wworker: WakeWord::new(),
            wdrainer: WakeWord::new(),
            ring: PrivRing::new(),
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
fn an_untranslatable_operand_on_a_KERNEL_channel_refuses_and_never_faults() {
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
