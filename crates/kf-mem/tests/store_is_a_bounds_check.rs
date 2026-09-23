use kf_mem::*;

#[test]
fn a_slice_inside_the_object_is_its_own_offset() {
    let s = Store::new(HostToken(1), 11857 << 20);
    assert_eq!(s.slice(0, 4096), Ok(StoreOffset(0)));
    assert_eq!(s.slice((11857 << 20) - 4096, 4096), Ok(StoreOffset((11857 << 20) - 4096)));
}

#[test]
fn a_slice_past_the_end_or_wrapping_is_refused_by_name() {
    let s = Store::new(HostToken(1), 1 << 30);
    assert_eq!(s.slice(1 << 30, 1).unwrap_err().name(), "out_of_object");
    assert_eq!(s.slice((1 << 30) - 4095, 4096).unwrap_err().name(), "out_of_object");
    // ⊘ KNOWN-POSITIVE for the wrap: without checked_add this would pass.
    assert_eq!(s.slice(u64::MAX - 10, 4096).unwrap_err().name(), "out_of_object");
    assert_eq!(s.slice(0, 0).unwrap_err().name(), "store_zero_length");
}
