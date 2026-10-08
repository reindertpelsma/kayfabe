//! ★ 2026-10-08: `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` for an ENGINE's non-stall notifier
//! (`kf_abi::eventnotify::ENGINE_NONSTALL_NOTIFIERS`) — served for every copy engine, GR0 and the
//! video/OFA engines, as the real GSP serves it (`[measured 2026-10-08]` the 4070's host RM armed
//! CE0..CE9 although the die has COPY0..COPY3, `bare_ce-interrupt_9925108e_run1.log`); every index
//! outside the admitting lists, and every index the guest's own handler would never forward, is
//! refused by name.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::eventnotify::{
    self, ACTION_OFF, ACTION_REPEAT, EVENT_OFF, EVENT_SET_NOTIFICATION_PARAMS_SIZE,
    NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION, NV2080_NOTIFIERS_MAXCOUNT, NV2080_NOTIFIERS_TIMER,
};
use kf_abi::generated::matrix as m;
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::inittables::InitTablePolicy;

const PARAMS_AT: usize = 40;

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(
        ga106::board(),
        ga106::host(),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
    )
}

/// One arming (REPEAT) of `event` on subdevice `object`, as the guest's CPU-RM forwards it.
fn arming(object: u32, event: u32, action: u32) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION.to_le_bytes());
    payload[16..20].copy_from_slice(&(EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32).to_le_bytes());
    payload[PARAMS_AT + EVENT_OFF..PARAMS_AT + EVENT_OFF + 4].copy_from_slice(&event.to_le_bytes());
    payload[PARAMS_AT + ACTION_OFF..PARAMS_AT + ACTION_OFF + 4]
        .copy_from_slice(&action.to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// `Some(status)` of the reply.
fn status(p: &mut InitTablePolicy, object: u32, event: u32, action: u32) -> u32 {
    p.respond(&arming(object, event, action))
        .expect("the policy claims the control")
        .rpc_result
}

fn index(runs: &kf_abi::matrix::ValueRuns) -> u32 {
    runs.at_u32(BENCH_DRIVER)
        .ok()
        .flatten()
        .expect("named at the bench driver")
}

#[test]
fn every_copy_engine_gr0_and_video_notifier_is_served_as_the_real_gsp_serves_it() {
    let mut p = policy();
    // One subdevice per index: the transition rule is per (subdevice, index), and the slots are
    // bounded, so arm them on one subdevice.
    let mut n = 0;
    for runs in eventnotify::ENGINE_NONSTALL_NOTIFIERS {
        let Some(ix) = runs.at_u32(BENCH_DRIVER).ok().flatten() else {
            continue; // CE10..19 do not exist before 560 — absent, so never admitted
        };
        assert_eq!(
            status(&mut p, 0xabcd_2080, ix, ACTION_REPEAT),
            0,
            "{} ({ix}) must be served",
            runs.name
        );
        n += 1;
    }
    assert!(n >= 25, "GR0 + CE0..9 + NVENC + NVDEC + OFA at least, got {n}");
}

#[test]
fn ce2_and_ce4_to_ce9_the_indices_kf3_refused_0x56_are_now_served() {
    let mut p = policy();
    for runs in [
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE2,
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE4,
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE9,
    ] {
        assert_eq!(status(&mut p, 0xabcd_2080, index(runs), ACTION_REPEAT), 0, "{}", runs.name);
    }
}

#[test]
fn an_engine_notifier_already_armed_is_refused_the_way_rm_refuses_it() {
    let mut p = policy();
    let ce2 = index(&m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE2);
    assert_eq!(status(&mut p, 0xabcd_2080, ce2, ACTION_REPEAT), 0);
    assert_ne!(
        status(&mut p, 0xabcd_2080, ce2, ACTION_REPEAT),
        0,
        "subdevice_ctrl_event_kernel.c:124-131: must be disabled first"
    );
}

#[test]
fn notifiers_of_engines_this_device_announces_no_vector_for_stay_refused() {
    // SEC2, PPP, NVJPEG0, GR1 (MIG): OGKM maps them to an engine, but the interrupt plane raises
    // nothing for them, so accepting would be a promise without a mechanism.
    let mut p = policy();
    for runs in [
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_SEC2,
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_PPP,
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_NVJPEG0,
        &m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_GR1,
    ] {
        let ix = index(runs);
        assert!(!eventnotify::is_engine_nonstall_notifier(BENCH_DRIVER, ix));
        assert_ne!(status(&mut p, 0xabcd_2080, ix, ACTION_REPEAT), 0, "{}", runs.name);
    }
}

#[test]
fn hostile_indices_are_refused_by_name() {
    let mut p = policy();
    for ix in [
        NV2080_NOTIFIERS_TIMER,
        NV2080_NOTIFIERS_MAXCOUNT,
        NV2080_NOTIFIERS_MAXCOUNT + 1,
        0x8000_0000 | index(&m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE0),
        u32::MAX,
        3, // THERMAL_SW: a legal index no list admits
    ] {
        assert_ne!(status(&mut p, 0xabcd_2080, ix, ACTION_REPEAT), 0, "index {ix:#x}");
    }
    // An action outside the three, on an admitted engine index.
    let ce0 = index(&m::NV2080_NOTIFIERS_NV2080_NOTIFIERS_CE0);
    assert_ne!(status(&mut p, 0xabcd_2080, ce0, 7), 0);
}
