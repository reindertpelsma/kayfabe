//! ★ **Replay of a measured failure: the slot-pool ceiling killed the split channel** (2026-11-16; FIXED 2026-11-18).
//!
//! STATUS: LIVE 2026-11-18 (the defect is fixed in `Capacity::slot_needed`; this file is its regression
//! test). The trace -> fixture -> test pattern is in `docs/design/V3_REPLAY_TESTS.md`; the measured run is
//! `/root/night/PILOT_REPLAY_TEST_20261116.md` (and `PC_WUKONG_20261115.md`, mechanism M2).
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
//! ## The invariant (held since 2026-11-18)
//!
//! **A slot whose growth fits in the free run directly after its own region is not at a ceiling.**
//! `Capacity::slot_needed` used to look only for a NEW region of `required` runs, so a pool with 1.2 M free
//! runs in blocks of at most 590 k refused a 6 k-run growth by name; the refused diff overflowed its slot,
//! the space did not apply, the split failed and the guest's UVM channel was DEAD. It now falls back to
//! growing in place into that free run (no copy, same bounds); every growth that relocated before relocates
//! the same way (the controls below).
//!
//! ## What this replay can and cannot show
//!
//! - CAN: the allocator's decision for a logged pool layout and a logged growth request. `Capacity`
//!   is pure (no CUDA); the layout is rebuilt through the replay seam and `slot_geometry` must print
//!   the logged geometry line byte for byte, so the replay starts from the measured state.
//! - CANNOT: timing (`SPLIT-LATE`, `SLOW-APPLY`), real GPU or driver effects, how the pool came to be
//!   fragmented (the log carries only the final geometry), or the link ceiling -> DEAD (measured as
//!   adjacency in the log; the manager side is `kf-mem` `vasmgr.rs`: an `overflow` entry fails its space).
//!   **No hardware run confirms the fix**: a Wukong run is the confirmation step.
//! - No GPU work: the device is a [`RecordingDevice`] that only records the copies the walker WOULD
//!   queue (`Move::copy`), and checks the layout stays disjoint and inside the pool.
//!
//! ## Why the extension needs no mapping of its own (read from `walk.rs`, not measured on a GPU)
//!
//! The slot pool is ONE `cuMemAlloc` of `slot_pool` runs, zeroed, made when the walker is built; slot
//! regions are offsets into it, not separate allocations, and the committed placements are the walker's
//! private device scratch, not guest page tables or host mappings. The kernel learns a slot's region only
//! from `KfLayout.slot_off/slot_cap`, which `Capacity::plan` rebuilds from the region table on the re-walk
//! submitted after the growth. So an extended slot is addressable exactly when `plan` reports the new cap
//! (tested below), and the copies and the re-walk share one stream, so a copy of a relocated
//! slot is ordered before the walk that writes into runs it freed.

use kf_cuda::capacity::{Capacity, Move, Region};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/slot_ceiling_replay.txt");

/// Every logged case: Wukong boots p1 and p2 and the Windows Camera Raw run (dates and sources in the fixture).
const LOGGED: [&str; 3] = ["p1-slot32", "p2-slot32", "adobe-slot29"];

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

#[test]
fn adobe_slot29_pool_matches_the_logged_geometry() {
    assert_logged_geometry("adobe-slot29");
}

/// The second logged occurrence (fixture case `adobe-slot29`: Windows 11, Photoshop Camera Raw model
/// downloads, binary 6bfa2dbc, which predates the fix). The slot is the LAST region of the pool, so its tail is the pool's
/// end: the logged request grows in place to the logged required size (175 360 runs of the 471 296-run
/// tail), not to the preferred 966 912 (497 664 extra, more than the tail). The pool keeps 295 936 runs
/// after the slot and its 309 504-run head untouched. Before the fix this request was refused by name
/// (the logged ceiling line), the split did not apply and channel 0x80c died.
#[test]
fn adobe_slot29_growth_takes_the_required_size_from_the_pool_tail() {
    let c = case("adobe-slot29");
    let mut cap = build(&c);
    let mut dev = RecordingDevice::new(&c);
    let m = cap
        .slot_needed(29, c.need_hi)
        .unwrap_or_else(|e| panic!("still refused at the logged ceiling: {e}"))
        .expect("grows");
    assert_eq!(m.from, r(1_156_608, 469_248));
    assert_eq!(m.to, r(1_156_608, 644_608), "in place, required size");
    assert_eq!(m.copy(), None);
    dev.growth(m);
    assert!(dev.copies.is_empty());
    assert_eq!(cap.stats.ceilings, 0);
    assert_eq!(cap.slot_pool_free(), 1_268_992 - 175_360);
    let g = cap.slot_geometry(29, 644_608);
    assert!(
        g.contains("adjacent free: head 309504 tail 295936"),
        "the head is untouched and the tail shrank by the extension: {g}"
    );
    // the same slot can still reach exactly its new tail end in place, and one grain more is a ceiling
    let to_end = cap
        .slot_needed(29, 644_608 + 295_936)
        .expect("the rest of the tail")
        .expect("grows");
    assert_eq!(to_end.to, r(1_156_608, 940_544));
    let e = cap.slot_needed(29, 940_544 + 1).unwrap_err();
    assert!(e.contains("slot pool exhausted"), "{e}");
}

/// The runs a logged ceiling line names after `→` (the `required` size).
fn logged_required(c: &Case) -> u32 {
    let after = c
        .logged_ceiling
        .split_once("→ ")
        .expect("a grows A → R line")
        .1;
    num(after.split_whitespace().next().expect("a number"))
}

/// `need_lo..=need_hi` all get the logged required size, and the grain edges around it are exact. The
/// range itself is derived from the logged required AND preferred pair (the refusal text that printed the
/// pair is not reachable for these layouts any more); only the required side is observable now.
#[test]
fn the_need_range_is_exactly_the_logged_required_size() {
    for name in LOGGED {
        let c = case(name);
        let req = logged_required(&c);
        let grown = |need: u32| {
            build(&c)
                .slot_needed(c.slot, need)
                .expect("fits in place")
                .expect("grows")
                .to
                .cap
        };
        for need in [c.need_lo, (c.need_lo + c.need_hi) / 2, c.need_hi] {
            assert_eq!(grown(need), req, "{name} {need}");
        }
        // the logged pair's upper end (`need_hi`) comes from the PREFERRED size, which is not observable
        // here; the required size covers exactly (req - 256, req]
        assert_eq!(grown(req), req, "{name}");
        assert_eq!(
            grown(req - 255),
            req,
            "{name}: the lowest need of the grain"
        );
        assert_eq!(grown(req - 256), req - 256, "{name}: one grain below");
        assert_eq!(grown(req + 1), req + 256, "{name}: one grain above");
    }
}

/// ★ FIXED 2026-11-18 (was `known_defect_slot_ceiling_refuses_growth_that_fits_in_place`, which pinned the
/// logged refusal byte for byte): the logged request, from the logged layout, now grows IN PLACE to
/// exactly the logged `required` size, with no ceiling counted, the offset kept, no copy to issue, and
/// the extension taken from the free run that the geometry line named.
#[test]
fn the_logged_growth_now_happens_in_place_without_a_copy() {
    for name in LOGGED {
        let c = case(name);
        let mut cap = build(&c);
        let mut dev = RecordingDevice::new(&c);
        let old = cap.slot_region(c.slot).expect("held");
        let free0 = cap.slot_pool_free();
        assert!(
            c.logged_geometry.contains("in-place growth would fit"),
            "{name}: the logged geometry says the tail was free"
        );
        let m = cap
            .slot_needed(c.slot, c.need_hi)
            .unwrap_or_else(|e| panic!("{name}: still refused: {e}"))
            .expect("grows");
        assert_eq!(m.from, old, "{name}");
        assert_eq!(m.to.off, old.off, "{name}: in place keeps the offset");
        assert_eq!(
            m.to.cap,
            logged_required(&c),
            "{name}: the logged required size"
        );
        assert_eq!(m.copy(), None, "{name}: no copy");
        dev.growth(m);
        assert!(dev.copies.is_empty(), "{name}");
        assert_eq!(cap.stats.ceilings, 0, "{name}");
        assert_eq!(cap.stats.grows, 1, "{name}");
        assert_eq!(
            cap.slot_pool_free(),
            free0 - u64::from(m.to.cap - old.cap),
            "{name}: exactly the extension left the pool"
        );
        // the kernel's view: the next plan reports the new capacity at the same offset
        let lay = cap.plan(&[c.slot]).expect("the walk fits");
        assert_eq!(lay.slot_off[c.slot as usize], old.off, "{name}");
        assert_eq!(lay.slot_cap[c.slot as usize], m.to.cap, "{name}");
        assert!(
            lay.walk_cap[0] >= m.to.cap,
            "{name}: the walk region covers the slot"
        );
    }
}

/// ★ The acceptance test of the fix (was red from 2026-11-16, green since 2026-11-18). A growth whose required size
/// fits in the slot's own region plus the free run directly after it must succeed, over no other slot,
/// without a copy that overlaps its source, leaving every region disjoint and inside the pool.
#[test]
fn a_growth_that_fits_in_place_is_not_a_ceiling() {
    for name in LOGGED {
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

/// Control (green before and after the fix): when a free block holds the whole growth, the slot
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
    // the copy is of the OLD length (260 352 live-at-most runs), not the new region's 393 216
    assert_eq!(m.copy(), Some((36_864, 626_688, 260_352)));
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
    assert_eq!(
        m.copy(),
        Some((36_864, 0, 1024)),
        "the old 1 024 runs are copied"
    );
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
    for name in LOGGED {
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

// ---------------------------------------------------------------------------------------------------
// The fix's own cases, on small synthetic pools (`seam`: 4096 runs, `max_cap` 4096, default
// 1024). Nothing below is measured; each is a state built to hit one edge of the in-place rule.
// ---------------------------------------------------------------------------------------------------

fn pool(max_cap: u32, held: &[(u32, Region)]) -> Capacity {
    Capacity::from_slot_regions(8, 4096, 4096, max_cap, 1024, held).expect("valid layout")
}

/// What must hold after ANY decision: the held regions are disjoint, inside the pool and within `max_cap`,
/// and held + free is the pool (nothing leaked, nothing counted twice).
fn assert_pool_sound(cap: &Capacity, max_cap: u32, what: &str) {
    let mut held: Vec<Region> = (0..8).filter_map(|s| cap.slot_region(s)).collect();
    held.sort_by_key(|r| r.off);
    for w in held.windows(2) {
        assert!(w[0].off + w[0].cap <= w[1].off, "{what}: overlap {w:?}");
    }
    for r in &held {
        assert!(r.off + r.cap <= 4096, "{what}: past the pool {r:?}");
        assert!(r.cap <= max_cap, "{what}: past max_cap {r:?}");
    }
    let used: u64 = held.iter().map(|r| u64::from(r.cap)).sum();
    assert_eq!(used + cap.slot_pool_free(), 4096, "{what}: partition");
}

/// Growth that takes EXACTLY the free run after the slot (the edge that fits): the run is consumed whole.
#[test]
fn growth_exactly_the_free_tail_fits_and_consumes_it() {
    // slot 0 [0,1024), 2 048 free [1024,3072), slot 1 [3072,4096): need 3072 -> required 3072 = 1 024 + 2 048
    let mut c = pool(4096, &[(0, r(0, 1024)), (1, r(3072, 1024))]);
    let m = c
        .slot_needed(0, 3072)
        .expect("exactly fits")
        .expect("grows");
    assert_eq!((m.from, m.to), (r(0, 1024), r(0, 3072)));
    assert_eq!(m.copy(), None);
    assert_eq!(c.slot_pool_free(), 0, "the whole run was taken");
    assert_eq!(c.stats.ceilings, 0);
    assert_pool_sound(&c, 4096, "exact tail");
}

/// One run past the edge (a grain more) is a ceiling, named exactly as before, and changes nothing.
#[test]
fn growth_one_grain_past_the_free_tail_is_still_a_ceiling() {
    let mut c = pool(4096, &[(0, r(0, 1024)), (1, r(3072, 1024))]);
    let e = c
        .slot_needed(0, 3073)
        .expect_err("2 304 extra > 2 048 free");
    assert_eq!(
        e,
        "slot pool exhausted: slot 0 grows 1024 → 3328 required runs (4096 preferred), 2048 free (largest 2048) of 4096"
    );
    assert_eq!(c.stats.ceilings, 1);
    assert_eq!(c.stats.grows, 0);
    assert_eq!(c.slot_region(0), Some(r(0, 1024)));
    assert_eq!(c.slot_pool_free(), 2048);
    assert_pool_sound(&c, 4096, "one past");
}

/// Free runs that do not touch the slot are never combined into its growth: 1 024 runs are free in two
/// runs of 512, slot 0 needs 768 more, so it is a ceiling although the total would hold it; each slot
/// still grows into ITS OWN adjacent run.
#[test]
fn fragmented_free_runs_are_not_combined_but_each_slot_uses_its_own() {
    let layout = [(0, r(0, 1024)), (1, r(1536, 1024)), (2, r(3072, 1024))];
    let mut c = pool(4096, &layout);
    assert_eq!(c.slot_pool_free(), 1024);
    let e = c
        .slot_needed(0, 1700)
        .expect_err("768 extra > 512 adjacent");
    assert_eq!(
        e,
        "slot pool exhausted: slot 0 grows 1024 → 1792 required runs (2560 preferred), 1024 free (largest 512) of 4096"
    );
    assert_eq!(c.stats.ceilings, 1);
    assert_pool_sound(&c, 4096, "fragmented refusal");
    // 512 extra each is exactly its own run
    let m0 = c.slot_needed(0, 1400).expect("fits").expect("grows");
    let m1 = c.slot_needed(1, 1400).expect("fits").expect("grows");
    assert_eq!((m0.to, m1.to), (r(0, 1536), r(1536, 1536)));
    assert_eq!((m0.copy(), m1.copy()), (None, None));
    assert_eq!(c.slot_pool_free(), 0);
    assert_pool_sound(&c, 4096, "fragmented growth");
}

/// A free run that does not START at the slot's end is not extended into (a run further on, or one that
/// ends where the slot starts): the slot is refused rather than grown over a neighbour.
#[test]
fn a_free_run_that_does_not_touch_the_slots_end_is_not_extended_into() {
    // slot 0 [0,1024), slot 1 [1024,2048), 512 free at [2048,2560), slot 2 [2560,4096)
    let mut c = pool(
        4096,
        &[(0, r(0, 1024)), (1, r(1024, 1024)), (2, r(2560, 1536))],
    );
    let e = c
        .slot_needed(0, 1280)
        .expect_err("slot 1 sits after slot 0");
    assert!(
        e.starts_with("slot pool exhausted: slot 0 grows 1024 → 1280"),
        "{e}"
    );
    assert_eq!(c.slot_region(0), Some(r(0, 1024)));
    assert_pool_sound(&c, 4096, "not adjacent");
    // slot 2 ends at the pool end: the free run BEFORE it is not growth room
    let mut c = pool(4096, &[(0, r(0, 1024)), (2, r(3072, 1024))]);
    assert!(
        c.slot_needed(2, 1500).is_ok(),
        "the 2 048-run free block holds a relocation"
    );
    let mut c = pool(
        4096,
        &[(0, r(0, 1024)), (1, r(1536, 1536)), (2, r(3072, 1024))],
    );
    assert!(
        c.slot_needed(2, 1500).is_err(),
        "512 free before it is not growth room"
    );
    assert_eq!(c.slot_region(2), Some(r(3072, 1024)));
    assert_eq!(c.stats.ceilings, 1);
    assert_pool_sound(&c, 4096, "pool end");
}

/// When the tail holds the PREFERRED (1.5x) size, the slot takes the headroom too; when only the required size
/// fits it takes that. Mirrors the allocation's own preferred-then-required order.
#[test]
fn in_place_growth_takes_the_headroom_when_the_tail_holds_it() {
    // slot 0 [0,1024), 1 024 free [1024,2048), slot 1 [2048,4096); need 1100: required 1280, preferred 1792
    let mut c = pool(4096, &[(0, r(0, 1024)), (1, r(2048, 2048))]);
    let m = c.slot_needed(0, 1100).expect("fits").expect("grows");
    assert_eq!(
        m.to,
        r(0, 1792),
        "preferred fits: 768 of the 1 024-run tail"
    );
    assert_eq!(c.slot_pool_free(), 256);
    // need 1400: required 1536, preferred 2304 (> 1 024 + 1 024): only the required size fits
    let mut c = pool(4096, &[(0, r(0, 1024)), (1, r(2048, 2048))]);
    let m = c.slot_needed(0, 1400).expect("fits").expect("grows");
    assert_eq!(m.to, r(0, 1536));
    assert_eq!(c.slot_pool_free(), 512);
    assert_pool_sound(&c, 4096, "headroom");
}

/// Two slots that want the same free run, in both orders (a pending relocation and an in-place growth, one
/// walk's batch): the first to be decided gets it, the second loses CLEANLY (or takes what is left),
/// and the regions never overlap.
#[test]
fn two_slots_wanting_the_same_tail_never_overlap() {
    // A = slot 0 [0,1024); T = [1024,2048) free; C = slot 1 [2048,2304); D = slot 2 [2304,4096)
    let layout = [(0, r(0, 1024)), (1, r(2048, 256)), (2, r(2304, 1792))];
    // C first: its relocation takes T whole; A's tail is gone, and A cannot relocate either
    let mut c = pool(4096, &layout);
    let mc = c
        .slot_needed(1, 683)
        .expect("T holds it")
        .expect("relocates");
    assert_eq!(mc.to, r(1024, 1024));
    assert_eq!(mc.copy(), Some((1024, 2048, 256)));
    assert_pool_sound(&c, 4096, "C first");
    let free = c.slot_pool_free();
    let e = c.slot_needed(0, 1100).expect_err("T is C's now");
    assert!(
        e.starts_with("slot pool exhausted: slot 0 grows 1024 → 1280"),
        "{e}"
    );
    assert_eq!(c.slot_region(0), Some(r(0, 1024)));
    assert_eq!(c.slot_pool_free(), free, "a refusal changes nothing");
    assert_pool_sound(&c, 4096, "A refused");
    // A first: it grows in place into T (preferred 1 792), 256 remain, C can neither relocate nor grow
    let mut c = pool(4096, &layout);
    let ma = c.slot_needed(0, 1100).expect("in place").expect("grows");
    assert_eq!(ma.to, r(0, 1792));
    assert_pool_sound(&c, 4096, "A first");
    let e = c
        .slot_needed(1, 683)
        .expect_err("only 256 left, D follows C");
    assert!(e.starts_with("slot pool exhausted: slot 1"), "{e}");
    assert_eq!(c.slot_region(1), Some(r(2048, 256)));
    assert_pool_sound(&c, 4096, "C refused");
}

/// One batch: slot 1 relocates away, and slot 0 (left of it) grows in place into the run slot 1 just
/// freed. The live regions are disjoint at every step; the source of slot 1's pending copy overlaps
/// slot 0's extension, which is safe ONLY because the copy is queued before the walk that writes into it
/// (same stream: `walk.rs` issues `Move::copy` on `self.stream`, then re-walks on it). That ordering is
/// read from the code, not tested here; a relocation into a just-released run had the same property.
#[test]
fn growth_into_the_run_a_pending_relocation_freed_is_disjoint_from_the_destination() {
    let layout = [(0, r(0, 1024)), (1, r(1024, 1024))];
    let mut c = pool(4096, &layout);
    let m1 = c.slot_needed(1, 1100).expect("fits").expect("relocates");
    assert_eq!((m1.from, m1.to), (r(1024, 1024), r(2048, 1792)));
    assert_eq!(m1.copy(), Some((2048, 1024, 1024)));
    assert_pool_sound(&c, 4096, "after the relocation");
    let m0 = c.slot_needed(0, 1100).expect("fits").expect("grows");
    assert_eq!(m0.to, r(0, 1792), "into the run slot 1 left");
    assert_eq!(m0.copy(), None);
    assert!(
        m0.to.off + m0.to.cap > m1.from.off,
        "it covers the pending copy's source"
    );
    assert!(
        m0.to.off + m0.to.cap <= m1.to.off,
        "and never its destination"
    );
    assert_pool_sound(&c, 4096, "after both");
}

/// The hostile-guest bounds hold for in-place growth: a need past `max_cap` is refused however large the
/// free tail is, and over every layout and every need the result stays inside `max_cap` and the pool,
/// disjoint, with the partition intact (a sweep, so no single hand-picked case carries the bound).
#[test]
fn in_place_growth_keeps_max_cap_and_the_pool_bound() {
    // refused past max_cap, with 3 072 runs free right after the slot
    let mut c = pool(2048, &[(0, r(0, 1024))]);
    for need in [2049, 4096, 1 << 20, u32::MAX] {
        let e = c.slot_needed(0, need).expect_err("past max_cap");
        assert_eq!(
            e,
            "slot 0: ".to_string()
                + &format!("{need} committed placements; one space may hold 2048")
        );
    }
    assert_eq!(c.stats.ceilings, 4);
    assert_eq!(c.slot_region(0), Some(r(0, 1024)));
    assert_pool_sound(&c, 2048, "past max_cap");
    // the sweep
    let layouts: [&[(u32, Region)]; 4] = [
        &[(0, r(0, 1024)), (1, r(1792, 2048))],
        &[(0, r(0, 1024)), (1, r(1024, 1024)), (2, r(2560, 1536))],
        &[(0, r(0, 256)), (1, r(1280, 1024)), (2, r(2816, 1280))],
        &[(0, r(2048, 1024)), (1, r(0, 1024))],
    ];
    for max_cap in [2048u32, 3072, 4096] {
        for (n, layout) in layouts.iter().enumerate() {
            for slot in 0..3u32 {
                if !layout.iter().any(|&(s, _)| s == slot) {
                    continue;
                }
                for need in (1..=4200u32).step_by(37) {
                    let mut c = pool(max_cap, layout);
                    let old = c.slot_region(slot).expect("held");
                    let what = format!("max_cap {max_cap} layout {n} slot {slot} need {need}");
                    match c.slot_needed(slot, need) {
                        Ok(None) => assert!(need <= old.cap, "{what}"),
                        Ok(Some(m)) => {
                            assert!(m.to.cap >= need && m.to.cap <= max_cap, "{what}: {m:?}");
                            assert_eq!(m.copy().is_none(), m.to.off == old.off, "{what}");
                        }
                        Err(_) => assert_eq!(c.slot_region(slot), Some(old), "{what}"),
                    }
                    assert_pool_sound(&c, max_cap, &what);
                }
            }
        }
    }
}
