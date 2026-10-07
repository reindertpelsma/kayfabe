//! ★★★ **The plan→commit overlap race, made falsifiable** — issue #145.
//!
//! Three properties of [`Vmm::map_guest`]'s claim, and they pull against each other on
//! purpose:
//!
//! 1. two planners racing for the **same** range: one gets it, the other is refused, and
//!    the loser's `MAP_FIXED` never runs;
//! 2. two planners racing for **disjoint** ranges in the same live window: **both** get
//!    them, genuinely concurrently — the property a generation counter would have
//!    destroyed;
//! 3. a plan that never reaches commit gives its range **back**.
//!
//! ## What was wrong
//!
//! The overlap check ran only at plan time, against `Window::placements`, inside the
//! installer guard — and that guard is released before the commit, which is the whole
//! reason R5 re-validation exists. Nothing re-checked overlap. So two threads could plan
//! overlapping placements into the same LIVE window, both `MAP_FIXED`, the second silently
//! overwriting the first, and both commit successfully. No refusal, no counter, no log.
//!
//! ## ★★ Why the obvious instruments cannot see it
//!
//! - **R5's presence token cannot.** R5 asks whether the window is still *there*; here it
//!   is perfectly alive. Presence is the right token for teardown and says nothing about
//!   this. `r5_revalidation.rs` stays exactly as it was.
//! - **A placement-versioned generation counter WOULD — and is still the wrong
//!   instrument.** #89 established it: bumping a version on every placement refuses
//!   concurrent publications into a live window *at publication frequency*, converting a
//!   real hazard into a livelock. Test 2 below is what keeps that cure from being adopted
//!   by accident — it fails if disjoint publications ever start serialising.
//!
//! ## ★★★ The tests RACE, they do not sequence
//!
//! A sequential pair of calls proves nothing here: sequentially the first placement has
//! already moved into `placements`, so the second is refused by the check that was always
//! there. Both racing tests therefore start their threads on a spin rendezvous — see
//! [`rendezvous`], where a `Barrier` was measured to make one port's race unreachable —
//! and **retry until the ledger confirms the interleaving was actually reached** — `plan_conflicts` for
//! test 1, `peak_plan_reservations >= 2` for test 2 — so they can only pass by observing
//! the thing they are about.
//!
//!
//! ## ★★ The bites, watched fail (2026-08-01, `crates/kayfabe-vmm-kvm/src/lib.rs`)
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
//! The sibling `kayfabe-vmm-qemu/tests/plan_reservation.rs` was bitten with the same six.
//! ## ★ Its own binary
//!
//! Same reason `r5_revalidation.rs` and `window_retirement.rs` are: cargo runs
//! integration-test **targets** one at a time, and these deliberately lose races in a
//! loop. `memory_plane.rs` asserts a process-wide `VmSize`, which retries on another
//! thread of the same process would perturb.

use core::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_mocks::watchdog::watchdog;
use kayfabe_vmm::{BarId, HostRegion, Prot, Vmm, VmmError};
use kayfabe_vmm_kvm::{BarPlacement, KvmMachine, MachineConfig};

const GPA_RAM: u64 = 0x1000_0000;
/// ★★ **A second window that is never touched** — an instrument, not scenery, and the
/// same one `r5_revalidation.rs` needs for the same reason: without it, *"an entry is
/// present"* and *"**this** entry is present"* are the same sentence in every run. It is
/// installed first, so it holds the lowest region id and is what a
/// `windows.values_mut().next()` mutant reaches for.
const GPA_DECOY: u64 = 0x2000_0000;
const GPA_BAR0: u64 = 0x7000_0000;
const BAR0_LEN: u64 = 0x1_0000;

/// The overlap refusal — **the same text a committed overlap gets**, deliberately. The
/// caller asked for a range somebody else has; that the other holder is a microsecond from
/// committing rather than already committed is a timing fact about us, not a fact about
/// the request. `plan_conflicts` is what tells the two apart from in here.
const OVERLAP: &str = "a placement overlapping one already live in the same window";

/// ★ A backstop on the retry loops — **not** the instrument. The instrument is the ledger
/// moving; this bounds only the case where it never moves at all, which is what a neutered
/// claim looks like from out here. An attempt costs well under a millisecond.
const RACE_DEADLINE: Duration = Duration::from_secs(60);

/// ★★ And a watchdog on top of the deadline, because the deadline cannot see the failure
/// mode this defect class actually has: a claim released under a lock that is already held
/// **self-deadlocks** on a non-reentrant `Mutex`, and a deadlocked worker thread never
/// returns to the loop that would check `Instant::now()`. A hang says nothing; the
/// watchdog turns it into a named abort with every thread's wait channel — and it writes
/// to a real fd, because libtest's capture swallows a watchdog's `eprintln!` and dies with
/// the abort it exists to explain.
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

fn page() -> HostPageSize {
    HostPageSize::query()
}

/// The same machine shape the sibling test files use, so all four describe one device.
fn machine() -> KvmMachine {
    KvmMachine::realize(MachineConfig {
        shareable_ram: true,
        bars: vec![BarPlacement {
            bar: BarId::Bar0,
            base: GPA_BAR0,
            len: BAR0_LEN,
        }],
    })
    .expect(
        "/dev/kvm must be present and permitted for the KVM-direct harness (§10, decision \
         #48) — a deployment fact no code gate can observe, so it refuses loudly here",
    )
}

/// ★★★ **Two planners racing for the same range: exactly one gets it, and the loser is
/// stopped at PLAN — before its `MAP_FIXED`, not after it.**
#[test]
fn two_plans_racing_for_one_range_do_not_both_get_it() {
    kayfabe_linux_raw::require_kvm!("two_plans_racing_for_one_range_do_not_both_get_it");
    let _wd = watchdog(
        "two_plans_racing_for_one_range_do_not_both_get_it",
        WEDGE_LIMIT,
    );
    let m = machine();
    let p = page().bytes();
    let backing = m.register_backing(p).expect("a host backing");
    let decoy = m
        .install_ram_window(GPA_DECOY, 16 * p)
        .expect("the decoy window installs first, so it holds the lowest region id");

    let deadline = Instant::now() + RACE_DEADLINE;
    let mut attempts = 0u64;
    let (refused, made_delta) = loop {
        assert!(
            Instant::now() < deadline,
            "★ {attempts} attempts in {RACE_DEADLINE:?} and two plans were never observed \
             to collide. Either the plan no longer CLAIMS its range (so the second planner \
             sees an empty window and the race is silently back), or `plan_conflicts` \
             stopped being bumped. FAIL here rather than spin: the worker threads' own \
             panic messages sit in libtest's capture buffer until this test ENDS"
        );
        attempts += 1;

        let region = m
            .install_ram_window(GPA_RAM, 16 * p)
            .expect("a fresh window per attempt");
        // ★ A NEIGHBOUR, committed, at a disjoint offset in the SAME window — the decoy
        // for the range predicate itself. A `hits` that ignored offsets (`!is_empty()`,
        // or a predicate that matched everything) would refuse BOTH racers, and "exactly
        // one Ok" below is what sees that. Without it, an offset-blind check passes.
        let neighbour = m
            .vmm()
            .map_guest(GPA_RAM + 8 * p, p, backing, Prot::ReadWrite)
            .expect("a committed neighbour, disjoint from the raced range");
        let before = m.audit();

        let gate = AtomicUsize::new(0);
        let (a, b) = std::thread::scope(|s| {
            let one = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(GPA_RAM, p, backing, Prot::ReadWrite)
            });
            let two = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(GPA_RAM, p, backing, Prot::ReadWrite)
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
             not. Got {a:?} and {b:?} on attempt {attempts}. Two `Ok`s is #145 itself — \
             two `MAP_FIXED`s over one host range, the second silently overwriting the \
             first. Two `Err`s means the check stopped being about ranges, and the \
             committed neighbour at +8 pages is what it would have tripped over"
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
        // Attempts that did not collide are just an unlucky interleaving; drop the window
        // (which takes both placements with it) and try again.
        m.remove_window(region).expect("this attempt's window");
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
        "★★★ THE DEFECT, AS A NUMBER. `placements_made` counts `MAP_FIXED`s that actually \
         ran. On the colliding attempt exactly ONE may have run: the loser was refused at \
         PLAN, before the syscall. Before #145 this was 2 — both planners placed over the \
         same host range and both committed"
    );

    let a = m.audit();
    assert_eq!(
        a.live_windows, 1,
        "every attempt's window was removed; the decoy is the survivor"
    );
    assert_eq!(
        a.live_placements, 0,
        "★ CONSERVATION: the decoy is still live and still empty, so nothing was parked in \
         somebody else's window"
    );
    assert_eq!(a.live_plan_reservations, 0, "and no claim is outstanding");
    assert_eq!(
        a.syscall_ranked_depth.1, 0,
        "R1 still holds across the race: no syscall-shaped method ran under a ranked lock"
    );
    m.remove_window(decoy).expect("the decoy comes down last");
}

/// ★★★ **Two planners racing for DISJOINT ranges in one live window both succeed** — the
/// anti-livelock witness, and the reason this fix is a range claim and not a version.
///
/// A generation counter bumped on every placement would refuse one of these two at
/// publication frequency. `peak_plan_reservations >= 2` is the retry condition precisely
/// because it is the fact that would stop being true: it says two claims were held at the
/// same instant, which a serialising cure makes unreachable rather than merely rare.
#[test]
fn two_disjoint_plans_into_one_live_window_proceed_together() {
    kayfabe_linux_raw::require_kvm!("two_disjoint_plans_into_one_live_window_proceed_together");
    let _wd = watchdog(
        "two_disjoint_plans_into_one_live_window_proceed_together",
        WEDGE_LIMIT,
    );
    let m = machine();
    let p = page().bytes();
    let backing = m.register_backing(p).expect("a host backing");

    let deadline = Instant::now() + RACE_DEADLINE;
    let mut attempts = 0u64;
    loop {
        assert!(
            Instant::now() < deadline,
            "★ {attempts} attempts in {RACE_DEADLINE:?} and two claims were NEVER held at \
             once. That is what a serialising cure looks like from out here — a version \
             counter, or a claim held across the execute phase's syscall under the \
             installer lock. Disjoint publications into a live window must not wait for \
             each other"
        );
        attempts += 1;

        let region = m
            .install_ram_window(GPA_RAM, 16 * p)
            .expect("a fresh window per attempt");
        let before = m.audit();

        let gate = AtomicUsize::new(0);
        // ★ ADJACENT, not merely disjoint: `[0, p)` and `[p, 2p)` touch at exactly one
        // boundary, so an overlap predicate that used `<=` where it uses `<` refuses them
        // and this test goes red. A wide gap would hide that.
        let (a, b) = std::thread::scope(|s| {
            let one = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(GPA_RAM, p, backing, Prot::ReadWrite)
            });
            let two = s.spawn(|| {
                let mut v = m.vmm();
                rendezvous(&gate);
                v.map_guest(GPA_RAM + p, p, backing, Prot::ReadWrite)
            });
            (
                one.join().expect("planner one"),
                two.join().expect("planner two"),
            )
        });

        let after = m.audit();
        assert!(
            a.is_ok() && b.is_ok(),
            "★ two ADJACENT, non-overlapping publications into one live window must BOTH \
             succeed. Got {a:?} and {b:?}. A refusal here is the livelock #89 named — a \
             cure that refuses concurrent publication into a perfectly live window"
        );
        assert_eq!(
            after.plan_conflicts, before.plan_conflicts,
            "and neither may be scored a conflict: the ranges do not overlap"
        );
        assert_eq!(
            after.live_placements, 2,
            "both placements are live in the same window at once"
        );
        assert_eq!(
            after.live_plan_reservations, 0,
            "and no claim is outstanding"
        );
        assert_eq!(
            after.plan_reservations_abandoned, before.plan_reservations_abandoned,
            "★ neither plan was abandoned: both committed, and a commit turns its own \
             claim into its placement rather than leaving it for the destructor"
        );
        let concurrent = after.peak_plan_reservations >= 2;
        m.remove_window(region).expect("this attempt's window");
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
    assert_eq!(a.live_windows, 0, "every attempt's window was removed");
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
    kayfabe_linux_raw::require_kvm!("a_plan_that_dies_before_commit_releases_its_range");
    let m = machine();
    let p = page().bytes();
    let backing = m.register_backing(p).expect("a host backing");
    let decoy = m
        .install_ram_window(GPA_DECOY, 16 * p)
        .expect("the decoy window, first and lowest");
    let region = m.install_ram_window(GPA_RAM, 16 * p).expect("the window");

    let mut v = m.vmm();
    // ★ A committed placement in the decoy AND one in our own window, both at offset 0.
    // They are what a destructor reaching into the wrong map is caught by: a release that
    // removed from `placements` rather than `planned`, or from `windows.values_mut()
    // .next()` rather than from its own region, leaves the claim behind — and the re-map
    // below is then refused, which is the red.
    v.map_guest(GPA_DECOY, p, backing, Prot::ReadWrite)
        .expect("the decoy's placement");
    v.map_guest(GPA_RAM, p, backing, Prot::ReadWrite)
        .expect("our window's committed placement");
    let before = m.audit();

    // The claim is made, and then the execute phase refuses: a backing id this backend
    // never minted is the first exit after the claim.
    assert_eq!(
        v.map_guest(
            GPA_RAM + p,
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
        "and no `MAP_FIXED` ran: the refusal came before the syscall"
    );

    // ★★★ THE LEAK ASSERTION, and it is behavioural rather than a counter: the range is
    // usable again. A reservation that outlived its planner would refuse this forever, and
    // a permanent denial of a guest-physical range is strictly worse than the race the
    // claim exists to close — the race needs two threads, the leak needs one.
    let slot = v
        .map_guest(GPA_RAM + p, p, backing, Prot::ReadWrite)
        .expect(
            "★ the range the failed plan claimed must be usable again. A refusal here is a \
             LEAKED RESERVATION — the range is denied for the lifetime of the window",
        );
    // …and the two committed placements were not collateral damage.
    assert_eq!(
        v.map_guest(GPA_RAM, p, backing, Prot::ReadWrite),
        Err(VmmError::Unsupported(OVERLAP)),
        "★ our own window's committed placement survived the release — a destructor that \
         removed from `placements` would have freed it"
    );
    assert_eq!(
        v.map_guest(GPA_DECOY, p, backing, Prot::ReadWrite),
        Err(VmmError::Unsupported(OVERLAP)),
        "★ and so did the DECOY's — a destructor that reached the first window rather than \
         its own would have freed that one instead, and nothing else in this file would \
         have noticed"
    );

    v.unmap_guest(slot).expect("the re-made placement");
    m.remove_window(region).expect("the window");
    m.remove_window(decoy).expect("the decoy");
    let end = m.audit();
    assert_eq!(end.live_windows, 0);
    assert_eq!(end.live_plan_reservations, 0);
    assert_eq!(
        end.syscall_ranked_depth.1, 0,
        "R1: no syscall-shaped method ran under a ranked lock"
    );
}
