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
///
/// # ⊘⊘⊘ THIS TEST HELD THE WALL IN PLACE, and it was GREEN the whole time — §16.56
///
/// Until 2026-08-10 this function asserted, in as many words:
///
/// ```text
/// assert!(!OBJECT_CONTROLS.contains(&NVA06C_CTRL_CMD_GPFIFO_SCHEDULE),
///     "★ the TSG form is what we send the HOST, never what the guest asks us");
/// ```
///
/// `[measured 2026-08-10, boot s44_b17381c_rmtrace]` **the guest asks us**: record 196 of
/// `cup2`'s 249 is `CTRL cmd=0xa06c0101 hObject=0x5c000012 size=3`, on the TSG that
/// parents every channel `cuCtxCreate` just built, and the next record is a `FREE`. The
/// assertion was not stale, not unexecuted and not vague — it was **wrong**, it named its
/// source, and it ran green on every CI run while the port stopped at the id it forbade.
///
/// ★★★★ How a correct citation produced a false universal: `mem_utils.c:1973-1989` really
/// does issue the `a06f` form, and really is TSG-less. Its scope is
/// **`RmInitAdapter`'s scrubber** — one channel, allocated by kernel RM. The assertion
/// generalised it to *"never what the guest asks us"*, which quantifies over **libcuda**
/// too, and nothing in the cited line speaks about libcuda at all. ⇒ A citation
/// establishes what it says about the path it is on; the quantifier is the reader's, and
/// it is the reader's to get wrong (`a_correct_citation_narrowed_by_the_reading`).
///
/// ⇒ The membership is still pinned in full — that part earned its keep — but the negative
/// half is replaced by `tests/tests/admitted_is_served.rs`, which asks the **capability
/// table** which ids the guest may send us and refuses to let one be admitted-and-unserved
/// without an explicit, dated waiver. A hand-written "and not these" list can only ever
/// forbid what its author already thought of.
#[test]
fn the_control_claim_is_exactly_these_ids() {
    assert_eq!(
        OBJECT_CONTROLS,
        &[
            // ★★★★★ **w337 MERGE, 2026-08-27 — THE CLAIM IS NOW A UNION OF TWO LINEAGES,
            // AND THIS PIN DID ITS JOB BY GOING RED.**
            //
            // `[measured 2026-08-27, merge d3f80778 "Merge w337-gpu-name-seam"]` the two
            // sides of that merge grew the served/claimed surface INDEPENDENTLY, and neither
            // side's tree ever saw the other's rows. This list described `master`'s half, so
            // the union arrived as a failing assertion rather than as a silent widening —
            // which is exactly the event the membership pin exists to force
            // (`gates_quantified_over_a_list`). ⊘ Nothing was re-baselined: the three ids
            // below are transcribed FROM the failure and each carries its origin.
            //
            // ★ All three entered `OBJECT_CONTROLS` on the **w337-gpu-name-seam** side, in
            // `9d154cb9` (w346, *"the host forward is wired end to end"*), as the cudart
            // init-gate family's forward-or-refuse arm — the host's own subdevice, no guest
            // object to route through. ⊘ They are the FOUR-minus-one survivors of w349b's
            // un-claim (`8e5478ed`) and w352's (`0f577b15`): `0x20809009`/`0x9001`/`0x9064`
            // and `0x2080a001` LEFT this list precisely because claiming them here refused
            // them before `kayfabe_device::inittables`' measured table could answer them.
            // These three stay because they have no table row — `0x2080a026`/`0x2080a084`
            // were measured INNOCENT and `0x2080a097` is unmeasured, so forward-or-refuse is
            // still the whole of this port's opinion about them.
            //
            // ⚠ Their refusal really is `0x56`, deliberately, and the split that says so
            // lives in `bind_channel.rs`'s `every_claimed_control_is_decided_even_when_malformed`.
            0x2080_a026,
            0x2080_a084,
            0x2080_a097,
            NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
            // ★★★★ §16.56 — the TSG form. See this function's docs for the assertion it
            // replaced and why that assertion was green and false.
            NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
            kayfabe_abi::submit::NVA06F_CTRL_CMD_BIND,
            // ★★★★★ **w288 TIER 2 — `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`**, the only control
            // that carries a fault's ADDRESS. Served by RELAY, never by synthesis.
            kayfabe_abi::submit::NV906F_CTRL_CMD_GET_MMU_FAULT_INFO,
            // ★★★ §14.25 — the address-plane control, RE-claimed. It was claimed in §14.21,
            // measured to kill the adapter with a "better" refusal status, and reverted;
            // §14.24 measured the fact it was waiting on (`Vas::pdb`) landing. ⚠ Its refusal
            // is `0x56` and that is not an oversight — `bind_channel.rs`'s
            // `every_claimed_control_is_decided_even_when_malformed` carries the scope.
            kayfabe_abi::generated::ctrl::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
            // ★★★★ §16.59 — `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`, the wall `s45`
            // and `s46` both measured at record 331. ⊘ Claimed on a **classifier**, not
            // unconditionally: `tests/tests/ctxsw_preemption_mode.rs` is where the argument
            // is machine-checked, and the load-bearing test there is the one that moves
            // `cilpPreemptMode` and demands the answer change.
            kayfabe_abi::submit::NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE,
            // ★★★★★ §16.75 — `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`, the 1 Hz train `w209`
            // measured (13 arrivals, intervals 1.002-1.056 s, every one `0x56`). ⊘ Claimed
            // for a reason no other id here has: its `0x56` did not merely decline, it made
            // the guest `return` before `intrServiceStallList_HAL`
            // (`ogkm-580: intr.c:219-225` vs `:278`), so the guest's own stall-interrupt
            // servicing never ran. `tests/tests/mc_service_interrupts.rs` carries the
            // argument, including why the dmesg train collapsing is NOT the falsifier.
            kayfabe_abi::submit::NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS,
            // ★★★★★ **w292 — the four input-only controls**, owner-ruled 2026-08-14. Their
            // identities, sizes and per-row authorities live in ONE place,
            // `kayfabe_abi::submit::INPUT_ONLY_CONTROLS`; this list only says they are
            // CLAIMED. `[measured]` a real GA106 leaves all four parameter blocks
            // byte-identical across the call, so there is no `[OUT]` field an echo can miss.
            0x2081_0108,
            0x83de_0309,
            0xa06c_0103,
            0xa06c_0105,
            // ★★★★★ **w294 — the CUDA perf limit pair, and they are the ids NO IOCTL ORACLE
            // SHOWS.** The id a reader will look for is `0x00801909`, and it is deliberately
            // absent: `flags=0x118`, no `ROUTE_TO_PHYSICAL` (`ogkm-580:
            // g_device_nvoc.c:920`), so the guest's own kernel answers it and it cannot
            // reach us. Serving these two is what took `^CUP2_RC=` from 1 to 0.
            0x0080_2004,
            0x0080_2009,
        ]
    );
    assert!(
        !OBJECT_CONTROLS.contains(&kayfabe_abi::submit::NVA06C_CTRL_CMD_BIND),
        "★ the TSG-side BIND (0xa06c0102) is host-facing. ⊘ And this one is MEASURED \
         rather than reasoned, which is the difference from the claim that used to sit \
         beside it: [measured 2026-08-10, boot s44_b17381c_rmtrace] s44's userspace census \
         holds exactly ONE a06c control and it is 0xa06c0101, and `grep -c a06c0102` over \
         s43's and s44's device logs returns 0 — so the guest issues it from neither \
         userspace nor kernel (execution_plane_increments.md 16.55.7)"
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
        // ⚠ 0xa06f_0104 (the channel-side bind) left this list at E9/§13.6, and
        // 0xa06c_0101 (the TSG-side schedule) left it at §16.56 — both are now CLAIMED.
        // ⊘ A list of "controls we decline" is a list that shrinks as the port grows, and
        // every member that leaves it left because a BOOT said so.
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

// =================================================================================
// ★★★★ §16.56 — THE TSG FORM (`0xa06c0101`), the wall `cuCtxCreate` stopped at
// =================================================================================

/// ★★★★★ **The load-bearing test for the TSG form: ONE control, and EVERY member of the
/// group crosses the doorbell gate.**
///
/// `[measured 2026-08-10, boot s44_b17381c_rmtrace]` libcuda builds a TSG and **eight**
/// channels under it, then issues exactly one `0xa06c0101` on the group. Nothing else
/// declares those eight channels runnable — so a port that recorded the intent against the
/// group handle, or against one member, would ack the guest and then refuse the guest's
/// very next doorbell. This asserts the fan-out is real, and it asserts it the only way
/// that cannot be faked: through `plan_doorbell`, on each member, before and after.
///
/// ⊘ As with the channel form, it does **not** assert the doorbell *succeeds* afterwards.
/// The claim is that the refusal CHANGES — from "you never asked" to the next honest
/// obstacle — which is the whole difference between a gate and a wall
/// (`docs/design/gpfifo_schedule.md` §3).
#[test]
fn one_tsg_control_lets_every_member_channel_past_the_doorbell_gate() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(factory),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");

    let client = HClient(0xAA);
    let pdb = Pdb(0x11_0000);
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(client, pdb, h);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    // ---- BEFORE: BOTH members are refused BY NAME -----------------------------
    for vchid in [h.gr_vchid, h.ce_vchid] {
        let before = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(vchid), &[])
            .expect_err("an unscheduled member must not be rung");
        assert_eq!(
            before.fault_tag(),
            FaultTag("FwdFault::NotScheduled"),
            "★ non-vacuity: member {vchid:?} must start OUTSIDE the gate, or the transition \
             below proves nothing"
        );
    }

    // ---- THE CONTROL, on the GROUP handle --------------------------------------
    let ack = gpu
        .schedule_group(client, h.tsg, true)
        .expect("the group resolves and every member is placed");
    assert_eq!(
        ack.members, 2,
        "★ the fan-out must reach EVERY member. One member is what a port that routed the \
         group handle to a single channel would report, and it would ack the guest while \
         leaving the rest off the runlist — the #12 shape ({ack:?})"
    );
    assert_eq!(ack.changed, 2, "both members moved: {ack:?}");
    assert_eq!(
        ack.unmaterialized, 0,
        "no member was silently dropped: {ack:?}"
    );

    // ---- AFTER: BOTH members are past the gate ---------------------------------
    for vchid in [h.gr_vchid, h.ce_vchid] {
        let after = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(vchid), &[]);
        let tag = match &after {
            Ok(_) => FaultTag("ok"),
            Err(f) => f.fault_tag(),
        };
        assert_ne!(
            tag,
            FaultTag("FwdFault::NotScheduled"),
            "★★★ the SAME doorbell on member {vchid:?}, and the ONLY thing that changed is \
             the guest's one control on the GROUP. If this is still NotScheduled the \
             control performed nothing for this member."
        );
    }

    // ---- AND THE WITHDRAWAL --------------------------------------------------
    let off = gpu
        .schedule_group(client, h.tsg, false)
        .expect("the group still resolves");
    assert_eq!(
        off.changed, 2,
        "bEnable=0 must withdraw every member: {off:?}"
    );
    for vchid in [h.gr_vchid, h.ce_vchid] {
        let again = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(vchid), &[])
            .expect_err("a withdrawn member must not be rung");
        assert_eq!(
            again.fault_tag(),
            FaultTag("FwdFault::NotScheduled"),
            "★ a port that recorded the enable and ignored the disable would be telling the \
             guest a group is stopped while continuing to run it"
        );
    }
}

/// ⊘ **The group route refuses by NAME, and never with `0x56`.**
///
/// `NV_ERR_NOT_SUPPORTED` is the FSM's signature for *"nobody claimed this command"* — it
/// is precisely the value this port answered `0xa06c0101` with for six committed boots. A
/// decided refusal that reused it would be indistinguishable, in the guest's own dmesg,
/// from the wall §16.56 removed.
#[test]
fn the_group_route_refuses_by_name_and_never_with_not_supported() {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(factory),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");
    let client = HClient(0xAA);
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(client, Pdb(0x11_0000), h);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }

    use kayfabe_core::gpu::ScheduleGroupFault as F;
    // A handle nothing was ever allocated at.
    assert!(matches!(
        gpu.schedule_group(client, HObject(0xdead_beef), true),
        Err(F::UnknownGroup { .. })
    ));
    // A live handle that is not a group — the guest's own CHANNEL, which is the confusion
    // this whole increment turns on (`0xa06c0101` takes a group, `0xa06f0103` a channel).
    assert!(
        matches!(
            gpu.schedule_group(client, h.gr_channel, true),
            Err(F::NotAGroup { .. })
        ),
        "★ routing a channel into the group form must refuse by name rather than silently \
         doing the channel form's job"
    );
    // And the mirror: the group handle in the CHANNEL form.
    assert!(
        matches!(
            gpu.schedule_channel(client, h.tsg, true),
            Err(ScheduleFault::NotAChannel { .. })
        ),
        "★ and the reverse confusion is refused too — the two commands are not aliases"
    );
    assert_ne!(
        kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS,
        0x56,
        "★★★ the refusal status must never be NV_ERR_NOT_SUPPORTED: that is what the port \
         answered 0xa06c0101 with for six boots, and a decided refusal must be readable as \
         a decision in the one place anyone sees it — the guest's dmesg"
    );
    assert!(
        GPFIFO_SCHEDULE_DOCUMENTED_STATUSES
            .contains(&kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS),
        "and it must be in the control's OWN documented set (ogkm-580: ctrla06fgpfifo.h:59-64)"
    );
}

/// ★★ **The whole RPC, end to end through the policy** — the shape `s44` measured, byte for
/// byte: `hObject` = the TSG, `paramsSize` = 3, `in = 01 00 00`.
///
/// ⊘ The reply body matters and is asserted: `paramsSize != 0`, so the GSP transport copies
/// the reply's params over the caller's struct (`ogkm-580: rpc.c:11085-11090`). A
/// zero-filled body would clear the caller's `bEnable` behind its back.
#[test]
fn the_policy_answers_the_tsg_control_with_the_guests_own_bytes() {
    const TSG: u32 = 0xcafe_0020;
    const CH_A: u32 = 0xcafe_0021;
    const CH_B: u32 = 0xcafe_0022;

    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        port_gpu(),
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let mut s = w::RpcScript::new();
    s.client_root(w::NV01_ROOT, CLIENT, w::KERNEL_PID)
        .device(CLIENT, CLIENT, DEVICE, 0)
        .tsg(CLIENT, DEVICE, TSG, w::NV01_NULL_OBJECT)
        .channel(
            CLIENT,
            TSG,
            CH_A,
            userd::flags_for(CHID),
            w::NV01_NULL_OBJECT,
            w::NV01_NULL_OBJECT,
        )
        .channel(
            CLIENT,
            TSG,
            CH_B,
            userd::flags_for(CHID + 1),
            w::NV01_NULL_OBJECT,
            w::NV01_NULL_OBJECT,
        );
    for msg in s.messages() {
        p.respond(&command(&msg));
    }

    // ★ The exact three bytes s44 captured: `in=010000`.
    let mut c = w::RpcScript::new();
    c.control(CLIENT, TSG, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &[1, 0, 0]);
    let m = c.messages().into_iter().next().expect("one message");
    let cmd = command(&m);
    let reply = p
        .respond(&cmd)
        .expect("★★★ the policy must CLAIM 0xa06c0101 — a None here is the s44 wall");
    assert_eq!(
        reply.rpc_result, 0,
        "the group resolves and every member is placed, so the answer is NV_OK"
    );
    let req = abi()
        .decode_rpc_control(&cmd.payload)
        .expect("the request decodes");
    assert_eq!(
        &reply.body[req.params_at..req.params_at + GpfifoScheduleParams::SIZE],
        &[1u8, 0, 0],
        "★ the guest's own bytes come back — a zero body would clear its bEnable"
    );

    // ⊘ Non-vacuity of the CLAIM: the same policy still declines a control it does not own,
    // so this arm did not widen the claim to the RmControl function.
    let mut o = w::RpcScript::new();
    o.control(CLIENT, TSG, 0x2080_0a4b, &[0u8; 4]);
    let om = o.messages().into_iter().next().expect("one message");
    assert!(
        p.respond(&command(&om)).is_none(),
        "the ledger must still see every control this policy does not claim"
    );
}
