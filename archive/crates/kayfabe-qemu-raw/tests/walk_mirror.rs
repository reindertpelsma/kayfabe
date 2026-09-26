//! w826 — the host mirror of the walk kernel's state, edited by deltas.
use kayfabe_qemu_raw::walkmirror::{DeltaOp, MRun, RunWhere, VasMirror};

fn v(va: u64, len: u64, gpga: u64) -> MRun {
    MRun { va, len, gpga, at: RunWhere::Vidmem }
}

#[test]
fn a_drop_inside_a_run_keeps_both_ends_with_their_backing() {
    let mut m = VasMirror::default();
    m.resync([(0, v(0x10_0000, 0x10_000, 0x50_0000))]);
    assert_eq!(m.apply(0, DeltaOp::Drop { va: 0x10_4000, len: 0x2000 }), Some((0x10_4000, 0x10_6000)));
    let mut all = m.all();
    all.sort_by_key(|r| r.va);
    assert_eq!(all, vec![v(0x10_0000, 0x4000, 0x50_0000), v(0x10_6000, 0xa000, 0x50_6000)]);
}

#[test]
fn a_put_replaces_what_it_covers_in_its_own_class_only() {
    let mut m = VasMirror::default();
    // 4 KiB class (0) and 64 KiB class (1) at the same VA — a dual PDE.
    m.resync([(0, v(0x20_0000, 0x1000, 0x1000)), (1, v(0x20_0000, 0x1_0000, 0x40_0000))]);
    m.apply(1, DeltaOp::Put(v(0x20_0000, 0x1_0000, 0x80_0000)));
    let mut all = m.all();
    all.sort_by_key(|r| (r.va, r.len));
    assert_eq!(all, vec![v(0x20_0000, 0x1000, 0x1000), v(0x20_0000, 0x1_0000, 0x80_0000)]);
    // Dropping the 64 KiB class leaves the 4 KiB run.
    m.apply(1, DeltaOp::Drop { va: 0x20_0000, len: 0x1_0000 });
    assert_eq!(m.all(), vec![v(0x20_0000, 0x1000, 0x1000)]);
}

#[test]
fn overlapping_finds_a_run_that_starts_before_the_range() {
    let mut m = VasMirror::default();
    m.resync([(0, v(0x1000, 0x3000, 0x9000)), (0, v(0x8000, 0x1000, 0xa000))]);
    assert_eq!(m.overlapping(0x2000, 0x2001), vec![v(0x1000, 0x3000, 0x9000)]);
    assert!(m.overlapping(0x4000, 0x8000).is_empty());
}

#[test]
fn a_resync_replaces_everything_and_counts_bad_classes() {
    let mut m = VasMirror::default();
    m.resync([(0, v(0x1000, 0x1000, 0))]);
    assert_eq!(m.resync([(0, v(0x5000, 0x1000, 0)), (9, v(0x6000, 0x1000, 0))]), 1);
    assert_eq!(m.all(), vec![v(0x5000, 0x1000, 0)]);
}
