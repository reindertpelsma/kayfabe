//! ★★★★★ **How much of BAR0 is pages no register lives in (w544).**
//!
//! Owner: *"Most of bar0 is constant registers or mappable from userspace except the doorbell
//! write."* `[measured w542]` the read census agrees and sharpens it:
//!
//! ```text
//! BAR0-READS total=241722  unclaimed=124415  rom=4632  cpu_intr=16944  gsp=645  ptimer=138
//! window[SERVED r=24 w=69730 moves=42]
//! ```
//!
//! **Every one of those 124 415 unclaimed reads returns `0`** — `ReadOutcome::Unclaimed => 0`.
//! So a read-only zero page over the pages that hold no register is **byte-identical** to what
//! this device answers today, and removes the exit.
//!
//! ⊘ The unit has to be the PAGE, not the register: "unclaimed" is not a range, it is whatever
//! no arm claims, scattered across 16 MiB. A page is backable exactly when **no offset in it**
//! is served by any arm — so one live register poisons its whole 4 KiB.
//!
//! ⚠ This file exists to produce a NUMBER before any plumbing is built. A mapping that covered
//! 3 % of the surface and a mapping that covered 90 % are the same amount of work and very
//! different decisions.

use kayfabe_device::plane::{ReadOutcome, RegPlane};
use kayfabe_device::{NanoClock, SteppingClock, abi};

/// GA106's BAR0, and the page size a memslot is quantised to.
const BAR0_LEN: u64 = 0x0100_0000;
const PAGE: u64 = 4096;

fn plane() -> RegPlane {
    let p = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    p.set_fb(Box::new(kayfabe_device::fbwin::SparseFb::new(12288 << 20)));
    p
}

/// ★★★ **Sweep every dword of BAR0 and report what fraction of pages hold no register.**
///
/// ⊘ Asked through `RegPlane::read` — the REAL path, every arm in its real order — rather than
/// a predicate written beside it. A predicate would be fast and could drift; the point of this
/// measurement is to be the thing a fast predicate is later checked against.
#[test]
fn report_how_much_of_bar0_is_backable_by_a_zero_page() {
    let p = plane();
    let pages = BAR0_LEN / PAGE;
    let (mut backable, mut live) = (0u64, 0u64);
    let mut first_live: Option<u64> = None;

    for page in 0..pages {
        let base = page * PAGE;
        let mut any_claimed = false;
        for off in (base..base + PAGE).step_by(4) {
            if !matches!(p.read(0, off, 4), ReadOutcome::Unclaimed) {
                any_claimed = true;
                break;
            }
        }
        if any_claimed {
            live += 1;
            first_live.get_or_insert(base);
        } else {
            backable += 1;
        }
    }

    // ⊘ RUNS, not pages, are the implementation's real cost: one memslot covers a contiguous
    // run, and a sparse read-only memfd of BAR0's length answers every page in it with zeros
    // without allocating a byte — so the slot COUNT is what matters, not the page count.
    let mut runs = 0u64;
    let mut in_run = false;
    let mut longest = 0u64;
    let mut cur = 0u64;
    for page in 0..pages {
        let base = page * PAGE;
        let mut any_claimed = false;
        for off in (base..base + PAGE).step_by(4) {
            if !matches!(p.read(0, off, 4), ReadOutcome::Unclaimed) {
                any_claimed = true;
                break;
            }
        }
        if any_claimed {
            in_run = false;
            cur = 0;
        } else {
            if !in_run {
                runs += 1;
                in_run = true;
            }
            cur += 1;
            longest = longest.max(cur);
        }
    }

    eprintln!(
        "BAR0-BACKABLE pages={pages} backable={backable} live={live} \
         ({:.1}% backable) runs={runs} longest_run={longest} pages first_live={:#x}",
        100.0 * backable as f64 / pages as f64,
        first_live.unwrap_or(0)
    );

    // ⊘ NON-VACUITY, both directions. All-backable would mean the sweep never reached a
    // register and the number is a lie; none-backable would mean there is nothing to map and
    // the whole idea is dead. Only a mixture is a measurement.
    assert!(
        backable > 0,
        "no page of BAR0 is free of registers — then a zero page maps nothing and this \
         direction is dead"
    );
    assert!(
        live > 0,
        "EVERY page is free of registers — the sweep cannot have reached a single arm, so the \
         backable count is an artefact of a broken probe, not a property of the device"
    );
}


/// ★★★★★ **The production run-list must agree with the exhaustive sweep, and cost nothing.**
///
/// `RegPlane::bar0_backable_runs` is what a memslot placement will trust. This checks it
/// against the sweep above — the same arm chain, asked the slow way — and, separately, that
/// running it does **not** move the read census.
///
/// ⊘ That second half is not hygiene. The run-list sweeps four million offsets; if it went
/// through `RegPlane::read` instead of the classifier, every figure in `bar0_read_census`
/// would be poisoned before the guest issued one access, and the number I used to justify this
/// whole direction — `unclaimed=124415` — would have been measuring the sweep.
#[test]
fn the_production_run_list_matches_the_sweep_and_does_not_disturb_the_census() {
    let p = plane();

    let before = p.counters().reads;
    let runs = p.bar0_backable_runs();
    assert_eq!(
        p.counters().reads,
        before,
        "computing the run list moved the READ COUNTER — it is going through the counting          wrapper, and every number in `bar0_read_census` is then partly this sweep"
    );

    assert!(!runs.is_empty(), "no backable run at all — nothing to map");
    let covered: u64 = runs.iter().map(|(_, l)| *l).sum();
    eprintln!(
        "BAR0-RUNS n={} covering {covered} bytes ({} pages)",
        runs.len(),
        covered / PAGE
    );

    // ★ The load-bearing check: NO live register may fall inside a run. A run that swallowed
    // one would answer it with zero forever, silently, and the guest would see a defaulted
    // register rather than a refusal.
    for (start, len) in &runs {
        for off in (*start..*start + *len).step_by(4) {
            assert!(
                matches!(p.read(0, off, 4), ReadOutcome::Unclaimed),
                "offset {off:#x} is inside a backable run but this device SERVES it — mapping                  that run would answer a live register with zero, forever, with nothing logged"
            );
        }
    }

    // ⊘ And the converse as non-vacuity: the runs must not cover everything, or the check
    // above passed only because the device serves nothing.
    assert!(
        covered < BAR0_LEN,
        "the runs cover ALL of BAR0 — then no register was found anywhere and the check above          is vacuous"
    );
}

/// ★★★★★ **THE RUN LIST DOES NOT MOVE WHEN THE PLANE IS USED — w550.**
///
/// # What this is a regression test for, and how it was found
///
/// The sweep this file measures used to call `RegPlane::read_inner`, the real read path. Two
/// of its arms are **producers**: the GSP arm takes the plane lock and steps the boot state
/// machine, and the framebuffer-window arm materialises pages in the store. So sweeping the
/// aperture did not describe it — it drove it, and then described the wreckage.
///
/// `[measured w550, bench boot]` the device asks the plane for its run list **twice** at
/// realize and refuses if the two disagree. They disagreed: **4 runs, then 12**. That refusal
/// is the only reason this was ever seen, because a single sweep returns a plausible list
/// either way, and every test in this file swept a FRESH plane exactly once — precisely the
/// case where the mutation has not happened yet.
///
/// ⇒ So the check here is not "sweep twice". It is **sweep, then USE the plane hard, then
/// sweep again** — the shape a single boot actually has.
#[test]
fn the_run_list_is_a_fact_about_the_map_and_not_about_the_traffic() {
    let p = plane();
    let before = p.bar0_backable_runs();
    assert!(!before.is_empty(), "non-vacuity: the chip must have dead runs");

    // Drive the plane the way a boot does: every arm, over the whole aperture.
    let mut served = 0u64;
    for off in (0..BAR0_LEN).step_by(4096) {
        if !matches!(p.read(0, off, 4), ReadOutcome::Unclaimed) {
            served += 1;
        }
    }
    assert!(served > 0, "non-vacuity: the sweep must have reached live arms");

    let after = p.bar0_backable_runs();
    assert_eq!(
        before, after,
        "the dead-run list moved after the plane was read: it is describing traffic, not the \
         register map. {} runs became {}.",
        before.len(),
        after.len()
    );
}

/// ★★★ **The pure classifier and the real read path agree, both ways, on a fresh plane.**
///
/// ⊘ This is what lets the production path use the state-free predicate at all. Without it,
/// "dead" would be two definitions that merely happen to coincide today: the predicate could
/// drift to call a SERVED register dead, and the only symptom would be a guest reading zero
/// out of a memory page where a value belongs — silently, with no fault and no counter.
#[test]
fn the_state_free_classifier_agrees_with_the_read_path() {
    let p = plane();
    let (mut dead, mut live) = (0u64, 0u64);
    for off in (0..BAR0_LEN).step_by(4) {
        let pure = p.bar0_dword_is_dead_for_test(off);
        let real = matches!(p.read(0, off, 4), ReadOutcome::Unclaimed);
        assert_eq!(
            pure, real,
            "offset {off:#x}: the state-free classifier says dead={pure} and the read path \
             says unclaimed={real}"
        );
        if pure {
            dead += 1;
        } else {
            live += 1;
        }
    }
    assert!(dead > 0 && live > 0, "non-vacuity both ways: dead={dead} live={live}");
}
