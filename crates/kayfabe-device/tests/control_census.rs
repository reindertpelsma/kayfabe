//! `kayfabe_device::census` — the report's third state, and the two properties it rests on.
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

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::eventnotify::{
    ACTION_OFF, ACTION_REPEAT, EVENT_OFF, EVENT_SET_NOTIFICATION_PARAMS_SIZE, INFO16_OFF,
    INFO32_OFF, NOTIFY_STATE_OFF, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
    NV2080_NOTIFIERS_POWER_RESUME,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::census::ControlCensusLog;
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 60 - 20 = 40`.
const PARAMS_AT: usize = 40;

/// A control this port has never modelled — the never-seen probe.
const UNKNOWN_CMD: u32 = 0x2080_beef;

fn driver() -> kayfabe_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// The full production chain, with the census's own log handed back beside it.
fn chain_with_census() -> (Box<dyn CommandPolicy>, ControlCensusLog) {
    let census = ControlCensusLog::new();
    let chain = kayfabe_device::served_policy(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        driver(),
        kayfabe_device::ChainLogs::default(),
        census.clone(),
        None,
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
        vec![kayfabe_device::census::ServedControl {
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
        kayfabe_device::census::ServedControl {
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
            kayfabe_device::census::NotifierArming {
                client: 0xc1e0_0004,
                object: 0xabcd_2080,
                event: NV2080_NOTIFIERS_POWER_RESUME,
                action: ACTION_REPEAT,
                rpc_result: 0,
                count: 1,
            },
            kayfabe_device::census::NotifierArming {
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
        Box::new(kayfabe_device::sticky::StickyAnswerGuard::new(
            driver(),
            kayfabe_device::served_chain(
                kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
                driver(),
                kayfabe_device::ChainLogs::default(),
                kayfabe_abi::eventnotify::ProbeArmSet::default(),
                None,
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
    use kayfabe_abi::eventnotify::ProbeArmSet;

    let log = ControlCensusLog::new();
    assert!(
        log.snapshot().probe_arm.is_empty(),
        "a census nobody told about a probe reports the shipping configuration"
    );
    log.set_probe_arm(ProbeArmSet::parse("35,37").expect("parses"));
    assert_eq!(log.snapshot().probe_arm.as_slice(), &[35, 37]);
}

/// The PLANE records the same value it hands the served chain — so a probed device's own
/// end-of-run report states the set in effect, and a default-built plane states empty.
#[test]
fn a_plane_reports_the_probe_set_it_was_built_with() {
    use kayfabe_abi::eventnotify::ProbeArmSet;
    use kayfabe_device::{NanoClock, RegPlane, abi};

    /// A fixed clock: this test reads no timer, but the plane wants one.
    #[derive(Debug)]
    struct StillClock;
    impl NanoClock for StillClock {
        fn now_ns(&self) -> u64 {
            0
        }
    }

    let stock = RegPlane::new(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(StillClock),
    )
    .expect("GA106 is servable");
    assert!(
        stock.control_census().probe_arm.is_empty(),
        "RegPlane::new is the shipping constructor and must report an empty probe set"
    );

    let probed = RegPlane::with_objects(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(StillClock),
        ProbeArmSet::parse("35").expect("parses"),
        None,
    )
    .expect("GA106 is servable");
    assert_eq!(
        probed.control_census().probe_arm.as_slice(),
        &[35],
        "the report must state the set the served chain consults, or a probe boot is \
         indistinguishable from a stock one — the exact misreading this field kills"
    );
}
