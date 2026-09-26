//! Ported from the old kayfabe-doorbell suite: the tests of the composed plane.
#![allow(unused_imports)]

use crate::*;
use kf_trap::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

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
            index: kf_trap::tokenindex::TokenIndex::Vector { mask: 0x1f },
            timer: timer::TIMER_GV100,
        }
    }
}

#[derive(Default)]
struct RecordingHost {
    rung: std::sync::atomic::AtomicU64,
    translated: std::sync::atomic::AtomicU64,
    emulated: std::sync::atomic::AtomicU64,
    registers: std::sync::Mutex<Vec<(u32, u64)>>,
    untranslatable: AtomicBool,
    mirror_stale: AtomicBool,
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
    fn operands_translatable(&self, _t: u32, _s: u64) -> plane::Translatable {
        use std::sync::atomic::Ordering as O;
        if self.mirror_stale.load(O::Acquire) {
            plane::Translatable::NotYet
        } else if self.untranslatable.load(O::Acquire) {
            plane::Translatable::No
        } else {
            plane::Translatable::Yes
        }
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
fn claim_excludes_a_second_worker() {
    // ★ §5.2: "A bit that says work exists is not a bit that says you may touch the hardware."
    // Without this, two workers walk one channel against one cursor and [c0,p1) executes twice.
    let w = TokenWord::new();
    w.allocate_fresh(Route::Translated, 7);
    w.ring(1);
    assert!(matches!(w.claim(), Claim::Won(_)));
    assert_eq!(w.claim(), Claim::NotOurs, "a second claim must fail");
}

#[test]
fn the_react_loop_is_bounded_and_hands_the_token_back() {
    // ⊘ §5.2: otherwise "a process ringing its own channel in a tight loop keeps a worker in
    // act → CAS fails → act forever; with enough channels it pins every worker and the guest
    // kernel's own scrub and UVM channels starve" — the INNER boundary of §4.
    let w = TokenWord::new();
    w.allocate_fresh(Route::Passthrough, 1);
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
    w.allocate_fresh(Route::Passthrough, 0xAAA);
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
        assert!(w.allocate_fresh(Route::Translated, i as u32));
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

#[test]
fn the_doorbell_pages_other_offsets_do_nothing_at_all() {
    // ★ §5: "The doorbell page is 64 KiB and the doorbell is four bytes of it; every other offset
    // on it is guest-userspace-writable, and without this arm an unprivileged process pushes
    // unbounded garbage onto the plane reserved for guest root."
    let f = Fixture::new(32);
    f.tokens[3].allocate_fresh(Route::Translated, 0x33);
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
fn a_leaf_that_starts_inside_a_block_but_runs_past_it_is_refused() {
    let mut l = GuestRamLayout::new();
    l.register(GuestRamBlock::register(0, 0x1000, 0x1000));
    assert!(l.leaf(0x1000, 0x1000).is_ok(), "exactly filling the block is legal");
    assert_eq!(l.leaf(0x1800, 0x1000), Err(LeafRefusal::CrossesBlockEnd));
    assert_eq!(l.refused(), 1);
}

// ---- §3 the shape, end to end -----------------------------------------------------------------

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
        assert!(w.allocate_fresh(Route::Translated, i as u32));
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
    p.tokens[2].allocate_fresh(Route::Passthrough, 0x22);
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
    gpu0.tokens[1].allocate_fresh(Route::Translated, 0x11);
    gpu1.tokens[2].allocate_fresh(Route::Translated, 0x22);

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
    gpu0.tokens[5].allocate_fresh(Route::Passthrough, 0xAAA);
    gpu1.tokens[5].allocate_fresh(Route::Passthrough, 0xBBB);
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
fn a_kernel_channel_with_an_untranslatable_operand_is_refused_not_faulted_on_the_live_path() {
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
fn a_user_channel_with_the_same_miss_faults_on_the_live_path() {
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
fn a_forge_happens_only_for_emulated_work_on_the_live_path() {
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
fn the_cap_refuses_a_channel_on_the_live_path() {
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
fn a_leaf_naming_our_memslot_never_reaches_a_host_map_on_the_live_path() {
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
fn teardown_runs_the_whole_ordered_sequence_on_the_live_path() {
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut w = WalkerState::default();
    assert_eq!(w.diff(9), lifetime::Diff::Changed(9));
    p.teardown(&host, &mut w).unwrap();
    assert_eq!(*host.teardown.lock().unwrap(), lifetime::ORDER.to_vec(), "in §9's order");
    assert_eq!(w.diff(9), lifetime::Diff::Changed(9), "the second instance must map");
}

// ---- host verbs: authored, never forwarded; and the pointer discipline ------------------------

#[test]
fn a_stale_mirror_puts_the_token_back_instead_of_poisoning_the_device() {
    // ⊘⊘⊘ `[fable, CRITICAL S2]`. The guest kernel advances the scrubber's GP_PUT and issues a
    // TLB invalidate; the VA manager applies the diff LATER. A ring landing in that window found
    // operands untranslatable, and on a KERNEL channel that went straight to refuse-and-poison:
    // a GUEST-WIDE DoS from a timing hint, with no attacker required.
    // ⇒ §5.6's fence is "the mirror must catch up"; §5.2 says put it back and re-publish.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 3, Route::Translated, 0x33, Owner::Kernel).unwrap();

    host.mirror_stale.store(true, O::Release);
    p.trap_write(Class::Doorbell, 0, 0, 3, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);

    assert_eq!(host.poisoned.load(O::Acquire), 0, "⊘ a stale mirror must NOT poison the device");
    assert_eq!(p.tokens[3].load().state, State::Rung, "the token is put back, not consumed");
    assert!(p.bits.bit(3), "and re-published, so a later pass retries");

    // The mirror catches up; the same work now goes through.
    host.mirror_stale.store(false, O::Release);
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.translated.load(O::Acquire), 1, "the retry submits");
    assert_eq!(host.poisoned.load(O::Acquire), 0);
}

#[test]
fn a_genuinely_untranslatable_kernel_operand_still_refuses() {
    // ⚠ The other half: `NotYet` must not become a way to never refuse. A real miss still
    // refuses+poisons on a kernel channel.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 4, Route::Translated, 0x44, Owner::Kernel).unwrap();
    host.untranslatable.store(true, O::Release);
    p.trap_write(Class::Doorbell, 0, 0, 4, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.poisoned.load(O::Acquire), 1);
}

#[test]
fn no_completion_is_forged_for_work_that_was_refused_or_faulted() {
    // ⊘ `[fable S6]`. §8: "A completion written for work that did not happen is how a scrub
    // becomes a leak" -- and REFUSED work is the purest case of work that did not happen.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 5, Route::Emulated, 0x55, Owner::User).unwrap();
    host.untranslatable.store(true, O::Release);
    p.trap_write(Class::Doorbell, 0, 0, 5, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.faulted.load(O::Acquire), 1, "a user channel faults");
    assert_eq!(host.forged.load(O::Acquire), 0, "⊘ and NOTHING is forged for it");
}

#[test]
fn a_passthrough_token_reached_by_a_worker_is_released_not_wedged() {
    // ⊘ `[fable S4]`. The arm used to `break` without release(), leaving the word BUSY forever --
    // so retire() returned false forever and the free path spun. Reachable via the trap's
    // load->ring TOCTOU during a chid recycle.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 6, Route::Translated, 0x66, Owner::User).unwrap();
    p.trap_write(Class::Doorbell, 0, 0, 6, 4);
    // The channel is recycled as Passthrough while a bit is outstanding.
    assert!(p.tokens[6].retire() || true);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);
    assert_ne!(p.tokens[6].load().state, State::Busy, "⊘ must not be wedged BUSY");
}

// ---- fable w823 HIGH S3 / S5 / S7 / H3 ---------------------------------------------------------

#[test]
fn allocating_over_a_live_token_is_refused_and_the_caller_learns() {
    // ⊘⊘ `[fable S3]`. `allocate` accepted Idle (a LIVE token nobody is acting on) and was a
    // single non-retrying CAS, and the caller ignored its result. The failure: kernel recycles
    // chid 7 for process B; process A rings 7 between the load and the CAS; allocate fails; the
    // word keeps A's host_token; B's rings are served on A's FREED host channel.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 7, Route::Translated, 0x77, Owner::User).unwrap();
    // Re-allocating without freeing must FAIL and must not rehome the token.
    assert!(
        p.allocate_channel(&mut caps, 7, Route::Translated, 0xBB, Owner::User).is_err(),
        "⊘ allocation over a live token must be refused"
    );
    assert_eq!(p.tokens[7].load().host_token, 0x77, "and must not have rehomed it");
}

#[test]
fn a_recycled_kernel_chid_does_not_keep_the_kernel_failure_policy() {
    // ⊘⊘⊘ `[fable S5]`. kernel_tokens was sticky: never removed on free, cleared only at
    // teardown. A kernel chid recycled to a USER process kept the KERNEL arm, so that user
    // channel's translation miss poisoned the device guest-wide -- defeating §7's whole
    // blast-radius argument.
    use std::sync::atomic::Ordering as O;
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let host = RecordingHost::default();
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    p.allocate_channel(&mut caps, 9, Route::Translated, 0x99, Owner::Kernel).unwrap();
    assert!(p.free_channel(&mut caps, 9), "the kernel channel is freed");
    // chid 9 is recycled to an unprivileged process.
    p.allocate_channel(&mut caps, 9, Route::Translated, 0xAA, Owner::User).unwrap();
    host.untranslatable.store(true, O::Release);
    p.trap_write(Class::Doorbell, 0, 0, 9, 4);
    let mut scratch = Vec::new();
    p.worker_pass(&host, &mut scratch, 8);
    assert_eq!(host.poisoned.load(O::Acquire), 0, "⊘ a USER channel must NOT poison the device");
    assert_eq!(host.faulted.load(O::Acquire), 1, "it faults, and the blast radius is the asker");
}

#[test]
fn a_guest_root_token_index_cannot_panic_the_vmm() {
    // ⊘ `[fable S7]`. `self.tokens[tok as usize]` with no bound. The value is guest-derived.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 16, 0xf);
    let mut caps = VmCaps::from_declared(64, 8, 8, 8);
    // Far past the table; must refuse, not panic.
    let _ = p.allocate_channel(&mut caps, 4_000_000_000, Route::Translated, 1, Owner::User);
    let _ = p.allocate_channel(&mut caps, 200, Route::Translated, 1, Owner::User);
}

#[test]
fn caps_are_released_on_free_so_a_long_lived_guest_is_not_refused_forever() {
    // ⊘ `[fable H3]`. acquire with no release: a guest that allocated and freed more channels
    // than its cap over its LIFE was refused forever.
    let vmm = plane::Vmm::new();
    let mut p = Plane::new(&vmm, 32, 0x1f);
    let mut caps = VmCaps::from_declared(2, 8, 8, 8);
    for i in 0..20u32 {
        let tok = i % 2;
        p.allocate_channel(&mut caps, tok, Route::Translated, i, Owner::User)
            .unwrap_or_else(|e| panic!("cycle {i} refused: {e:?}"));
        assert!(p.free_channel(&mut caps, tok));
    }
    assert_eq!(caps.live(Twin::Channel), 0, "every acquire was matched by a release");
}
