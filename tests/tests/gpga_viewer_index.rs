//! # The GPGA viewer index — the owner's answer to `Q13`, and its eight tests
//!
//! Subject: [`kayfabe_mmu::gpga`]. The design is the owner's, decided 2026-07-31:
//!
//! > Each framebuffer window (`PRAMIN`, `BAR1`, `BAR2`) has its own mappings in GPGA, and
//! > the same GPGA can be mapped in multiple windows. Whenever a GPGA object or page is
//! > allocated, deallocated or remapped, the system asks **who can see this page** — which
//! > isolates, which windows, including **partial** maps — and updates them all, so every
//! > view is correct and **passthrough by construction**. Objects are the authority;
//! > **GPGA is the key**.
//!
//! ★★★ Governing rule: **never ask a single address where it belongs — always ask, per
//! region, what it contains.**
//!
//! ## The eight, and where each lives
//!
//! | # | claim | test |
//! |---|---|---|
//! | 1 | updating GPGA updates **all** viewers | [`t1_one_change_reaches_every_viewer_at_its_own_offset`] |
//! | 2 | a **new** view gets what it already contains | [`t2_a_new_view_is_seeded_with_the_objects_already_under_it`] |
//! | 3 | a **hanging** viewer must not wedge the update | [`t3_a_viewer_that_never_drains_wedges_nobody_and_is_named_desynced`] |
//! | 4 | a **race**: a view update in the midst of an allocation | [`t4_a_view_registered_between_plan_and_apply_refuses_the_plan_whole`] |
//! | 5 | **slicing** at the edge of a window | [`t5_a_window_edge_slices_the_object_as_an_ownership_regime`] |
//! | 6 | **mean**, not happy path | [`t6_the_aperture_is_half_the_key_and_a_bare_address_would_alias`] + [`t6b_hostile_and_malformed_input_refuses_by_name_and_never_panics`] |
//! | 7 | ★★ **the dual** — no viewer sees what it must not | [`t7_the_dual_no_viewer_sees_what_it_must_not`] |
//! | 8 | ★★ **deallocation** — every viewer loses it | [`t8_a_free_reaches_every_viewer_including_the_one_that_never_read`] |
//!
//! ## ⚠ Fixture addresses are DERIVED, not invented
//!
//! Two fixtures in this repository both picked `0x0077_7777` as *"an offset nobody owns"*
//! and both were **inside `PRAMIN`** — green while asserting that a framebuffer access was
//! an unclaimed register. So every geometry below comes from the real chip row
//! ([`kayfabe_device::CHIPS`]): the `PRAMIN` view's extent is that chip's
//! `pramin_window.len`, and every GPGA address is checked to be inside that chip's
//! `fb_length`. [`fixtures_come_from_the_real_chip_row`] is the assertion that keeps it
//! that way — if a chip row moves, this file fails rather than drifting.

use kayfabe_arch::{Aperture, FbWindow, ids::GpuId};
use kayfabe_isolate::IsolateId;
use kayfabe_mmu::HostExtent;
use kayfabe_mmu::gpga::{
    Applied, GpgaRegion, MAX_PENDING_UPDATES, ObjectChange, ObjectId, RegionError, ViewFault,
    ViewState, ViewUpdate, ViewerId, ViewerIndex, ViewerKind,
};

// ─────────────────────────────────────────────────────────────────────────────────────
// Geometry, derived from the real chip row
// ─────────────────────────────────────────────────────────────────────────────────────

/// The chip whose window map every fixture here is DERIVED from.
fn chip() -> &'static kayfabe_device::ChipProfile {
    kayfabe_device::CHIPS[0]
}

/// The `PRAMIN` view's extent — 1 MiB on GA10x, and read from the row rather than typed.
fn pramin_len() -> u64 {
    chip().pramin_window.len
}

/// A framebuffer address that is genuinely inside this chip's framebuffer.
fn fb(addr: u64) -> u64 {
    assert!(
        addr < chip().fb_length,
        "fixture address {addr:#x} is outside {}'s {:#x}-byte framebuffer — \
         pick an address against the real map, not a memorable hex constant",
        chip().name,
        chip().fb_length
    );
    addr
}

/// A vidmem GPGA region at a framebuffer address.
fn vid(base: u64, len: u64) -> GpgaRegion {
    GpgaRegion::new(Aperture::Vidmem, fb(base), len).expect("well-formed fixture region")
}

/// The same coordinates in system memory — the aliasing partner a bare-address key would
/// have merged with [`vid`].
fn sys(base: u64, len: u64) -> GpgaRegion {
    GpgaRegion::new(Aperture::SysmemCoherent, base, len).expect("well-formed fixture region")
}

fn iso(n: u32) -> IsolateId {
    IsolateId::new(n, GpuId(0))
}

const K4: u64 = 0x1000;
const M1: u64 = 0x10_0000;

/// The base of the region every test builds around: 64 MiB into the framebuffer, which is
/// inside `fb_length` and clear of the low bring-up structures.
const BASE: u64 = 64 * M1;

/// ⚠ The guard on this file's whole fixture strategy: the numbers come from the chip row,
/// and if the row moves, this fails rather than the tests quietly measuring the wrong map.
#[test]
fn fixtures_come_from_the_real_chip_row() {
    let c = chip();
    assert_eq!(
        c.pramin_window.len, M1,
        "{}'s PRAMIN window is {:#x} bytes, not the 1 MiB every view fixture here assumes. \
         Re-derive the fixtures; do NOT edit this constant to match.",
        c.name, c.pramin_window.len
    );
    // The widest fixture is `t3`'s window: one page per queued update, plus slack.
    assert!(
        BASE + (MAX_PENDING_UPDATES as u64 + 64) * K4 < c.fb_length,
        "the fixture working set must fit inside {}'s framebuffer",
        c.name
    );
    // The trap, stated: 0x0077_7777 is a PRAMIN *register-aperture* offset. It is not a
    // GPGA address and nothing here may use it as one.
    assert!(
        c.pramin_window.contains(0x0077_7777),
        "0x0077_7777 must still be inside PRAMIN — this assertion is the reminder of why \
         no fixture in this file is a memorable hex constant"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────────────

/// Plan → apply, one step, refusing to hide either half's error.
fn change(ix: &mut ViewerIndex, c: ObjectChange) -> Result<Applied, ViewFault> {
    let plan = ix.plan(&c)?;
    ix.apply(&plan)
}

/// Allocate an object and return its id.
fn alloc(ix: &mut ViewerIndex, region: GpgaRegion, owner: IsolateId) -> ObjectId {
    change(ix, ObjectChange::Allocated { region, owner })
        .expect("allocation must land")
        .object
        .expect("an allocation mints an object")
}

/// Every `Shows` a viewer currently holds, as `(view_off, base, len, extent)`.
fn shows(ix: &ViewerIndex, v: ViewerId) -> Vec<(u64, u64, u64, HostExtent)> {
    ix.view_contents(v)
        .expect("view must be vouchable")
        .into_iter()
        .filter_map(|u| match u {
            ViewUpdate::Shows {
                view_off,
                region,
                occupant,
            } => Some((view_off, region.base, region.len, occupant.extent)),
            _ => None,
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 1 — updating GPGA updates ALL viewers
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **Test 1.** One allocation reaches every viewer that covers it — **at that viewer's
/// own offset**, and covering only the part that viewer actually sees.
///
/// Mean rather than happy path in three ways at once, because the cheap version of this
/// test (one object, one viewer, assert "got it") passes against a fan-out that hands
/// everybody viewer 0's answer:
///
/// - three viewers of **different kinds** (`PRAMIN`, `BAR2`, an isolate),
/// - each mapping the same GPGA at a **different view offset**, so a fan-out that reuses
///   one offset is caught,
/// - one of them covering only **half** the object, so a fan-out that reports the object
///   rather than the intersection is caught.
///
/// A fourth viewer covers a disjoint region and must receive **nothing** — the fan-out
/// being wide is only half the claim.
#[test]
fn t1_one_change_reaches_every_viewer_at_its_own_offset() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    let pramin = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let bar2 = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    let isolate = ix.add_view(ViewerKind::Isolate(owner));
    let elsewhere = ix.add_view(ViewerKind::Window(FbWindow::FbAperture));

    // Same GPGA, three windows, three different view offsets — the design's
    // "the same GPGA can be mapped in multiple windows".
    ix.map_into_view(pramin, vid(BASE, pramin_len()), 0)
        .expect("PRAMIN covers the whole working set at offset 0");
    ix.map_into_view(bar2, vid(BASE, pramin_len()), 0x2_0000)
        .expect("BAR2 covers the same GPGA at a different view offset");
    // Half coverage: the isolate sees only the first 8 KiB of the object.
    ix.map_into_view(isolate, vid(BASE, 2 * K4), 0x5000)
        .expect("the isolate covers half the object");
    // Disjoint: 1 MiB above the working set.
    ix.map_into_view(elsewhere, vid(BASE + 2 * M1, K4), 0)
        .expect("a disjoint window");

    let obj = vid(BASE, 4 * K4);
    let plan = ix
        .plan(&ObjectChange::Allocated { region: obj, owner })
        .expect("plan must succeed");

    assert_eq!(
        plan.fan_out(),
        3,
        "exactly the three viewers that cover the object — not the fourth, and not \
         one entry per page"
    );

    let applied = ix.apply(&plan).expect("apply must land");
    assert_eq!(applied.viewers_updated, 3);
    assert_eq!(applied.viewers_desynced, 0);

    // ★ Each viewer at ITS OWN offset, over the part IT sees.
    assert_eq!(
        ix.drain(pramin).unwrap(),
        vec![ViewUpdate::Shows {
            view_off: 0,
            region: obj,
            occupant: occ(&ix, pramin, obj),
        }],
        "PRAMIN sees the whole object at view offset 0"
    );
    let bar2_updates = ix.drain(bar2).unwrap();
    assert_eq!(bar2_updates.len(), 1);
    let ViewUpdate::Shows {
        view_off, region, ..
    } = bar2_updates[0]
    else {
        panic!("expected a Shows, got {:?}", bar2_updates[0])
    };
    assert_eq!(
        (view_off, region),
        (0x2_0000, obj),
        "BAR2 must see the same GPGA at ITS OWN view offset, not PRAMIN's"
    );

    let iso_updates = ix.drain(isolate).unwrap();
    let ViewUpdate::Shows {
        view_off, region, ..
    } = iso_updates[0]
    else {
        panic!("expected a Shows, got {:?}", iso_updates[0])
    };
    assert_eq!(
        (view_off, region.base, region.len),
        (0x5000, BASE, 2 * K4),
        "the isolate sees only the 8 KiB it covers — the INTERSECTION, not the object"
    );

    assert!(
        ix.drain(elsewhere).unwrap().is_empty(),
        "a viewer that does not cover the region must receive NOTHING"
    );
}

/// The occupant a viewer sees over `region`, for building an expected `Shows`.
fn occ(ix: &ViewerIndex, _v: ViewerId, region: GpgaRegion) -> kayfabe_mmu::gpga::Occupant {
    ix.contents(region)[0]
        .occupant
        .expect("the region is occupied")
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 2 — a NEW view gets everything it already contains
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **Test 2 — the most-forgotten direction.** A view registered *after* the objects
/// exist must be seeded with them, not merely subscribed to what happens next.
///
/// Mean: five objects laid out so that a seed which is anything other than *exactly the
/// intersection* is visible —
///
/// | object | relation to the window |
/// |---|---|
/// | A | entirely **below** it — must not appear |
/// | B | **straddles the start** — must appear, clipped |
/// | C | wholly **inside** — must appear whole |
/// | D | **straddles the end** — must appear, clipped |
/// | E | entirely **above** it — must not appear |
///
/// and both routes are checked: the value `map_into_view` returns **and** what the viewer
/// later drains, because a seed delivered by only one of the two is a seed a caller can
/// miss by choosing the other.
#[test]
fn t2_a_new_view_is_seeded_with_the_objects_already_under_it() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    // Window: [BASE, BASE + 1 MiB).
    let win = vid(BASE, M1);

    let _a = alloc(&mut ix, vid(BASE - 2 * K4, K4), owner);
    let b = alloc(&mut ix, vid(BASE - K4, 2 * K4), owner);
    let c = alloc(&mut ix, vid(BASE + 0x1000, K4), owner);
    let d = alloc(&mut ix, vid(BASE + M1 - K4, 2 * K4), owner);
    let _e = alloc(&mut ix, vid(BASE + M1 + 2 * K4, K4), owner);
    assert_eq!(ix.object_count(), 5);

    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let seed = ix.map_into_view(v, win, 0).expect("mapping must land");

    let seen: Vec<(u64, u64, u64, ObjectId)> = seed
        .iter()
        .map(|u| match *u {
            ViewUpdate::Shows {
                view_off,
                region,
                occupant,
            } => (view_off, region.base, region.len, occupant.object),
            ref other => panic!("a seed carries only Shows, got {other:?}"),
        })
        .collect();

    assert_eq!(
        seen,
        vec![
            // B, clipped to the window's first byte.
            (0, BASE, K4, b),
            // C, whole.
            (0x1000, BASE + 0x1000, K4, c),
            // D, clipped at the window's end.
            (M1 - K4, BASE + M1 - K4, K4, d),
        ],
        "the seed must be exactly the intersection: A and E are outside, B and D are \
         clipped, C is whole"
    );

    assert_eq!(
        ix.drain(v).unwrap(),
        seed,
        "a viewer that drains rather than reading the return value must get the same \
         picture — otherwise a caller can be seeded by only one of the two routes"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 3 — a HANGING viewer must not wedge the update
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **Test 3.** A viewer that never drains delays nobody, loses nothing silently, and can
/// still be rebuilt.
///
/// The design resolves this by making delivery a **pull**: `apply` enqueues and never calls
/// into a viewer. So the claim has three parts and the cheap test only checks the first:
///
/// 1. every `apply` still returns `Ok` while the hanging viewer's queue is full;
/// 2. the **live** viewer's updates stay exact right through the overflow — a queue-full
///    condition must not corrupt a neighbour's stream;
/// 3. the hanging viewer is [`ViewState::Desynced`] **by name** and it is **counted** in
///    [`Applied::viewers_desynced`] — a dropped update with no name is the stale view this
///    whole module exists to prevent — and it can still rebuild from the index, which
///    stayed authoritative the whole time.
#[test]
fn t3_a_viewer_that_never_drains_wedges_nobody_and_is_named_desynced() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    let hanging = ix.add_view(ViewerKind::Isolate(owner));
    let live = ix.add_view(ViewerKind::Window(FbWindow::Pramin));

    // Enough allocations to overflow one queue by a clear margin — and a window sized to
    // cover all of them, so that "the viewer was not reached" can only mean the fan-out
    // stopped, never that the object fell outside the window.
    let n = MAX_PENDING_UPDATES + 16;
    let win = vid(BASE, (n as u64 + 1) * K4);
    ix.map_into_view(hanging, win, 0).unwrap();
    ix.map_into_view(live, win, 0).unwrap();
    let mut desynced_at = None;
    for i in 0..n {
        let region = vid(BASE + (i as u64) * K4, K4);
        let applied =
            change(&mut ix, ObjectChange::Allocated { region, owner }).unwrap_or_else(|e| {
                panic!("apply {i} must not be blocked by the hanging viewer: {e:?}")
            });
        assert_eq!(applied.viewers_updated, 2, "both viewers are still reached");
        if applied.viewers_desynced > 0 && desynced_at.is_none() {
            desynced_at = Some(i);
        }
        // (2) The live viewer's stream stays exact — drained every round.
        let got = ix.drain(live).unwrap();
        assert_eq!(
            got.len(),
            1,
            "round {i}: the live viewer gets exactly one update"
        );
        let ViewUpdate::Shows { region: r, .. } = got[0] else {
            panic!("round {i}: expected Shows, got {:?}", got[0])
        };
        assert_eq!(
            r, region,
            "round {i}: the live viewer's update is for THIS object"
        );
        assert_eq!(
            ix.viewer_state(live).unwrap(),
            ViewState::Live,
            "round {i}: a neighbour's full queue must not desync the live viewer"
        );
    }

    // (3) Named, counted, and bounded.
    assert_eq!(
        desynced_at,
        Some(MAX_PENDING_UPDATES),
        "the desync must be COUNTED, exactly once, at the update that overflowed"
    );
    assert_eq!(
        ix.viewer_state(hanging).unwrap(),
        ViewState::Desynced,
        "the hanging viewer must say so BY NAME, not merely have lost updates"
    );
    assert_eq!(
        ix.pending_len(hanging).unwrap(),
        MAX_PENDING_UPDATES,
        "the queue is BOUNDED — a viewer that never reads is not a memory-exhaustion path"
    );

    // The index stayed authoritative: the hanging viewer can rebuild completely.
    let rebuilt = shows(&ix, hanging);
    assert_eq!(
        rebuilt.len(),
        n,
        "a Desynced view rebuilds from the INDEX, which never stopped being correct"
    );
    ix.resynced(hanging).unwrap();
    assert_eq!(ix.viewer_state(hanging).unwrap(), ViewState::Live);
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 4 — a RACE: a view update in the midst of an allocation
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ **Test 4 — the hardest one.** A view registered between an allocation's plan and its
/// apply must not produce a half-applied fan-out.
///
/// This is the shape the lock constraint forces: the plan is built at rank 0, the issuing
/// act phase holds a rank-1 lock, and R3 forbids a second one — so the plan crosses a lock
/// boundary and the world can move under it. R5's answer is *re-validate after re-lock*,
/// and here that is [`ViewFault::PlanStale`].
///
/// Mean: the cheap version asserts the second apply errors. This asserts the **whole**
/// property —
///
/// - the refused apply left the index **byte-for-byte unchanged**: no object minted, no
///   viewer enqueued to, generation not advanced by it;
/// - the *new* viewer would have been **missed** had the plan been honoured, which is what
///   makes the refusal load-bearing rather than pedantic;
/// - re-planning succeeds and reaches **both** viewers;
/// - and the symmetric case: two plans built at one generation, the first applies, the
///   second is stale.
#[test]
fn t4_a_view_registered_between_plan_and_apply_refuses_the_plan_whole() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);
    let early = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let win = vid(BASE, M1);
    ix.map_into_view(early, win, 0).unwrap();

    let obj = vid(BASE, 4 * K4);
    let plan = ix
        .plan(&ObjectChange::Allocated { region: obj, owner })
        .expect("plan against the world as it is");
    assert_eq!(plan.fan_out(), 1, "one viewer exists at plan time");

    // ── The race: a view update lands in the middle of the allocation. ───────────────
    let late = ix.add_view(ViewerKind::Isolate(owner));
    ix.map_into_view(late, win, 0x8000).unwrap();

    let before_objects = ix.object_count();
    let before_generation = ix.generation();
    let err = ix.apply(&plan).expect_err("a stale plan must be refused");
    assert!(
        matches!(err, ViewFault::PlanStale { planned_at, now }
                 if planned_at == plan.planned_at() && now == before_generation),
        "the refusal must name BOTH generations so a retry can tell what moved: {err:?}"
    );

    // Refused WHOLE — nothing moved.
    assert_eq!(ix.object_count(), before_objects, "no object was minted");
    assert_eq!(
        ix.generation(),
        before_generation,
        "the index did not advance"
    );
    assert!(
        ix.drain(early).unwrap().is_empty(),
        "no viewer was enqueued to"
    );
    assert!(ix.drain(late).unwrap().is_empty());

    // ★ Why the refusal matters: the stale plan knew nothing about `late`.
    assert_eq!(
        plan.deliveries_for(late),
        &[],
        "the stale plan would have skipped the new viewer entirely — that is the \
         half-applied fan-out PlanStale exists to prevent"
    );

    // Retry: the same change, re-planned, now reaches both.
    let plan2 = ix
        .plan(&ObjectChange::Allocated { region: obj, owner })
        .expect("re-plan");
    assert_eq!(
        plan2.fan_out(),
        2,
        "the retry sees the world that actually exists"
    );
    assert_eq!(ix.apply(&plan2).unwrap().viewers_updated, 2);
    assert_eq!(shows(&ix, early).len(), 1);
    assert_eq!(shows(&ix, late).len(), 1);

    // ── The symmetric race: two plans at one generation, only one may land. ──────────
    let p1 = ix
        .plan(&ObjectChange::Allocated {
            region: vid(BASE + M1 / 2, K4),
            owner,
        })
        .unwrap();
    let p2 = ix
        .plan(&ObjectChange::Allocated {
            region: vid(BASE + M1 / 2 + K4, K4),
            owner,
        })
        .unwrap();
    assert_eq!(p1.planned_at(), p2.planned_at());
    ix.apply(&p1).expect("the first plan lands");
    assert!(
        matches!(ix.apply(&p2), Err(ViewFault::PlanStale { .. })),
        "the second plan was built against a world that no longer exists — even though \
         the two allocations do not overlap. Coarse ON PURPOSE: deciding which changes \
         commute is how a half-applied fan-out gets built."
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 5 — SLICING at the edge of a window
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **Test 5.** Partial coverage is expressed as an **ownership regime**
/// ([`HostExtent`]), not as a bare offset — reusing the type `b9500bf` introduced when a
/// bare `offset == 0` turned out to be ambiguous between *"the sole owner of a small
/// object"* and *"the first slice of a large arena"*, and a live double free came of it.
///
/// Mean: four geometries in one index, so a slicer that returns a constant is caught in
/// every direction —
///
/// | geometry | expected |
/// |---|---|
/// | object exactly equals the window | `Whole` |
/// | object straddles the window's **start** | `Slice` at a **non-zero** object offset |
/// | object straddles the window's **end** | `Slice` at object offset **0** |
/// | object **contains** the window (both edges) | `Slice` in the middle |
///
/// ★ The third row is the one that matters most: its slice offset is `0`, so a
/// `Whole`-vs-`Slice` decision made by *"is the offset zero"* rather than by *"is this the
/// whole object"* gets it wrong — which is exactly the ambiguity `HostExtent` exists to
/// remove.
#[test]
fn t5_a_window_edge_slices_the_object_as_an_ownership_regime() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);
    let win_base = BASE + M1;
    let win = vid(win_base, M1);

    // (a) exactly the window.
    alloc(&mut ix, win, owner);
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, win, 0).unwrap();
    assert_eq!(
        shows(&ix, v),
        vec![(0, win_base, M1, HostExtent::Whole)],
        "an object that IS the window is Whole"
    );

    // (b) straddling the start: object [win_base - 4K, win_base + 4K).
    let mut ix = ViewerIndex::new();
    alloc(&mut ix, vid(win_base - K4, 2 * K4), owner);
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, win, 0).unwrap();
    let got = shows(&ix, v);
    assert_eq!(got.len(), 1);
    let HostExtent::Slice(s) = got[0].3 else {
        panic!(
            "a partly-covered object must be a Slice, got {:?}",
            got[0].3
        )
    };
    assert_eq!(
        (s.offset(), s.len()),
        (K4, K4),
        "the window sees the object's SECOND 4 KiB — offset into the OBJECT, not into \
         the window"
    );

    // (c) straddling the end: object [win_base + 1M - 4K, ... + 4K).
    let mut ix = ViewerIndex::new();
    alloc(&mut ix, vid(win_base + M1 - K4, 2 * K4), owner);
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, win, 0).unwrap();
    let got = shows(&ix, v);
    let HostExtent::Slice(s) = got[0].3 else {
        panic!("expected a Slice, got {:?}", got[0].3)
    };
    assert_eq!(
        (s.offset(), s.len()),
        (0, K4),
        "★ offset ZERO and yet a Slice — the object outlives this view of it. A \
         `offset == 0 => Whole` shortcut fails exactly here, which is why the regime is \
         in the type."
    );

    // (d) the window is inside the object — sliced at BOTH edges.
    let mut ix = ViewerIndex::new();
    alloc(&mut ix, vid(win_base - M1, 3 * M1), owner);
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, win, 0).unwrap();
    let got = shows(&ix, v);
    let HostExtent::Slice(s) = got[0].3 else {
        panic!("expected a Slice, got {:?}", got[0].3)
    };
    assert_eq!(
        (s.offset(), s.len()),
        (M1, M1),
        "the window is the object's middle megabyte"
    );

    // And the same object seen by a viewer that covers ALL of it is Whole — the regime is
    // a property of the VIEW, not of the object.
    let all = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    ix.map_into_view(all, vid(win_base - M1, 3 * M1), 0)
        .unwrap();
    assert_eq!(
        shows(&ix, all)[0].3,
        HostExtent::Whole,
        "one object, two viewers, two regimes — Whole for the one that covers it all"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 6 — MEAN: the aperture is half the key, and hostile input refuses by name
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **Test 6 — correction 1, executed.** `(Aperture, address)` is the key, and a
/// bare-address key would alias vidmem offset `X` with sysmem offset `X`.
///
/// This is the identity/uniqueness defect family that produced four bugs in one day here,
/// and the same shape as the client-aliasing bug measured on an RTX 3060 / 580.159.04
/// and written down in `l1_concurrency.md` §12.27. The test builds the *exact*
/// collision a bare key would suffer — identical coordinates, different apertures — and
/// asserts every consequence separately, because each one is a different way to lose the
/// aperture:
///
/// 1. two objects may coexist at the same numeric address in different apertures
///    (a bare key would have refused the second as an overlap);
/// 2. a region query in one aperture does not see the other's object;
/// 3. a viewer covering one aperture is not in `viewers_of` for the other;
/// 4. the fan-out of a change in one aperture does not reach the other's viewer;
/// 5. and freeing one does not disturb the other.
#[test]
fn t6_the_aperture_is_half_the_key_and_a_bare_address_would_alias() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    // Deliberately IDENTICAL coordinates. A bare-address key merges these two.
    let v_region = vid(BASE, 4 * K4);
    let s_region = sys(BASE, 4 * K4);
    assert_eq!((v_region.base, v_region.len), (s_region.base, s_region.len));
    assert_ne!(v_region.aperture, s_region.aperture);
    assert_eq!(
        v_region.intersect(s_region),
        None,
        "two regions with identical coordinates in different apertures DO NOT intersect \
         — they are not the same memory"
    );

    // (1) Both coexist.
    let v_obj = alloc(&mut ix, v_region, owner);
    let s_obj = alloc(&mut ix, s_region, owner);
    assert_ne!(v_obj, s_obj);
    assert_eq!(
        ix.object_count(),
        2,
        "a bare-address key would have refused the second"
    );

    // (2) A region query is aperture-scoped.
    assert_eq!(ix.contents(v_region)[0].occupant.unwrap().object, v_obj);
    assert_eq!(ix.contents(s_region)[0].occupant.unwrap().object, s_obj);

    // (3) and (4) Viewers are aperture-scoped too.
    let v_view = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let s_view = ix.add_view(ViewerKind::Isolate(owner));
    ix.map_into_view(v_view, vid(BASE, M1), 0).unwrap();
    ix.map_into_view(s_view, sys(BASE, M1), 0).unwrap();
    let _ = ix.drain(v_view);
    let _ = ix.drain(s_view);

    let seers = ix.viewers_of(v_region);
    assert_eq!(
        seers.len(),
        1,
        "only the vidmem viewer sees the vidmem region"
    );
    assert_eq!(seers[0].viewer, v_view);

    let applied = change(
        &mut ix,
        ObjectChange::Allocated {
            region: vid(BASE + M1 / 2, K4),
            owner,
        },
    )
    .unwrap();
    assert_eq!(applied.viewers_updated, 1);
    assert!(
        ix.drain(s_view).unwrap().is_empty(),
        "a vidmem allocation must not reach a sysmem viewer"
    );

    // (5) Freeing one leaves the other alone.
    change(&mut ix, ObjectChange::Freed { object: v_obj }).unwrap();
    assert_eq!(
        ix.contents(s_region)[0].occupant.unwrap().object,
        s_obj,
        "freeing the vidmem object must not free its sysmem alias"
    );
}

/// ★ **Test 6b — hostile and malformed input refuses by name and never panics.**
///
/// Boundary-1 posture: every one of these coordinates is guest-influenceable, so the
/// answer is a typed refusal, never an arithmetic panic and never a silent clamp.
#[test]
fn t6b_hostile_and_malformed_input_refuses_by_name_and_never_panics() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    // Malformed regions, refused at construction.
    assert_eq!(
        GpgaRegion::new(Aperture::Vidmem, BASE, 0),
        Err(RegionError::Empty),
        "a region of no bytes is not a region"
    );
    assert_eq!(
        GpgaRegion::new(Aperture::Vidmem, u64::MAX - 3, 16),
        Err(RegionError::Wraps),
        "a wrapping region refuses; it must not wrap round to address 0"
    );

    // ★ A range ending EXACTLY at 2^64 is refused, consistently with
    // `IntervalMap::insert` and `HostSlice::new`, which both use `checked_add`. The
    // topmost byte of the space is therefore not addressable by a region — a real, tiny
    // limit, named here rather than discovered by whoever first maps the top page.
    assert_eq!(
        GpgaRegion::new(Aperture::Vidmem, u64::MAX - K4 + 1, K4),
        Err(RegionError::Wraps),
        "ending exactly at 2^64 is refused — same rule as IntervalMap and HostSlice"
    );

    // A query at the very top of the addressable space CLIPS rather than wraps.
    let top = GpgaRegion::new(Aperture::Vidmem, u64::MAX - K4, K4).unwrap();
    let spans = ix.contents(top);
    assert_eq!(spans.len(), 1, "an empty index answers one null-page span");
    assert!(
        spans[0].occupant.is_none(),
        "a hole is a null page, not a miss"
    );
    assert_eq!(spans[0].region, top, "and the partition is TOTAL");

    // Unknown ids refuse by name.
    assert_eq!(
        ix.viewer_kind(ViewerId(9999)),
        Err(ViewFault::UnknownViewer(ViewerId(9999)))
    );
    assert_eq!(
        ix.plan(&ObjectChange::Freed {
            object: ObjectId(9999)
        }),
        Err(ViewFault::UnknownObject(ObjectId(9999)))
    );

    // Overlap is loud, and it is loud for a PARTIAL overlap too — the off-by-one case a
    // "same base?" check would wave through.
    alloc(&mut ix, vid(BASE, 4 * K4), owner);
    let overlap = vid(BASE + 3 * K4, 4 * K4);
    assert_eq!(
        ix.plan(&ObjectChange::Allocated {
            region: overlap,
            owner
        }),
        Err(ViewFault::ObjectOverlap { region: overlap }),
        "a partial overlap is an overlap"
    );
    // ...and an exactly-abutting allocation is NOT an overlap.
    assert!(
        change(
            &mut ix,
            ObjectChange::Allocated {
                region: vid(BASE + 4 * K4, K4),
                owner
            }
        )
        .is_ok(),
        "abutting is not overlapping — the boundary is half-open"
    );

    // A viewer may not alias the same GPGA at two of its own offsets.
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, vid(BASE, M1), 0).unwrap();
    let again = vid(BASE + M1 / 2, K4);
    assert!(
        matches!(
            ix.map_into_view(v, again, 0x9_0000),
            Err(ViewFault::SelfAlias { viewer, .. }) if viewer == v
        ),
        "a self-alias makes the view's own coverage ambiguous — refused by name"
    );
    // ...but ANOTHER viewer may map the very same GPGA. That is the design.
    let w = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    assert!(
        ix.map_into_view(w, again, 0).is_ok(),
        "the same GPGA in multiple windows is the whole point"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 7 — ★★ THE DUAL: no viewer sees what it must not
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ **Test 7 — the security direction.** Every other test says *"everyone gets
/// updated"*; this one says *"and nobody else does"*. It is the one most worth having,
/// because a security audit found there is **no tenant axis in this system at all** — this
/// is that axis, at this layer.
///
/// Four ways a viewer could see what it must not, each refused by a different mechanism:
///
/// 1. **cross-isolate** — isolate B's view must not be seeded with isolate A's object, and
///    must not be in A's fan-out. `ViewFault::ForeignObject`, at **both** entrances.
/// 2. **out of coverage** — a viewer whose coverage merely *abuts* the region gets
///    nothing. The off-by-one a `<=` would let through.
/// 3. **wrong aperture** — a vidmem viewer must not see a sysmem object at the same
///    numeric address.
/// 4. **after removal** — a removed viewer is in no fan-out at all.
///
/// ★ And the deliberate asymmetry is asserted, not assumed: a **window** may see any of
/// its own VM's objects regardless of which isolate owns them, because a window is the
/// *guest's* and the guest kernel is the intra-VM authority. If that ever stops being
/// true, this assertion is where it breaks.
#[test]
fn t7_the_dual_no_viewer_sees_what_it_must_not() {
    let mut ix = ViewerIndex::new();
    let a = iso(1);
    let b = iso(2);

    let a_obj_region = vid(BASE, 4 * K4);
    let a_obj = alloc(&mut ix, a_obj_region, a);

    // (1a) Seeding B's view over A's object is refused.
    let b_view = ix.add_view(ViewerKind::Isolate(b));
    let err = ix
        .map_into_view(b_view, vid(BASE, M1), 0)
        .expect_err("isolate B must not be seeded with isolate A's object");
    assert!(
        matches!(err, ViewFault::ForeignObject { object, object_owner, viewer, viewer_owner }
                 if object == a_obj && object_owner == a && viewer == b_view && viewer_owner == b),
        "the refusal must name all four parties — which object, whose, and for whom: {err:?}"
    );
    assert!(
        ix.viewers_of(a_obj_region).is_empty(),
        "the refused mapping must not have half-landed"
    );

    // (1b) And the other entrance: B's view exists first, A allocates into it.
    let mut ix = ViewerIndex::new();
    let b_view = ix.add_view(ViewerKind::Isolate(b));
    ix.map_into_view(b_view, vid(BASE, M1), 0).unwrap();
    let err = ix
        .plan(&ObjectChange::Allocated {
            region: a_obj_region,
            owner: a,
        })
        .expect_err("A's allocation must not become visible in B's view");
    assert!(
        matches!(err, ViewFault::ForeignObject { object_owner, viewer_owner, .. }
                 if object_owner == a && viewer_owner == b),
        "{err:?}"
    );
    assert_eq!(ix.object_count(), 0, "and nothing was allocated");

    // ★ The asymmetry: a WINDOW may see it. A window is the guest's, and the guest kernel
    // is the intra-VM authority for rights.
    let win = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(win, vid(BASE + M1, M1), 0).unwrap();
    change(
        &mut ix,
        ObjectChange::Allocated {
            region: vid(BASE + M1, K4),
            owner: a,
        },
    )
    .expect("a window is not an isolate and does not carry the cross-tenant refusal");
    assert_eq!(shows(&ix, win).len(), 1);

    // (2) Out of coverage — ABUTTING, the off-by-one.
    let mut ix = ViewerIndex::new();
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, vid(BASE, 4 * K4), 0).unwrap();
    let _ = ix.drain(v);
    let just_after = vid(BASE + 4 * K4, K4);
    assert!(
        ix.viewers_of(just_after).is_empty(),
        "coverage is half-open: a viewer ending exactly at the region's base sees NOTHING \
         of it"
    );
    let applied = change(
        &mut ix,
        ObjectChange::Allocated {
            region: just_after,
            owner: a,
        },
    )
    .unwrap();
    assert_eq!(applied.viewers_updated, 0);
    assert!(ix.drain(v).unwrap().is_empty());
    // ...and the byte before the end IS covered, so the bound is exact and not merely far.
    assert_eq!(
        ix.viewers_of(vid(BASE + 4 * K4 - 1, 1)).len(),
        1,
        "the last covered byte must still be covered — otherwise the test above passes \
         for the wrong reason"
    );

    // (3) Wrong aperture, same numeric address.
    assert!(
        ix.viewers_of(sys(BASE, 4 * K4)).is_empty(),
        "a vidmem viewer must not appear for a sysmem region at the same address"
    );

    // (4) After removal, a viewer is in no fan-out.
    ix.remove_view(v).unwrap();
    assert_eq!(ix.viewers_of(vid(BASE, 4 * K4)).len(), 0);
    assert_eq!(
        ix.viewer_kind(v),
        Err(ViewFault::UnknownViewer(v)),
        "and it is gone by name"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// 8 — ★★ DEALLOCATION: every viewer loses it
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ **Test 8 — where a stale view becomes a use-after-free**, which this repository has
/// already had once (`b9500bf`: a teardown queued the same handle once per slice).
///
/// Three claims, and the third is the one a cheap test misses:
///
/// 1. every covering viewer is told, with the right region and the right view offset;
/// 2. the object is gone from the index, so it is gone from **`view_contents`**;
/// 3. ★★★ **including for the viewer that never drained.** The queue is a notification;
///    the **index** is the authority. A viewer cannot keep resolving a freed object by
///    refusing to read its mail.
///
/// Plus the identity half: a new object in the freed region gets a **different**
/// [`ObjectId`], so a stale id sitting in an undrained queue can never be confused for the
/// new occupant — the recycled-range ABA, defeated by never reusing an id.
///
/// And a remap is checked as the same shape: revoke where it was, show where it went.
#[test]
fn t8_a_free_reaches_every_viewer_including_the_one_that_never_read() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);

    let drainer = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let sleeper = ix.add_view(ViewerKind::Isolate(owner));
    let partial = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));

    let obj_region = vid(BASE, 4 * K4);
    ix.map_into_view(drainer, vid(BASE, M1), 0).unwrap();
    ix.map_into_view(sleeper, vid(BASE, M1), 0x1_0000).unwrap();
    // `partial` covers only the object's last 4 KiB.
    ix.map_into_view(partial, vid(BASE + 3 * K4, K4), 0x40)
        .unwrap();

    let obj = alloc(&mut ix, obj_region, owner);
    let _ = ix.drain(drainer);
    let _ = ix.drain(partial);
    // `sleeper` deliberately never drains.
    assert_eq!(ix.pending_len(sleeper).unwrap(), 1);

    let applied = change(&mut ix, ObjectChange::Freed { object: obj }).unwrap();
    assert_eq!(applied.viewers_updated, 3, "every covering viewer is told");

    // (1) Told correctly, each at its own offset and over its own sub-range.
    assert_eq!(
        ix.drain(drainer).unwrap(),
        vec![ViewUpdate::Revoked {
            view_off: 0,
            region: obj_region,
            object: obj,
        }]
    );
    let p = ix.drain(partial).unwrap();
    assert_eq!(
        p,
        vec![ViewUpdate::Revoked {
            view_off: 0x40,
            region: vid(BASE + 3 * K4, K4),
            object: obj,
        }],
        "the partial viewer is revoked over the part it SAW, at its own offset"
    );

    // (2) and (3) The index is the authority — for all three, drained or not.
    for (name, v) in [
        ("the drainer", drainer),
        ("the sleeper", sleeper),
        ("the partial viewer", partial),
    ] {
        assert!(
            shows(&ix, v).is_empty(),
            "{name} must no longer resolve the freed object — a view that still does is \
             a use-after-free, and refusing to read your mail must not buy you one"
        );
    }
    assert!(
        ix.pending_len(sleeper).unwrap() >= 2,
        "the sleeper still has its undrained mail; the point is that the mail is not what \
         made it correct"
    );

    // The identity half: no id reuse.
    let again = alloc(&mut ix, obj_region, owner);
    assert_ne!(
        again, obj,
        "an ObjectId is never reused — otherwise the sleeper's stale Revoked would name \
         the NEW object"
    );
    assert_eq!(ix.contents(obj_region)[0].occupant.unwrap().object, again);

    // A remap is the same shape: revoked where it was, shown where it went.
    let to = vid(BASE + M1 / 2, 4 * K4);
    let _ = ix.drain(drainer);
    change(&mut ix, ObjectChange::Remapped { object: again, to }).unwrap();
    let updates = ix.drain(drainer).unwrap();
    assert_eq!(
        updates,
        vec![
            ViewUpdate::Revoked {
                view_off: 0,
                region: obj_region,
                object: again,
            },
            ViewUpdate::Shows {
                view_off: M1 / 2,
                region: to,
                occupant: ix.contents(to)[0].occupant.unwrap(),
            },
        ],
        "a remap revokes the old placement BEFORE showing the new one — the other order \
         leaves a window in which the view holds two placements of one object"
    );
    assert_eq!(
        ix.contents(obj_region)[0].occupant,
        None,
        "and the vacated region is a null page again"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Correction 3 — a view that could be stale says so BY NAME
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ **Correction 3, executed.** The witness closes on every CPU write path — measured at
/// `ee3c808`, `211 836/211 836` on the cold boot and `250 041/250 041` on the matmul, with
/// **zero** unwitnessed bytes — but it does **not** close for a framebuffer-to-framebuffer
/// copy-engine write, and ⊘ the C recorder cannot observe framebuffer accesses at all, so
/// no capture can say how often that happens.
///
/// So the rule is: a view whose content came through that transport is a **named refusal**
/// under miss = fault, never a silently stale view.
///
/// Mean: the unwitnessed run is a **sub-range** of what the viewer covers, so a check that
/// asks about the region's first byte rather than partitioning it would pass while the
/// stale bytes sit in the middle.
#[test]
fn an_unwitnessed_framebuffer_copy_makes_the_view_a_named_refusal() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);
    let win = vid(BASE, M1);
    let v = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    ix.map_into_view(v, win, 0).unwrap();
    alloc(&mut ix, win, owner);
    let _ = ix.drain(v);
    assert!(
        ix.view_contents(v).is_ok(),
        "witnessed content vouches fine"
    );

    // The stale run is in the MIDDLE of the coverage, not at its start.
    let stale = vid(BASE + M1 / 2, K4);
    let applied = change(
        &mut ix,
        ObjectChange::UnwitnessedWrite {
            region: stale,
            transport: kayfabe_mmu::gpga::UnwitnessedTransport::FbToFbCopyEngine,
        },
    )
    .unwrap();
    assert_eq!(applied.viewers_updated, 1, "the viewer is told, by name");
    assert_eq!(
        ix.drain(v).unwrap(),
        vec![ViewUpdate::Unwitnessed {
            view_off: M1 / 2,
            region: stale,
            transport: kayfabe_mmu::gpga::UnwitnessedTransport::FbToFbCopyEngine,
        }]
    );

    let err = ix
        .view_contents(v)
        .expect_err("a view containing unvouchable bytes must REFUSE, not answer");
    assert!(
        matches!(
            err,
            ViewFault::UnwitnessedContent {
                region,
                transport: kayfabe_mmu::gpga::UnwitnessedTransport::FbToFbCopyEngine,
            } if region == stale
        ),
        "the refusal must name the exact run and the transport: {err:?}"
    );

    // And a NEW view over the stale run is refused at registration, not merely on read.
    let w = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    assert!(
        matches!(
            ix.map_into_view(w, stale, 0),
            Err(ViewFault::UnwitnessedContent { .. })
        ),
        "a fresh view of unvouchable bytes is refused at its ENTRANCE"
    );

    // A view over a region the copy did not touch is unaffected — the refusal is scoped to
    // the run, not to the aperture.
    let clean = ix.add_view(ViewerKind::Window(FbWindow::FbAperture));
    ix.map_into_view(clean, vid(BASE, K4), 0)
        .expect("an untouched run still vouches");
    assert!(ix.view_contents(clean).is_ok());
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The governing rule, as a property
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **Never ask a single address where it belongs — always ask, per region, what it
/// contains.** The partition returned by `contents` is TOTAL, ascending, non-overlapping
/// and free of zero-length runs, over a landscape of objects and holes, for every region
/// including ones that start and end mid-object.
///
/// A partition that is not total is silently a dropped sub-range, which is the same defect
/// class as `#13 CE-DROP`: the answer is simply absent later, at an address that names
/// nothing.
#[test]
fn every_region_query_is_a_total_partition() {
    let mut ix = ViewerIndex::new();
    let owner = iso(1);
    for i in [0u64, 2, 3, 7, 8, 9] {
        alloc(&mut ix, vid(BASE + i * K4, K4), owner);
    }
    // Query straddling holes and objects, starting and ending MID-object.
    let q = vid(BASE + K4 / 2, 9 * K4);
    let spans = ix.contents(q);
    assert!(!spans.is_empty());
    let mut at = q.base;
    for s in &spans {
        assert_eq!(s.region.aperture, q.aperture);
        assert_eq!(s.region.base, at, "spans must be contiguous, with no gap");
        assert_ne!(s.region.len, 0, "no zero-length span");
        at = s.region.end();
    }
    assert_eq!(at, q.end(), "the partition must cover the region EXACTLY");

    // Adjacent spans never repeat the same answer — they are MAXIMAL runs.
    for w in spans.windows(2) {
        assert_ne!(
            (w[0].occupant.map(|o| o.object), w[0].witness),
            (w[1].occupant.map(|o| o.object), w[1].witness),
            "adjacent spans with the same answer should have been one run"
        );
    }
}
