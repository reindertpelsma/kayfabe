//! ★★★ **R5's re-validation, made falsifiable** — `l1_concurrency.md` §3.3 R5, issue #89.
//!
//! One property: *a `map_guest` whose window was torn down between its PLAN and its COMMIT
//! must be refused, must say so in the ledger, and must leave nothing behind.*
//!
//! ## Why this file exists
//!
//! The commit phase used to read `Some(w) if w.generation == generation` — a `u64` stamped
//! into each window at install from an installer-wide counter that `remove_window` bumped.
//! It reads as an ABA defence. It is not one, and it **could not fail**: the key the commit
//! re-looks-up is a `RamRegionId` minted from a strictly monotonic cursor and never
//! recycled, so a present window is necessarily *the same* window the plan read, and its
//! stamp is necessarily the stamp the plan copied out of it. The guard compared a constant
//! against itself, and deleting it turned nothing red — the definition of a survivor.
//!
//! So the guard was replaced by the token that can actually differ, the window's
//! **presence**, and this file is what makes that a claim anybody can refute. Both halves
//! were watched go red on 2026-08-01 by planting them into
//! `crates/kayfabe-vmm-kvm/src/lib.rs`'s commit block:
//!
//! - the refusal arm returning `Ok(slot_id)` instead of `Err` — the loop below never
//!   observes an R5 failure and the deadline fires;
//! - the refusal arm crediting `live_placements` — the conservation assertion fires.
//!
//! ## ★ Why the interleaving is a race and not a rendezvous
//!
//! `window_retirement.rs` can synchronise on a clock-free edge because `accesses_served` is
//! bumped *between* the two instants it cares about. There is no such edge inside
//! `map_guest`: the gap this test must land in is plan-drops-the-lock → commit-retakes-it,
//! and nothing observable happens in it by construction (that is the whole point of the
//! phase — every lock is dropped). So the two threads are started together and the attempt
//! is **retried until the ledger says the interleaving was reached**, which is a loop on an
//! observation rather than on a hope: the test can only pass by seeing R5 fire.
//!
//! The odds are structural rather than lucky. Both threads' first act is to take the
//! installer lock. When the mapper wins it, the remover is *already queued behind it* and
//! takes the entry out the instant the plan drops the guard — microseconds before the
//! mapper returns from its `MAP_FIXED`, so the commit finds nothing. When the remover wins,
//! the mapper's plan finds no window and refuses early, and that attempt is retried.
//!
//! ★ **The retry budget, measured** (2026-08-01, this file, 900 runs of the built test
//! binary in a loop, `taskset`-pinned to vary the contention because a big box hides a
//! race): 900/900 green. Attempts to first R5 — 38 cores, n=500: median 42, p90 235, max
//! 1174. 4 cores, n=200: median 222, p90 576, max 1522. 2 cores, n=200: median 21, p90 353,
//! max 2387. An attempt costs about 50 µs (a window install, a race, a teardown), so the
//! worst run observed spent well under a second inside a deadline of a minute. The tail is
//! the reason there is a loop at all: a fixed attempt count would be a flake generator, and
//! a single unretried attempt would pass **vacuously** about nine times in ten.
//!
//! ## ★ Why its own binary
//!
//! Same reason `window_retirement.rs` is: cargo runs integration-test **targets** one at a
//! time, and this one deliberately loses races in a loop. Sharing a binary with
//! `memory_plane.rs` would put its retries on a thread of the same process as that file's
//! process-wide `VmSize` assertion.

use core::time::Duration;
use std::sync::Barrier;
use std::time::Instant;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_vmm::{BarId, Prot, Vmm, VmmError};
use kayfabe_vmm_kvm::{BarPlacement, KvmMachine, MachineConfig};

const GPA_RAM: u64 = 0x1000_0000;
/// ★★ **A second, innocent window that is never torn down** — and it is an instrument, not
/// scenery. Without it "the entry is present" and "*this* entry is present" are the same
/// sentence in every run, so a commit that re-looked-up the wrong key (`values_mut().next()`
/// rather than `get_mut(&region)`) would satisfy the test. With a decoy sitting at the
/// lowest region id, that mutant commits the refused placement into a live window belonging
/// to somebody else — and the assertions below see an `Ok` and a credited placement.
const GPA_DECOY: u64 = 0x2000_0000;
const GPA_BAR0: u64 = 0x7000_0000;
const BAR0_LEN: u64 = 0x1_0000;

/// ★ **A backstop on the retry loop — not the instrument.** The instrument is
/// `r5_revalidation_failures` moving; this only bounds the case where it never moves *at
/// all*, which is what a neutered refusal arm looks like from out here. Without it that
/// mutant is an infinite loop with libtest's capture swallowing every word, and a hang says
/// nothing while a red test says something.
///
/// It cannot make the test flaky: the interleaving is reached in one or two attempts and an
/// attempt costs well under a millisecond, so a minute is several orders of magnitude of
/// slack. Exceeding it is a real defect, not a slow box.
const RACE_DEADLINE: Duration = Duration::from_secs(60);

fn page() -> HostPageSize {
    HostPageSize::query()
}

/// The same machine shape the sibling test files use, so all three describe one device.
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

/// ★★★ **A teardown that lands in the plan→commit gap is refused at commit**, by the name
/// the guest would have got had it asked a moment later, and the refused publication leaves
/// no placement in the ledger and no bookkeeping in the plane.
#[test]
fn a_teardown_racing_a_publication_is_refused_at_commit_and_leaves_no_trace() {
    kayfabe_linux_raw::require_kvm!(
        "a_teardown_racing_a_publication_is_refused_at_commit_and_leaves_no_trace"
    );
    let m = machine();
    let p = page();
    let backing = m.register_backing(p.bytes()).expect("a host backing");
    let decoy = m
        .install_ram_window(GPA_DECOY, 16 * p.bytes())
        .expect("the decoy window installs first, so it holds the lowest region id");

    let deadline = Instant::now() + RACE_DEADLINE;
    let mut attempts = 0u64;
    let mut plans_lost = 0u64;
    let refused = loop {
        assert!(
            Instant::now() < deadline,
            "★ {attempts} attempts in {RACE_DEADLINE:?} and the plan→commit gap was never \
             observed to refuse. Either the commit no longer re-validates (the R5 arm was \
             removed or now returns success), or `r5_revalidation_failures` stopped being \
             bumped. FAIL here rather than spin: the worker threads' own panic messages sit \
             in libtest's capture buffer until this test ENDS"
        );
        attempts += 1;

        let region = m
            .install_ram_window(GPA_RAM, 16 * p.bytes())
            .expect("a fresh window per attempt");
        let before = m.audit();

        // Both threads' first act is to take the installer lock; the barrier is what makes
        // which one gets it a genuine coin flip rather than a foregone conclusion.
        let gate = Barrier::new(2);
        let (mapped, removed) = std::thread::scope(|s| {
            let mapper = s.spawn(|| {
                let mut v = m.vmm();
                gate.wait();
                v.map_guest(GPA_RAM, p.bytes(), backing, Prot::ReadWrite)
            });
            let remover = s.spawn(|| {
                gate.wait();
                m.remove_window(region)
            });
            (
                mapper.join().expect("the mapper thread"),
                remover.join().expect("the remover thread"),
            )
        });
        removed.expect(
            "the teardown always wins the entry: it is the only thread that removes, and \
             the window existed when this attempt began",
        );

        let after = m.audit();
        if after.r5_revalidation_failures > before.r5_revalidation_failures {
            // ★ NON-VACUITY, and the reason the counter alone is not enough. A refusal from
            // the PLAN phase — the remover winning the lock outright — returns the *same*
            // `BadGpa` and would be indistinguishable from out here. The `MAP_FIXED` having
            // already run is what says the refusal came from the commit, i.e. that the
            // syscall this whole plan/execute/commit shape exists for was performed and
            // then disowned.
            assert!(
                after.placements_made > before.placements_made,
                "the R5 counter moved but no placement was ever made — the refusal did not \
                 come from the commit phase, so this test observed something else"
            );
            break mapped;
        }
        if mapped.is_err() {
            plans_lost += 1;
        }
    };

    assert!(
        matches!(refused, Err(VmmError::BadGpa { gpa }) if gpa == GPA_RAM),
        "★ the refused publication must come back as the refusal the guest would have got \
         had it asked a moment later — not a bespoke error, and emphatically not `Ok`. \
         Got {refused:?} after {attempts} attempts ({plans_lost} lost in the plan phase)"
    );

    let a = m.audit();
    assert_eq!(
        a.live_placements, 0,
        "★ CONSERVATION, and the half that catches a commit re-looking-up the WRONG key. A \
         refusal must not have credited the ledger it declined to join — and must not have \
         parked its placement in the decoy, which is still live and still empty"
    );
    assert_eq!(
        a.live_windows, 1,
        "every attempt's window was removed by its remover thread; the decoy is the survivor"
    );
    m.remove_window(decoy).expect("the decoy comes down last");
    assert_eq!(
        m.audit().live_windows,
        0,
        "and nothing else was left installed"
    );
    assert!(
        a.peak_placements <= 1,
        "one publication per attempt, and each attempt's window is gone before the next"
    );
    assert_eq!(
        a.syscall_ranked_depth.1, 0,
        "R1 still holds across the race: no syscall-shaped method ran under a ranked lock"
    );
}
