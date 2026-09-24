//! w826 — the v3 publisher's pure half: a walk's COMPLETE state vs our own handle ledger.
use kayfabe_qemu_raw::storemap::{Desired, plan_reconcile};

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
