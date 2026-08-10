//! ★★★★★ **`NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (`0x20801702`) — §16.75.**
//!
//! The 1 Hz train, `[measured 2026-08-10, boot `w209_ffc80f8_ctl`, rev `ffc80f8`]`. Thirteen
//! arrivals in `traces/guest_boots/run_w209_ffc80f8_ctl_probe.log`, every one refused, each
//! producing one line in the same file's `dmesg`:
//!
//! ```text
//! CTRL cmd=0x20801702 hClient=0xc1d0000c hObject=0x5c000003 size=4 status=0x00000056
//!      in=ffffffff out=ffffffff
//! [  56.134495] NVRM: subdeviceCtrlCmdMcServiceInterrupts_IMPL: NVRM_RPC:
//!               NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS failed with error 0x56
//! …  57.173620  58.200313  59.235998  60.248689  61.256314  62.262604  63.264724
//!    64.310367  65.313316  66.319974  67.321869  68.377535   (intervals 1.002-1.056 s)
//! ```
//!
//! and the same boot's QEMU log carries `nvkvm: unserviced fn 76 cmd 0x20801702` — so it
//! arrives as a **generic `GSP_RM_CONTROL`**, not as the specialised
//! `NV_VGPU_MSG_FUNCTION_CTRL_MC_SERVICE_INTERRUPTS` that `rpcCtrlMcServiceInterrupts_v1A_0E`
//! (`ogkm-580: rpc.c:6270-6296`) builds for vGPU. ⚠ That second path exists in the same tree
//! and reading it first would have aimed this arm at a function number the guest never uses.
//!
//! # ★★★★★ Why this id is different from every other one in the unserviced census
//!
//! Because the `0x56` **cancelled work inside the guest**, which is a fact in `ogkm` rather
//! than a reading of the stream. `subdeviceCtrlCmdMcServiceInterrupts_IMPL`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/intr/intr.c:186-280`) is two halves in order:
//!
//! 1. `:216` `NV_RM_RPC_CONTROL(...)` to us, and `:219-225` — **`return status;` on any
//!    non-`NV_OK`**, after printing the line above;
//! 2. `:262-278` — convert `pServiceInterruptParams->engines` into an `MC_ENGINE_BITVECTOR`
//!    and `intrServiceStallList_HAL(pGpu, pIntr, &engines, NV_TRUE)`.
//!
//! ⇒ half 2 never ran, thirteen times. ⊘ The `LEDGER` row in `admitted_is_served.rs` called
//! this id *"forgiven every time"* on the evidence that the guest kept going for another 50
//! records. **"The process continued" is not "the request was forgiven"**: every other id on
//! that list is forgiven by a caller that maps `0x56` to `NV_OK`, and this one was *obeyed*.
//!
//! # ⊘⊘⊘ WHAT THIS FILE CANNOT TEST, AND THE FALSIFIER IT WOULD OTHERWISE CORRUPT
//!
//! The dmesg line above exists **only on the failure branch**. Answering `NV_OK` deletes it
//! *by construction*, whether or not anything was delivered and whether or not the guest got
//! any further. ⊘ So *"the 12-line 1 Hz train collapses"* is **not a falsifier for this
//! rung** — it is a restatement of the change. What discriminates at boot level is the
//! **request** cadence (our own `CTRL cmd=0x20801702` records, which we log regardless of the
//! status we return) and what `cuCtxCreate` does next.
//!
//! ⊘ And this file is not a boot (`only_live_boots_are_proof`).

use kayfabe_abi::GuestOs;
use kayfabe_abi::submit::{
    MC_ENGINE_ID_ALL, MC_ENGINE_ID_GRAPHICS, MC_SERVICE_INTERRUPTS_REFUSED_STATUS,
    McServiceInterruptsError, McServiceInterruptsRequest, NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS,
    decode_mc_service_interrupts, encode_mc_service_interrupts,
};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{CommandPolicy, RpcCommand};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{OBJECT_CONTROLS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w};
use kayfabe_tests::{Scenario, identical_handles};

/// `NV_ERR_NOT_SUPPORTED` — the FSM's *"nobody claimed this"* signature, and the value this
/// rung exists to remove from this id. Named here so the negative assertions read as the
/// claim they are rather than as a magic number.
const NV_ERR_NOT_SUPPORTED: u32 = 0x0000_0056;

/// ★ The guest's own params word, `[measured 2026-08-10, boot w209_ffc80f8_ctl]`:
/// `size=4 in=ffffffff` — `NV2080_CTRL_MC_ENGINE_ID_ALL`. `nvGpuOpsServiceDeviceInterruptsRM`
/// sends exactly this (`ogkm-580: rmapi/nv_gpu_ops.c:7856`), and so does the userspace caller
/// this boot recorded under `hClient=0xc1d0000c`.
const GUEST_W209_PARAMS_BYTES: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

// =====================================================================================
// Harness — the same shape as `ctxsw_preemption_mode.rs`
// =====================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
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

const CLIENT: HClient = HClient(0xAA);
/// The object the control is asked **on** — `[measured 2026-08-10, boot w209_ffc80f8_ctl]`
/// the subdevice, `hObject=0x5c000003`.
const SUBDEVICE: HObject = HObject(0xcafe_0003);

fn policy() -> ObjectPolicy {
    let (factory, _rec) = MockIsolateFactory::new();
    let mut gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(factory),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(CLIENT, Pdb(0x11_0000), h);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    )
}

/// Post one control through the whole policy. `cmd` and the params image are both callers'
/// choices so the negative cases can be built from the same helper.
fn answer(p: &mut ObjectPolicy, cmd: u32, params: &[u8]) -> Option<kayfabe_gsp::Reply> {
    let mut s = w::RpcScript::new();
    s.control(CLIENT.0, SUBDEVICE.0, cmd, params);
    let m = s.messages().into_iter().next().expect("one message");
    p.respond(&command(&m))
}

/// The `params_at` offset for a one-element control message — derived from the reply the
/// policy built rather than restated, so a wire-layout change cannot make this file assert
/// against a stale constant.
fn params_of(reply_body: &[u8], want: usize) -> Vec<u8> {
    let at = reply_body.len() - want;
    reply_body[at..].to_vec()
}

// =====================================================================================
// ★★★★★ THE RUNG
// =====================================================================================

/// ★★★★★ **THE WALL, as a claim.** A `None` here is the `w209` behaviour exactly: the chain
/// falls through to `UnservicedLedger`, the FSM answers `0x56`, and the guest's
/// `intrServiceStallList_HAL` never runs.
#[test]
fn the_policy_claims_0x20801702_and_answers_nv_ok() {
    let mut p = policy();
    let reply = answer(
        &mut p,
        NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS,
        &GUEST_W209_PARAMS_BYTES,
    )
    .expect(
        "★★★ the policy must CLAIM 0x20801702 — a None here is the w209 wall, where the \
         chain falls through to the unserviced ledger",
    );
    assert_eq!(
        reply.rpc_result, 0,
        "★ this port's GSP runs no firmware, raises no engine interrupt and queues no \
         deferred GSP-side interrupt work, so the set the guest asked us to service is \
         empty on every call and NV_OK reports exactly that"
    );
    assert_ne!(
        reply.rpc_result, NV_ERR_NOT_SUPPORTED,
        "⊘ 0x56 is the signature this rung exists to remove from this id"
    );
}

/// ★★★★★ **THE ECHO IS LOAD-BEARING HERE, unlike on `0x20801210`.**
///
/// `paramsSize` is 4, non-zero, so `rpcRmApiControl_GSP` copies the reply's params over the
/// caller's struct (`ogkm-580: rpc.c:11085-11090`) — and the guest then reads its **own**
/// struct back at `intr.c:262-278` to build the engine bitvector it services. A zero body
/// would hand it `engines = 0`, `bitVectorClrAll` would stand, and
/// `intrServiceStallList_HAL` would service the **empty set**: an `NV_OK` that silently
/// un-does the very thing it enabled, green on both sides.
///
/// ⊘ That the FINN header marks no field `[OUT]` is not a transport fact
/// (`mem: an_in_annotation_is_not_a_transport_fact`).
#[test]
fn the_reply_carries_the_guests_own_engines_word_back() {
    let want = McServiceInterruptsRequest::SIZE;
    for engines in [MC_ENGINE_ID_ALL, MC_ENGINE_ID_GRAPHICS, 0x0000_0007] {
        let params = encode_mc_service_interrupts(&McServiceInterruptsRequest { engines });
        let mut p = policy();
        let reply = answer(&mut p, NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS, &params)
            .expect("claimed for every engine mask");
        assert_eq!(reply.rpc_result, 0);
        assert_eq!(
            params_of(&reply.body, want),
            params,
            "★★★ engines={engines:#010x} must come back unchanged; a zero-filled body makes \
             the guest service the EMPTY set at intr.c:278"
        );
        assert_ne!(
            params_of(&reply.body, want),
            vec![0u8; want],
            "⊘ the failure this assertion exists for is specifically a ZERO body"
        );
    }
}

/// ★★ The measured bytes, pinned. `[measured 2026-08-10, boot w209_ffc80f8_ctl]` the guest
/// sends `MC_ENGINE_ID_ALL`, so the mask that must survive the round trip is `0xffffffff` —
/// the one whose loss (`0 == NV2080_CTRL_MC_ENGINE_ID_ALL & 0`) is silent.
#[test]
fn the_w209_payload_decodes_to_engine_id_all() {
    let req = decode_mc_service_interrupts(&GUEST_W209_PARAMS_BYTES).expect("4 bytes decode");
    assert_eq!(req.engines, MC_ENGINE_ID_ALL);
    assert_eq!(encode_mc_service_interrupts(&req), GUEST_W209_PARAMS_BYTES);
}

/// ⊘ A malformed params image is **refused by decision**, and the status is this control's
/// own (`NV_ERR_INVALID_PARAM_STRUCT`, `ogkm-580: ctrl2080mc.h:171-174`) — never `0x56`,
/// which would make a decided refusal indistinguishable from the wall this rung removes.
#[test]
fn malformed_params_are_refused_by_decision_never_with_0x56() {
    for params in [
        vec![],
        vec![0u8; 1],
        vec![0u8; 3],
        vec![0u8; 8],
        vec![0u8; 32],
    ] {
        let mut p = policy();
        let reply = answer(&mut p, NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS, &params).expect(
            "★ a claimed id must be DECIDED even when malformed — a None here would put the \
             id back in the unserviced ledger while OBJECT_CONTROLS says it is decided",
        );
        assert_eq!(
            reply.rpc_result,
            MC_SERVICE_INTERRUPTS_REFUSED_STATUS,
            "paramsSize={} is not sizeof(NV2080_CTRL_MC_SERVICE_INTERRUPTS_PARAMS)",
            params.len()
        );
        assert_ne!(reply.rpc_result, NV_ERR_NOT_SUPPORTED);
        assert!(
            reply.body.is_empty(),
            "a refusal carries no body — the caller's struct is left alone"
        );
    }
}

/// ⊘ **The claim is by id, and the neighbours must stay unclaimed.** `0x20801701`
/// (`MC_GET_ARCH_INFO`) and `0x20801703` (`MC_GET_MANUFACTURER`) are one and two away in the
/// same interface; a widened check — `cmd >> 8 == 0x208017`, say — would swallow both and
/// silence the ledger for them. This is the `gates_quantified_over_a_list` discipline as an
/// executing assertion rather than a comment.
#[test]
fn the_neighbouring_mc_controls_are_not_claimed() {
    for cmd in [0x2080_1701_u32, 0x2080_1703] {
        let mut p = policy();
        assert!(
            answer(&mut p, cmd, &GUEST_W209_PARAMS_BYTES).is_none(),
            "{cmd:#010x} is NOT on OBJECT_CONTROLS and must fall through to the ledger"
        );
        assert!(!OBJECT_CONTROLS.contains(&cmd));
    }
}

/// The list and the id agree. ⊘ Asked of the public constant rather than restated, so a
/// future edit that drops the row from `OBJECT_CONTROLS` cannot leave this file green.
#[test]
fn the_id_is_on_the_claimed_list() {
    assert!(OBJECT_CONTROLS.contains(&NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS));
    assert_eq!(NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS, 0x2080_1702);
    assert_eq!(McServiceInterruptsRequest::SIZE, 4);
    assert_eq!(MC_SERVICE_INTERRUPTS_REFUSED_STATUS, 0x3A);
}

/// The decoder refuses a short image by name rather than reading past it.
#[test]
fn a_short_params_image_is_named_not_read() {
    for n in 0..McServiceInterruptsRequest::SIZE {
        assert_eq!(
            decode_mc_service_interrupts(&vec![0xffu8; n]),
            Err(McServiceInterruptsError::ShortParams { got: n })
        );
    }
}
