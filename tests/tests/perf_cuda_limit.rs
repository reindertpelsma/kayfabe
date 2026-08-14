//! ★★★★★ **w294 — THE CUDA PERF LIMIT PAIR, AND THE ID THE IOCTL RECORDER SHOWS IS THE
//! WRONG ONE.**
//!
//! # The measurement that started it
//!
//! `[measured 2026-08-14, traces/nvdiff_w292/serve_r1.jsonl.zst record 412]` the guest issues
//! `NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL` (`0x00801909`, `paramsSize=1`, the byte
//! `0x01`) and gets `NV_ERR_NOT_SUPPORTED` back. A **real GA106** answers it `NV_OK`, twice
//! — `0x01` at context create and `0x00` at destroy — with `ppre == ppost` on every call
//! (`../nvidia-gpu-passthrough/traces/host_reference_ga106/ctx_r1.jsonl.zst` i=431, i=460).
//! It is one of only two divergences of ours left in the whole 572-record stream that is
//! neither retried into success nor in agreement with hardware.
//!
//! # ⊘⊘⊘ AND THE OBVIOUS FIX IS A NO-OP THAT LOOKS LIKE A FIX
//!
//! *"Serve `0x00801909`"* would compile, would pass every gate in
//! `admitted_is_served.rs`, and **would change nothing**, because that id never reaches us:
//!
//! | id | `flags` | `ROUTE_TO_PHYSICAL` | who answers it |
//! |---|---|---|---|
//! | `0x00801909` | `0x118` (`ogkm-580: g_device_nvoc.c:920`) | ⊘ **no** | the guest's **own kernel** |
//! | `0x00802009` | `0x1d8` (`:1025`) | ★ **yes** | **us** |
//! | `0x00802004` | `0x0c0` (`:1010`) | ★ **yes** | **us** |
//!
//! `deviceCtrlCmdKPerfCudaLimitSetControl_IMPL` (`ogkm-580:
//! src/nvidia/src/kernel/gpu/perf/kern_cuda_limit.c:94-137`) increments a per-`Device`
//! refcount in guest RAM and, **only on the 0↔1 edge**, issues the *internal* `0x00802009`
//! to physical RM, returning that call's status verbatim. ⇒ **The `0x56` the guest reports
//! on `0x00801909` is our `0x56` on `0x00802009`.**
//!
//! # ★★★ THE SEAM: two boundaries, two ids, and no instrument sees both
//!
//! The `LD_PRELOAD` nvdiff recorder sits at the **ioctl** boundary and can only ever record
//! `0x00801909`. Our `UnservicedLedger` sits at the **GSP RPC** boundary and can only ever
//! record `0x00802009`/`0x00802004`. `[measured]` each id appears in exactly one of the two,
//! and in **zero** of the other — `0x00801909` in no boot log of ours, `0x00802009` in no
//! ioctl capture anywhere, and it *can* appear in none, being `RMCTRL_FLAGS_INTERNAL`.
//! ⇒ A reader holding one instrument reaches the wrong id **with a correct citation**.
//!
//! # What serving them asserts, said out loud
//!
//! - `0x00802009` declares *"clocks will be limited based on Cuda"* (`ogkm-580:
//!   ctrl0080perf.h:49-50`). This device models **no clock domain at all**, so there is no
//!   observable the declaration can make false — the `kayfabe_device::inert` eligibility
//!   rule (*the observable consequence of our doing nothing is a TRUE statement about this
//!   device*), applied to one control.
//! - `0x00802004` is the teardown half, and ★ **refusing it is the actively unsafe side.**
//!   `deviceKPerfCudaLimitCliDisable` (`kern_cuda_limit.c:62-75`) checks our status **before**
//!   `pDevice->nCudaLimitRefCnt = 0`, so a refusal leaves the guest's own refcount
//!   permanently non-zero at device teardown. ⚠ This is the concrete instance of the trap
//!   *"do not assume refusing is the safe side"*.
//!
//! ⊘ **What this file does not claim.** It is not a boot. Whether serving the pair moves
//! `CUP2_RC` off `1` is a question only a boot answers.

use kayfabe_abi::submit::{
    INPUT_ONLY_REFUSED_STATUS, PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES, input_only_control,
};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{HClient, HObject};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{CommandPolicy, RpcCommand};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{OBJECT_CONTROLS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire as w;

const CLIENT: HClient = HClient(0xAA);
/// `[measured, traces/nvdiff_w292/serve_r1 i=412]` the control is asked on the **device**
/// (`hObject=0x5c000002`), not the subdevice — `RES_GET_HANDLE(pDevice)` in
/// `kern_cuda_limit.c:131`.
const DEVICE: HObject = HObject(0xcafe_0002);

fn abi() -> DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver is supported")
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

fn policy() -> ObjectPolicy {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(factory),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");
    ObjectPolicy::new(
        &abi(),
        kayfabe_abi::GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    )
}

/// Post one control through the whole policy; `None` means nothing claimed the id.
fn answer(cmd: u32, params: &[u8]) -> Option<kayfabe_gsp::Reply> {
    let mut s = w::RpcScript::new();
    s.control(CLIENT.0, DEVICE.0, cmd, params);
    let m = s.messages().into_iter().next().expect("one message");
    policy().respond(&command(&m))
}

/// ★★★★★ **The pair is claimed and answered `NV_OK`, at the sizes the ABI measured.**
///
/// `0x00802009` carries one `NvBool` (`ogkm-580: ctrl0080perf.h:39-43`; `NvBool` is `NvU8`,
/// `nvtypes.h:272`) and `0x00802004` carries **none** — `/*paramSize=*/ 0 /* Singleton
/// parameter list */` (`g_device_nvoc.c:1011`), its only caller passing `NULL, 0`.
#[test]
fn the_pair_that_actually_arrives_is_served() {
    let (_ioctl, set_control, disable) = PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;

    // Both values the guest ever sends: 0x01 at create, 0x00 at destroy.
    for byte in [0x01u8, 0x00u8] {
        let r = answer(set_control, &[byte])
            .unwrap_or_else(|| panic!("0x{set_control:08x} is claimed by nobody"));
        assert_eq!(
            r.rpc_result, 0,
            "0x{set_control:08x} with bCudaLimit={byte} was not served NV_OK",
        );
    }

    let r = answer(disable, &[]).unwrap_or_else(|| panic!("0x{disable:08x} is claimed by nobody"));
    assert_eq!(r.rpc_result, 0, "0x{disable:08x} was not served NV_OK");

    for cmd in [set_control, disable] {
        assert!(
            OBJECT_CONTROLS.contains(&cmd),
            "0x{cmd:08x} is not claimed by OBJECT_CONTROLS — it would fall to the \
             unserviced ledger as 0x56, which is the defect being fixed",
        );
        let row = input_only_control(cmd).expect("a row with its authority");
        assert!(
            !row.authority.trim().is_empty(),
            "0x{cmd:08x} is served with no stated authority",
        );
    }
}

/// ★★★ **The reply is the guest's own bytes, and for the zero-params half it is empty.**
///
/// ⊘ Asserted only because a *wrong-length* body would rewrite the caller's struct
/// (`ogkm-580: rpc.c:11085-11090` copies the reply's params over the caller's whenever
/// `paramsSize != 0`), never as evidence that anything was performed. `[measured]` a native
/// GA106 leaves the single byte untouched across the call on all 15 of its calls, so there
/// is no `[OUT]` field an echo could get wrong.
#[test]
fn the_served_reply_carries_the_requests_own_byte_back() {
    let (_ioctl, set_control, _disable) = PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;
    let mut s = w::RpcScript::new();
    s.control(CLIENT.0, DEVICE.0, set_control, &[0x01]);
    let m = s.messages().into_iter().next().expect("one message");
    let cmd = command(&m);
    let r = policy().respond(&cmd).expect("served");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(
        r.body, cmd.payload,
        "the reply is not the request's own payload — a short or rewritten body here \
         rewrites the caller's struct behind its back",
    );
}

/// ⊘ **A wrong `paramsSize` is refused BY NAME, not accommodated and not `0x56`.**
///
/// `INPUT_ONLY_REFUSED_STATUS` is `NV_ERR_INVALID_PARAM_STRUCT`, deliberately distinct from
/// `NV_ERR_NOT_SUPPORTED`: `0x56` is what this port emitted when it did not serve the id at
/// all, and reusing it would make *"we refused the shape"* and *"we never heard of it"* the
/// same observation on the wire.
#[test]
fn a_wrong_params_size_is_refused_by_name() {
    let (_ioctl, set_control, disable) = PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;
    for (cmd, bad) in [
        (set_control, vec![0u8; 4]),  // the NvBool is ONE byte, not a word
        (set_control, Vec::new()),    // and not zero
        (disable, vec![0u8; 1]),      // the singleton list takes NOTHING
    ] {
        let n = bad.len();
        let r = answer(cmd, &bad)
            .unwrap_or_else(|| panic!("0x{cmd:08x} declined to claim a wrong-sized request"));
        assert_eq!(
            r.rpc_result, INPUT_ONLY_REFUSED_STATUS,
            "0x{cmd:08x} with {n} params bytes was not refused by name",
        );
    }
}

/// ★★★★★ **`0x00801909` MUST NOT BE SERVED — it cannot arrive.**
///
/// The whole point of the rung: a row for it would be an answer to traffic that does not
/// exist, and from outside it is indistinguishable from a fix. See this file's module doc.
#[test]
fn the_ioctl_id_is_not_served_because_it_cannot_reach_us() {
    let (ioctl_id, _set_control, _disable) = PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;
    assert!(
        input_only_control(ioctl_id).is_none(),
        "0x{ioctl_id:08x} is in INPUT_ONLY_CONTROLS — it is flags=0x118, NOT \
         ROUTE_TO_PHYSICAL, so the guest's own kernel answers it and we never see it",
    );
    assert!(
        !OBJECT_CONTROLS.contains(&ioctl_id),
        "0x{ioctl_id:08x} is claimed by OBJECT_CONTROLS — see above",
    );
    assert!(
        answer(ioctl_id, &[0x01]).is_none(),
        "0x{ioctl_id:08x} is answered by the chain",
    );
}

/// ⊘ **Non-vacuity of the probe.** If `answer` could only ever produce one outcome, every
/// assertion above passes while checking nothing.
#[test]
fn the_probe_can_both_serve_and_decline() {
    let (_ioctl, set_control, _disable) = PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;
    assert!(
        answer(set_control, &[0x01]).is_some(),
        "the probe cannot detect a control this port certainly claims",
    );
    assert!(
        answer(0xdead_0000, &[0x01]).is_none(),
        "the probe answers a command nobody claims — it cannot detect a gap",
    );
}
