//! The host-side ledger of commands nothing answered — `kayfabe_device::unserviced`.
//!
//! ## ★★ Why this is tested at all, when it answers no guest
//!
//! It is diagnostics, and diagnostics are the first thing to rot silently. The default the
//! emulated GSP now uses is a **named refusal**, and the guest logs that refusal at a level
//! a release module never prints (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11109-11115`)
//! — so if this ledger is wrong or empty, *"which controls has this port not built"* has no
//! answer at all and nobody finds out, because a missing list looks exactly like an empty
//! one. `the_ledger_answers_nothing` is the load-bearing one: a diagnostic that changed
//! what the guest sees would be worse than none.

use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::unserviced::{
    UNSERVICED_SAMPLE_MAX, UnservicedCommand, UnservicedLedger, UnservicedLog,
};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// A `GSP_RM_CONTROL` whose header names `cmd`. 40 bytes is `RpcControlReq::HEADER`.
fn control(cmd: u32) -> RpcCommand {
    let mut payload = vec![0u8; 40];
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn other(code: u32) -> RpcCommand {
    RpcCommand {
        function: RpcFunction::Other(code),
        code,
        sequence: 2,
        payload: Vec::new(),
        elements: 1,
        delivered: Vec::new(),
    }
}

fn ledger() -> (UnservicedLedger, UnservicedLog) {
    let log = UnservicedLog::new();
    let driver = *table_for(BENCH_DRIVER).expect("bench ABI");
    (UnservicedLedger::new(driver, log.clone()), log)
}

#[test]
fn the_ledger_answers_nothing() {
    let (mut l, _log) = ledger();
    assert!(l.respond(&control(0x2080_1803)).is_none());
    assert!(l.respond(&other(1234)).is_none());
}

#[test]
fn a_control_is_recorded_by_its_command_and_a_bare_function_by_its_id() {
    let (mut l, log) = ledger();
    l.respond(&control(0x2080_1803));
    l.respond(&other(0x2001));

    assert_eq!(
        log.sample(),
        vec![
            UnservicedCommand {
                function: 76,
                cmd: Some(0x2080_1803),
            },
            UnservicedCommand {
                function: 0x2001,
                cmd: None,
            },
        ],
        "a control names its command; a function that is not one says so rather than \
         reporting command zero"
    );
    assert_eq!(log.total(), 2);
}

#[test]
fn a_repeated_control_counts_but_does_not_grow_the_list() {
    let (mut l, log) = ledger();
    for _ in 0..500 {
        l.respond(&control(0x2080_1803));
    }
    assert_eq!(log.sample().len(), 1, "distinct, not a tally");
    assert_eq!(log.total(), 500, "…and the tally is still kept");
}

#[test]
fn the_distinct_set_is_bounded_so_a_guest_cannot_grow_it() {
    let (mut l, log) = ledger();
    // 64 is the cap, spelled out; more than the cap is offered.
    assert_eq!(UNSERVICED_SAMPLE_MAX, 64);
    for i in 0..80u32 {
        l.respond(&control(0x2080_0000 + i));
    }
    assert_eq!(log.sample().len(), 64);
    assert_eq!(log.total(), 80, "the counter is the honest one");
    // First-seen order, so the list names the EARLIEST commands rather than a random 64.
    assert_eq!(log.sample()[0].cmd, Some(0x2080_0000));
    assert_eq!(log.sample()[63].cmd, Some(0x2080_003f));
}

/// ⊘⊘ **The property whose ABSENCE produced a wrong root cause.**
///
/// `[measured 2026-08-09, boot `gt1431_ff7a0ea`]` the boot ledger printed
/// `67 UNSERVICED …, 32 distinct` from a list saturated at its 32-slot cap, and the count it
/// printed was the *sample's* clamped length — so a full list was byte-identical in the log
/// to a complete one. `execution_plane_increments.md` §14.31 read `0x20801303`'s absence
/// from it as *"the command never reaches the emulated GSP"* and specified a rung on that.
///
/// The counter must therefore be **larger than the sample** when the sample saturates, and
/// [`UnservicedLog::truncated`] must say so. A test that only checked `sample().len()` — the
/// one above — passes identically either way, which is why it could not have caught this.
#[test]
fn a_saturated_sample_says_so_rather_than_reading_as_complete() {
    let (mut l, log) = ledger();
    for i in 0..(UNSERVICED_SAMPLE_MAX as u32 + 17) {
        l.respond(&control(0x2080_0000 + i));
    }
    assert_eq!(
        log.sample().len(),
        UNSERVICED_SAMPLE_MAX,
        "the sample is capped"
    );
    assert_eq!(
        log.distinct(),
        UNSERVICED_SAMPLE_MAX as u64 + 17,
        "…and the distinct COUNT is not — it counts before the capacity test"
    );
    assert!(log.truncated(), "a saturated list must announce itself");
    // …and an unsaturated one must not, or the banner becomes noise nobody reads.
    let (mut l2, log2) = ledger();
    for i in 0..8u32 {
        l2.respond(&control(0x2080_0000 + i));
    }
    assert_eq!(log2.distinct(), 8);
    assert!(!log2.truncated());
    // ⊘ Repeats must not inflate the distinct count — the counter sits behind the
    // membership test, not in front of it.
    for _ in 0..50 {
        l2.respond(&control(0x2080_0000));
    }
    assert_eq!(log2.distinct(), 8, "a repeat is not a new distinct command");
    assert_eq!(log2.total(), 58);
}

#[test]
fn a_control_too_short_to_decode_is_recorded_without_inventing_a_command() {
    let (mut l, log) = ledger();
    l.respond(&RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 3,
        payload: vec![0u8; 8],
        elements: 1,
        delivered: Vec::new(),
    });
    assert_eq!(
        log.sample(),
        vec![UnservicedCommand {
            function: 76,
            cmd: None
        }]
    );
}
