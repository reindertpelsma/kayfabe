//! Readback facts never author state or affect the channel result.
use kf_host::Channel;
use kf_qemu::tsgfacts::{self, Facts, Reader, Request};
use std::cell::Cell;
struct Host<'a> {
    calls: Cell<usize>,
    fail: bool,
    book: &'a Facts,
}
impl Reader for Host<'_> {
    fn timeslice(&self, _: Channel) -> Result<u64, u32> {
        assert!(!self.book.summary().contains("busy"));
        self.calls.set(self.calls.get() + 1);
        if self.fail { Err(0x56) } else { Ok(1001) }
    }
    fn modes(&self, _: Channel) -> Result<[u32; 5], u32> {
        assert!(!self.book.summary().contains("busy"));
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            Err(0x40)
        } else {
            Ok([1, 2, 0, 2, 1])
        }
    }
}
fn chan(g: u32) -> Channel {
    Channel {
        tsg: g,
        chan: g + 1,
        token: g + 2,
        born_user: None,
    }
}
fn host(b: &Facts) -> Host<'_> {
    Host {
        calls: Cell::new(0),
        fail: false,
        book: b,
    }
}
#[test]
fn default_off_and_exact_one_only() {
    for v in [None, Some("0"), Some("true"), Some("")] {
        assert!(!tsgfacts::flag_on(v));
    }
    assert!(tsgfacts::flag_on(Some("1")));
}
#[test]
fn disabled_has_no_native_reads_or_metadata() {
    let b = Facts::new(false, 2, 8);
    let h = host(&b);
    assert!(
        b.birth((1, 10), 2, chan(100), true, Some(400), &h)
            .is_none()
    );
    assert!(
        b.after(chan(100), "set", Request::Timeslice(1000), 0, &h)
            .is_none()
    );
    assert_eq!(h.calls.get(), 0);
    assert!(b.summary().contains("tsg_fact_groups=0"));
}
#[test]
fn birth_has_desired_generation_birth_and_both_getters() {
    let b = Facts::new(true, 2, 8);
    let h = host(&b);
    let s = b.birth((1, 10), 2, chan(100), true, Some(400), &h).unwrap();
    for field in [
        "phase=birth",
        "generation=1",
        "host_birth=1",
        "desired_us=Some(400)",
        "timeslice_get=Ok(1001)",
        "ctxsw_get=Ok([1, 2, 0, 2, 1])",
    ] {
        assert!(s.contains(field), "{s}");
    }
    assert_eq!(h.calls.get(), 2);
}
#[test]
fn mode_setter_logs_requested_modes_and_native_result() {
    let b = Facts::new(true, 2, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    let s = b
        .after(chan(100), "ctxsw-set", Request::Ctxsw(1, 0, 2), 0x40, &h)
        .unwrap();
    assert!(s.contains("ctxsw_requested=Some((1, 0, 2))"));
    assert!(s.contains("setter_status=0x40"));
    assert!(s.contains("ctxsw_get=Ok("));
}
#[test]
fn timeslice_setter_logs_failed_desire_and_readback_separately() {
    let b = Facts::new(true, 2, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    let s = b
        .after(
            chan(100),
            "timeslice-set",
            Request::Timeslice(400),
            0x1f,
            &h,
        )
        .unwrap();
    assert!(s.contains("desired_us=Some(400)"));
    assert!(s.contains("setter_status=0x1f"));
    assert!(s.contains("timeslice_get=Ok(1001)"));
}
#[test]
fn readback_refusals_are_facts_with_both_statuses() {
    let b = Facts::new(true, 2, 8);
    let h = Host {
        fail: true,
        ..host(&b)
    };
    let s = b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    assert!(s.contains("timeslice_get=Err(86)"));
    assert!(s.contains("ctxsw_get=Err(64)"));
}
#[test]
fn same_group_join_retains_generation_and_birth_identity() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    let s = b.birth((1, 10), 2, chan(100), false, None, &h).unwrap();
    assert!(s.contains("phase=bind"));
    assert!(s.contains("generation=1 host_birth=1"));
}
#[test]
fn shards_share_logical_generation_but_have_distinct_births() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    let s = b.birth((1, 10), 2, chan(200), true, None, &h).unwrap();
    assert!(s.contains("generation=1 host_birth=2"));
}
#[test]
fn free_reuse_has_a_new_logical_generation() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    b.forget(1, 10);
    let s = b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    assert!(s.contains("generation=2 host_birth=2"));
}
#[test]
fn device_and_client_free_remove_never_reused_group_facts() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    b.forget(1, 2);
    assert!(b.after(chan(100), "set", Request::None, 0, &h).is_none());
    b.birth((1, 11), 3, chan(200), true, None, &h).unwrap();
    b.forget(1, 1);
    assert!(b.after(chan(200), "set", Request::None, 0, &h).is_none());
}
#[test]
fn metadata_and_event_caps_suppress_native_reads() {
    let b = Facts::new(true, 1, 2);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    assert!(b.birth((1, 11), 2, chan(200), true, None, &h).is_none());
    b.after(chan(100), "bind", Request::Zcull(2), 0, &h)
        .unwrap();
    assert!(b.after(chan(100), "set", Request::None, 0, &h).is_none());
    assert_eq!(h.calls.get(), 4);
    assert!(b.summary().contains("tsg_fact_dropped=2"));
}
#[test]
fn native_free_and_reset_remove_records_without_host_calls() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    b.group_freed(100);
    assert!(b.after(chan(100), "set", Request::None, 0, &h).is_none());
    b.birth((1, 10), 2, chan(200), true, None, &h).unwrap();
    b.clear();
    assert!(b.after(chan(200), "set", Request::None, 0, &h).is_none());
    assert_eq!(h.calls.get(), 4);
}
#[test]
fn disabled_book_ignores_joiners_setters_and_native_failures() {
    let b = Facts::new(false, 3, 8);
    let h = Host {
        fail: true,
        ..host(&b)
    };
    assert!(b.birth((1, 10), 2, chan(100), true, None, &h).is_none());
    assert!(
        b.after(chan(100), "ctxsw-set", Request::Ctxsw(1, 0, 2), 0x1f, &h)
            .is_none()
    );
    assert_eq!(h.calls.get(), 0);
}
#[test]
fn all_explicit_members_keep_one_birth_identity() {
    let b = Facts::new(true, 3, 16);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    for _ in 0..4 {
        let s = b.birth((1, 10), 2, chan(100), false, None, &h).unwrap();
        assert!(s.contains("generation=1 host_birth=1"));
    }
}
#[test]
fn most_recent_failed_set_is_still_the_logged_guest_desire() {
    let b = Facts::new(true, 3, 8);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    b.after(chan(100), "set", Request::Timeslice(400), 0, &h)
        .unwrap();
    let s = b
        .after(chan(100), "set", Request::Timeslice(2048), 0x40, &h)
        .unwrap();
    assert!(s.contains("desired_us=Some(2048)"));
    assert!(s.contains("timeslice_get=Ok(1001)"));
}
#[test]
fn one_tsg_cannot_bypass_group_metadata_cap_by_creating_shards() {
    let b = Facts::new(true, 1, 16);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    assert!(b.birth((1, 10), 2, chan(200), true, None, &h).is_none());
    assert_eq!(h.calls.get(), 2);
}
#[test]
fn full_group_book_does_not_read_back_or_log_a_third_shard() {
    let b = Facts::new(true, 2, 16);
    let h = host(&b);
    for group in [100, 200] {
        b.birth((1, 10), 2, chan(group), true, None, &h).unwrap();
    }
    assert!(b.birth((1, 10), 2, chan(300), true, None, &h).is_none());
    assert_eq!(h.calls.get(), 4);
    assert!(b.summary().contains("tsg_fact_groups=2"));
}
#[test]
fn hostile_repeated_setters_stop_readbacks_at_the_event_bound() {
    let b = Facts::new(true, 2, 3);
    let h = host(&b);
    b.birth((1, 10), 2, chan(100), true, None, &h).unwrap();
    for _ in 0..2 {
        b.after(chan(100), "set", Request::Timeslice(400), 0, &h)
            .unwrap();
    }
    for _ in 0..32 {
        assert!(b.after(chan(100), "set", Request::None, 0, &h).is_none());
    }
    assert_eq!(h.calls.get(), 6);
    assert!(b.summary().contains("tsg_fact_dropped=32"));
}
