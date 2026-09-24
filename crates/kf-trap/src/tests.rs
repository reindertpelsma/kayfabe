//! Each test names the §5 hazard it guards. ⊘ A test here that does not cite a paragraph of the
//! design is testing the implementation against itself.

use crate::*;

// ---- §5.2 the four states + DEAD -------------------------------------------------------------

#[test]
fn ring_on_idle_publishes_and_ring_on_rung_does_not() {
    let w = TokenWord::new();
    assert!(w.allocate_fresh(Route::Passthrough, 0x123));
    // §5.2: IDLE → RUNG is the transition that owes a bit and a wake.
    assert!(w.ring(1), "IDLE→RUNG must ask the caller to publish");
    // §5.2: "a further ring returns early because RUNG is already set, so it produces no bit and
    // no wake" — correct, because a bit is already outstanding.
    assert!(!w.ring(2), "RUNG→RUNG must NOT publish again");
    assert_eq!(w.load().state, State::Rung);
    assert_eq!(w.load().applied_seq, 2, "the stamp must still advance");
}

#[test]
fn ring_while_busy_becomes_busy_rung_and_is_re_acted() {
    let w = TokenWord::new();
    w.allocate_fresh(Route::Translated, 7);
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
            timer: timer::TIMER_GV100,
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
fn a_passthrough_doorbell_is_inline_with_no_queue_no_wake_no_ring() {
    // §5: "write the host doorbell INLINE. No queue, no wake, no lock. Return."
    let f = Fixture::new(32);
    f.tokens[7].allocate_fresh(Route::Passthrough, 0xABC);
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
    f.tokens[5].allocate_fresh(Route::Translated, 0x55);
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
    f.tokens[9].allocate_fresh(Route::Emulated, 0x99);
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

use std::sync::atomic::AtomicBool;

#[test]
fn the_model_check_finds_the_single_cas_claim_bug() {
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
fn a_refused_time_write_touches_nothing_a_neighbour_write_still_queues() {
    // ⊘ The refusal is by NAME: the register next door on the same page is an ordinary
    // privileged write and must still reach the ring, or the refusal has become a page-wide hole.
    let f = Fixture::new(4);
    let p = f.path();
    let sem = shadow::WriteSemantics::Plain;
    let priv_ = Class::Privileged { readable: true, semantics: sem };
    assert_eq!(p.write(priv_, 0, 0x9400, 1, 4), Action::RefusedByName);
    assert_eq!(f.ring.occupancy(), 0);
    assert_ne!(p.write(priv_, 0, 0x9404, 1, 4), Action::RefusedByName, "0x9404 is not named");
    assert_eq!(f.ring.occupancy(), 1, "the neighbour was queued");
    // And the refusal is BAR0-scoped: the same offset on another BAR is not the timer.
    assert_ne!(p.write(priv_, 1, 0x9400, 1, 4), Action::RefusedByName);
}

#[test]
fn an_unprivileged_process_cannot_starve_the_kernels_channels() {
    // ⊘⊘⊘ THE ATTACK, reproduced. `[fable, CRITICAL]`: scan() began at summary word 0 and took
    // the first `limit` bits in ASCENDING token order. The guest's chid allocator hands out
    // ascending ids, so an early-starting unprivileged process owns the low tokens. Ringing
    // >= limit of them in a loop fills every scan and the kernel's scrubber/UVM token is NEVER
    // reached. Measured on the old code: kernel token served 0 times in 10 000 passes.
    //
    // ⚠ And the reason it shipped: §5.2's K-round timeslice bounds how long a worker holds ONE
    // claim. It says nothing about which tokens a scan LOOKS AT. Fairness of holding is not
    // fairness of finding.
    let b = RungBitmap::new();
    const LIMIT: usize = 16;
    const KERNEL_TOK: u32 = 9_000; // a high chid, as the kernel's channels get
    let mut out = Vec::new();
    let mut kernel_seen = 0usize;

    for _pass in 0..400 {
        // The attacker re-rings its whole low-chid block every pass.
        for t in 0..64u32 {
            b.publish(t);
        }
        // The kernel rings once and waits.
        b.publish(KERNEL_TOK);
        b.scan(&mut out, LIMIT);
        if out.contains(&KERNEL_TOK) {
            kernel_seen += 1;
        }
    }
    assert!(
        kernel_seen > 0,
        "⊘ STARVED: the kernel's token was never scanned in 400 passes while an unprivileged \
         process held the low chids — this is §4's inner boundary breached"
    );
}

#[test]
fn the_scan_start_rotates_so_no_group_holds_priority() {
    // ★ The mechanism, asserted directly: two disjoint groups, a limit that can only serve one
    // group per pass, and both must get served across passes.
    let b = RungBitmap::new();
    let mut out = Vec::new();
    let (mut low, mut high) = (0usize, 0usize);
    for _ in 0..64 {
        b.publish(1);      // summary word 0
        b.publish(70_000); // a far-away summary word
        b.scan(&mut out, 1);
        if out.contains(&1) { low += 1 }
        if out.contains(&70_000) { high += 1 }
    }
    assert!(low > 0 && high > 0, "both groups must be reached: low={low} high={high}");
}

// ---- fable w823 CRITICAL S2 / S4 / S6 ----------------------------------------------------------


#[test]
fn the_timer_hal_is_per_family_and_only_gv100_has_a_priv_level_mask() {
    use kf_chip::Family;
    use timer::*;
    assert_eq!(timer_regs_for(Family::Ga10x), TIMER_GV100);
    assert_eq!(timer_regs_for(Family::Ad10x), TIMER_GV100);
    assert_eq!(timer_regs_for(Family::Gh100), TIMER_GH100);
    assert_eq!(timer_regs_for(Family::Gb20x), TIMER_GH100);
    // The PLM shadow: bit 4 = WRITE_PROTECTION_LEVEL0_ENABLE, so ogkm takes its `if` branch and
    // never reaches the NV_ASSERT(0) in the else (`timer_gv100.c:56,77-81`).
    assert_eq!(TIMER_GV100.plm_shadow(), Some((0x9430, 1 << 4)));
    assert_eq!(TIMER_GH100.plm_shadow(), None, "the GH100 body has no privilege test");
    assert_eq!(TIMER_GB10B.plm_shadow(), None);
    assert!(TIMER_GB10B.refused_writes.is_empty(), "GB10B writes no time register at all");
}
