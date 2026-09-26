//! ★★★ **The plan→commit overlap race, made falsifiable** — issue #145, this port's half.
//!
//! The sibling `kayfabe-vmm-kvm/tests/plan_reservation.rs` covers the same three
//! properties against a real `/dev/kvm`. This file is not a courtesy copy: the defect was
//! **verbatim identical** in both `map_guest` implementations, and this is the port whose
//! tests run on a runner with no KVM at all — the KVM half is gate-skipped on GitHub, so a
//! suite that had only it would carry the fix and never once exercise it in CI.
//!
//! Three properties:
//!
//! 1. two planners racing for the **same** range: one gets it, the other is refused, and
//!    the loser's `MAP_FIXED` never runs;
//! 2. two planners racing for **disjoint** ranges in one live reservation: **both** get
//!    them, genuinely concurrently — the property a generation counter would destroy;
//! 3. a plan that never reaches commit gives its range **back**.
//!
//! ## What was wrong
//!
//! The overlap check ran only at plan time, against `Window::placements`, inside the
//! installer guard — and that guard is released before the commit so the `MAP_FIXED` can
//! run lock-free. Nothing re-checked overlap. Two threads could plan overlapping
//! placements into the same LIVE reservation, both place, the second silently overwriting
//! the first, and both commit. No refusal, no counter, no log.
//!
//! ## ★★ Why the obvious instruments cannot see it
//!
//! - **R5's presence token cannot**: here the reservation is perfectly alive. Presence is
//!   the right token for teardown and says nothing about this.
//! - **A placement-versioned generation counter would — and is the wrong cure**: it
//!   refuses concurrent publication into a live reservation at publication frequency,
//!   i.e. converts a real hazard into a livelock. Test 2 is what stops that cure being
//!   adopted by accident.
//!
//! ## ★★★ The tests RACE, they do not sequence
//!
//! Sequentially the first placement has already moved into `placements`, so the second is
//! refused by the check that was always there and nothing is proved. Both racing tests start on
//! a spin rendezvous — see [`rendezvous`], where a `Barrier` was measured to make this
//! port's race unreachable — and **retry until the ledger confirms it was reached**
//! — `plan_conflicts` for test 1, `peak_plan_reservations >= 2` for test 2.
//!
//! ## ★★ The bites, watched fail (2026-08-01, `crates/kayfabe-vmm-qemu/src/lib.rs`)
//!
//! Six mutations planted into the source, each re-run against this file and each seen RED;
//! all six compiled, because a mutation that breaks the build is not a bite:
//!
//! | mutation | what went red |
//! |---|---|
//! | the pre-#145 code restored (no claim, no `planned` check) | test 1: **two `Ok`s** |
//! | `PlanReservation::drop` made a no-op | test 3: the range never came back |
//! | `drop` reaching `windows.values_mut().next()` instead of its own region | test 3, via the decoy |
//! | the overlap predicate `<` → `<=` | test 2: adjacent publications refused |
//! | the commit reading `planned.get` instead of `remove` | tests 1 and 3, via ledger underflow |
//! | the `plan_conflicts` bump deleted | test 1: the deadline, at 60 s |
//!
//! The sibling `kayfabe-vmm-kvm/tests/plan_reservation.rs` was bitten with the same six.

mod common;

use core::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use kayfabe_mocks::watchdog::watchdog;
use kayfabe_vmm::{HostRegion, Prot, Vmm, VmmError};

use common::{BAR1_BASE, machine, page};

/// The overlap refusal — **the same text a committed overlap gets**, deliberately: the
/// caller asked for a range somebody else has, and that the other holder is a microsecond
/// from committing rather than already committed is a timing fact about us, not about the
/// request. `plan_conflicts` is what tells the two apart from in here.
const OVERLAP: &str = "a placement overlapping one already live in the same reservation";

/// ★ A backstop on the retry loops — **not** the instrument. The instrument is the ledger
/// moving; this bounds only the case where it never moves at all.
const RACE_DEADLINE: Duration = Duration::from_secs(60);

/// ★★ A watchdog on top of the deadline, because the deadline cannot see this defect
/// class's real failure mode: a claim released under a lock already held **self-deadlocks**
/// on a non-reentrant `Mutex`, and a deadlocked worker never returns to the loop that would
/// check `Instant::now()`. It writes to a real fd — libtest's capture swallows a watchdog's
/// `eprintln!` and dies with the abort it exists to explain.
const WEDGE_LIMIT: Duration = Duration::from_secs(120);

/// ★★★ **A SPIN rendezvous, and NOT a `Barrier` — measured, not stylistic** (2026-08-01,
/// this file, on the 38-core build box).
///
/// `Barrier::wait` parks the first arriver on a futex and returns immediately to the
/// second, so the second starts already on a CPU while the first has to be woken. That
/// wake-up costs tens of microseconds; the whole plan→commit gap it has to land in is a
/// `dup` plus one `mmap`. The second planner therefore commits before the first has
/// resumed, and the interleaving is never entered.
///
/// It is not a small effect and it is not a flake. With a `Barrier`, the QEMU port's
/// same-range race reported **191 709 attempts in 60 s and zero collisions** in the
/// full-workspace run, and 6/6 red when re-run alone; the KVM port passed in the same run,
/// because its execute phase is a real `KVM_SET_USER_MEMORY_REGION`-backed window and a
/// memfd `MAP_FIXED` — wide enough to absorb the wake-up. A difference in *test
/// scaffolding* that makes one port's race unreachable and the other's routine is the
/// sharpest kind of vacuous green: the QEMU suite would have carried the fix and never
/// once exercised it.
///
/// With this spin both threads are on a CPU when the last one arrives, and the
/// interleaving becomes near-deterministic. **Attempts to the first collision, measured
/// over 460 runs of the two built test binaries** (2026-08-01): 38 cores, n=100 (QEMU) and
/// n=60 (KVM) — median 1, p90 1, max 1; `taskset -c 0,1`, n=60 (QEMU) and n=40 (KVM) —
/// median 1, p90 1, max 2. Whole-binary runs with all three tests concurrent: 60/60 and
/// 40/40 green per port, on 38 cores and on 2. Zero failures in 460.
///
/// ⊘ The loop is still a loop, and the deadline still a backstop. Near-deterministic is
/// not deterministic, and a fixed attempt count would be a flake generator the first time
/// a box behaves differently — the retry exits on the LEDGER, never on a count.
///
/// A thread that never arrives spins forever, which is what the watchdog is for.
fn rendezvous(gate: &AtomicUsize) {
    gate.fetch_add(1, Ordering::SeqCst);
    while gate.load(Ordering::SeqCst) < 2 {
        core::hint::spin_loop();
    }
}

/// Where each attempt's throwaway reservation goes: inside BAR1, clear of the realize-time
/// one at [`BAR1_BASE`] and clear of the decoy.
fn scratch_gpa() -> u64 {
    BAR1_BASE + 128 * page()
}

/// ★★ **The decoy reservation** — an instrument, not scenery. Without a second live
/// reservation, *"an entry is present"* and *"**this** entry is present"* are the same
/// sentence in every run, and a release that reached `windows.values_mut().next()` rather
/// than its own region would satisfy every assertion below.
fn decoy_gpa() -> u64 {
    BAR1_BASE + 64 * page()
}

/// ★★★ **Two planners racing for the same range: exactly one gets it, and the loser is
/// stopped at PLAN — before its `MAP_FIXED`, not after it.**
#[test]
fn two_plans_racing_for_one_range_do_not_both_get_it() {
    let _wd = watchdog(
        "qemu two_plans_racing_for_one_range_do_not_both_get_it",
        WEDGE_LIMIT,
    );
    let (m, _host, _slots) = machine();
    let p = page();
    let backing = m.register_backing(p).expect("a host backing");
    let decoy = m
        .install_ram_window(decoy_gpa(), 16 * p)
        .expect("the decoy reservation");

    let deadline = Instant::now() + RACE_DEADLINE;
    let mut attempts = 0u64;
    let (refused, made_delta) = loop {
        assert!(
            Instant::now() < deadline,
            "★ {attempts} attempts in {RACE_DEADLINE:?} and two plans were never observed \
             to collide. Either the plan no longer CLAIMS its range — so the second planner \
             sees an empty reservation and the race is silently back — or `plan_conflicts` \
             stopped being bumped. FAIL here rather than spin: a worker thread's own panic \
             message sits in libtest's capture buffer until this test ENDS"
        );
        attempts += 1;

        let region = m
            .install_ram_window(scratch_gpa(), 16 * p)
            .expect("a fresh reservation per attempt");
        // ★ A NEIGHBOUR, committed, at a disjoint offset in the SAME reservation — the
        // decoy for the range predicate itself. A check that ignored offsets would refuse
        // BOTH racers, and "exactly one Ok" is what sees that.
        let neighbour = m
            .vmm()
            .map_guest(scratch_gpa() + 8 * p, p, backing, Prot::ReadWrite)
            .expect("a committed neighbour, disjoint from the raced range");
        let before = m.audit();

        let gate = AtomicUsize::new(0);
        let (a, b) = std::thread::scope(|s| {
            let one = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(scratch_gpa(), p, backing, Prot::ReadWrite)
            });
            let two = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(scratch_gpa(), p, backing, Prot::ReadWrite)
            });
            (
                one.join().expect("planner one"),
                two.join().expect("planner two"),
            )
        });

        let after = m.audit();
        let collided = after.plan_conflicts > before.plan_conflicts;
        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "★ EXACTLY ONE of two publications of the SAME range may succeed, collision or \
             not. Got {a:?} and {b:?} on attempt {attempts}. Two `Ok`s is #145 itself — two \
             `MAP_FIXED`s over one host range, the second silently overwriting the first. \
             Two `Err`s means the check stopped being about ranges, and the committed \
             neighbour at +8 pages is what it would have tripped over"
        );
        assert_eq!(
            after.live_plan_reservations, 0,
            "no claim may outlive its planner — a leaked one is a permanent denial of that \
             range, which is worse than the race it closes"
        );
        assert_eq!(
            after.plan_reservations_abandoned, before.plan_reservations_abandoned,
            "★ and a COMMITTED plan is not an abandoned one. The winner's claim must be \
             turned into its placement by the commit, in one lock hold; a commit that left \
             it for the destructor to sweep up would work by accident, double-book the \
             range for an instant, and make `plan_reservations_abandoned` mean nothing"
        );
        m.remove_window(region).expect("this attempt's reservation");
        let _ = neighbour;
        if collided {
            break (
                if a.is_err() { a } else { b },
                after.placements_made - before.placements_made,
            );
        }
    };

    assert_eq!(
        refused,
        Err(VmmError::Unsupported(OVERLAP)),
        "★ the refused publication must come back as the refusal the caller would have got \
         had it asked a moment later — the same words a committed overlap gets, not a \
         bespoke error that leaks our timing into the guest's vocabulary"
    );
    assert_eq!(
        made_delta, 1,
        "★★★ THE DEFECT, AS A NUMBER. `placements_made` counts placements that actually \
         ran. On the colliding attempt exactly ONE may have run: the loser was refused at \
         PLAN, before it touched the mapping. Before #145 this was 2"
    );

    let a = m.audit();
    assert_eq!(
        a.live_placements, 0,
        "★ CONSERVATION: the decoy is still empty"
    );
    assert_eq!(a.live_plan_reservations, 0, "and no claim is outstanding");
    assert_eq!(
        a.syscall_ranked_depth.1, 0,
        "R1 still holds across the race: no syscall-shaped method ran under a ranked lock"
    );
    m.remove_window(decoy).expect("the decoy comes down last");
}

/// ★★★ **Two planners racing for DISJOINT ranges in one live reservation both succeed** —
/// the anti-livelock witness, and the reason this fix is a range claim and not a version.
///
/// `peak_plan_reservations >= 2` is the retry condition precisely because it is the fact a
/// serialising cure makes *unreachable* rather than merely rare: it says two claims were
/// held at the same instant.
#[test]
fn two_disjoint_plans_into_one_live_window_proceed_together() {
    let _wd = watchdog(
        "qemu two_disjoint_plans_into_one_live_window_proceed_together",
        WEDGE_LIMIT,
    );
    let (m, _host, _slots) = machine();
    let p = page();
    let backing = m.register_backing(p).expect("a host backing");

    let deadline = Instant::now() + RACE_DEADLINE;
    let mut attempts = 0u64;
    loop {
        assert!(
            Instant::now() < deadline,
            "★ {attempts} attempts in {RACE_DEADLINE:?} and two claims were NEVER held at \
             once. That is what a serialising cure looks like from out here — a version \
             counter, or a claim held across the execute phase under the installer lock. \
             Disjoint publications into a live reservation must not wait for each other"
        );
        attempts += 1;

        let region = m
            .install_ram_window(scratch_gpa(), 16 * p)
            .expect("a fresh reservation per attempt");
        let before = m.audit();

        // ★ ADJACENT, not merely disjoint: `[0, p)` and `[p, 2p)` touch at exactly one
        // boundary, so an overlap predicate that used `<=` where it uses `<` refuses them
        // and this test goes red. A wide gap would hide that.
        let gate = AtomicUsize::new(0);
        let (a, b) = std::thread::scope(|s| {
            let one = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(scratch_gpa(), p, backing, Prot::ReadWrite)
            });
            let two = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(scratch_gpa() + p, p, backing, Prot::ReadWrite)
            });
            (
                one.join().expect("planner one"),
                two.join().expect("planner two"),
            )
        });

        let after = m.audit();
        assert!(
            a.is_ok() && b.is_ok(),
            "★ two ADJACENT, non-overlapping publications into one live reservation must \
             BOTH succeed. Got {a:?} and {b:?}. A refusal here is the livelock a version \
             counter would have caused — a cure that refuses concurrent publication into a \
             perfectly live reservation"
        );
        assert_eq!(
            after.plan_conflicts, before.plan_conflicts,
            "and neither may be scored a conflict: the ranges do not overlap"
        );
        assert_eq!(
            after.live_placements, 2,
            "both placements are live in the same reservation at once"
        );
        assert_eq!(
            after.live_plan_reservations, 0,
            "and no claim is outstanding"
        );
        assert_eq!(
            after.plan_reservations_abandoned, before.plan_reservations_abandoned,
            "★ neither plan was abandoned: both committed, and a commit turns its own claim \
             into its placement rather than leaving it for the destructor"
        );
        let concurrent = after.peak_plan_reservations >= 2;
        m.remove_window(region).expect("this attempt's reservation");
        if concurrent {
            break;
        }
    }

    let a = m.audit();
    assert!(
        a.peak_plan_reservations >= 2,
        "★★ NON-VACUITY: the loop above may only exit having seen two claims held at the \
         same instant. Anything less and every attempt was really sequential, and 'both \
         succeeded' says nothing about concurrency"
    );
    assert_eq!(
        a.syscall_ranked_depth.1, 0,
        "R1 still holds: no syscall-shaped method ran under a ranked lock"
    );
}

/// ★★★ **A plan that never reaches commit gives its range back** — the leak half, and the
/// reason the claim lives in a destructor rather than in three matching cleanup edits.
///
/// No race is needed and none is used: this is about the exits *between* the claim and the
/// commit, and a sequential call reaches one of them exactly.
#[test]
fn a_plan_that_dies_before_commit_releases_its_range() {
    let (m, _host, _slots) = machine();
    let p = page();
    let backing = m.register_backing(p).expect("a host backing");
    let decoy = m
        .install_ram_window(decoy_gpa(), 16 * p)
        .expect("the decoy reservation");
    let region = m
        .install_ram_window(scratch_gpa(), 16 * p)
        .expect("the reservation");

    let mut v = m.vmm();
    // ★ A committed placement in the decoy AND one in our own reservation, both at offset
    // 0. They are what a destructor reaching into the wrong map is caught by: a release
    // that removed from `placements` rather than `planned`, or from the first reservation
    // rather than its own, leaves the claim behind — and the re-map below is then refused.
    v.map_guest(decoy_gpa(), p, backing, Prot::ReadWrite)
        .expect("the decoy's placement");
    v.map_guest(scratch_gpa(), p, backing, Prot::ReadWrite)
        .expect("our reservation's committed placement");
    let before = m.audit();

    // The claim is made, and then the execute phase refuses: a backing id this backend
    // never minted is the first exit after the claim.
    assert_eq!(
        v.map_guest(
            scratch_gpa() + p,
            p,
            HostRegion {
                id: 9_999,
                offset: 0
            },
            Prot::ReadWrite
        ),
        Err(VmmError::Unsupported(
            "a host backing id this backend never minted"
        )),
        "the execute phase refuses — after the plan has already claimed the range"
    );

    let after = m.audit();
    assert_eq!(
        after.plan_reservations_abandoned,
        before.plan_reservations_abandoned + 1,
        "★★ NON-VACUITY, and the half that says the RELEASE PATH RAN. \
         `live_plan_reservations == 0` below is satisfied identically by a plan that never \
         claimed anything at all, so without this the leak assertion is about nothing"
    );
    assert_eq!(
        after.live_plan_reservations, 0,
        "the claim is gone, not merely uncounted"
    );
    assert_eq!(
        after.placements_made, before.placements_made,
        "and no placement ran: the refusal came before it"
    );

    // ★★★ THE LEAK ASSERTION, behavioural rather than a counter: the range is usable
    // again. A reservation that outlived its planner would refuse this forever, and a
    // permanent denial of a guest-physical range is strictly worse than the race the claim
    // exists to close — the race needs two threads, the leak needs one.
    let slot = v
        .map_guest(scratch_gpa() + p, p, backing, Prot::ReadWrite)
        .expect(
            "★ the range the failed plan claimed must be usable again. A refusal here is a \
             LEAKED RESERVATION — the range is denied for the lifetime of the reservation",
        );
    assert_eq!(
        v.map_guest(scratch_gpa(), p, backing, Prot::ReadWrite),
        Err(VmmError::Unsupported(OVERLAP)),
        "★ our own reservation's committed placement survived the release — a destructor \
         that removed from `placements` would have freed it"
    );
    assert_eq!(
        v.map_guest(decoy_gpa(), p, backing, Prot::ReadWrite),
        Err(VmmError::Unsupported(OVERLAP)),
        "★ and so did the DECOY's — a destructor that reached the first reservation rather \
         than its own would have freed that one instead, and nothing else in this file \
         would have noticed"
    );

    v.unmap_guest(slot).expect("the re-made placement");
    m.remove_window(region).expect("the reservation");
    m.remove_window(decoy).expect("the decoy");
    let end = m.audit();
    assert_eq!(end.live_plan_reservations, 0);
    assert_eq!(
        end.syscall_ranked_depth.1, 0,
        "R1: no syscall-shaped method ran under a ranked lock"
    );
}
