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
