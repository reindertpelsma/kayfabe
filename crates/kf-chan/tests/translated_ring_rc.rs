//! Tests-first B1 regression (EXPECTED RED committed before the fix): an Xid 32 raised by the host on one of
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
//! `hObjectError`, observe it, deliver `RC_TRIGGERED` with the guest chid) now exists in production code.
//! CODE READ: these GPU-free checks do not establish hardware delivery or identify the Blackwell faulting ring.

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

#[test]
fn refusal_at_each_guest_word_retains_the_original_host_error() {
    for refused in [0, 4, 8, 12] {
        let mut d = RcDelivery::armed(ROUTE, [0; 4]);
        assert_eq!(
            d.poll(record(32, 0xffff), |off, _| {
                if off == refused {
                    Err("blocked")
                } else {
                    Ok(())
                }
            }),
            Err("blocked")
        );
        let mut guest = [0; 4];
        assert_eq!(
            publish(&mut d, record(31, 0xffff), &mut guest)
                .unwrap()
                .except_type,
            32
        );
        assert_eq!(guest[2], 32);
        assert!(d.failed());
    }
}

#[test]
fn the_status_store_never_exposes_old_metadata_to_a_guest_poller() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    let mut guest = [0xaaaa; 4];
    assert!(
        d.poll(record(32, 0xffff), |off, word| {
            guest[off as usize / 4] = word;
            if off == 12 {
                assert_eq!(guest[2], 32, "reason must already be visible");
            }
            Ok::<_, ()>(())
        })
        .unwrap()
        .is_some()
    );
}

#[test]
fn a_guest_that_waived_its_notifier_still_gets_the_routed_error() {
    let mut d = RcDelivery::armed(ROUTE, [0; 4]);
    assert_eq!(
        d.poll(record(32, 1), |_, _| Ok::<_, ()>(())).unwrap(),
        Some(RoutedRc {
            route: ROUTE,
            except_type: 32
        })
    );
}

#[test]
fn only_one_complete_guest_stop_record_is_suppressed() {
    let mut w = RcWatch::armed([0; 4]);
    w.expect_guest_stop_write();
    assert_eq!(w.observe(record(45, 0)), None);
    assert_eq!(w.observe(record(45, 0xffff)), None);
    let mut later_host_removal = record(45, 0xffff);
    later_host_removal[0] += 1;
    assert_eq!(
        w.observe(later_host_removal),
        Some(ObservedRc { except_type: 45 })
    );
}

#[test]
fn private_ring_birth_carries_the_error_context_in_both_layouts() {
    use kf_chan::host::{LEGACY_LAYOUT, TSPACE_LAYOUT, private_ring_spec};
    for layout in [LEGACY_LAYOUT, TSPACE_LAYOUT] {
        let spec = private_ring_spec(0x200000, layout, 0x1122, 0x8877).unwrap();
        assert_eq!(
            spec.err_notifier, 0x8877,
            "native birth must arm the private record"
        );
        assert_eq!(spec.userd_memory, 0x1122);
        assert_eq!(spec.userd_offset, layout.userd_off);
        assert_eq!(spec.gp_fifo_va, 0x200000 + layout.gpfifo_off);
        assert!(private_ring_spec(0x200000, layout, 0x1122, 0).is_err());
    }
}

#[test]
fn the_rc_context_is_never_replaced_by_a_memory_handle_or_ring_address() {
    use kf_chan::host::{TSPACE_LAYOUT, private_ring_spec};
    for context in [1, 0xdead, u32::MAX] {
        let spec = private_ring_spec(0x400000, TSPACE_LAYOUT, 0xbaba, context).unwrap();
        assert_eq!(spec.err_notifier, context);
    }
}

#[test]
fn the_private_record_is_outside_the_tspace_gpu_maps_and_userd_footprint() {
    use kf_chan::host::{ERROR_NOTIFIER_OFF, RING_BYTES, TSPACE_LAYOUT, private_ring_spec};
    assert_eq!(ERROR_NOTIFIER_OFF + 16, RING_BYTES);
    let spec = private_ring_spec(0x400000, TSPACE_LAYOUT, 1, 2).unwrap();
    assert!(spec.userd_offset + kf_abi::submit::USERD_SIZE <= ERROR_NOTIFIER_OFF);
    for (off, len, _) in TSPACE_LAYOUT.maps() {
        assert!(off + len <= ERROR_NOTIFIER_OFF);
    }
    assert!(private_ring_spec(u64::MAX, TSPACE_LAYOUT, 1, 2).is_err());
    let overlap = kf_chan::host::RingLayout {
        userd_off: ERROR_NOTIFIER_OFF,
        ..TSPACE_LAYOUT
    };
    assert!(private_ring_spec(0x400000, overlap, 1, 2).is_err());
}

#[test]
fn private_snapshot_cannot_pair_a_new_status_with_old_info32() {
    let mut host = record(7, 0);
    let first = kf_chan::rc::snapshot(|off| {
        let result = host[off as usize / 4];
        host = record(32, 0xffff);
        Some(result)
    });
    assert_eq!(first, None);
    assert_eq!(
        kf_chan::rc::snapshot(|off| Some(host[off as usize / 4])),
        Some(record(32, 0xffff))
    );
}

#[test]
fn snapshot_waits_for_status_and_rejects_a_record_changed_while_reading() {
    let mut reads = 0;
    assert_eq!(
        kf_chan::rc::snapshot(|_| {
            reads += 1;
            Some(0)
        }),
        None
    );
    assert_eq!(reads, 1, "metadata is not sampled before status");
    let mut status_reads = 0;
    assert_eq!(
        kf_chan::rc::snapshot(|off| {
            if off == 12 {
                status_reads += 1;
                Some(if status_reads == 1 { 0xffff_0001 } else { 0 })
            } else {
                Some(32)
            }
        }),
        None
    );
}
