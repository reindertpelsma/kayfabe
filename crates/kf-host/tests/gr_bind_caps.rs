//! Ordinary GR calls must not borrow root capabilities from a VMM worker.
use kf_host::grbind::ordinary_on;
use kf_linux_raw::capability::{ThreadCapOps, ThreadCaps};
use std::cell::Cell;
const ADMIN: u64 = (1 << 21) | (1 << 23) | (1 << 38);
struct Caps {
    state: Cell<ThreadCaps>,
    fail_read: bool,
    fail_clear: bool,
    fail_restore: bool,
}
impl Caps {
    fn root() -> Self {
        Self {
            state: Cell::new(ThreadCaps {
                effective: ADMIN | 1,
                permitted: ADMIN | 1,
                inheritable: 1,
            }),
            fail_read: false,
            fail_clear: false,
            fail_restore: false,
        }
    }
}
impl ThreadCapOps for Caps {
    fn get(&self) -> Result<ThreadCaps, i32> {
        if self.fail_read {
            Err(1)
        } else {
            Ok(self.state.get())
        }
    }
    fn set(&self, caps: ThreadCaps) -> Result<(), i32> {
        if self.fail_clear || (self.fail_restore && caps.effective & ADMIN == ADMIN) {
            return Err(1);
        }
        self.state.set(caps);
        Ok(())
    }
}
#[test]
fn query_and_successful_bind_run_without_admin_nice_or_perfmon() {
    let caps = Caps::root();
    let before = caps.state.get();
    assert_eq!(
        ordinary_on(&caps, || {
            assert_eq!(caps.state.get().effective & ADMIN, 0);
            assert_eq!(caps.state.get().permitted, before.permitted);
            Ok(4096u64)
        }),
        Ok(4096)
    );
    assert_eq!(caps.state.get(), before);
}
#[test]
fn native_refusal_is_returned_without_using_admin_nice_or_perfmon() {
    let caps = Caps::root();
    let before = caps.state.get();
    assert_eq!(
        ordinary_on(&caps, || {
            assert_eq!(caps.state.get().effective & ADMIN, 0);
            Err::<(), _>(0x1f)
        }),
        Err(0x1f)
    );
    assert_eq!(caps.state.get(), before);
}
#[test]
fn unreadable_caps_do_not_issue_a_native_query_or_bind() {
    let caps = Caps {
        fail_read: true,
        ..Caps::root()
    };
    let called = Cell::new(false);
    assert_eq!(
        ordinary_on(&caps, || {
            called.set(true);
            Ok(())
        }),
        Err(0x40)
    );
    assert!(!called.get());
}
#[test]
fn refused_cap_clear_does_not_issue_a_native_query_or_bind() {
    let caps = Caps {
        fail_clear: true,
        ..Caps::root()
    };
    let called = Cell::new(false);
    assert_eq!(
        ordinary_on(&caps, || {
            called.set(true);
            Ok(())
        }),
        Err(0x40)
    );
    assert!(!called.get());
}

#[test]
fn success_with_failed_restore_is_reported_as_uncertain_and_keeps_caps_reduced() {
    let caps = Caps {
        fail_restore: true,
        ..Caps::root()
    };
    let called = Cell::new(false);
    assert_eq!(
        ordinary_on(&caps, || {
            called.set(true);
            Ok(4096u64)
        }),
        Err(0x40)
    );
    assert!(called.get());
    assert_eq!(caps.state.get().effective & (1 << 21), 0);
}
#[test]
fn native_refusal_with_failed_restore_keeps_capability_failure_visible() {
    let caps = Caps {
        fail_restore: true,
        ..Caps::root()
    };
    let called = Cell::new(false);
    assert_eq!(
        ordinary_on(&caps, || {
            called.set(true);
            Err::<(), _>(0x1f)
        }),
        Err(0x40)
    );
    assert!(called.get());
    assert_eq!(caps.state.get().effective & (1 << 21), 0);
}
