//! OS clock perimeter fixtures; these do not prove invariant/synchronized TSC.
#![cfg(feature = "prof")]
#[test]
fn profiler_counter_is_monotonic_when_supported() {
    let Some(a) = kf_linux_raw::prof_clock::counter() else {return;};
    let b = kf_linux_raw::prof_clock::counter().unwrap();
    assert!(a > 0 && b >= a);
}
#[test]
fn profiler_clock_pair_is_bracketed_and_has_a_host_thread_identity() {
    let a = kf_linux_raw::prof_clock::pair().unwrap();
    assert!(a.monotonic_ns > 0);
    assert!(a.raw_ns > 0);
    assert!(a.before > 0 && a.after >= a.before);
    assert!(kf_linux_raw::prof_clock::thread_id() > 0);
}
