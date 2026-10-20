#![cfg(feature = "prof")]
#[test]
fn profiler_counter_is_monotonic_when_supported() {
    if !kf_linux_raw::prof_clock::counter_available() {return;}
    let a=kf_linux_raw::prof_clock::counter().unwrap();let b=kf_linux_raw::prof_clock::counter().unwrap();assert!(b>=a);
}
#[test]
fn profiler_clock_pair_is_bracketed_and_has_a_host_thread_identity() {
    let a=kf_linux_raw::prof_clock::pair().unwrap();
    assert!(a.monotonic_ns>0);assert!(a.raw_ns>0);assert!(a.after>=a.before);
    assert!(kf_linux_raw::prof_clock::thread_id()>0);
}
