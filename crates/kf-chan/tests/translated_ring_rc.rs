//! EXPECTED RED (product bug B1, owner 2026-10-22): an Xid 32 (PBDMA error) raised by the host on one of
//! kayfabe's own Translated rings must reach kayfabe's RC handling.
//!
//! Measured (`traces/blackwell_resume_20261021/`, GB202 Windows 580.88, 4 runs over two leases): the host
//! raised two Xid 32 on QEMU-owned channels about 3 s after the guest finished init, the guest then timed out
//! (first refused `RESET_CHANNEL` 2.08 s later), unloaded and bugchecked 0x116. kayfabe saw nothing:
//! Translated rings are born with `err_notifier: 0` (`kf-chan/src/host.rs`), so the host never writes an
//! error record kayfabe could read, and `dead=None` at retire.
//!
//! The seam under test is the pure decision a ring's watcher makes from its 16-byte host error record
//! (`NvNotification { timeStamp: 8, info32: 4, info16: 2, status: 2 }`, status is the high half of word 3 and
//! is written last; `ogkm-580 kernel_rc_notification.c:326-338`), the same rule the passthrough twins already
//! use in `kf-qemu` (`rc_scan`). Wiring it to a real ring (allocate the notifier object, pass it as
//! `hObjectError`, poll it, deliver `RC_TRIGGERED` with the guest chid) is hardware-verified follow-up work;
//! this file only pins the rule and fails until the seam exists.

use kf_chan::rc::{ObservedRc, RcWatch};

/// `ROBUST_CHANNEL_PBDMA_ERROR` (`ogkm-580 nverror.h`): the Xid number the host prints.
const XID_PBDMA_ERROR: u32 = 32;
/// `ROBUST_CHANNEL_PREEMPTIVE_REMOVAL` (`nverror.h:61`).
const PREEMPTIVE_REMOVAL: u32 = 45;

fn record(info32: u32, status: u16) -> [u32; 4] {
    [0x1111, 0x2222, info32, u32::from(status) << 16]
}

#[test]
fn an_xid32_record_written_by_the_host_is_observed_exactly_once() {
    let mut w = RcWatch::armed([0; 4]);
    assert_eq!(
        w.observe([0; 4]),
        None,
        "an untouched record is not an event"
    );
    let got = w.observe(record(XID_PBDMA_ERROR, 0xffff));
    assert_eq!(
        got,
        Some(ObservedRc {
            except_type: XID_PBDMA_ERROR
        })
    );
    assert_eq!(
        w.observe(record(XID_PBDMA_ERROR, 0xffff)),
        None,
        "one RC per ring life"
    );
}

#[test]
fn a_half_written_record_is_not_an_event_until_its_status_lands() {
    let mut w = RcWatch::armed([0; 4]);
    assert_eq!(
        w.observe(record(XID_PBDMA_ERROR, 0)),
        None,
        "status is written last"
    );
    assert_eq!(
        w.observe(record(XID_PBDMA_ERROR, 0xffff)),
        Some(ObservedRc {
            except_type: XID_PBDMA_ERROR
        })
    );
}

#[test]
fn a_guests_own_post_stop_write_is_rebaselined_never_reported() {
    // The guest's CPU-RM writes PREEMPTIVE_REMOVAL itself after an NV_OK STOP_CHANNEL (kchannelNotifyRc).
    let mut w = RcWatch::armed([0; 4]);
    w.expect_guest_stop_write();
    assert_eq!(w.observe(record(PREEMPTIVE_REMOVAL, 0xffff)), None);
    // ... but a later host-written record is still an event.
    assert_eq!(
        w.observe(record(XID_PBDMA_ERROR, 0xffff)),
        Some(ObservedRc {
            except_type: XID_PBDMA_ERROR
        })
    );
}

#[test]
fn a_record_that_was_already_nonzero_at_arming_is_a_baseline_not_an_event() {
    let armed = record(7, 0xffff);
    let mut w = RcWatch::armed(armed);
    assert_eq!(
        w.observe(armed),
        None,
        "a guest may initialise its notifier to anything"
    );
}

use kf_chan::rc::{RcDelivery, RcRoute, RoutedRc};

const ROUTE: RcRoute = RcRoute {
    chid: 0x0e00_0017,
    engine: 1,
    host_token: 0x2b,
};

fn publish(d: &mut RcDelivery, host: [u32; 4], guest: &mut [u32; 4]) -> Option<RoutedRc> {
    d.poll(host, |off, word| {
        guest[off as usize / 4] = word;
        Ok::<_, ()>(())
    })
    .unwrap()
}

#[test]
fn the_host_token_does_not_replace_the_guest_chid_or_guest_engine() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut guest = [0; 4];
    assert_eq!(
        publish(&mut d, record(32, 0xffff), &mut guest),
        Some(RoutedRc {
            route: ROUTE,
            except_type: 32
        })
    );
    assert!(d.failed(), "the channel must stop submitting after host RC");
}

#[test]
fn the_guest_notifier_contains_the_observed_xid_and_the_declared_engine() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut guest = [0; 4];
    let _ = publish(&mut d, record(32, 0xffff), &mut guest);
    assert_eq!(guest[2], 32);
    assert_eq!(
        guest[3], 0xffff_0001,
        "status and guest engine use the public layout"
    );
}

#[test]
fn a_guest_notifier_write_failure_keeps_one_pending_error_without_an_event() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut writes = Vec::new();
    let r = d.poll(record(32, 0xffff), |off, word| {
        writes.push((off, word));
        if off == 12 {
            Err("write refused")
        } else {
            Ok(())
        }
    });
    assert_eq!(r, Err("write refused"));
    assert!(
        d.failed(),
        "write failure cannot revive the faulted channel"
    );
    let mut guest = [0; 4];
    // Changed/cleared source cannot erase a previously observed host error.
    assert_eq!(
        publish(&mut d, [0; 4], &mut guest),
        Some(RoutedRc {
            route: ROUTE,
            except_type: 32
        })
    );
    assert_eq!(publish(&mut d, record(32, 0xffff), &mut guest), None);
}

#[test]
fn publication_writes_status_last_before_returning_the_event() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut writes = Vec::new();
    assert!(
        d.poll(record(32, 0xffff), |off, word| {
            writes.push((off, word));
            Ok::<_, ()>(())
        })
        .unwrap()
        .is_some()
    );
    assert_eq!(
        writes.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![0, 4, 8, 12]
    );
    assert_eq!(writes.last(), Some(&(12, 0xffff_0001)));
}

#[test]
fn repeated_wakes_do_not_republish_or_deliver_a_second_event() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut n = 0;
    let mut events = 0;
    for _ in 0..1000 {
        events += usize::from(
            d.poll(record(32, 0xffff), |_, _| {
                n += 1;
                Ok::<_, ()>(())
            })
            .unwrap()
            .is_some(),
        );
    }
    assert_eq!((events, n), (1, 4));
}

#[test]
fn unrelated_channels_and_their_notifiers_are_untouched() {
    let mut a = RcDelivery::armed(ROUTE, [0; 4]);
    let other = RcRoute {
        chid: 0x0e00_0018,
        host_token: 0x2c,
        ..ROUTE
    };
    let mut b = RcDelivery::armed(other, [0; 4]);
    let mut ga = [0; 4];
    let mut gb = [0x1234; 4];
    assert!(publish(&mut a, record(32, 0xffff), &mut ga).is_some());
    assert_eq!(publish(&mut b, [0; 4], &mut gb), None);
    assert!(!b.failed());
    assert_eq!(gb, [0x1234; 4]);
}

#[test]
fn hostile_guest_bytes_never_become_a_host_error() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut guest = record(32, 0xffff);
    for _ in 0..1000 {
        assert_eq!(publish(&mut d, [0; 4], &mut guest), None);
        guest[2] = guest[2].wrapping_add(1);
    }
    assert!(!d.failed());
    // A real host error replaces even a hostile nonzero guest status.
    assert!(publish(&mut d, record(32, 0xffff), &mut guest).is_some());
    assert_eq!(guest[2..], [32, 0xffff_0001]);
}

#[test]
fn a_pending_host_error_cannot_be_replaced_by_later_host_words() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    assert_eq!(
        d.poll(record(32, 0xffff), |_, _| Err("blocked")),
        Err("blocked")
    );
    let mut guest = [0; 4];
    assert_eq!(
        publish(&mut d, record(45, 0xffff), &mut guest)
            .unwrap()
            .except_type,
        32
    );
    assert_eq!(guest[2], 32);
}

#[test]
fn partial_record_and_repeated_baseline_never_write_guest_memory() {
    let baseline = record(7, 0xffff);
    let mut d = RcDelivery::armed(ROUTE, baseline);
    for host in [baseline, record(32, 0), [0; 4]] {
        assert_eq!(
            d.poll(host, |_, _| -> Result<(), ()> {
                panic!("no complete new record")
            })
            .unwrap(),
            None
        );
        assert!(!d.failed());
    }
}

#[test]
fn info16_without_status_cannot_turn_into_an_xid() {
    let mut w = RcWatch::armed([0; 4]);
    assert_eq!(w.observe([1, 2, 32, 9]), None);
    assert_eq!(
        w.observe([1, 2, 32, 0xffff_0009]),
        Some(ObservedRc { except_type: 32 })
    );
}

#[test]
fn a_stop_expectation_does_not_hide_a_different_host_error() {
    let mut w = RcWatch::armed([0; 4]);
    w.expect_guest_stop_write();
    assert_eq!(
        w.observe(record(32, 0xffff)),
        Some(ObservedRc { except_type: 32 })
    );
}

#[test]
fn changing_bytes_after_a_report_does_not_make_a_second_record() {
    let mut w = RcWatch::armed([0; 4]);
    assert!(w.observe(record(32, 0xffff)).is_some());
    assert_eq!(w.observe(record(31, 0xffff)), None);
}
