//! ★★★ **`NVA06F_CTRL_CMD_BIND` (`0xa06f0104`) — E9/§13.6.**
//!
//! The fourth ask of the execution plane's one requirement — after `0xa06f0103`
//! (schedule), `0xc36f0108` (token) and the index-35 arming — and the control the bench
//! guest's *global CeUtils* channel dies on:
//!
//! ```text
//! NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to bind Channel, status: 56
//! ```
//!
//! The design decision under test is `docs/design/execution_plane_increments.md` §13.6
//! **option (2)**: the engine check asks *"is this an engine THIS DEVICE advertised?"*
//! against the same `ChipProfile::engines` slice the device-info path serves the guest,
//! carried by `ObjectPolicy` from construction — required, never defaulted.
//!
//! # ★★ What makes the green mean something here
//!
//! Unlike `0xa06f0103`, this control has a real decision surface: the engine ordinal
//! arrives in `NV2080_ENGINE_TYPE` space, the advertised table is in `RM_ENGINE_TYPE`
//! space, and the two spaces **collide above `0x12`** while agreeing below it — which is
//! precisely what makes a raw compare look correct in every obvious test. The
//! load-bearing test is therefore [`the_sw_engine_is_served_only_because_the_spaces_are_converted`]:
//! `SW` is the one ordinal where the two spaces disagree *and* this device has a row, so
//! it goes red under a raw compare while every COPY/GR test stays green.
//!
//! ⊘ **What this file does not claim.** It is not a boot, and `NV_OK` here records a
//! declaration (`ExecPlane::bound`) — the host-side runlist act stays deferred to the
//! doorbell. Only a live boot says where the guest actually stops
//! (`only_live_boots_are_proof`).

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_abi::GuestOs;
use kayfabe_abi::submit::{
    BIND_DOCUMENTED_STATUSES, BIND_REFUSED_STATUS, BIND_STATUSES_THE_CODE_PRODUCES,
    BIND_UNKNOWN_ENGINE_STATUS, BindParams, NVA06F_CTRL_CMD_BIND, decode_bind, encode_bind,
    nv2080_to_rm_engine_type,
};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{HClient, HObject};
use kayfabe_chips::Ga10xArch;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{BindFault, Gpu};
use kayfabe_device::ga10x::GA106_ENGINES;
use kayfabe_gsp::{CommandPolicy, RpcCommand};
use kayfabe_isolate::StillbornIsolates;
use kayfabe_rmrpc::{OBJECT_CONTROLS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w};

// =================================================================================
// The fixture — the same client/device/channel world `gpfifo_schedule.rs` builds
// =================================================================================

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_*` — transcribed from `ogkm-580: nvos.h`
/// independently of the decoder (`isolate_the_drivers_own_checks`), exactly as
/// `gpfifo_schedule.rs` does and for its reason.
mod userd {
    pub const VALUE_SHIFT: u32 = 8;
    pub const INDEX_FIXED_SHIFT: u32 = 11;
    pub const PAGE_VALUE_SHIFT: u32 = 12;
    pub const PAGE_FIXED_SHIFT: u32 = 21;
    pub const CHANNELS_PER_PAGE: u32 = 8;

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

/// `[measured]` what a real GA106 was asked and answered —
/// `traces/real_ga106/rpc_transcript_real_ga106.txt:63`: `cmd=0xa06f0104 psize=4
/// gspst=0x0 head=0b 00 00 00`. `engineType = 11` = `NV2080_ENGINE_TYPE_COPY2`.
const MEASURED_GA106_ENGINE: u32 = 11;

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
/// the state RM is in when the global CeUtils channel's bind arrives.
fn policy_with_a_channel() -> ObjectPolicy {
    let mut p = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu(), GA106_ENGINES);
    let mut s = w::RpcScript::new();
    s.client_root(w::NV01_ROOT, CLIENT, w::KERNEL_PID)
        .device(CLIENT, CLIENT, DEVICE, 0)
        .channel(
            CLIENT,
            DEVICE,
            CHANNEL,
            userd::flags_for(CHID),
            w::NV01_NULL_OBJECT,
            w::NV01_NULL_OBJECT,
        );
    for msg in s.messages() {
        p.respond(&command(&msg));
    }
    p
}

/// A `0xa06f0104` on `(CLIENT, object)` carrying `params`.
fn bind_cmd_on(object: u32, params: &[u8]) -> RpcCommand {
    let mut s = w::RpcScript::new();
    s.control(CLIENT, object, NVA06F_CTRL_CMD_BIND, params);
    let m = s.messages().into_iter().next().expect("one message");
    command(&m)
}

fn bind_cmd(params: &[u8]) -> RpcCommand {
    bind_cmd_on(CHANNEL, params)
}

/// The advertised RM-space engine set, computed from the slice under test rather than
/// restated — shortening `GA106_ENGINES` moves this set with it, so no assertion below
/// quietly outlives the table it is about.
fn advertised_rm_types() -> Vec<u32> {
    GA106_ENGINES
        .iter()
        .map(|e| e.engine_data[kayfabe_abi::inittables::engine_info_type::RM_ENGINE_TYPE])
        .collect()
}

// =================================================================================
// ★★★ THE RUNG — the measured request is served, with the measured reply
// (the run: traces/real_ga106/rpc_transcript_real_ga106.txt:63, 2026-08-01)
// =================================================================================

/// ★★★ The exact request a real GA106 was asked — `engineType = 11`, COPY2 — is served
/// `NV_OK`, and the reply body is the request's own four bytes, which is byte-for-byte
/// what the real GA106's GSP answered (`0b 00 00 00`).
///
/// ⊘ The echo is not decoration: the GSP transport copies a non-empty reply's params over
/// the caller's own struct (`ogkm-580: rpc.c:11085-11090`), so a zero-filled body would
/// rewrite the caller's `engineType` to `NV2080_ENGINE_TYPE_NULL` behind its back — and
/// the C's captured row for this id is one of the eleven empty ones, so the C would have
/// done exactly that. An empty capture is evidence of nothing.
#[test]
fn the_measured_ga106_bind_is_served_with_the_measured_echo() {
    let mut p = policy_with_a_channel();
    let wire = MEASURED_GA106_ENGINE.to_le_bytes();
    let cmd = bind_cmd(&wire);
    let reply = p.respond(&cmd).expect("★ this policy claims 0xa06f0104");
    assert_eq!(reply.rpc_result, 0, "NV_OK");

    let req = abi()
        .decode_rpc_control(&cmd.payload)
        .expect("the fixture is a control");
    assert_eq!(
        &reply.body[req.params_at..req.params_at + BindParams::SIZE],
        &[0x0b, 0, 0, 0],
        "★ the four bytes a real GA106 answers — the request echoed, not zeros"
    );
}

/// The served bind is **recorded**: `ExecPlane::bound` maps the channel to the RM-space
/// engine. This is what makes the `NV_OK` a performed transition rather than a word —
/// the declared-versus-performed split `exec.requested` already draws.
#[test]
fn a_served_bind_is_recorded_in_the_owning_procs_plane() {
    let mut p = policy_with_a_channel();
    let reply = p
        .respond(&bind_cmd(&MEASURED_GA106_ENGINE.to_le_bytes()))
        .expect("claimed");
    assert_eq!(reply.rpc_result, 0);

    let gpu = p.gpu().expect("a bare Gpu is behind this policy");
    let (&_key, &(pid, cid)) = gpu
        .spine
        .by_chan
        .iter()
        .next()
        .expect("★ non-vacuity: the channel routed at all");
    let bound = if pid == Gpu::SYSTEM_PROC {
        &gpu.system.exec.bound
    } else {
        &gpu.procs[&pid].exec.bound
    };
    assert_eq!(
        bound.get(&cid),
        Some(&MEASURED_GA106_ENGINE),
        "★ COPY2 is 11 in BOTH spaces (the identity range), so the recorded RM value is 11"
    );
}

/// Re-binding to the same engine is `NV_OK` and reports `changed = false`; re-binding to
/// a *different* advertised engine moves the record. Idempotent, and honest about it —
/// [`kayfabe_core::gpu::ScheduleAck::changed`]'s census reason.
#[test]
fn rebinding_is_idempotent_and_a_new_engine_moves_the_record() {
    let mut p = policy_with_a_channel();
    let gpu = p.gpu_mut().expect("a bare Gpu");
    let first = gpu
        .bind_channel(HClient(CLIENT), HObject(CHANNEL), 11)
        .expect("routes");
    let second = gpu
        .bind_channel(HClient(CLIENT), HObject(CHANNEL), 11)
        .expect("routes");
    let third = gpu
        .bind_channel(HClient(CLIENT), HObject(CHANNEL), 9)
        .expect("routes");
    assert!(first.changed);
    assert!(!second.changed, "the second identical ask moved nothing");
    assert!(third.changed, "a different engine is a real transition");
    assert_eq!(third.rm_engine_type, 9);
    assert_eq!(first.chan, third.chan);
}

// =================================================================================
// ★★★ The engine check — converted FIRST, then asked of THIS device's own table
// =================================================================================

/// ★★★ **The load-bearing test for the conversion.** `NV2080_ENGINE_TYPE_SW` is `0x22`;
/// `RM_ENGINE_TYPE_SW` is `0x2d`; this device's table carries a `SOFTWARE` row at `0x2d`
/// and **no row at `0x22`**. So a bind for `SW`:
///
/// - **serves** under convert-then-lookup (0x22 → 0x2d → found), and
/// - **refuses 0x57** under a raw compare (0x22 not in the table).
///
/// Every GR/COPY ordinal sits in the identity range where the two spaces agree, so this
/// is the one test in the file a raw-compare mutant cannot pass
/// (`mock_fidelity_both_directions` — bitten, see the bite log in the commit).
#[test]
fn the_sw_engine_is_served_only_because_the_spaces_are_converted() {
    // Non-vacuity for the premise, from the slice itself:
    let rm = advertised_rm_types();
    assert!(rm.contains(&0x2d), "the SOFTWARE row is advertised at 0x2d");
    assert!(
        !rm.contains(&0x22),
        "★ and NOTHING is advertised at raw 0x22 — the premise a raw compare fails on"
    );
    let mut p = policy_with_a_channel();
    let reply = p
        .respond(&bind_cmd(&0x22u32.to_le_bytes()))
        .expect("claimed");
    assert_eq!(
        reply.rpc_result, 0,
        "★★★ SW must be SERVED: the wire's 0x22 names the same engine the table's 0x2d \
         row advertises. A 0x57 here is the raw-compare defect — the same species as \
         reading a VA as a GPA"
    );
}

/// A structurally valid engine this chip does **not** have — `COPY9` (`0x12`), which
/// converts cleanly (identity range) but has no row — answers
/// `NV_ERR_OBJECT_NOT_FOUND` (`0x57`), the status a real GSP's linear scan returns
/// (`ogkm-580: kernel_fifo_gm107.c:736`). Quantified over several such ordinals, with
/// the premise checked against the slice each time.
#[test]
fn an_engine_this_device_never_advertised_is_refused_object_not_found() {
    let rm = advertised_rm_types();
    // COPY4..COPY9 convert in the identity range; GA106's wire table stops at COPY3.
    // GR1 (0x02) likewise converts and is absent — one GA102-shaped probe.
    for nv2080 in [0x0du32, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x02] {
        let converted = nv2080_to_rm_engine_type(nv2080).expect("identity range converts");
        assert!(
            !rm.contains(&converted),
            "premise: {converted:#x} is not advertised by GA106_ENGINES"
        );
        let mut p = policy_with_a_channel();
        let reply = p
            .respond(&bind_cmd(&nv2080.to_le_bytes()))
            .expect("claimed");
        assert_eq!(
            reply.rpc_result, BIND_UNKNOWN_ENGINE_STATUS,
            "★ {nv2080:#x} converts but names an engine this device never advertised"
        );
        assert!(reply.body.is_empty(), "a refusal carries no params");
    }
}

/// ★★ The collision ordinal: raw `0x13` is `NVDEC0` in NV2080 space and `COPY10` in RM
/// space. This port models no conversion for it, so it refuses `0x57` — and must NEVER
/// be accepted as a bind to an eleventh copy engine. Same for `NULL` (0) and the
/// video/second-decade ordinals.
#[test]
fn unconverted_ordinals_are_refused_not_reinterpreted() {
    for nv2080 in [0x00u32, 0x13, 0x14, 0x1b, 0x34, 0xffff_ffff] {
        assert_eq!(
            nv2080_to_rm_engine_type(nv2080),
            None,
            "premise: {nv2080:#x} is unmodelled"
        );
        let mut p = policy_with_a_channel();
        let reply = p
            .respond(&bind_cmd(&nv2080.to_le_bytes()))
            .expect("claimed");
        assert_eq!(
            reply.rpc_result, BIND_UNKNOWN_ENGINE_STATUS,
            "★ {nv2080:#x} names nothing bindable on this device"
        );
    }
}

/// Every engine the device DOES advertise in the NV2080-convertible range is served —
/// the acceptance direction, quantified over the slice rather than sampled, so a row
/// added to `GA106_ENGINES` is automatically covered.
#[test]
fn every_advertised_convertible_engine_is_served() {
    // Walk the NV2080 ordinals this port converts; serve exactly those whose RM image is
    // advertised. (GR0=1, COPY0..3=9..12 are identity; SW is 0x22→0x2d.)
    for nv2080 in (0x01u32..=0x12).chain([0x22]) {
        let Some(converted) = nv2080_to_rm_engine_type(nv2080) else {
            continue;
        };
        if !advertised_rm_types().contains(&converted) {
            continue;
        }
        let mut p = policy_with_a_channel();
        let reply = p
            .respond(&bind_cmd(&nv2080.to_le_bytes()))
            .expect("claimed");
        assert_eq!(
            reply.rpc_result, 0,
            "★ {nv2080:#x} converts to advertised {converted:#x} and must be served"
        );
    }
}

// =================================================================================
// The channel refusal — a different question, a different status
// =================================================================================

/// A real engine on a handle that is not a live channel is the **channel** refusal
/// (`0x40`), not the engine refusal — and each fault keeps its own name in the core.
#[test]
fn a_handle_that_is_not_a_live_channel_is_refused_invalid_state() {
    let mut p = policy_with_a_channel();
    {
        let gpu = p.gpu_mut().expect("a bare Gpu");
        assert!(matches!(
            gpu.bind_channel(HClient(CLIENT), HObject(0xdead_beef), 11),
            Err(BindFault::UnknownChannel { .. })
        ));
        assert!(
            matches!(
                gpu.bind_channel(HClient(CLIENT), HObject(DEVICE), 11),
                Err(BindFault::NotAChannel { .. })
            ),
            "★ a device is a live resource and is not a channel; the two refusals must \
             not conflate — only ChannelNotMaterialized is OUR defect"
        );
    }
    for object in [0xdead_beef, DEVICE] {
        let reply = p
            .respond(&bind_cmd_on(object, &MEASURED_GA106_ENGINE.to_le_bytes()))
            .expect("claimed");
        assert_eq!(reply.rpc_result, BIND_REFUSED_STATUS);
        assert!(reply.body.is_empty());
    }
}

/// ★ **Order of the two checks is observable and pinned:** a bad engine on a bad channel
/// answers the ENGINE status. The GSP-side receiver translates the engine before it
/// touches the channel (`ogkm-580: kernel_fifo_gm107.c:447-488` before `:672-759`'s
/// caller acts), and a port that flipped the order would leak channel-existence on an
/// engine the device does not even have.
#[test]
fn the_engine_is_checked_before_the_channel() {
    let mut p = policy_with_a_channel();
    let reply = p
        .respond(&bind_cmd_on(0xdead_beef, &0x12u32.to_le_bytes()))
        .expect("claimed");
    assert_eq!(
        reply.rpc_result, BIND_UNKNOWN_ENGINE_STATUS,
        "unknown engine + unknown channel = the engine answer"
    );
}

/// `paramsSize` is **4** — a `NvU32`. Anything else is refused, not read past or padded.
#[test]
fn the_params_are_four_bytes_and_other_sizes_are_refused() {
    assert_eq!(BindParams::SIZE, 4);
    let mut p = policy_with_a_channel();
    for params in [&[0x0bu8][..], &[0x0b, 0, 0], &[0x0b, 0, 0, 0, 0]] {
        let reply = p.respond(&bind_cmd(params)).expect("claimed");
        assert_eq!(
            reply.rpc_result,
            BIND_REFUSED_STATUS,
            "{} bytes",
            params.len()
        );
        assert!(reply.body.is_empty());
    }
}

// =================================================================================
// The refusal vocabulary — the statuses, against the driver's own code
// =================================================================================

/// ★★ Both refusal statuses are ones the **code** produces (`gm107.c:736`, `:410`), the
/// engine refusal is NOT in the header's documented list (the header is wrong by
/// omission — `BIND_STATUSES_THE_CODE_PRODUCES`'s whole reason), and neither is `0x56`,
/// the unclaimed-command signature the bench printed for weeks. `0x57` and `0x56` are
/// one apart and mean opposite things; this pins the difference.
#[test]
fn the_refusal_statuses_are_the_codes_own_and_never_the_unclaimed_signature() {
    assert!(BIND_STATUSES_THE_CODE_PRODUCES.contains(&BIND_UNKNOWN_ENGINE_STATUS));
    assert!(BIND_STATUSES_THE_CODE_PRODUCES.contains(&BIND_REFUSED_STATUS));
    assert!(
        !BIND_DOCUMENTED_STATUSES.contains(&BIND_UNKNOWN_ENGINE_STATUS),
        "★ 0x57 is the answer the header FORGOT — a gate built from the header would \
         have refused the true answer"
    );
    assert_ne!(
        BIND_UNKNOWN_ENGINE_STATUS,
        kayfabe_abi::NV_ERR_NOT_SUPPORTED
    );
    assert_ne!(BIND_REFUSED_STATUS, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(
        !BIND_STATUSES_THE_CODE_PRODUCES.contains(&kayfabe_abi::NV_ERR_NOT_SUPPORTED),
        "0x56 means 'nobody claimed this' and the bind path never produces it"
    );
}

/// The decode/encode pair round-trips the measured wire image — the four bytes a real
/// GA106 was asked on 2026-08-01 (`traces/real_ga106/rpc_transcript_real_ga106.txt:63`,
/// `0b 00 00 00`) — the ABI half's own acceptance, re-asserted where the policy
/// consumes it.
#[test]
fn decode_and_encode_round_trip_the_measured_image() {
    let wire = [0x0bu8, 0, 0, 0];
    let got = decode_bind(&wire).expect("four bytes is the whole struct");
    assert_eq!(got.engine_type, MEASURED_GA106_ENGINE);
    assert_eq!(&encode_bind(&got)[..], &wire[..]);
}

// =================================================================================
// The chain — the claim grew by exactly one id, and every claimed id is DECIDED
// =================================================================================

/// ★★★ **The list gates and the match dispatches, and this keeps them in lockstep**
/// (`a_table_does_not_decide_behaviour`): for EVERY id in `OBJECT_CONTROLS`, even a
/// malformed request gets `Some` — a decided refusal — never `None`. An id added to the
/// list without a dispatch arm falls through to the unserviced ledger as `0x56` while
/// the list claims it is decided; this test makes that drift red.
#[test]
fn every_claimed_control_is_decided_even_when_malformed() {
    for &cmd_id in OBJECT_CONTROLS {
        let mut p = policy_with_a_channel();
        // ⊘⊘ **w294 — "one garbage byte" WAS AN UNSTATED ASSUMPTION, AND IT EXPIRED.**
        // This probe sent `&[0xee]` unconditionally, relying on *"1 is the wrong size for
        // every claimed control"* — true until `NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_SET_CONTROL`
        // (`0x00802009`) arrived, whose params really are **one `NvBool`**
        // (`ogkm-580: ctrl0080perf.h:39-43`). One byte is then a *well-formed* request, the
        // arm serves it `NV_OK`, and the gate reads a correct answer as a drift.
        // ⇒ The length is now chosen **against the id's own measured size**, so "malformed"
        // is a property of the request rather than a coincidence about the table. ★ The gate
        // stays quantified over the whole list; it is the probe that was wrong, not the list.
        let want = kayfabe_abi::submit::input_only_control(cmd_id).map(|r| r.params_size);
        let n = if want == Some(1) { 2 } else { 1 };
        let garbage = vec![0xeeu8; n];
        let mut s = w::RpcScript::new();
        s.control(CLIENT, CHANNEL, cmd_id, &garbage);
        let m = s.messages().into_iter().next().expect("one");
        let reply = p.respond(&command(&m));
        let reply = reply.unwrap_or_else(|| {
            panic!("★ {cmd_id:#010x} is in OBJECT_CONTROLS and must be DECIDED, not dropped")
        });
        assert_ne!(
            reply.rpc_result, 0,
            "{cmd_id:#010x}: a malformed request is refused, not served"
        );
        // ★★★ **The "never `0x56`" rule needed a SCOPE, and a boot is what gave it one.**
        //
        // It is right wherever the guest's own error path *reads* the status — the two
        // channel controls — and it is WRONG for `GPU_PROMOTE_CTX`, whose failure
        // propagates into an engine's `StatePostLoad`, where `gpuStatePostLoad` converts
        // **only** `NV_ERR_NOT_SUPPORTED` to `NV_OK` and bails on everything else
        // (`ogkm-580: gpu.c:3437-3439`). `[measured 2026-08-08, boot ship2_7c5d74d]`:
        // answering `0x40` there ended `RmInitAdapter failed! (0x25:0x40:1249)` and cost
        // the milestone; §14.21 reverted the whole claim over it.
        //
        // ⊘ The gate is still quantified over the WHOLE list — the expectation is split
        // per id with its reason, never the list shortened (`gates_quantified_over_a_list`).
        if cmd_id == kayfabe_abi::generated::ctrl::NV2080_CTRL_CMD_GPU_PROMOTE_CTX {
            assert_eq!(
                reply.rpc_result,
                kayfabe_abi::NV_ERR_NOT_SUPPORTED,
                "{cmd_id:#010x}: this control's refusal reaches gpuStatePostLoad, where 0x56 \
                 is the ONLY status that keeps the adapter alive",
            );
        } else if cmd_id == kayfabe_abi::submit::NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE {
            // ★★★★ **§16.59 — the second id in the split, and it is there for a DIFFERENT
            // reason from `GPU_PROMOTE_CTX`'s, which is why the arm is separate.**
            //
            // `PROMOTE_CTX` uses `0x56` because any other status kills the adapter — the
            // status is chosen for its *effect*. Here the status is chosen because it is
            // **true and documented**: `ctrl2080gr.h:791-795` says, of this exact command,
            // *"A value of `NV_ERR_NOT_SUPPORTED` is returned if the target channel does not
            // support preemption context switch mode changes."* This port supports none.
            //
            // ⊘ So the standing "never reuse the unclaimed signature" rule is not being bent
            // here, it is being *satisfied by the header*: the rule forbids borrowing a
            // status whose meaning is "absent" for a decision, and this control's own
            // vocabulary supplies `0x56` for the meaning we intend
            // (`refuse_by_name_means_the_NAME_IS_TRUE`). What the collision does cost —
            // wire-indistinguishability from an unserviced id — is paid in the same coin
            // §14.25 pays it: the difference is legible in this port's own control census,
            // and a *claimed* id leaves the unserviced ledger, which is a one-line diff on
            // any boot log.
            assert_eq!(
                reply.rpc_result,
                kayfabe_abi::submit::CTXSW_PREEMPTION_REFUSED_STATUS,
                "{cmd_id:#010x}: this control's own header documents NV_ERR_NOT_SUPPORTED \
                 for a target that does not support preemption mode changes",
            );
        } else {
            assert_ne!(
                reply.rpc_result,
                kayfabe_abi::NV_ERR_NOT_SUPPORTED,
                "{cmd_id:#010x}: and the refusal is a decision, never the unclaimed signature"
            );
        }
    }
}

/// The triage row survives being served, corrected rather than deleted, still naming
/// what is still true — the same discipline `0xa06f0103`'s row set.
#[test]
fn the_triage_row_survives_and_records_the_correction() {
    let row = kayfabe_device::sweep::triage_for(NVA06F_CTRL_CMD_BIND)
        .expect("★ the row must not be deleted when the control is served");
    assert!(
        row.why.contains("SERVED as of E9"),
        "the row must say it is served"
    );
    assert!(
        row.why.contains("CORRECTED rather than deleted"),
        "and must keep the argument it is correcting"
    );
    assert!(
        row.why.contains("nv2080_to_rm_engine_type"),
        "★ and must name the conversion, because the raw compare is the silent defect"
    );
    assert!(
        row.why.contains("cannot close the execution plane"),
        "★ and must keep naming what is STILL true — a reply defers the host act, it \
         does not perform it"
    );
}
