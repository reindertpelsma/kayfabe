//! `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (`0x20800301`) — the thirteenth control this
//! port serves, the first that is a **verb** rather than a description, and the first whose
//! reply may not be empty.
//!
//! ## What this file establishes, in the order it matters
//!
//! 1. **An empty reply is not available**, and that is the finding. Every earlier control
//!    this port could have answered inertly is `[OUT]`-only, so an empty body is merely
//!    uninformative. Here the reply is copied back over the *caller's own request struct*
//!    (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`) and the caller then
//!    switches on `pSetEventParams->action` (`ogkm-580:
//!    src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_event_kernel.c:119-146`). An
//!    all-zero body therefore rewrites `event = 194, action = REPEAT` into
//!    `event = 0, action = DISABLE` and returns `NV_OK`.
//!    [`the_reply_carries_the_registration_back_because_an_empty_body_would_rearm_notifier_zero`]
//!    is the test that fails if the body is ever emptied.
//! 2. **The reply is a re-encode and not an echo.** The request is poisoned with `0xAA`
//!    everywhere the struct does not define a field, and both pad runs come back **zero**.
//! 3. **The promise is scoped.** A notifier this device cannot promise silence for is
//!    refused, even though it is a perfectly legal index.
//! 4. **The guest's own preconditions are enforced**, transcribed with citations rather
//!    than invented.
//!
//! ## ⚠ What it does NOT establish
//!
//! ⊘ **Nothing about delivery.** This port raises no interrupt and delivers no
//! notification; [`kayfabe_abi::eventnotify::SILENT_NOTIFIERS`] is the argument for why
//! accepting a registration is nonetheless honest, and it is an argument about
//! `NV2080_NOTIFIERS_POWER_RESUME` specifically. No test here can show that an event which
//! *can* occur is delivered, because none is.
//!
//! ⊘ **No boot is claimed here.** That a served `0x20800301` lets
//! `memmgrStateInitLocked_IMPL` complete is `[inferred]` from `ogkm-580: mem_mgr.c:625,
//! :777` — a source reading, said as one — until a boot says otherwise. The two boots that
//! *are* `[measured]` (2026-08-01, **rev `2ced035`** and **rev `a6412c0`**,
//! `docs/design/boot_measured_2026_08_01.md` §6 and §7) establish only where the wall is,
//! not what clearing it reaches.

use kayfabe_abi::eventnotify::{
    self, ACTION_DISABLE, ACTION_OFF, ACTION_REPEAT, ACTION_SINGLE, EVENT_OFF,
    EVENT_SET_NOTIFICATION_PARAMS_SIZE, EventNotifyError, EventSetNotification, INFO16_OFF,
    INFO32_OFF, NOTIFY_STATE_OFF, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
    NV2080_NOTIFIERS_MAXCOUNT, NV2080_NOTIFIERS_POWER_RESUME, NV2080_NOTIFIERS_TIMER,
    SILENT_NOTIFIERS,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::inittables::{InitTablePolicy, NOTIFY_SUBDEVICE_SLOTS, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 60 - 20 = 40`.
const PARAMS_AT: usize = 40;

/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying an event registration.
///
/// ★★ The params are laid down over a `0xAA` fill, so every byte the struct does not define
/// is poison. That is what makes "re-encoded" distinguishable from "echoed": an echo would
/// bring `0xAA` back in the two pad runs, and a re-encode brings zeros.
fn event_command(reg: &EventSetNotification, params_size: u32) -> RpcCommand {
    event_command_on(0xc1e0_0004, 0xabcd_2080, reg, params_size)
}

/// [`event_command`] with the subdevice spelled out — the handles are load-bearing since
/// the arming state went per-subdevice, so the fixture tests name them explicitly.
fn event_command_on(
    client: u32,
    object: u32,
    reg: &EventSetNotification,
    params_size: u32,
) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&client.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&object.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    // The fields, written over the poison — leaving both pad runs at 0xAA.
    let at = PARAMS_AT;
    if params_size as usize >= EVENT_SET_NOTIFICATION_PARAMS_SIZE {
        payload[at + EVENT_OFF..at + EVENT_OFF + 4].copy_from_slice(&reg.event.to_le_bytes());
        payload[at + ACTION_OFF..at + ACTION_OFF + 4].copy_from_slice(&reg.action.to_le_bytes());
        payload[at + NOTIFY_STATE_OFF] = u8::from(reg.notify_state);
        payload[at + INFO32_OFF..at + INFO32_OFF + 4].copy_from_slice(&reg.info32.to_le_bytes());
        payload[at + INFO16_OFF..at + INFO16_OFF + 2].copy_from_slice(&reg.info16.to_le_bytes());
    }
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The registration `memmgrRegisterSuspendCallbacks` actually sends
/// (`ogkm-580: mem_mgr.c:619-627`): `POWER_RESUME`, `REPEAT`, everything else pre-zeroed by
/// its `= { 0 }` initialiser.
fn memmgr_registration() -> EventSetNotification {
    EventSetNotification {
        event: NV2080_NOTIFIERS_POWER_RESUME,
        action: ACTION_REPEAT,
        notify_state: false,
        info32: 0,
        info16: 0,
    }
}

/// The params half of a served reply, or `None` if the policy refused.
fn served_params(cmd: &RpcCommand) -> Option<Vec<u8>> {
    let reply = policy().respond(cmd)?;
    if reply.rpc_result != 0 {
        return None;
    }
    Some(reply.body[PARAMS_AT..PARAMS_AT + EVENT_SET_NOTIFICATION_PARAMS_SIZE].to_vec())
}

// ── The control is served at all ───────────────────────────────────────────────────

#[test]
fn the_registration_memmgr_actually_sends_is_served() {
    // ★★★ The rung. `[measured]` `docs/design/boot_measured_2026_08_01.md` §7: two boots one
    // commit apart established that this control — not another alloc class, and not another
    // init table — is where `memmgrStateInitLocked_IMPL` stops.
    let cmd = event_command(
        &memmgr_registration(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    let reply = policy().respond(&cmd).expect("this port serves 0x20800301");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    // ⊘ And the *inner* status too. RM reads `rpc_params->status` before it copies params
    // out (`ogkm-580: rpc.c:11061-11065`), so an NV_OK envelope wrapped around a non-zero
    // inner status would skip the copyout and the registration would be lost silently.
    assert_eq!(
        u32::from_le_bytes(
            reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
                .try_into()
                .expect("four bytes")
        ),
        0,
        "the control header's own status"
    );
}

#[test]
fn the_served_universe_names_it_and_agrees_with_its_own_id_and_size() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION),
        Some(WantedTable::EventSetNotification),
        "in ALL, and therefore served — see WantedTable::ALL"
    );
    assert_eq!(
        WantedTable::EventSetNotification.params_size(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE
    );
    // ★ Twenty, and the oracle's own wire says so independently: `cap1b` carries this
    // control at `paylen 60` with a 40-byte control header.
    assert_eq!(EVENT_SET_NOTIFICATION_PARAMS_SIZE, 20);
    assert_eq!(60 - PARAMS_AT, EVENT_SET_NOTIFICATION_PARAMS_SIZE);
}

// ── ★★★ The reply may not be empty, and may not be an echo ─────────────────────────

#[test]
fn the_reply_carries_the_registration_back_because_an_empty_body_would_rearm_notifier_zero() {
    // ★★★ **The whole reason this control is not in `kayfabe_device::inert`.**
    //
    // `inert`'s eligibility rule is "the guest reads nothing but the status", and by the
    // letter of it this control qualifies: `NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS` has
    // zero `[OUT]` fields. The rule is defeated by the TRANSPORT, not by the control —
    // `rpcRmApiControl_GSP` copies the reply's params over `pParamStructPtr`, which IS the
    // caller's `eventNotificationParams` (`ogkm-580: rpc.c:11085-11090`), and the caller
    // reads `->action` and `->event` out of it afterwards to drive its own
    // `notifyActions[]` switch (`subdevice_ctrl_event_kernel.c:119-146`).
    //
    // So an empty (zero-filled) body makes the guest execute the `ACTION_DISABLE` arm for
    // notifier **0** instead of arming notifier 194, and return `NV_OK` while doing it.
    // That is a silently wrong success, which is the one outcome this port refuses
    // everywhere.
    let reg = memmgr_registration();
    let params = served_params(&event_command(
        &reg,
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    ))
    .expect("served");
    let back = eventnotify::decode_event_set_notification(&params).expect("a whole struct");
    assert_eq!(
        back, reg,
        "the guest must find its own registration in its struct after the RPC"
    );
    assert_ne!(
        back.event, 0,
        "an empty body would land here as notifier 0 — the exact defect this test names"
    );
    assert_eq!(
        back.action, ACTION_REPEAT,
        "and as ACTION_DISABLE rather than ACTION_REPEAT"
    );
}

#[test]
fn the_reply_is_re_encoded_and_not_reflected_so_the_pad_runs_come_back_zero() {
    // ★★ The distinction `kayfabe_device::inert`'s own table draws between "the request,
    // reflected" and "nothing of the guest's comes back". This reply is neither: it carries
    // the guest's *field values* because it must (see the test above), and it carries none
    // of the guest's *bytes*, because it is rebuilt from a decoded struct.
    //
    // The request is filled with 0xAA, so a reflecting implementation returns 0xAA in both
    // pad runs and this test goes red.
    let params = served_params(&event_command(
        &memmgr_registration(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    ))
    .expect("served");
    assert_eq!(
        &params[NOTIFY_STATE_OFF + 1..INFO32_OFF],
        &[0, 0, 0],
        "the pad run after the one-byte NvBool"
    );
    assert_eq!(
        &params[INFO16_OFF + 2..EVENT_SET_NOTIFICATION_PARAMS_SIZE],
        &[0, 0],
        "the pad run that rounds the NvU16 up to the struct's alignment"
    );
}

#[test]
fn a_registrations_client_supplied_info_fields_survive_the_round_trip() {
    // ★ `info32`/`info16`/`bNotifyState` are `[IN]` and `memmgrRegisterSuspendCallbacks`
    // leaves all three zero, so nothing on the boot path would notice if they were dropped.
    // ⊘ Which is exactly why they are asserted with NON-zero values: a field that is only
    // ever exercised at its default is a field with no test.
    let reg = EventSetNotification {
        event: NV2080_NOTIFIERS_POWER_RESUME,
        action: ACTION_SINGLE,
        notify_state: true,
        info32: 0xDEAD_BEEF,
        info16: 0xC0DE,
    };
    let params = served_params(&event_command(
        &reg,
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    ))
    .expect("served");
    assert_eq!(
        eventnotify::decode_event_set_notification(&params).expect("whole"),
        reg
    );
}

// ── ★★★ The promise is scoped to notifiers this device can defend ──────────────────

#[test]
fn a_legal_notifier_this_device_cannot_promise_silence_for_is_refused() {
    // ★★★ **The gate that makes accepting the registration honest rather than convenient.**
    //
    // This port delivers no events. Accepting an arming is a promise to deliver, so it may
    // only be made for events that cannot occur — which is a per-notifier argument, not a
    // property of the control. `SILENT_NOTIFIERS` carries the argument;
    // everything else is refused.
    //
    // ⊘ Index 1 is a perfectly legal notifier and the decoder accepts it. What refuses it is
    // policy, and that is the point: widening the rule to "anything below MAXCOUNT" would
    // quietly cover fault and completion notifiers whose silence is a hang nobody can
    // attribute.
    let reg = EventSetNotification {
        event: 1,
        action: ACTION_REPEAT,
        notify_state: false,
        info32: 0,
        info16: 0,
    };
    assert!(
        eventnotify::decode_event_set_notification(&eventnotify::encode_event_set_notification(
            &reg
        ))
        .is_ok(),
        "the WIRE accepts it — so what follows is a policy refusal, not a decode failure"
    );
    let reply = policy()
        .respond(&event_command(
            &reg,
            EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
        ))
        .expect("the policy claims the control");
    assert_ne!(reply.rpc_result, 0, "and the policy refuses it by name");
}

#[test]
fn the_silent_list_is_exactly_power_resume_and_every_row_carries_an_argument() {
    // ⊘ Pinned, so widening the promise is a visible change. `gates_quantified_over_a_list`:
    // a rule whose universe can grow without a red test is not a gate.
    let indices: Vec<u32> = SILENT_NOTIFIERS.iter().map(|n| n.index).collect();
    assert_eq!(indices, vec![NV2080_NOTIFIERS_POWER_RESUME]);
    assert_eq!(NV2080_NOTIFIERS_POWER_RESUME, 194);
    for n in SILENT_NOTIFIERS {
        assert!(
            n.why.len() > 80,
            "notifier {} is accepted without an argument",
            n.index
        );
        assert!(
            n.why.contains("ogkm-580:"),
            "notifier {} accepts without citing the tree it read",
            n.index
        );
    }
    assert!(eventnotify::is_silent_notifier(
        NV2080_NOTIFIERS_POWER_RESUME
    ));
    assert!(!eventnotify::is_silent_notifier(0));
}

// ── The guest's own preconditions, transcribed ─────────────────────────────────────

#[test]
fn the_decoder_enforces_the_two_bounds_the_guests_own_handler_enforces() {
    // ★ Both are checks `subdeviceCtrlCmdEventSetNotification_IMPL` makes BEFORE the RPC
    // (`ogkm-580: subdevice_ctrl_event_kernel.c:96-106`), so a request that fails them
    // cannot legitimately have reached us — which is a reason to refuse, not to be lenient.
    let mut params = vec![0u8; EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    params[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&NV2080_NOTIFIERS_MAXCOUNT.to_le_bytes());
    assert_eq!(
        eventnotify::decode_event_set_notification(&params),
        Err(EventNotifyError::EventOutOfRange {
            event: NV2080_NOTIFIERS_MAXCOUNT
        })
    );
    // ⊘ And the boundary is exclusive: MAXCOUNT-1 is legal, so the check is `>=` and not `>`.
    params[EVENT_OFF..EVENT_OFF + 4]
        .copy_from_slice(&(NV2080_NOTIFIERS_MAXCOUNT - 1).to_le_bytes());
    assert!(eventnotify::decode_event_set_notification(&params).is_ok());

    params[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&NV2080_NOTIFIERS_TIMER.to_le_bytes());
    assert_eq!(
        eventnotify::decode_event_set_notification(&params),
        Err(EventNotifyError::TimerEvent)
    );
}

#[test]
fn an_action_outside_the_three_the_sdk_names_is_refused() {
    // ★ The guest's own `switch` has `default: status = NV_ERR_INVALID_ARGUMENT`
    // (`ogkm-580: subdevice_ctrl_event_kernel.c:141-145`). ⊘ Note the ORDER, which is the
    // subtle part: the guest issues the RPC *before* its switch runs, so RM would accept
    // our NV_OK and only then fault the action itself. We refuse first.
    let mut params = vec![0u8; EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    params[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&NV2080_NOTIFIERS_POWER_RESUME.to_le_bytes());
    params[ACTION_OFF..ACTION_OFF + 4].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(
        eventnotify::decode_event_set_notification(&params),
        Err(EventNotifyError::UnknownAction { action: 3 })
    );
    for a in [ACTION_DISABLE, ACTION_SINGLE, ACTION_REPEAT] {
        params[ACTION_OFF..ACTION_OFF + 4].copy_from_slice(&a.to_le_bytes());
        assert!(
            eventnotify::decode_event_set_notification(&params).is_ok(),
            "action {a} is one of the three the SDK names"
        );
    }
}

#[test]
fn a_short_params_struct_is_refused_rather_than_read_past() {
    assert_eq!(
        eventnotify::decode_event_set_notification(&[0u8; 19]),
        Err(EventNotifyError::ShortParams { got: 19 })
    );
    // And through the policy: a declared size that is not the struct's is refused ahead of
    // any decode, by `InitTablePolicy`'s own size check.
    let reply = policy()
        .respond(&event_command(&memmgr_registration(), 16))
        .expect("the policy claims the control");
    assert_ne!(reply.rpc_result, 0);
}

// ── ★★ The arming table is state, and it behaves like RM's ─────────────────────────

#[test]
fn arming_an_already_armed_notifier_is_refused_the_way_rm_refuses_it() {
    // ★★ `subdevice_ctrl_event_kernel.c:124-131`: "must be in disabled state to transition
    // to an active state". The physical RM on a real GSP keeps the same `notifyActions[]`
    // for its own subdevice, so modelling this is transcription rather than invention —
    // and it is the only reason `notify_actions` has to be state at all.
    let mut p = policy();
    let cmd = event_command(
        &memmgr_registration(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    assert_eq!(
        p.respond(&cmd).expect("served").rpc_result,
        0,
        "the first arming succeeds"
    );
    assert_ne!(
        p.respond(&cmd).expect("claimed").rpc_result,
        0,
        "the second is refused — REPEAT over REPEAT is not a legal transition"
    );
}

#[test]
fn disabling_is_always_legal_and_re_arms_the_notifier_for_a_later_registration() {
    // ⊘ The other half of the transition rule, and the half a one-way test would miss:
    // `ACTION_DISABLE` is accepted from any state (`subdevice_ctrl_event_kernel.c:133-137`
    // has no precondition), and it must actually clear the slot or the refusal above
    // becomes permanent.
    let mut p = policy();
    let arm = event_command(
        &memmgr_registration(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    let disable = event_command(
        &EventSetNotification {
            action: ACTION_DISABLE,
            ..memmgr_registration()
        },
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    assert_eq!(p.respond(&arm).expect("served").rpc_result, 0);
    assert_eq!(
        p.respond(&disable).expect("served").rpc_result,
        0,
        "DISABLE has no precondition"
    );
    assert_eq!(
        p.respond(&arm).expect("served").rpc_result,
        0,
        "and it cleared the slot, so the notifier can be armed again"
    );
}

#[test]
fn a_fresh_policy_starts_with_every_notifier_disabled() {
    // ★ RM's `Subdevice` is zeroed at construction, so `ACTION_DISABLE` (= 0) is the only
    // starting state that agrees with the guest. A policy that started armed would refuse
    // the very first registration `memmgrRegisterSuspendCallbacks` sends.
    assert_eq!(ACTION_DISABLE, 0);
    let mut p = policy();
    assert_eq!(
        p.respond(&event_command(
            &memmgr_registration(),
            EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32
        ))
        .expect("served")
        .rpc_result,
        0
    );
}

// ── ★★★ The arming state is PER-SUBDEVICE, from the bench's own census rows ────────

/// The three rows of `docs/reference/bench_evidence/census_probe35_6c51da7_census.log`
/// (`[measured]` 2026-08-07, boot `census_probe35`, probe `35` armed) — the fixture the
/// per-subdevice fix is written from. The third row is the defect: the device-global array
/// refused it `0x56` where RM's per-subdevice rule
/// (`ogkm-580: subdevice_ctrl_event_kernel.c:124-131`) accepts.
const CENSUS_PROBE35_ROWS: [(u32, u32, u32); 3] = [
    (0xc1e0_0002, 0xcaf0_0001, NV2080_NOTIFIERS_POWER_RESUME), // event 194, served
    (0xc1e0_0005, 0x0000_000b, FIFO_EVENT_MTHD),               // event 35, served
    (0xc1e0_0006, 0x0000_000c, FIFO_EVENT_MTHD),               // event 35, was REFUSED
];

fn arming_on(client: u32, object: u32, event: u32) -> RpcCommand {
    event_command_on(
        client,
        object,
        &EventSetNotification {
            event,
            action: ACTION_REPEAT,
            notify_state: false,
            info32: 0,
            info16: 0,
        },
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    )
}

#[test]
fn the_census_fixture_two_subdevices_each_arm_event_35_once_and_all_three_rows_serve() {
    // The GA106 scrubber builds two channels (`runQueues=2`), each arming
    // `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` on its OWN subdevice. Aliasing them into one
    // device-global slot refused the second — which is what reached
    // `mem_utils_gm107.c:1027` as `event notification control failed`.
    let mut p = probed_policy("35");
    for (client, object, event) in CENSUS_PROBE35_ROWS {
        assert_eq!(
            p.respond(&arming_on(client, object, event))
                .expect("served")
                .rpc_result,
            0,
            "({client:#x}, {object:#x}) arming event {event} is a first arming on its \
             own subdevice and must serve"
        );
    }
    // ⊘ And the transition rule still bites where RM's would: a SECOND arming on the
    // SAME subdevice is the illegal transition, per-subdevice keying does not erase it.
    let (client, object, event) = CENSUS_PROBE35_ROWS[1];
    assert_ne!(
        p.respond(&arming_on(client, object, event))
            .expect("claimed")
            .rpc_result,
        0,
        "REPEAT over REPEAT on one subdevice stays refused"
    );
}

#[test]
fn a_disarmed_subdevice_releases_its_slot_and_the_bound_refuses_only_while_full() {
    // The key is guest-minted, so the state is bounded; the bound must fail LOUD and the
    // reclaim must make an ordinary disarm free its slot. POWER_RESUME is silent, so no
    // probe is involved.
    let mut p = policy();
    let arm = |i: u32| arming_on(0xc1e0_0000 + i, 0x100 + i, NV2080_NOTIFIERS_POWER_RESUME);
    for i in 0..NOTIFY_SUBDEVICE_SLOTS as u32 {
        assert_eq!(
            p.respond(&arm(i)).expect("served").rpc_result,
            0,
            "subdevice {i} fits in the table"
        );
    }
    let overflow = NOTIFY_SUBDEVICE_SLOTS as u32;
    assert_ne!(
        p.respond(&arm(overflow)).expect("claimed").rpc_result,
        0,
        "the arming that would need one slot more is refused, never silently evicted"
    );
    // Fully disarm one subdevice: its slot must be released…
    let disable = event_command_on(
        0xc1e0_0003,
        0x103,
        &EventSetNotification {
            event: NV2080_NOTIFIERS_POWER_RESUME,
            action: ACTION_DISABLE,
            notify_state: false,
            info32: 0,
            info16: 0,
        },
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    assert_eq!(p.respond(&disable).expect("served").rpc_result, 0);
    // …and the arming that was refused above now serves.
    assert_eq!(
        p.respond(&arm(overflow)).expect("served").rpc_result,
        0,
        "an all-disabled subdevice released its slot"
    );
}

#[test]
fn a_disable_on_an_unknown_subdevice_is_a_served_no_op_that_allocates_nothing() {
    // DISABLE has no precondition (`subdevice_ctrl_event_kernel.c:133-137`) — and it must
    // not burn a slot, or a disarm-happy guest could exhaust the table without arming
    // anything. This test is the check: disable on NOTIFY_SUBDEVICE_SLOTS unknown
    // subdevices, then NOTIFY_SUBDEVICE_SLOTS armed ones must still all fit.
    let mut p = policy();
    for i in 0..NOTIFY_SUBDEVICE_SLOTS as u32 {
        let disable = event_command_on(
            0xdead_0000 + i,
            0x200 + i,
            &EventSetNotification {
                event: NV2080_NOTIFIERS_POWER_RESUME,
                action: ACTION_DISABLE,
                notify_state: false,
                info32: 0,
                info16: 0,
            },
            EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
        );
        assert_eq!(
            p.respond(&disable).expect("served").rpc_result,
            0,
            "disabling an unarmed notifier is legal from any state"
        );
    }
    for i in 0..NOTIFY_SUBDEVICE_SLOTS as u32 {
        assert_eq!(
            p.respond(&arming_on(0xc1e0_0000 + i, 0x100 + i, NV2080_NOTIFIERS_POWER_RESUME))
                .expect("served")
                .rpc_result,
            0,
            "the disables above allocated no slots"
        );
    }
}

// ── The policy does not overreach ──────────────────────────────────────────────────

#[test]
fn the_policy_still_declines_a_function_that_is_not_a_control() {
    let mut cmd = event_command(
        &memmgr_registration(),
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    );
    cmd.function = RpcFunction::RmAlloc;
    assert!(
        policy().respond(&cmd).is_none(),
        "an event registration is not an allocation, whatever its payload says"
    );
}

// ── The notifier probe: a device property, admitting exactly what it names ─────────

/// The `m1` boot's index: `NV2080_NOTIFIERS_FIFO_EVENT_MTHD`, a completion notifier.
const FIFO_EVENT_MTHD: u32 = 35;

fn probed_policy(csv: &str) -> InitTablePolicy {
    InitTablePolicy::with_probe_arm(
        chip(),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
        kayfabe_abi::eventnotify::ProbeArmSet::parse(csv).expect("test set parses"),
    )
}

fn arming_of(event: u32) -> RpcCommand {
    event_command(
        &EventSetNotification {
            event,
            action: ACTION_REPEAT,
            notify_state: false,
            info32: 0,
            info16: 0,
        },
        EVENT_SET_NOTIFICATION_PARAMS_SIZE as u32,
    )
}

#[test]
fn the_probe_admits_exactly_the_named_index_and_the_default_still_refuses_it() {
    // ⊘ The default first, because the default is what every shipping boot runs: index 35
    // is a COMPLETION notifier and `InitTablePolicy::new` must refuse to arm it.
    assert_ne!(
        policy()
            .respond(&arming_of(FIFO_EVENT_MTHD))
            .expect("claimed")
            .rpc_result,
        0,
        "the DEFAULT policy probe-arms index 35 — the probe has become the product"
    );
    // The probe admits what it names…
    let mut probed = probed_policy("35");
    assert_eq!(
        probed
            .respond(&arming_of(FIFO_EVENT_MTHD))
            .expect("served")
            .rpc_result,
        0,
        "a probe naming 35 serves the arming — reachability instrumentation"
    );
    // …and nothing it does not name: 37 is another legal, non-silent index.
    assert_ne!(
        probed.respond(&arming_of(37)).expect("claimed").rpc_result,
        0,
        "the probe is a set, not a switch: an unnamed non-silent index stays refused"
    );
}

#[test]
fn the_probe_does_not_bypass_the_transition_rule_or_the_wire_bounds() {
    // ★ The probe widens ONE gate (SILENT_NOTIFIERS) and only that one. The already-armed
    // transition rule is RM's own and still applies to a probe-armed index…
    let mut probed = probed_policy("35");
    assert_eq!(
        probed
            .respond(&arming_of(FIFO_EVENT_MTHD))
            .expect("served")
            .rpc_result,
        0
    );
    assert_ne!(
        probed
            .respond(&arming_of(FIFO_EVENT_MTHD))
            .expect("claimed")
            .rpc_result,
        0,
        "REPEAT over REPEAT is refused even when the index is probe-armed"
    );
    // …and so does the decoder's own bound: a probe cannot admit an index the wire
    // refuses to decode.
    let out_of_range = NV2080_NOTIFIERS_MAXCOUNT;
    let mut probed_oob = probed_policy(&out_of_range.to_string());
    assert_ne!(
        probed_oob
            .respond(&arming_of(out_of_range))
            .expect("claimed")
            .rpc_result,
        0,
        "an out-of-range index is refused by the DECODE before any probe is consulted"
    );
}
