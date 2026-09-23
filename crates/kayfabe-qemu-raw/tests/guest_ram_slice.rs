//! ★★★★★ §18 (w825) — the GPU-free half of guest RAM as ONE object: the bounds rule every
//! slice passes before an ioctl, and the coalescing that turns per-page rows into runs.
use kayfabe_qemu_raw::storemap::{StoreMapRefusal, coalesce_ram_rows, ram_slice_in_bounds};

const GIB2: u64 = 2 << 30;

#[test]
fn a_slice_inside_the_object_is_accepted() {
    assert!(ram_slice_in_bounds(0, 0x1000, GIB2).is_ok());
    assert!(ram_slice_in_bounds(GIB2 - 0x1000, 0x1000, GIB2).is_ok());
}

#[test]
fn a_slice_past_the_end_or_empty_or_overflowing_is_refused_by_name() {
    for (off, len) in [
        (GIB2 - 0x1000, 0x2000),
        (GIB2, 0x1000),
        (0, 0),
        (u64::MAX - 1, 0x1000),
    ] {
        assert!(
            matches!(
                ram_slice_in_bounds(off, len, GIB2),
                Err(StoreMapRefusal::OutOfRange { .. })
            ),
            "off={off:#x} len={len:#x} must be refused"
        );
    }
}

#[test]
fn rows_contiguous_in_va_and_file_merge_into_one_run() {
    let rows = [
        (0x10_0000, 0x5000, 0x1000),
        (0x10_1000, 0x6000, 0x1000),
        (0x10_2000, 0x7000, 0x1000),
    ];
    assert_eq!(coalesce_ram_rows(&rows), vec![(0x10_0000, 0x5000, 0x3000)]);
}

#[test]
fn a_file_discontinuity_splits_the_run_even_when_the_va_is_contiguous() {
    // The guest mapped consecutive VAs onto non-consecutive guest pages: two runs, never one
    // slice that would map the WRONG bytes at the second page.
    let rows = [(0x10_0000, 0x5000, 0x1000), (0x10_1000, 0x9000, 0x1000)];
    assert_eq!(
        coalesce_ram_rows(&rows),
        vec![(0x10_0000, 0x5000, 0x1000), (0x10_1000, 0x9000, 0x1000)]
    );
}

#[test]
fn a_va_gap_splits_and_zero_length_rows_vanish() {
    let rows = [
        (0x10_0000, 0x5000, 0x1000),
        (0x10_0000, 0x5000, 0),
        (0x10_3000, 0x6000, 0x1000),
    ];
    assert_eq!(
        coalesce_ram_rows(&rows),
        vec![(0x10_0000, 0x5000, 0x1000), (0x10_3000, 0x6000, 0x1000)]
    );
}
