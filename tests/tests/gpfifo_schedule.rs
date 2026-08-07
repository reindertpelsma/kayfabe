//! ★★★ **`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) — task #177.**
//!
//! The control the bench guest died on for six weeks:
//!
//! ```text
//! NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
//! NVRM: nvAssertFailedNoLog: Assertion failed: status == NV_OK @ ce_utils.c:304
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
//! ```
//!
//! The argument for serving it is `docs/design/gpfifo_schedule.md`. This file is the part
//! of it a machine can check.
//!
//! # ★★ What makes the green mean something here, and it is NOT the reply
//!
//! Every field of `NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` is `[IN]` (`ogkm-580:
//! ctrla06fgpfifo.h:69-73`). There is no output to get right, so a test that only asserted
//! *"the control returns `NV_OK`"* would be asserting that this port can write a zero — and
//! it would pass just as happily against a policy that performed nothing at all. That is
//! the precise failure `sweep.rs`'s row for this id warned about, and it is why the
//! load-bearing test in this file is [`a_doorbell_is_refused_before_the_control_and_planned_after_it`]:
//! it checks the **transition**, from both sides, with the control as the only thing that
//! changes between them.
//!
//! ⊘ **What this file does not claim.** It is not a boot. Only a live boot says where the
//! guest actually stops (`only_live_boots_are_proof`); the boots this rung was measured
//! against are reported separately.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_abi::GuestOs;
use kayfabe_abi::submit::{
    GPFIFO_SCHEDULE_DOCUMENTED_STATUSES, GPFIFO_SCHEDULE_REFUSED_STATUS, GpfifoScheduleError,
    GpfifoScheduleParams, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
    decode_gpfifo_schedule, encode_gpfifo_schedule,
};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_chips::Ga10xArch;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, ScheduleFault};
use kayfabe_fwd::{FwdFault, handle_doorbell};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kayfabe_isolate::StillbornIsolates;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{OBJECT_CONTROLS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w};
use kayfabe_tests::{Scenario, identical_handles};
use kayfabe_trace::{FaultTag, Faulted};

// =================================================================================
// The wire fixture — transcribed from `ogkm`, INDEPENDENTLY of the decoder
// =================================================================================

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_*` field positions, transcribed from
/// `ogkm-580: src/common/sdk/nvidia/inc/nvos.h` rather than read out of
/// `kayfabe_chips::ga10x`'s private constants.
///
/// ★ The independence is the point (`isolate_the_drivers_own_checks`): flags built from the
/// decoder's own constants would assert the mirror against itself, and the whole reason
/// `vchid_from_userd_flags` exists is that this encoding is not obvious.
mod userd {
    /// `_CHANNEL_USERD_INDEX_VALUE` — 3 bits at 8.
    pub const VALUE_SHIFT: u32 = 8;
    /// `_CHANNEL_USERD_INDEX_FIXED` — 1 bit at 11. `_TRUE` makes RM answer
    /// `NV_ERR_INVALID_STATE`, so a schedulable channel always has it clear.
    pub const INDEX_FIXED_SHIFT: u32 = 11;
    /// `_CHANNEL_USERD_INDEX_PAGE_VALUE` — 9 bits at 12.
    pub const PAGE_VALUE_SHIFT: u32 = 12;
    /// `_CHANNEL_USERD_INDEX_PAGE_FIXED` — 1 bit at 21. Must be `_TRUE` or the flags name
    /// no channel at all.
    pub const PAGE_FIXED_SHIFT: u32 = 21;
    /// USERD entries per page — `RM_PAGE_SIZE / NV_RAMUSERD_CHAN_SIZE` = 4096 / 512.
    pub const CHANNELS_PER_PAGE: u32 = 8;

    /// The `NVOS04_FLAGS` word that names channel `chid`.
    #[must_use]
    pub fn flags_for(chid: u16) -> u32 {
        let chid = u32::from(chid);
        (1 << PAGE_FIXED_SHIFT)
            | (0 << INDEX_FIXED_SHIFT)
            | ((chid / CHANNELS_PER_PAGE) << PAGE_VALUE_SHIFT)
            | ((chid % CHANNELS_PER_PAGE) << VALUE_SHIFT)
    }
}

const CLIENT: u32 = 0xc1e0_0004;
const DEVICE: u32 = 0xcafe_0001;
const CHANNEL: u32 = 0xcafe_0010;
const CHID: u16 = 0x0010;

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

fn port_gpu() -> Gpu {
    Gpu::new(
        Box::new(Ga10xArch::new()),
        Box::new(StillbornIsolates::new("test: no forwarding plane")),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes")
}

fn command(msg: &[u8]) -> RpcCommand {
    let env = abi()
        .decode_rpc_envelope(msg)
        .expect("well-formed envelope");
    RpcCommand {
        function: FUNCTIONS.classify(env.function),
        code: env.function,
        sequence: env.sequence,
        payload: abi().rpc_payload(msg).expect("payload").to_vec(),
        elements: 1,
        delivered: msg[kayfabe_abi::view::RpcEnvelope::SIZE..].to_vec(),
    }
}

/// A policy that has already been told about one client, device, subdevice and channel —
/// i.e. the state the guest's `RmInitAdapter` is in when it issues this control.
fn policy_with_a_channel() -> ObjectPolicy {
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        port_gpu(),
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let mut s = w::RpcScript::new();
    s.client_root(w::NV01_ROOT, CLIENT, w::KERNEL_PID)
        .device(CLIENT, CLIENT, DEVICE, 0)
        .channel(
            CLIENT,
            DEVICE,
            CHANNEL,
            userd::flags_for(CHID),
            w::NV01_NULL_OBJECT,
            // ★ `hVASpace = NV01_NULL_OBJECT`, exactly as RM allocates the global CeUtils
            // scrubber: "For physical CE channels, we will use RM internal VAS to map
            // channel buffers" (`ogkm-580: channel_utils.c:86-93`). This is the channel
            // this control is asked about first, and it is asked about it *without* a VAS.
            w::NV01_NULL_OBJECT,
        );
    for msg in s.messages() {
        p.respond(&command(&msg));
    }
    p
}

/// A `0xa06f0103` on `(CLIENT, CHANNEL)` carrying `params`.
fn schedule_cmd(params: &[u8], seq: u32) -> RpcCommand {
    let mut s = w::RpcScript::new();
    s.control(CLIENT, CHANNEL, NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, params);
    let msgs = s.messages();
    let mut m = msgs.into_iter().next().expect("one message");
    // Re-stamp the sequence so a test can post several and tell them apart.
    let _ = seq;
    m.truncate(m.len());
    command(&m)
}

/// The exact bytes RM's scrubber sends: `bEnable = NV_TRUE`, both skips clear.
fn scrubber_params() -> Vec<u8> {
    vec![1, 0, 0]
}

// =================================================================================
// ★★★ THE RUNG — the transition, checked from both sides
// =================================================================================

/// ★★★ **The load-bearing test.** A doorbell on a channel the guest never scheduled is
/// refused **by name**; the identical doorbell after the guest's own
/// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` is not.
///
/// This is what makes serving `0xa06f0103` a *performed transition* rather than a word.
/// Before #177 `plan_doorbell` read scheduling as a **memo** —
/// `!proc.exec.scheduled.contains(&cid)` — and simply scheduled an unscheduled channel on
/// the fly, so an `NV_OK` here would have had nothing to perform and nothing could have
/// falsified it.
///
/// ⊘ Note what is deliberately NOT asserted: that the doorbell **succeeds** afterwards.
/// It does not, on this channel, and `docs/design/gpfifo_schedule.md` §3 says exactly why
/// (`hVASpace = NV01_NULL_OBJECT` ⇒ `FwdFault::NoVas`). The claim is that the refusal
/// **changes**, from "you never asked" to the next honest obstacle — which is the whole
/// difference between a gate and a wall.
#[test]
fn a_doorbell_is_refused_before_the_control_and_planned_after_it() {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu = Gpu::new(
        arch,
        Box::new(factory),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");

    let client = HClient(0xAA);
    let pdb = Pdb(0x11_0000);
    let mut s = Scenario::new();
    s.compute_process(client, pdb, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pid = *gpu
        .spine
        .by_pdb
        .get(&(GpuId::ZERO, pdb))
        .expect("routed by PDB");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("has a channel");

    // ---- BEFORE ---------------------------------------------------------------
    let before = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[])
        .expect_err("an unscheduled channel must not be rung");
    assert_eq!(
        before.fault_tag(),
        FaultTag("FwdFault::NotScheduled"),
        "★ the guest never issued 0xa06f0103, so its doorbell is refused BY NAME — not \
         silently scheduled on the fly, which is what this port did before #177"
    );
    assert!(
        matches!(before, FwdFault::NotScheduled { chan, .. } if chan == cid),
        "the refusal names the channel that was rung: {before:?}"
    );

    // ---- THE CONTROL ----------------------------------------------------------
    assert!(
        !gpu.procs[&pid].exec.requested.contains(&cid),
        "non-vacuity: the intent set is empty before the control"
    );
    gpu.procs
        .get_mut(&pid)
        .expect("live proc")
        .exec
        .requested
        .insert(cid);

    // ---- AFTER ----------------------------------------------------------------
    let after = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[]);
    let tag = match &after {
        Ok(_) => FaultTag("ok"),
        Err(f) => f.fault_tag(),
    };
    assert_ne!(
        tag,
        FaultTag("FwdFault::NotScheduled"),
        "★★★ the SAME doorbell, and the ONLY thing that changed is the guest's own \
         scheduling declaration. If this is still NotScheduled the control performs nothing."
    );
}

/// The withdrawal half: `bEnable = NV_FALSE` really un-schedules, and the doorbell goes
/// back to being refused by name.
///
/// ⊘ Not decoration. `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` with `bEnable` clear means *"remove
/// this channel from its runlist"*, and a port that recorded the enable but ignored the
/// disable would be telling the guest a channel is stopped while continuing to run it.
#[test]
fn withdrawing_the_declaration_refuses_the_doorbell_again() {
    let mut p = policy_with_a_channel();
    let gpu_client = HClient(CLIENT);
    let gpu_object = HObject(CHANNEL);

    let gpu = p.gpu().expect("a bare Gpu is behind this policy");
    let (pid, cid) = {
        let (&key, &(pid, cid)) = gpu
            .spine
            .by_chan
            .iter()
            .next()
            .expect("★ non-vacuity: the channel routed at all");
        let _ = key;
        (pid, cid)
    };

    // Enable, then disable, through the model's own entry point.
    {
        let gpu = p.gpu_mut().expect("a bare Gpu is behind this policy");
        assert!(
            gpu.schedule_channel(gpu_client, gpu_object, true)
                .expect("the channel routes")
                .changed
        );
        assert!(proc_requested(gpu, pid, cid), "enable recorded");
        let ack = gpu
            .schedule_channel(gpu_client, gpu_object, false)
            .expect("the channel still routes");
        assert!(ack.changed, "the withdrawal moved the set");
        assert!(!ack.enabled);
        assert!(
            !proc_requested(gpu, pid, cid),
            "★ a withdrawn declaration must really be gone, or a stopped channel keeps running"
        );
    }
}

fn proc_requested(gpu: &Gpu, pid: kayfabe_core::ProcId, cid: kayfabe_core::ChanId) -> bool {
    if pid == Gpu::SYSTEM_PROC {
        gpu.system.exec.requested.contains(&cid)
    } else {
        gpu.procs[&pid].exec.requested.contains(&cid)
    }
}

/// Re-scheduling an already-scheduled channel is `NV_OK` and reports `changed = false`.
/// RM does this on several paths; it is idempotent by construction and must not be an error.
#[test]
fn scheduling_twice_is_idempotent_and_says_so() {
    let mut p = policy_with_a_channel();
    let gpu = p.gpu_mut().expect("a bare Gpu");
    let first = gpu
        .schedule_channel(HClient(CLIENT), HObject(CHANNEL), true)
        .expect("routes");
    let second = gpu
        .schedule_channel(HClient(CLIENT), HObject(CHANNEL), true)
        .expect("routes");
    assert!(first.changed);
    assert!(!second.changed, "the second ask moved nothing");
    assert_eq!(first.chan, second.chan);
}

// =================================================================================
// The reply — hardware-sourced, and NOT from the C's empty row
// =================================================================================

/// The value RM's scrubber actually sends is served, and the reply carries the request's
/// own three bytes back.
///
/// ★★★ The expected body is `01 00 00` because that is what a **real GA106's own GSP**
/// answers — `traces/real_ga106/rpc_transcript_real_ga106.txt:59`, `cmd=0xa06f0103 psize=3
/// gspst=0x0 head=01 00 00`. ⊘ It is *not* taken from the C artifact's captured row, which
/// is `dlen = 0` (`C: mode2_initctrl_ga106.h:6234 = 0xa06f0103`) — one of the eleven empty rows the
/// FIFTH LIMIT contradicts. An empty capture is evidence of nothing.
#[test]
fn the_scrubbers_own_request_is_served_with_the_bytes_hardware_sends() {
    let mut p = policy_with_a_channel();
    let cmd = schedule_cmd(&scrubber_params(), 100);
    let reply = p.respond(&cmd).expect("★ this policy claims 0xa06f0103");
    assert_eq!(reply.rpc_result, 0, "NV_OK");

    let req = abi()
        .decode_rpc_control(&cmd.payload)
        .expect("the fixture is a control");
    let at = req.params_at;
    assert_eq!(
        &reply.body[at..at + GpfifoScheduleParams::SIZE],
        &[1u8, 0, 0],
        "★ the params the guest sent, echoed — the GSP transport copies a non-empty reply's \
         params over the caller's own struct (ogkm-580: rpc.c:11085-11090), so zero-filling \
         would clear the caller's bEnable behind its back"
    );
}

/// `paramsSize` is **3**, not 4. Three `NvBool`s are three bytes.
#[test]
fn the_params_are_three_bytes_and_a_four_byte_request_is_refused() {
    assert_eq!(GpfifoScheduleParams::SIZE, 3);
    let mut p = policy_with_a_channel();
    let reply = p
        .respond(&schedule_cmd(&[1, 0, 0, 0], 101))
        .expect("claimed");
    assert_eq!(reply.rpc_result, GPFIFO_SCHEDULE_REFUSED_STATUS);
    assert!(reply.body.is_empty());
}

// =================================================================================
// The refusal vocabulary — each variant, and the status it is NOT
// =================================================================================

/// ★★ `NV_ERR_NOT_SUPPORTED` (`0x56`) is **not** a documented return of this control, so
/// this port must never answer it once it has decided something. `0x56` is the FSM's
/// signature for *"nobody claimed this command"* — and it is exactly the number the bench
/// guest printed for six weeks.
#[test]
fn a_decided_refusal_is_never_the_unclaimed_signature() {
    assert!(
        !GPFIFO_SCHEDULE_DOCUMENTED_STATUSES.contains(&kayfabe_abi::NV_ERR_NOT_SUPPORTED),
        "ogkm-580: ctrla06fgpfifo.h:59-64 does not list NOT_SUPPORTED"
    );
    assert!(
        GPFIFO_SCHEDULE_DOCUMENTED_STATUSES.contains(&GPFIFO_SCHEDULE_REFUSED_STATUS),
        "★ the status this port refuses with must be one the guest's own driver documents"
    );
    assert_ne!(
        GPFIFO_SCHEDULE_REFUSED_STATUS,
        kayfabe_abi::NV_ERR_NOT_SUPPORTED
    );
}

/// `bSkipSubmit` / `bSkipEnable` name the **enabled-versus-scheduled split**, which this
/// port's single-membership model has no third value for. Refused by name, on every
/// combination — quantified over the set rather than sampled.
#[test]
fn the_enabled_versus_scheduled_split_is_refused_by_name() {
    for (skip_submit, skip_enable) in [(1u8, 0u8), (0, 1), (1, 1)] {
        for enable in [0u8, 1] {
            let err = decode_gpfifo_schedule(&[enable, skip_submit, skip_enable])
                .expect_err("this port does not model the split");
            assert_eq!(
                err,
                GpfifoScheduleError::UnmodelledSkip {
                    b_skip_submit: skip_submit,
                    b_skip_enable: skip_enable,
                },
                "the refusal names WHICH flags it cannot honour"
            );
        }
        let mut p = policy_with_a_channel();
        let reply = p
            .respond(&schedule_cmd(&[1, skip_submit, skip_enable], 110))
            .expect("claimed");
        assert_eq!(
            reply.rpc_result, GPFIFO_SCHEDULE_REFUSED_STATUS,
            "★ refused, and NOT quietly served by ignoring the flags — a guest that asked \
             for scheduled-but-not-submitted must not be given both"
        );
    }
}

/// A byte that is neither `NV_TRUE` nor `NV_FALSE` is a **decode** failure, not a policy
/// question — `l2evict`'s rule for an unnamed flag bit, applied to a malformed `NvBool`.
#[test]
fn a_byte_that_is_not_an_nvbool_is_a_decode_failure() {
    for (i, field) in ["bEnable", "bSkipSubmit", "bSkipEnable"].iter().enumerate() {
        let mut params = [0u8; 3];
        params[i] = 2;
        assert_eq!(
            decode_gpfifo_schedule(&params).expect_err("2 is not an NvBool"),
            GpfifoScheduleError::NonBoolean { field, value: 2 }
        );
    }
}

/// Both valid `bEnable` values decode, and neither skip flag set is the only modelled shape.
#[test]
fn both_enable_values_decode_and_only_they_do() {
    for enable in [0u8, 1] {
        let d = decode_gpfifo_schedule(&[enable, 0, 0]).expect("modelled");
        assert_eq!(d.b_enable, enable);
        assert!(d.is_modelled());
        assert_eq!(encode_gpfifo_schedule(&d), vec![enable, 0, 0]);
    }
}

/// Short params are refused rather than read past.
#[test]
fn short_params_are_refused() {
    for n in 0..GpfifoScheduleParams::SIZE {
        assert_eq!(
            decode_gpfifo_schedule(&vec![1u8; n]).expect_err("too short"),
            GpfifoScheduleError::ShortParams { got: n }
        );
    }
}

/// A handle that names nothing, and a handle that names something that is not a channel,
/// are **different** refusals — and neither is the unclaimed signature.
#[test]
fn a_handle_that_is_not_a_live_channel_is_refused_by_its_own_name() {
    let mut p = policy_with_a_channel();
    let gpu = p.gpu_mut().expect("a bare Gpu");
    assert!(matches!(
        gpu.schedule_channel(HClient(CLIENT), HObject(0xdead_beef), true),
        Err(ScheduleFault::UnknownChannel { .. })
    ));
    assert!(
        matches!(
            gpu.schedule_channel(HClient(CLIENT), HObject(DEVICE), true),
            Err(ScheduleFault::NotAChannel { .. })
        ),
        "★ a subdevice is a live resource and is not a channel; conflating the two would \
         report the guest naming the wrong object as us failing to place a right one"
    );

    let reply = {
        let mut s = w::RpcScript::new();
        s.control(
            CLIENT,
            0xdead_beef,
            NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
            &scrubber_params(),
        );
        let m = s.messages().into_iter().next().expect("one");
        p.respond(&command(&m)).expect("claimed")
    };
    assert_eq!(reply.rpc_result, GPFIFO_SCHEDULE_REFUSED_STATUS);
}

// =================================================================================
// The chain — the claim is narrow, and the ledger survives it
// =================================================================================

/// ★★★ The policy claims `RmControl` **by command id**, and `OBJECT_CONTROLS` is that
/// closed list. Quantified over the constant rather than restating it
/// (`gates_quantified_over_a_list`).
///
/// ⚠ One id → two at E9/§13.6: the channel-side bind (`0xa06f0104`) joined the claim.
/// The membership is pinned in full so growing the list is a **visible** diff here, not a
/// silent widening of what the ledger can no longer see.
#[test]
fn the_control_claim_is_exactly_these_ids() {
    assert_eq!(
        OBJECT_CONTROLS,
        &[
            NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
            kayfabe_abi::submit::NVA06F_CTRL_CMD_BIND,
        ]
    );
    assert!(
        !OBJECT_CONTROLS.contains(&NVA06C_CTRL_CMD_GPFIFO_SCHEDULE),
        "★ the TSG form is what we send the HOST, never what the guest asks us — \
         ogkm-580: mem_utils.c:1973-1989 issues the a06f form on a TSG-less channel"
    );
    assert!(
        !OBJECT_CONTROLS.contains(&kayfabe_abi::submit::NVA06C_CTRL_CMD_BIND),
        "★ likewise the TSG-side bind (0xa06c0102): host-facing, never guest-claimed"
    );
}

/// ⊘ **The cost of getting the claim wrong, made a test.** If `ObjectPolicy` claimed the
/// `RmControl` *function* instead of these ids, it would answer every control in the port;
/// `PolicyChain::respond` is a `find_map`, so the `UnservicedLedger` at the end of the
/// chain would go permanently silent — and that ledger is this port's primary instrument
/// for "what has the guest asked for that we do not answer".
///
/// This asserts the opposite holds: a control **not** in `OBJECT_CONTROLS` is declined by
/// this policy (`None`), so the chain carries on to whoever else wants it.
#[test]
fn every_other_control_is_still_declined_so_the_ledger_lives() {
    let mut p = policy_with_a_channel();
    // A spread of ids this port refuses, decides elsewhere, or has never seen — including
    // the TSG form, which is the one a careless claim would swallow.
    for cmd in [
        NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
        // ⚠ 0xa06f_0104 (the channel-side bind) left this list at E9/§13.6 — it is now
        // CLAIMED, and its own suite (`bind_channel.rs`) asserts it is always decided.
        kayfabe_abi::submit::NVA06C_CTRL_CMD_BIND,
        0xc36f_0108,
        0x2080_0a6c,
        0x2080_0301,
        0x2080_0a4b,
        0xdead_0000,
    ] {
        let mut s = w::RpcScript::new();
        s.control(CLIENT, CHANNEL, cmd, &[0u8; 4]);
        let m = s.messages().into_iter().next().expect("one");
        assert!(
            p.respond(&command(&m)).is_none(),
            "★ {cmd:#010x} must fall through to the rest of the chain, or the unserviced \
             ledger stops recording anything at all"
        );
    }
}

/// A non-control RPC function is still declined by the control arm and handled by the
/// object arm — i.e. adding the narrow control claim did not disturb `OBJECT_VERBS`.
#[test]
fn adding_the_control_claim_did_not_widen_the_object_verbs() {
    for f in kayfabe_rmrpc::OBJECT_VERBS {
        assert_ne!(
            *f,
            RpcFunction::RmControl,
            "★ RmControl must never be in OBJECT_VERBS — see \
             `every_other_control_is_still_declined_so_the_ledger_lives`"
        );
    }
}

/// The triage row for this id is still present and now records that it is served, with its
/// original argument corrected rather than deleted.
#[test]
fn the_triage_row_survives_and_records_the_correction() {
    let row = kayfabe_device::sweep::triage_for(NVA06F_CTRL_CMD_GPFIFO_SCHEDULE)
        .expect("★ the row must not be deleted when the control is served");
    assert!(
        row.why.contains("SERVED as of #177"),
        "the row must say it is served"
    );
    assert!(
        row.why.contains("CORRECTED rather than deleted"),
        "and must keep the argument it is correcting"
    );
    assert!(
        row.why.contains("hVASpace = NV01_NULL_OBJECT"),
        "★ and must keep naming what is STILL false — the scrubber has no VAS, so its \
         first doorbell refuses NoVas"
    );
}
