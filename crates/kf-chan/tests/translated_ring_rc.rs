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
    assert_eq!(w.observe([0; 4]), None, "an untouched record is not an event");
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
    assert_eq!(w.observe(record(XID_PBDMA_ERROR, 0)), None, "status is written last");
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
    assert_eq!(w.observe(armed), None, "a guest may initialise its notifier to anything");
}
