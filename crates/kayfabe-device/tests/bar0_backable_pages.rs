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

    // ★★★★★ THE LOAD-BEARING CHECK, and w563 CHANGED WHAT IT CHECKS.
    //
    // It used to be *"no run may contain anything this device SERVES"* — true while a run was
    // answered with zeros. A run is now answered from a SHADOW, so the property is stronger
    // and different: **every byte of every run must be one the shadow can fill**, and the
    // shadow's bytes must equal what this device would answer.
    //
    // ⊘ Asserting "unclaimed" here would now be wrong in the favourable direction: it would
    // refuse the VBIOS aperture, which the device serves and the shadow reproduces exactly.
    let mut shadow = vec![0u8; PAGE as usize];
    for (start, len) in &runs {
        for page in (*start..*start + *len).step_by(PAGE as usize) {
            shadow.fill(0xAA);
            let filled = p.bar0_shadow_fill(page, &mut shadow);
            assert_eq!(
                filled,
                PAGE as usize,
                "page {page:#x} is inside a backable run and the shadow could fill only \
                 {filled} of {PAGE} bytes — publishing it would hand the guest 0xAA, or a \
                 stale value, for the rest"
            );
            for off in (page..page + PAGE).step_by(4) {
                let served = p.read(0, off, 4).value();
                let i = (off - page) as usize;
                let shadowed = u32::from_le_bytes([
                    shadow[i],
                    shadow[i + 1],
                    shadow[i + 2],
                    shadow[i + 3],
                ]);
                assert_eq!(
                    u64::from(shadowed),
                    served,
                    "offset {off:#x} is inside a backable run and the SHADOW disagrees with \
                     what this device answers: shadow={shadowed:#x} served={served:#x}. A \
                     guest reading it would take no exit and get the wrong value, with no \
                     fault and no counter."
                );
            }
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

/// ★★★★★ **WHICH pages are live, and WHAT makes each one live (w561).**
///
/// Owner, on being told the remaining unclaimed reads all sit in live pages: *"which ones"*.
///
/// ⊘ This answers the MAP, not the TRAFFIC. It says which of BAR0's 4096 pages hold at least
/// one register and which arm claims it — computed from the chip table, needing no boot. The
/// per-page READ COUNT is a different fact and needs a live guest; `[measured w554]` all we
/// know from traffic is the total, 122 001 unclaimed reads, and that **zero** of them landed
/// in a page the cut backs.
#[test]
fn report_which_pages_are_live_and_why() {
    let p = plane();
    let mut by_arm: std::collections::BTreeMap<&'static str, Vec<u64>> =
        std::collections::BTreeMap::new();
    for page in 0..(BAR0_LEN / PAGE) {
        let base = page * PAGE;
        // The first arm that claims ANY dword in the page is what keeps the page trapping.
        let mut claim: Option<&'static str> = None;
        for off in (base..base + PAGE).step_by(4) {
            if p.bar0_dword_is_dead_for_test(off) {
                continue;
            }
            claim = Some(p.bar0_claim_name_for_test(off));
            break;
        }
        if let Some(c) = claim {
            by_arm.entry(c).or_default().push(base);
        }
    }
    let total: usize = by_arm.values().map(Vec::len).sum();
    println!("BAR0-LIVE-PAGES total={total} of {} pages", BAR0_LEN / PAGE);
    for (arm, pages) in &by_arm {
        let first = pages.first().copied().unwrap_or(0);
        let last = pages.last().copied().unwrap_or(0);
        println!(
            "  {arm:<16} pages={:<5} span=[{first:#010x}..{last:#010x}]  first_few={:x?}",
            pages.len(),
            &pages[..pages.len().min(6)]
        );
    }
    assert!(total > 0, "non-vacuity: some page must be live");
}

/// ★★★ **Where the doorbell sits, and which page it poisons for WRITES.**
///
/// Owner: *"Most of bar0 is constant registers or mappable from userspace except the doorbell
/// write ofc"*. This prints the page so the write-side surface is a stated address rather than
/// an assumption.
#[test]
fn report_the_doorbell_page() {
    let p = plane();
    let db = p.doorbell_reg().expect("GA106 declares a usermode doorbell");
    println!(
        "BAR0-DOORBELL reg={db:#010x} page={:#010x} claimed_by={}",
        db & !(PAGE - 1),
        p.bar0_claim_name_for_test(db)
    );
}

/// ★★★★★ **THE HOTSPOT CENSUS MUST NAME A PAGE IT WAS ACTUALLY READ FROM (w586).**
///
/// ⊘⊘ A census zero needs a known-positive, and this tree has shipped several that could
/// never fire. So the instrument is driven: read a page, and require the report to say that
/// page, with that count, on the correct side of the backed/live split.
///
/// ★ It also pins the discrimination the report exists for. `[measured w582]` BAR0 reads
/// reaching the handler are 161 422 with only 138 unclaimed — a total that cannot point at
/// any of the remaining work. The two sums here (`reads_from_live_pages` vs
/// `reads_from_BACKED_pages`) are what turns it into a work list, and they mean OPPOSITE
/// things: one is a page still to solve, the other is a backing defect.
#[test]
fn the_read_hotspot_census_names_the_page_it_was_read_from() {
    let p = plane();

    // ⊘ Non-vacuity first, and in the direction that matters: an instrument that reports
    // something before being driven is reporting noise.
    let before = p.bar0_read_hotspots(8);
    assert!(
        before.contains("pages_touched=0"),
        "the census reported traffic before any read: {before}"
    );

    // ⊘ Both pages are DERIVED from the cut, not hardcoded. My first draft of this test
    // named `0x110000` as the live page on the strength of it being the worst trap site —
    // and the cut BACKS it, so the test failed on my assumption rather than on the
    // instrument. Asking the classifier is the whole point of having one.
    let backed_pages: std::collections::BTreeSet<u64> = p
        .bar0_backable_runs()
        .into_iter()
        .flat_map(|(b, l)| (b..b + l).step_by(4096))
        .collect();
    let live_page = (0..0x1000u64)
        .map(|i| i * 4096)
        .find(|o| !backed_pages.contains(o))
        .expect("the cut leaves at least one page live, or goal 2 is already done");
    for _ in 0..7 {
        let _ = p.read(0, live_page + 0xc00, 4);
    }
    // And one from a page the cut DOES back, to prove the two sums separate.
    let backed = *backed_pages
        .iter()
        .next()
        .expect("GA106 backs at least one page");
    let _ = p.read(0, backed, 4);

    let after = p.bar0_read_hotspots(8);
    assert!(
        after.contains(&format!("+0x{live_page:x}=7")),
        "the census did not name the page it was read from, or lost the count: {after}"
    );
    assert!(
        after.contains("reads_from_live_pages=7"),
        "reads from a page the cut leaves live were not summed as such: {after}"
    );
    assert!(
        after.contains("reads_from_BACKED_pages=1"),
        "a read from a BACKED page must be counted separately — it means the backing is not \
         working, which is a different defect from a page still to solve: {after}"
    );
    assert!(
        after.contains("!BACKED"),
        "the backed page's row was not tagged, so a backing defect would read as ordinary \
         remaining work: {after}"
    );
}
