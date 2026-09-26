//! `kf_rm::census` — the report's third state, and the two properties it rests on.
//!
//! ## Why this file exists
//!
//! The unserviced ledger's list was twice misread as a map of the boot: a
//! **served-but-refused** control (`InitTablePolicy::refuse()` returns `Some(Reply)`)
//! structurally never reaches it, and a **served** control is also absent — so "id absent"
//! discriminated nothing. The census records the two positive states; these tests pin
//! that it records them **correctly** and that observing them **changes nothing**.
//!
//! ## The two properties
//!
//! 1. **Fidelity** — a served control appears with result `NV_OK`; a refused one with the
//!    result the guest read; an unserviced one in neither (it stays the ledger's). The
//!    arming rows carry the handles they arrived on, so one index armed on two subdevices
//!    is two rows — the exact signature the H1 aliasing hypothesis is tested by on the
//!    bench, pinned here against the day someone folds the rows together.
//! 2. **Neutrality** — the reply through `served_policy` (census installed) is byte-for-
//!    byte the reply through the same chain without it. Observing is not altering.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::NV_ERR_NOT_SUPPORTED;
use kf_abi::eventnotify::{
    ACTION_OFF, ACTION_REPEAT, EVENT_OFF, EVENT_SET_NOTIFICATION_PARAMS_SIZE, INFO16_OFF,
    INFO32_OFF, NOTIFY_STATE_OFF, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
    NV2080_NOTIFIERS_POWER_RESUME,
};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_rm::census::ControlCensusLog;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 60 - 20 = 40`.
const PARAMS_AT: usize = 40;

/// A control this port has never modelled — the never-seen probe.
const UNKNOWN_CMD: u32 = 0x2080_beef;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// The full production chain, with the census's own log handed back beside it.
fn chain_with_census() -> (Box<dyn CommandPolicy>, ControlCensusLog) {
    let census = ControlCensusLog::new();
    let chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs::default(),
        census.clone(),
        kf_rm::ObjectLinks::default(),
    );
    (chain, census)
}

/// A `GSP_RM_CONTROL` with an explicit `(hClient, hObject)` — the census records both, and
/// the arming tests turn on the object handle differing.
fn control_command(client: u32, object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The registration `memmgrRegisterSuspendCallbacks` sends
/// (`ogkm-580: mem_mgr.c:619-627`): `POWER_RESUME`, `REPEAT`, everything else zero.
fn arming_params(event: u32, action: u32) -> Vec<u8> {
    let mut p = vec![0u8; EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    p[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&event.to_le_bytes());
    p[ACTION_OFF..ACTION_OFF + 4].copy_from_slice(&action.to_le_bytes());
    p[NOTIFY_STATE_OFF] = 0;
    p[INFO32_OFF..INFO32_OFF + 4].copy_from_slice(&0u32.to_le_bytes());
    p[INFO16_OFF..INFO16_OFF + 2].copy_from_slice(&0u16.to_le_bytes());
    p
}

// ── Property 1: fidelity ───────────────────────────────────────────────────────────

#[test]
fn a_served_control_is_recorded_with_nv_ok_and_its_count() {
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(
        0xc1e0_0004,
        0xabcd_2080,
        NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
        &arming_params(NV2080_NOTIFIERS_POWER_RESUME, ACTION_REPEAT),
    );
    let reply = chain.respond(&cmd).expect("the arm serves POWER_RESUME");
    assert_eq!(
        reply.rpc_result, 0,
        "precondition: this registration is served"
    );

    let snap = census.snapshot();
    assert_eq!(snap.served_total, 1);
    assert_eq!(
        snap.served,
        vec![kf_rm::census::ServedControl {
            cmd: NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
            rpc_result: 0,
            count: 1,
        }]
    );
}

#[test]
fn a_served_but_refused_control_is_recorded_with_the_result_the_guest_read() {
    // ★★★ The class the unserviced ledger structurally cannot see, which is the whole
    // reason the census exists. Arming POWER_RESUME twice trips the already-armed
    // transition rule: first answer NV_OK, second NV_ERR_NOT_SUPPORTED — and BOTH rows
    // must appear, keyed on the (cmd, result) pair.
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(
        0xc1e0_0004,
        0xabcd_2080,
        NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
        &arming_params(NV2080_NOTIFIERS_POWER_RESUME, ACTION_REPEAT),
    );
    assert_eq!(chain.respond(&cmd).expect("served").rpc_result, 0);
    let second = chain
        .respond(&cmd)
        .expect("answered — a refusal IS an answer");
    assert_eq!(
        second.rpc_result, NV_ERR_NOT_SUPPORTED,
        "precondition: refused"
    );

    let snap = census.snapshot();
    assert_eq!(snap.served_total, 2);
    assert_eq!(
        snap.served_distinct, 2,
        "one control, two results, two rows"
    );
    assert_eq!(
        snap.served[1],
        kf_rm::census::ServedControl {
            cmd: NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
            rpc_result: NV_ERR_NOT_SUPPORTED,
            count: 1,
        }
    );
}

#[test]
fn one_index_armed_on_two_subdevices_is_two_rows_with_the_handles_they_arrived_on() {
    // ★★★ The H1 signature, and the census keying that found it. This test used to pin
    // the DEFECT (`notify_actions` device-global, second subdevice's arming refused by the
    // aliasing — `[measured]` boot `census_probe35` at `6c51da7`); RM's transition rule is
    // per-subdevice (`ogkm-580: subdevice_ctrl_event_kernel.c:126-131`) and so is the
    // policy's state now, so BOTH armings serve — and the census must still keep two rows
    // with the `object` handles they arrived on. Folding them into one line with a count
    // of two is the exact blindness the bench census exists to remove; a reintroduced
    // device-global slot would flip the second row's result back to `0x56` and go red here.
    let (mut chain, census) = chain_with_census();
    let params = arming_params(NV2080_NOTIFIERS_POWER_RESUME, ACTION_REPEAT);
    let first = control_command(
        0xc1e0_0004,
        0xabcd_2080,
        NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
        &params,
    );
    let second = control_command(
        0xc1e0_0004,
        0xabcd_2081, // a different subdevice object, same client
        NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
        &params,
    );
    assert_eq!(chain.respond(&first).expect("served").rpc_result, 0);
    assert_eq!(
        chain.respond(&second).expect("answered").rpc_result,
        0,
        "each subdevice arms its own copy of the index — a real GSP accepts both"
    );

    let snap = census.snapshot();
    assert_eq!(snap.arming_total, 2);
    assert_eq!(
        snap.armings,
        vec![
            kf_rm::census::NotifierArming {
                client: 0xc1e0_0004,
                object: 0xabcd_2080,
                event: NV2080_NOTIFIERS_POWER_RESUME,
                action: ACTION_REPEAT,
                rpc_result: 0,
                count: 1,
            },
            kf_rm::census::NotifierArming {
                client: 0xc1e0_0004,
                object: 0xabcd_2081,
                event: NV2080_NOTIFIERS_POWER_RESUME,
                action: ACTION_REPEAT,
                rpc_result: 0,
                count: 1,
            },
        ]
    );
}

#[test]
fn an_unserviced_control_reaches_neither_census_list() {
    // ⊘ Never-seen must stay inferable: an id in NO list is a control nothing issued, and
    // that inference collapses if an unserviced command leaks into the served rows.
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(0xc1e0_0004, 0xabcd_2080, UNKNOWN_CMD, &[0u8; 4]);
    assert!(
        chain.respond(&cmd).is_none(),
        "precondition: nothing in the chain answers {UNKNOWN_CMD:#x}"
    );
    let snap = census.snapshot();
    assert_eq!(
        snap.served_total, 0,
        "an unanswered control is not 'served'"
    );
    assert_eq!(snap.armings, vec![], "and it armed nothing");
}

// ── Property 2: neutrality ─────────────────────────────────────────────────────────

#[test]
fn the_census_changes_no_byte_of_any_reply() {
    // ★★ Observing is not altering. The same commands through the same chain, with and
    // without the census wrapper, must produce identical replies — result and body both.
    // The uncensused chain is the same sticky-guarded chain `served_policy` wraps.
    let (mut with, _census) = chain_with_census();
    let mut without: Box<dyn CommandPolicy> =
        Box::new(kf_rm::sticky::StickyAnswerGuard::new(
            driver(),
            kf_rm::served_chain(
                ga106::board(),
                ga106::host(),
                driver(),
                kf_rm::ChainLogs::default(),
                kf_abi::eventnotify::ProbeArmSet::default(),
                kf_rm::ObjectLinks::default(),
            ),
        ));
    let arming = arming_params(NV2080_NOTIFIERS_POWER_RESUME, ACTION_REPEAT);
    let commands = [
        control_command(
            0xc1e0_0004,
            0xabcd_2080,
            NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
            &arming,
        ),
        // Twice: the second is the refusal path, which must also be untouched.
        control_command(
            0xc1e0_0004,
            0xabcd_2080,
            NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
            &arming,
        ),
        control_command(0xc1e0_0004, 0xabcd_2080, UNKNOWN_CMD, &[0u8; 4]),
    ];
    for (i, cmd) in commands.iter().enumerate() {
        assert_eq!(
            with.respond(cmd),
            without.respond(cmd),
            "command {i}: the census wrapper altered a reply"
        );
    }
}

// ── The census reports the probe set the boot ran with ─────────────────────────────

/// ★ The report field exists because the probe's history is three boots that ran WITHOUT
/// it while looking armed from the launching shell (it was a process env var then). The
/// snapshot must carry the set the plane was constructed with — including, and especially,
/// the empty default.
#[test]
fn the_census_carries_the_probe_set_and_defaults_to_empty() {
    use kf_abi::eventnotify::ProbeArmSet;

    let log = ControlCensusLog::new();
    assert!(
        log.snapshot().probe_arm.is_empty(),
        "a census nobody told about a probe reports the shipping configuration"
    );
    log.set_probe_arm(ProbeArmSet::parse("35,37").expect("parses"));
    assert_eq!(log.snapshot().probe_arm.as_slice(), &[35, 37]);
}

/// The CHAIN consults the same value the census reports — so a probed device's own
/// end-of-run report states the set in effect, and a default-built chain states empty.
///
/// ⊘ v3 port: the old test built `RegPlane::new` / `RegPlane::with_objects` (the register
/// plane, dropped in v3 — `V3_P2_PORT_MAP.md` §1, plane.rs DROP). The property it guarded —
/// *"the set the report states is the set the served chain consults"* — now lives in
/// `served_policy`, which reads the probe set OUT of the census log. So it is asserted there:
/// the reported set, and the arming behaviour it licenses, from one log.
#[test]
fn a_plane_reports_the_probe_set_it_was_built_with() {
    use kf_abi::eventnotify::ProbeArmSet;

    // Index 36: on none of the admitting lists, so only a probe set can admit it.
    // ⊘ v3-appfix: it was 37 (RC_ERROR), which is now GUEST_RAISED — the guest's own CPU-RM
    // raises it, so the shipping policy serves its arming.
    const PROBED: u32 = 36;
    assert!(!kf_abi::eventnotify::is_silent_notifier(PROBED));
    assert!(!kf_abi::eventnotify::is_delivered_notifier(PROBED));
    assert!(!kf_abi::eventnotify::is_guest_raised_notifier(PROBED));
    let arming = arming_params(PROBED, ACTION_REPEAT);
    let cmd = control_command(0xc1e0_0004, 0xabcd_2080, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION, &arming);

    let (mut stock, stock_census) = chain_with_census();
    assert!(
        stock_census.snapshot().probe_arm.is_empty(),
        "served_policy with a fresh census is the shipping constructor and must report an \
         empty probe set"
    );
    let r = stock.respond(&cmd).expect("the arming is answered");
    assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED, "unprobed index refused");

    let census = ControlCensusLog::new();
    census.set_probe_arm(ProbeArmSet::parse("36").expect("parses"));
    let mut probed = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs::default(),
        census.clone(),
        kf_rm::ObjectLinks::default(),
    );
    assert_eq!(
        census.snapshot().probe_arm.as_slice(),
        &[PROBED],
        "the report must state the set the served chain consults, or a probe boot is \
         indistinguishable from a stock one — the exact misreading this field kills"
    );
    let r = probed.respond(&cmd).expect("the arming is answered");
    assert_eq!(r.rpc_result, 0, "the chain consulted the census's probe set");
}

// ── The channel-bind census: which copy engine the guest named ─────────────────────

/// `NVA06F_CTRL_BIND_PARAMS` — a single `NvU32 engineType`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:96`).
fn bind_params(engine_type: u32) -> Vec<u8> {
    engine_type.to_le_bytes().to_vec()
}

/// ★★★ The measurement this census exists for: the CE the scrubber picked, read off the
/// wire rather than inferred.
///
/// `[measured]` a real GA106 binds its CeUtils scrubber channel with `engineType = 11`
/// (`traces/real_ga106/rpc_transcript_real_ga106.txt:63`), which is `COPY2` — so the row
/// must say `ce_index = 2`, and it must say it in `NV2080_ENGINE_TYPE` space untranslated.
#[test]
fn a_bind_records_the_copy_engine_the_guest_named() {
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(
        0xc1e0_0005,
        0x0000_000b,
        kf_abi::submit::NVA06F_CTRL_CMD_BIND,
        &bind_params(11),
    );
    let _ = chain.respond(&cmd);

    let snap = census.snapshot();
    assert_eq!(snap.bind_total, 1);
    assert_eq!(snap.bind_distinct, 1);
    assert_eq!(snap.binds.len(), 1);
    let row = snap.binds[0];
    assert_eq!(row.client, 0xc1e0_0005);
    assert_eq!(row.object, 0x0000_000b);
    assert_eq!(
        row.engine_type, 11,
        "recorded raw, in the wire's own NV2080_ENGINE_TYPE space",
    );
    assert_eq!(row.ce_index, 2, "engineType 11 is COPY2, not COPY11");
    assert_eq!(row.count, 1);
}

/// ⊘ **Not-a-copy-engine must never read as CE0.** CE0 is one of exactly two indices the
/// captured `GA106_INTR_TABLE` publishes with `vectorNonStall = INVALID`, so a census that
/// defaulted an undecodable engine to `0` would manufacture the reading that decides
/// whether the index-35 refusal stands on hardware's authority. Three inputs that are all
/// "not a CE" for different reasons, and none of them may produce a `0`.
#[test]
fn a_bind_naming_something_that_is_not_a_copy_engine_is_never_recorded_as_ce0() {
    use kf_rm::census::BIND_NOT_A_COPY_ENGINE;
    for (engine_type, why) in [
        (1u32, "GRAPHICS"),
        (0x13, "NVDEC0 in NV2080 space — COPY10 only in RM space"),
        (0, "NV2080_ENGINE_TYPE_NULL"),
    ] {
        let (mut chain, census) = chain_with_census();
        let cmd = control_command(
            0xc1e0_0005,
            0x0000_000b,
            kf_abi::submit::NVA06F_CTRL_CMD_BIND,
            &bind_params(engine_type),
        );
        let _ = chain.respond(&cmd);
        let snap = census.snapshot();
        assert_eq!(snap.binds.len(), 1, "{why} was still recorded");
        assert_eq!(snap.binds[0].engine_type, engine_type);
        assert_eq!(
            snap.binds[0].ce_index, BIND_NOT_A_COPY_ENGINE,
            "{why} must not read as copy engine 0",
        );
    }
}

/// ⊘ A bind whose params are too short to hold an `engineType` is **recorded**, with the
/// no-reply marker in both fields. The census must be able to show the malformed request
/// whose refusal needs explaining — a decoder that dropped it would be blind to exactly
/// the row an operator would be looking for.
#[test]
fn a_bind_with_params_too_short_is_recorded_with_the_no_reply_marker() {
    use kf_rm::census::{ARMING_NO_REPLY, BIND_NOT_A_COPY_ENGINE};
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(
        0xc1e0_0005,
        0x0000_000b,
        kf_abi::submit::NVA06F_CTRL_CMD_BIND,
        &[0u8; 2],
    );
    let _ = chain.respond(&cmd);
    let snap = census.snapshot();
    assert_eq!(snap.bind_total, 1);
    assert_eq!(snap.binds[0].engine_type, ARMING_NO_REPLY);
    assert_eq!(snap.binds[0].ce_index, BIND_NOT_A_COPY_ENGINE);
}

/// Two channels bound to the same engine are **two rows**, keyed on the handles — the
/// `NotifierArming` keying argument, for its reason: a boot that binds one channel and a
/// boot that binds two must not print the same line.
#[test]
fn two_channels_bound_to_one_engine_are_two_rows_and_a_repeat_is_a_count() {
    let (mut chain, census) = chain_with_census();
    let first = control_command(
        0xc1e0_0005,
        0x0000_000b,
        kf_abi::submit::NVA06F_CTRL_CMD_BIND,
        &bind_params(11),
    );
    let second = control_command(
        0xc1e0_0005,
        0x0000_000c,
        kf_abi::submit::NVA06F_CTRL_CMD_BIND,
        &bind_params(11),
    );
    let _ = chain.respond(&first);
    let _ = chain.respond(&second);
    let _ = chain.respond(&first);

    let snap = census.snapshot();
    assert_eq!(snap.bind_total, 3);
    assert_eq!(
        snap.bind_distinct, 2,
        "different objects are different rows"
    );
    assert_eq!(snap.binds[0].object, 0x0000_000b);
    assert_eq!(snap.binds[0].count, 2, "the repeat folds into a count");
    assert_eq!(snap.binds[1].object, 0x0000_000c);
    assert_eq!(snap.binds[1].count, 1);
}

/// ⊘ A control that is not a bind reaches no bind row — the "never seen" inference the
/// whole census rests on, applied to this list too.
#[test]
fn a_control_that_is_not_a_bind_reaches_no_bind_row() {
    let (mut chain, census) = chain_with_census();
    let cmd = control_command(
        0xc1e0_0004,
        0xabcd_2080,
        NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
        &arming_params(NV2080_NOTIFIERS_POWER_RESUME, ACTION_REPEAT),
    );
    let _ = chain.respond(&cmd);
    let snap = census.snapshot();
    assert_eq!(snap.bind_total, 0);
    assert_eq!(snap.binds, vec![]);
}
