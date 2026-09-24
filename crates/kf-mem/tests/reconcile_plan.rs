//! w826 — the v3 publisher's pure half: a walk's COMPLETE state vs our own handle ledger.
use kf_mem::ledger::{Desired, plan_reconcile};

fn d(va: u64, len: u64, off: u64, ram: bool) -> Desired {
    Desired { va, len, off, ram }
}

#[test]
fn an_empty_ledger_maps_everything() {
    let p = plan_reconcile(&[], &[d(0x1000, 0x1000, 0x5000, true), d(0x20000, 0x10000, 0x0, false)]);
    assert!(p.unmap.is_empty());
    assert_eq!(p.map.len(), 2);
}

#[test]
fn an_unchanged_state_touches_nothing() {
    let led = [(0x1000, 0x2000, 0x5000, true), (0x20000, 0x10000, 0x0, false)];
    let p = plan_reconcile(&led, &[d(0x1000, 0x1000, 0x5000, true), d(0x2000, 0x1000, 0x6000, true), d(0x20000, 0x10000, 0x0, false)]);
    assert!(p.unmap.is_empty() && p.map.is_empty(), "{p:?}");
    assert_eq!(p.kept, 2);
}

#[test]
fn a_gone_or_repointed_row_is_unmapped_then_remapped() {
    let led = [(0x1000, 0x1000, 0x5000, true), (0x3000, 0x1000, 0x7000, true)];
    // 0x1000 re-pointed to other RAM; 0x3000 gone.
    let p = plan_reconcile(&led, &[d(0x1000, 0x1000, 0x9000, true)]);
    assert_eq!(p.unmap.len(), 2, "{p:?}");
    assert_eq!(p.map, vec![d(0x1000, 0x1000, 0x9000, true)]);
}

#[test]
fn a_run_that_grows_over_a_kept_slice_takes_it_down_first() {
    let led = [(0x10000, 0x1000, 0x40000, false)];
    let p = plan_reconcile(&led, &[d(0x10000, 0x10000, 0x40000, false)]);
    assert_eq!(p.unmap, vec![(0x10000, 0x1000)], "an overlapping FIXED map would be refused 0x51");
    assert_eq!(p.map.len(), 1);
}

#[test]
fn store_and_ram_never_back_each_other() {
    let led = [(0x1000, 0x1000, 0x5000, false)];
    let p = plan_reconcile(&led, &[d(0x1000, 0x1000, 0x5000, true)]);
    assert_eq!(p.unmap.len(), 1);
    assert_eq!(p.map.len(), 1);
}

// ── w826: server rows are whole pages and subordinate to the walk ───────────────────────────

use kf_mem::ledger::server_row_pieces;

#[test]
fn an_unaligned_promote_row_rounds_to_whole_pages() {
    // `[measured w826 m2]` 0x20409d000+0x8600 was refused by RM as a non-page multiple.
    assert_eq!(server_row_pieces(0x2_0409_d000, 0x8600, 0x40_d000, &[]), vec![(0x2_0409_d000, 0x9000, 0x40_d000)]);
}

#[test]
fn a_64k_aligned_row_takes_the_c_round_up() {
    assert_eq!(server_row_pieces(0x10_0000, 0x1234, 0x20_0000, &[]), vec![(0x10_0000, 0x1_0000, 0x20_0000)]);
}

#[test]
fn the_walk_wins_where_both_speak() {
    // A walked run in the middle of the row leaves only the two holes around it.
    let got = server_row_pieces(0x10_0000, 0x1_0000, 0x20_0000, &[(0x10_4000, 0x2000)]);
    assert_eq!(got, vec![(0x10_0000, 0x4000, 0x20_0000), (0x10_6000, 0xa000, 0x20_6000)]);
    // Fully covered ⇒ nothing, so the two never fight over one slice.
    assert!(server_row_pieces(0x10_0000, 0x1_0000, 0x20_0000, &[(0xf_0000, 0x3_0000)]).is_empty());
}

#[test]
fn a_row_whose_va_and_backing_disagree_inside_a_page_is_inexpressible() {
    assert!(server_row_pieces(0x10_0800, 0x100, 0x20_0000, &[]).is_empty());
    // Same in-page offset on both sides is expressible: it widens to the page.
    assert_eq!(server_row_pieces(0x10_0800, 0x100, 0x20_1800, &[]), vec![(0x10_0000, 0x1000, 0x20_1000)]);
}

mod leaves {
    use kf_mem::ledger::{AP_PEER, AP_SYS_COHERENT, AP_VIDMEM, Desired, LeafRefusal, desired_from_leaves};

    /// A layout with a hole: GPA [0, 3 GiB) is memfd [0, 3 GiB); GPA [4 GiB, 5 GiB) is memfd
    /// [3 GiB, 4 GiB). A leaf across the hole is not contiguous RAM.
    fn layout(gpa: u64, len: u64) -> Option<u64> {
        const G: u64 = 1 << 30;
        let end = gpa.checked_add(len)?;
        if end <= 3 * G {
            Some(gpa)
        } else if gpa >= 4 * G && end <= 5 * G {
            Some(gpa - G)
        } else {
            None
        }
    }

    #[test]
    fn leaves_are_classified_by_aperture_and_sysmem_goes_through_the_layout() {
        let got = desired_from_leaves(
            [(0x1000, 0x2000, 0x1000, AP_VIDMEM), (0x9000, (4 << 30) + 0x5000, 0x2000, AP_SYS_COHERENT)],
            1 << 20,
            &layout,
        )
        .unwrap();
        assert_eq!(got, vec![
            Desired { va: 0x1000, len: 0x1000, off: 0x2000, ram: false },
            Desired { va: 0x9000, len: 0x2000, off: (3 << 30) + 0x5000, ram: true },
        ]);
    }

    #[test]
    fn out_of_store_across_the_hole_and_peer_are_refused_by_name() {
        let r = |l| desired_from_leaves([l], 1 << 20, &layout);
        assert!(matches!(r((0, 0xF_F000, 0x2000, AP_VIDMEM)), Err(LeafRefusal::OutsideStore { .. })));
        assert!(matches!(r((0, (3 << 30) - 0x1000, 0x2000, AP_SYS_COHERENT)), Err(LeafRefusal::NotGuestRam { .. })));
        assert_eq!(r((0x5000, 0, 0x1000, AP_PEER)), Err(LeafRefusal::Aperture { va: 0x5000, ap: AP_PEER }));
    }
}
