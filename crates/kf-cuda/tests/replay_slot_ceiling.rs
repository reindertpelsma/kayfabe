//! ★ **Replay of a measured failure: the slot-pool ceiling kills the split channel** (2026-11-16).
//!
//! STATUS: LIVE 2026-11-16. The trace -> fixture -> test pattern is in
//! `docs/design/V3_REPLAY_TESTS.md`; the measured run is `/root/night/PILOT_REPLAY_TEST_20261116.md`
//! (and `PC_WUKONG_20261115.md`, mechanism M2).
//!
//! ## The measured failure (what the log says, nothing more)
//!
//! Wukong at 4096 MiB, FG off: four times (boots p1 x2, p2 x2 of the profile build) the walker logged
//! `walk capacity ceiling: slot pool exhausted: slot S grows A -> R required runs ...` with
//! `R - A` between 5 376 and 19 200 runs while 1.1-1.2 M runs of the 2 M-run pool were free, and
//! within three log lines `chan token 0x80b|0x80a DEAD: split ... did not apply`. The geometry line
//! logged beside the ceiling says, in all four, `in-place growth would fit`: the free run directly
//! after the slot was 209 664-603 136 runs.
//!
//! ## The invariant the code violates
//!
//! **A slot whose growth fits in the free run directly after its own region is not at a ceiling.**
//! `Capacity::slot_needed` only looks for a NEW region of `required` runs, so a pool with 1.2 M free
//! runs in blocks of at most 590 k refuses a 6 k-run growth by name; the refused diff overflows its slot,
//! the space does not apply, the split fails and the guest's UVM channel is DEAD.
//!
//! ## What this replay can and cannot show
//!
//! - CAN: the allocator's decision for a logged pool layout and a logged growth request. `Capacity` is
//!   pure (no CUDA); the layout is rebuilt through the replay seam and `slot_geometry` must print the
//!   logged geometry line byte for byte, so the replay starts from the measured state.
//! - CANNOT: timing (`SPLIT-LATE`, `SLOW-APPLY`), real GPU or driver effects, how the pool came to be
//!   fragmented (the log carries only the final geometry), or the link ceiling -> DEAD (measured as
//!   adjacency in the log; the manager side is `kf-mem` `vasmgr.rs`: an `overflow` entry fails its space).
//! - NOT a fix and no GPU work: the device is a [`RecordingDevice`] that only records the copies the
//!   walker WOULD queue (`Move`), and checks the layout stays disjoint and inside the pool.
//!
//! The ignored test is red today and is the acceptance test of the fix; the `known_defect_` test pins the
//! measured behaviour so the replay stays live in CI, and flips (delete it) when the fix lands.

use kf_cuda::capacity::{Capacity, Move, Region};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/slot_ceiling_replay.txt");

/// One fixture case (2026-11-16): the logged lines, copied verbatim from the boot log, and the inputs derived from them.
#[derive(Debug, Default)]
struct Case {
    name: String,
    max_slots: u32,
    walk_pool: u32,
    slot_pool: u32,
    max_cap: u32,
    slot_default: u32,
    slot: u32,
    need_lo: u32,
    need_hi: u32,
    logged_ceiling: String,
    logged_geometry: String,
    /// `(slot, region)` named by the log.
    measured: Vec<(u32, Region)>,
    /// `(off, cap, n)`: a used span the log implies, split into `n` invented filler slots.
    spans: Vec<(u32, u32, u32)>,
}

fn kv(line: &str, key: &str) -> u32 {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("`{key}=` missing in `{line}`"))
        .parse()
        .unwrap_or_else(|e| panic!("`{key}` in `{line}`: {e}"))
}

fn num(w: &str) -> u32 {
    w.parse().unwrap_or_else(|e| panic!("`{w}`: {e}"))
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    let mut cur: Option<Case> = None;
    for raw in FIXTURE.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "case" => {
                cur = Some(Case {
                    name: rest.to_string(),
                    ..Case::default()
                });
            }
            "end" => out.extend(cur.take()),
            _ => {
                let c = cur.as_mut().expect("a line outside a case");
                match word {
                    "source" => {}
                    "config" => {
                        c.max_slots = kv(rest, "max_slots");
                        c.walk_pool = kv(rest, "walk_pool");
                        c.slot_pool = kv(rest, "slot_pool");
                        c.max_cap = kv(rest, "max_cap");
                        c.slot_default = kv(rest, "slot_default");
                    }
                    "event" => {
                        c.slot = kv(rest, "slot");
                        c.need_lo = kv(rest, "need_lo");
                        c.need_hi = kv(rest, "need_hi");
                    }
                    "logged-ceiling" => c.logged_ceiling = rest.to_string(),
                    "logged-geometry" => c.logged_geometry = rest.to_string(),
                    "logged-consequence" => {}
                    "measured" => {
                        let w: Vec<&str> = rest.split_whitespace().collect();
                        assert_eq!(w[0], "slot", "{line}");
                        c.measured.push((
                            num(w[1]),
                            Region {
                                off: num(w[2]),
                                cap: num(w[3]),
                            },
                        ));
                    }
                    "synthetic" => {
                        let w: Vec<&str> = rest.split_whitespace().collect();
                        assert_eq!(w[0], "span", "{line}");
                        c.spans.push((num(w[1]), num(w[2]), num(w[3])));
                    }
                    other => panic!("unknown fixture word `{other}`"),
                }
            }
        }
    }
    assert!(cur.is_none(), "a case without `end`");
    out
}

fn case(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no fixture case {name}"))
}

/// Every region the case holds: the logged ones (2026-11-16 fixture), then the invented fillers (lowest free slot ids).
fn regions(c: &Case) -> Vec<(u32, Region)> {
    let mut held = c.measured.clone();
    let mut next = 0u32;
    for &(off, cap, n) in &c.spans {
        let mut at = off;
        for i in 0..n {
            while held.iter().any(|&(s, _)| s == next) {
                next += 1;
            }
            let this = if i + 1 == n {
                off + cap - at
            } else {
                c.slot_default
            };
            held.push((next, Region { off: at, cap: this }));
            next += 1;
            at += this;
        }
        assert_eq!(at, off + cap, "span {off}+{cap} not covered");
    }
    held
}

fn build(c: &Case) -> Capacity {
    Capacity::from_slot_regions(
        c.max_slots,
        c.walk_pool,
        c.slot_pool,
        c.max_cap,
        c.slot_default,
        &regions(c),
    )
    .expect("the logged layout is a valid pool")
}

/// The fake backend: what the walker's device half would be told, recorded and checked, never executed.
/// A [`Move`] is the walker's request for one device-to-device copy on its stream; here nothing runs.
#[derive(Default)]
struct RecordingDevice {
    pool: u32,
    held: BTreeMap<u32, Region>,
    copies: Vec<Move>,
}

impl RecordingDevice {
    fn new(c: &Case) -> RecordingDevice {
        RecordingDevice {
            pool: c.slot_pool,
            held: regions(c).into_iter().collect(),
            copies: Vec::new(),
        }
    }

    /// Record one growth request and check it cannot write outside the slot's own space.
    fn growth(&mut self, m: Move) {
        assert_eq!(
            self.held.get(&m.slot),
            Some(&m.from),
            "a move names the region held"
        );
        assert!(m.to.cap >= m.from.cap, "growth never shrinks: {m:?}");
        assert!(
            u64::from(m.to.off) + u64::from(m.to.cap) <= u64::from(self.pool),
            "past the pool: {m:?}"
        );
        for (&s, r) in &self.held {
            if s == m.slot {
                continue;
            }
            let disjoint = m.to.off + m.to.cap <= r.off || r.off + r.cap <= m.to.off;
            assert!(disjoint, "slot {} grows over slot {s}: {m:?}", m.slot);
        }
        // An overlapping device copy (source and destination share runs) is not safe to assume:
        // growth in place must keep the offset and need no copy at all.
        let overlaps = m.to.off < m.from.off + m.from.cap && m.from.off < m.to.off + m.to.cap;
        assert!(
            !overlaps || m.to.off == m.from.off,
            "an overlapping relocation: {m:?}"
        );
        if m.to.off != m.from.off {
            self.copies.push(m);
        }
        self.held.insert(m.slot, m.to);
    }
}

/// The replay starts from the logged state (2026-11-16 fixture): `slot_geometry` prints the logged geometry line.
fn assert_logged_geometry(name: &str) {
    let c = case(name);
    let cap = build(&c);
    for need in [c.need_lo, c.need_hi] {
        assert_eq!(
            cap.slot_geometry(c.slot, need),
            c.logged_geometry,
            "{name} need {need}"
        );
    }
}

#[test]
fn p1_slot32_pool_matches_the_logged_geometry() {
    assert_logged_geometry("p1-slot32");
}

#[test]
fn p2_slot32_pool_matches_the_logged_geometry() {
    assert_logged_geometry("p2-slot32");
}

/// `need_lo..=need_hi` is exactly the set of needs that print the logged required/preferred pair: the
/// fixture states a derived range, so the range itself is checked on both sides.
#[test]
fn the_need_range_is_exactly_the_logged_required_and_preferred() {
    for name in ["p1-slot32", "p2-slot32"] {
        let c = case(name);
        let msg = |need: u32| build(&c).slot_needed(c.slot, need);
        for need in [c.need_lo, (c.need_lo + c.need_hi) / 2, c.need_hi] {
            assert_eq!(
                msg(need).expect_err("ceiling"),
                c.logged_ceiling,
                "{name} {need}"
            );
        }
        for need in [c.need_lo - 1, c.need_hi + 1] {
            let m = msg(need);
            assert_ne!(
                m.as_ref().err(),
                Some(&c.logged_ceiling),
                "{name} {need}: {m:?}"
            );
        }
    }
}

/// ★ KNOWN DEFECT, pinned 2026-11-16 (flips when fixed): today `slot_needed` reproduces the logged refusal
/// byte for byte, from a layout whose own geometry line says growth in place would fit, and the
/// refusal changes nothing (the slot keeps its region, the pool its free runs). When the fix lands this
/// test fails: delete it and remove the `#[ignore]` below.
#[test]
fn known_defect_slot_ceiling_refuses_growth_that_fits_in_place() {
    for name in ["p1-slot32", "p2-slot32"] {
        let c = case(name);
        let mut cap = build(&c);
        let before = (cap.slot_region(c.slot), cap.slot_pool_free());
        let e = cap
            .slot_needed(c.slot, c.need_hi)
            .expect_err("the measured run was refused here");
        assert_eq!(e, c.logged_ceiling, "{name}");
        assert!(
            c.logged_geometry.contains("in-place growth would fit"),
            "{name}: the logged geometry says the tail was free"
        );
        assert_eq!(cap.stats.ceilings, 1, "{name}: counted as a ceiling");
        assert_eq!(
            (cap.slot_region(c.slot), cap.slot_pool_free()),
            before,
            "{name}: a refusal changes nothing"
        );
    }
}

/// ★ The acceptance test of the fix (red today, 2026-11-16: the logged failure). A growth whose required size
/// fits in the slot's own region plus the free run directly after it must succeed, over no other slot,
/// without a copy that overlaps its source, leaving every region disjoint and inside the pool.
#[test]
#[ignore = "known defect: slot-pool ceiling refuses growth that fits in the free tail, fix tracked in docs/design/V3_REPLAY_TESTS.md"]
fn a_growth_that_fits_in_place_is_not_a_ceiling() {
    for name in ["p1-slot32", "p2-slot32"] {
        let c = case(name);
        let old = build(&c).slot_region(c.slot).expect("held");
        for need in [c.need_lo, c.need_hi] {
            let mut cap = build(&c);
            let mut dev = RecordingDevice::new(&c);
            let m = cap
                .slot_needed(c.slot, need)
                .unwrap_or_else(|e| panic!("{name} need {need}: refused at a ceiling: {e}"))
                .expect("the need exceeds the region, so a move is reported");
            assert_eq!(m.from, old, "{name}");
            assert!(m.to.cap >= need, "{name}: grew to {} for {need}", m.to.cap);
            dev.growth(m);
            // the pool stays a partition: used + free = the pool
            let used: u64 = (0..c.max_slots)
                .filter_map(|s| cap.slot_region(s))
                .map(|r| u64::from(r.cap))
                .sum();
            assert_eq!(
                used + cap.slot_pool_free(),
                u64::from(c.slot_pool),
                "{name}"
            );
        }
    }
}

/// Control (passes today, and must after the fix): when a free block holds the whole growth, the slot
/// relocates into the first block that fits (first-fit) and the old region returns to the pool. Without
/// this the two tests above could be red or green for the wrong reason.
#[test]
fn control_growth_into_a_big_enough_block_relocates() {
    let c = case("p1-slot32");
    let mut cap = build(&c);
    let mut dev = RecordingDevice::new(&c);
    let free0 = cap.slot_pool_free();
    // slot 8 holds 260 352 runs; 262 000 prefers 393 216, which BOTH the 589 824-run block at 36 864 and
    // the 399 360-run block at 887 040 hold: first-fit takes the lower
    let m = cap
        .slot_needed(8, 262_000)
        .expect("a block holds it")
        .expect("grows");
    assert_eq!(
        m.from,
        Region {
            off: 626_688,
            cap: 260_352
        }
    );
    assert_eq!(
        m.to,
        Region {
            off: 36_864,
            cap: 393_216
        }
    );
    dev.growth(m);
    assert_eq!(dev.copies.len(), 1, "a relocation is one device copy");
    assert_eq!(cap.slot_pool_free(), free0 + 260_352 - 393_216);
}

/// Control 2: a small slot of the same pool grows into the LOWEST block that holds its preferred size.
#[test]
fn control_a_small_slot_grows_into_the_lowest_fitting_block() {
    let c = case("p1-slot32");
    let mut cap = build(&c);
    let mut dev = RecordingDevice::new(&c);
    // slot 0 is the first filler, `0+1024`; 2 000 prefers 3 072: the 2 560-run block at 10 240 is too
    // small, the 589 824-run block at 36 864 is the lowest that holds it (the 209 664-run tail also would)
    let m = cap.slot_needed(0, 2_000).expect("fits").expect("grows");
    assert_eq!(m.from, Region { off: 0, cap: 1024 });
    assert_eq!(
        m.to,
        Region {
            off: 36_864,
            cap: 3072
        }
    );
    dev.growth(m);
}

fn seam(held: &[(u32, Region)]) -> Result<Capacity, String> {
    Capacity::from_slot_regions(8, 4096, 4096, 4096, 1024, held)
}

fn r(off: u32, cap: u32) -> Region {
    Region { off, cap }
}

/// The replay seam refuses layouts no pool could hold, each by name, and derives the free list as the
/// complement of what is held.
#[test]
fn the_replay_seam_accepts_a_layout_and_frees_the_rest() {
    assert!(seam(&[(0, r(0, 1024)), (1, r(1024, 1024))]).is_ok());
    let c = seam(&[(0, r(1024, 1024))]).expect("valid");
    assert_eq!(c.slot_pool_free(), 3072);
    assert_eq!(c.slot_region(0), Some(r(1024, 1024)));
}

#[test]
fn the_replay_seam_refuses_overlapping_regions() {
    let e = seam(&[(0, r(0, 2048)), (1, r(1024, 1024))]).unwrap_err();
    assert!(e.contains("overlaps"), "{e}");
}

#[test]
fn the_replay_seam_refuses_a_region_past_the_pool() {
    let e = seam(&[(0, r(3072, 2048))]).unwrap_err();
    assert!(e.contains("past the pool"), "{e}");
}

#[test]
fn the_replay_seam_refuses_a_slot_named_twice_or_out_of_range_or_empty() {
    assert!(
        seam(&[(0, r(0, 1024)), (0, r(2048, 1024))])
            .unwrap_err()
            .contains("named twice")
    );
    assert!(
        seam(&[(8, r(0, 1024))])
            .unwrap_err()
            .contains("out of range")
    );
    assert!(seam(&[(0, r(0, 0))]).unwrap_err().contains("empty"));
}

/// The same refusals on a real fixture: one more filler that overlaps a logged region, and one that
/// sticks out past the pool end, are refused (a fixture edit that breaks the layout cannot pass silently).
#[test]
fn a_fixture_layout_with_an_overlapping_or_outside_region_is_refused() {
    let c = case("p1-slot32");
    let build_with = |extra: (u32, Region)| {
        let mut held = regions(&c);
        held.push(extra);
        Capacity::from_slot_regions(
            c.max_slots,
            c.walk_pool,
            c.slot_pool,
            c.max_cap,
            c.slot_default,
            &held,
        )
    };
    // inside slot 32's own region
    let e = build_with((100, r(1_300_000, 1024))).unwrap_err();
    assert!(e.contains("overlaps"), "{e}");
    // past the end of the 2 097 152-run pool
    let e = build_with((101, r(c.slot_pool - 512, 1024))).unwrap_err();
    assert!(e.contains("past the pool"), "{e}");
}

/// The fixture is a partition: held + free = the pool, and the logged free-block count is the count the
/// layout derives (so a hand edit of a span shows up here as well as in the geometry line).
#[test]
fn fixture_layouts_hold_the_logged_number_of_slots() {
    for name in ["p1-slot32", "p2-slot32"] {
        let c = case(name);
        let cap = build(&c);
        let held = regions(&c).len();
        assert_eq!(held, 21, "{name}: the logged line says slots held 21");
        let used: u64 = regions(&c).iter().map(|(_, r)| u64::from(r.cap)).sum();
        assert_eq!(
            used + cap.slot_pool_free(),
            u64::from(c.slot_pool),
            "{name}"
        );
    }
}
