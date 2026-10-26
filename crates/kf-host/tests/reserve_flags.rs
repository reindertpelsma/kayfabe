//! The guest-range reservation flags (`docs/design/V3_SPARSE_PTE.md`, section 3).
//!
//! SPARSE is a property of a whole host reservation. Carried by the product's guest reservations it
//! would make every plain-invalid guest VA absorb accesses that real hardware faults, which the
//! containment negative controls (exact channel, exact VA, `FAULT_PTE`) rely on. It is therefore a
//! measurement-only experiment, off unless asked for.

use kf_host::channel::{NVOS32_ALLOC_FLAGS_SPARSE, guest_reserve_flags, sparse_resv_diag_from};

/// FIXED_ADDRESS_ALLOCATE | LAZY | VIRTUAL (`nvos.h:1448-1464`).
const PRODUCT_FLAGS: u32 = 0x0000_0010 | 0x0000_0400 | 0x0008_0000;

#[test]
fn the_flag_value_is_the_header_value() {
    assert_eq!(NVOS32_ALLOC_FLAGS_SPARSE, 0x0400_0000);
}

#[test]
fn the_product_reservation_is_not_sparse() {
    assert_eq!(guest_reserve_flags(false), PRODUCT_FLAGS);
    assert_eq!(guest_reserve_flags(false) & NVOS32_ALLOC_FLAGS_SPARSE, 0);
}

#[test]
fn the_experiment_adds_exactly_the_sparse_flag() {
    assert_eq!(
        guest_reserve_flags(true),
        PRODUCT_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE
    );
}

/// Only the value `1` turns the experiment on: unset, empty, `0` and anything else are off.
#[test]
fn the_experiment_is_off_unless_the_variable_is_one() {
    assert!(sparse_resv_diag_from(Some("1")));
    for v in [
        None,
        Some(""),
        Some("0"),
        Some("true"),
        Some("11"),
        Some(" 1"),
    ] {
        assert!(!sparse_resv_diag_from(v), "{v:?}");
    }
}
