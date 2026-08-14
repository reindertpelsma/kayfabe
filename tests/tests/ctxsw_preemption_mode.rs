//! ★★★★★ **CORRECTED 2026-08-14 (w294) — READ THIS FIRST. "THE CONTROL `cuCtxCreate` STOPS
//! AT" IS FALSE, IN BOTH POLARITIES, AND TWO COMMITTED MEASUREMENTS SAY SO.**
//!
//! Everything below opens by calling `0x20801210` *the control `cuCtxCreate` stops at*, and
//! §16.59/§16.60 built a rung on it. It never stopped anything:
//!
//! **(1) Serving it changes NOTHING the guest does next.** `[measured, traces/guest_boots/
//! run_s45_748a207_tsgsched_probe.log:449 vs run_s47_81582e3_ctxsw_probe.log:449]` two boots
//! differing only in this record's status — `0x56` vs `NV_OK`, identical request bytes.
//! **456 records each; exactly one record differs, and it is that status field.** Records
//! 332…456 are byte-identical after pointer canonicalisation, and **`CUP2_RC=1` in both**.
//!
//! **(2) "Record 332 begins the `FREE` burst" is not a failure signature — a SUCCESSFUL run
//! has it too.** `[measured, ../nvidia-gpu-passthrough/traces/host_reference_ga106/
//! ctx_r1.jsonl.zst]` a native GA106 whose own stdout prints `CTX OK` also begins a large
//! `FREE` burst two records after this control (i=433, after `0x00801909` at i=431). The
//! burst is `cuCtxDestroy`/exit unwind, and **both outcomes produce one**. ⇒ A teardown
//! cannot discriminate success from failure, because both programs exit.
//!
//! **(3) When the guest asks for CILP and we refuse, IT DOWNGRADES AND RETRIES.**
//! `[measured, traces/nvdiff_w292/serve_r1.jsonl.zst]` i=391 `cilpPreemptMode=2` → our
//! `0x56`; **i=392 `cilpPreemptMode=0` → our `NV_OK`**, `psize=pgot=32` on both. ⇒ The
//! classifier's refusal lands the guest on the mode this port is actually in, one record
//! later, by libcuda's own fallback. **Serving CILP would delete record 392.**
//!
//! ⇒ ★★★ **The ruling (w294): the refusal STAYS, and it is a measurement rather than an
//! owner question.** The table below — *"C asks `2`, ours asks `0`"* — is also superseded:
//! `[measured, w292]` ours asks `2` as well now, because the `0` was itself a downstream
//! effect of our refusing `NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK` one control upstream.
//! The **lesson** below survives (a green C oracle is evidence about the C's payload); its
//! **example** does not.
//!
//! ---
//!
//! ★★★★ **`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`0x20801210`) — §16.59.**
//!
//! The control `cuCtxCreate` stops at, `[measured 2026-08-10, boots s45_748a207_tsgsched and
//! s46_1a9e93c_abi35]` — record **331** of 456, `status=0x56`, and record **332 begins the
//! `FREE` burst**:
//!
//! ```text
//! 331 CTRL cmd=0x20801210 hClient=0xc1d0000c hObject=0x5c000003 size=32 status=0x00000056
//!       in=01000000 1200005c 00000000 00000000 …
//! 332 FREE …
//! ```
//!
//! # ⊘⊘⊘ THE REPLY CANNOT DISCRIMINATE ANYTHING, so nothing here is judged by it
//!
//! Every field of `NV2080_CTRL_GR_SET_CTXSW_PREEMPTION_MODE_PARAMS` is `[IN]`
//! (`ogkm-580: ctrl2080gr.h:836-842`). The reply is the request's own bytes, so a test that
//! asserted *"the reply echoes the request"* would be asserting that this port can call
//! `copy_from_slice`, and it would pass against a policy that answered `NV_OK` to
//! **anything**.
//!
//! ⇒ What discriminates, and what this file is built on, is
//! [`the_mode_word_is_what_decides_and_the_reply_never_could`]: hold every other byte fixed,
//! move `cilpPreemptMode` from `0` to `2`, and demand the **answer change**. A port that
//! echoes unconditionally — which is what the C artifact does, and what this rung was
//! briefed to port — fails it.
//!
//! # ★★★★ AND THE C IS NOT THE ORACLE HERE: it answered a DIFFERENT REQUEST
//!
//! `[measured 2026-08-10, cap3_matmul_forwarding #453716/#453717 vs boot s46 record 331]`:
//!
//! | | `flags` | `hChannel` | `gfxpPreemptMode` | `cilpPreemptMode` |
//! |---|---|---|---|---|
//! | C, `cap3` #453716 | `1` | `0x5c000012` | `0` | ★ **`2`** = `COMPUTE_CILP` |
//! | ours, `s46` record 331 | `1` | `0x5c000012` | `0` | ★ **`0`** = `COMPUTE_WFI` |
//!
//! Three of the four words match and the fourth is the only one that decides whether an
//! `NV_OK` is a true sentence. Both payloads are pinned here as fixtures
//! ([`C_CAP3_REQUEST_BYTES`], [`GUEST_S46_REQUEST_BYTES`]) so the divergence cannot be
//! quietly re-merged by a later reader who remembers the brief rather than the measurement.
//!
//! ⊘ **What this file does not claim.** It is not a boot. Whether serving this control moves
//! the guest past record 332 is a question only a boot answers (`only_live_boots_are_proof`),
//! and §16.59's falsifier is written in terms of *what the guest does next*, never in terms
//! of the status we returned.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_abi::GuestOs;
use kayfabe_abi::submit::{
    CTXSW_PREEMPTION_COMPUTE_CILP, CTXSW_PREEMPTION_COMPUTE_WFI, CTXSW_PREEMPTION_FLAGS_CILP_SET,
    CTXSW_PREEMPTION_FLAGS_GFXP_SET, CTXSW_PREEMPTION_GFX_WFI, CTXSW_PREEMPTION_REFUSED_STATUS,
    CtxswPreemptionAsk, CtxswPreemptionError, CtxswPreemptionRequest,
    NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE, decode_ctxsw_preemption_mode,
    encode_ctxsw_preemption_mode,
};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{CtxswPreemptionFault, Gpu};
use kayfabe_gsp::{CommandPolicy, RpcCommand};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{OBJECT_CONTROLS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w};
use kayfabe_tests::{Scenario, identical_handles};

// =====================================================================================
// The two payloads, byte for byte
// `[measured 2026-08-10: cap3_matmul_forwarding #453716; boot s46_1a9e93c_abi35 record 331]`
// =====================================================================================

/// ★★★★ **The C artifact's request**, `[measured 2026-08-10, cap3_matmul_forwarding
/// #453716]`, `paramsSize=32`, answered `rpc_result 0xffffffff → 0x00000000` with the body
/// echoed verbatim (#453717).
///
/// `cilpPreemptMode = 2` = `NV2080_CTRL_SET_CTXSW_PREEMPTION_MODE_COMPUTE_CILP` — *"preempt
/// the channel at the instruction level"*. The C had no such machinery and said `NV_OK`
/// anyway; it reached `bad=0 maxerr=0` because a short matmul never preempts, so nothing
/// ever read the promise.
const C_CAP3_REQUEST_BYTES: [u8; 32] = [
    0x01, 0x00, 0x00, 0x00, // flags = FLAGS_CILP_SET
    0x12, 0x00, 0x00, 0x5c, // hChannel = 0x5c000012 (a TSG)
    0x00, 0x00, 0x00, 0x00, // gfxpPreemptMode = GFX_WFI
    0x02, 0x00, 0x00, 0x00, // ★ cilpPreemptMode = COMPUTE_CILP
    0x00, 0x00, 0x00, 0x00, // grRouteInfo.flags
    0x00, 0x00, 0x00, 0x00, // (padding to the u64's alignment)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // grRouteInfo.route
];

/// ★★★★ **Our guest's request**, `[measured 2026-08-10, boot s46_1a9e93c_abi35 record 331]`
/// — identical to [`C_CAP3_REQUEST_BYTES`] except for the one word that matters.
///
/// `cilpPreemptMode = 0` = `COMPUTE_WFI`, *"the normal wait-for-idle context switch mode"*.
/// That is the mode this port's execution plane is unconditionally in, so an `NV_OK` here is
/// a true sentence rather than a survivable one.
const GUEST_S46_REQUEST_BYTES: [u8; 32] = [
    0x01, 0x00, 0x00, 0x00, // flags = FLAGS_CILP_SET
    0x12, 0x00, 0x00, 0x5c, // hChannel = 0x5c000012 (a TSG)
    0x00, 0x00, 0x00, 0x00, // gfxpPreemptMode = GFX_WFI
    0x00, 0x00, 0x00, 0x00, // ★ cilpPreemptMode = COMPUTE_WFI
    0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
];

// =====================================================================================
// Harness
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
/// The object the control is asked **on** — `[measured 2026-08-10, boot s46_1a9e93c_abi35
/// record 331]` the subdevice (`hObject=0x5c000003`), which is deliberately NOT the object
/// the answer is about.
const SUBDEVICE: HObject = HObject(0xcafe_0003);

/// A `Gpu` carrying one compute process: a TSG with two member channels, both placed.
fn gpu_with_a_group() -> (Gpu, kayfabe_tests::ProcessHandles) {
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
    (gpu, h)
}

/// The request the guest sends, with `hChannel` re-pointed at this fixture's TSG.
fn request(h_channel: HObject, cilp: u32, gfxp: u32, flags: u32) -> CtxswPreemptionRequest {
    CtxswPreemptionRequest {
        flags,
        h_channel: h_channel.0,
        gfxp_preempt_mode: gfxp,
        cilp_preempt_mode: cilp,
        route_flags: 0,
        route: 0,
    }
}

/// Post one `0x20801210` through the whole policy and return the reply, or `None` if the
/// policy declined to claim it at all.
fn answer(p: &mut ObjectPolicy, req: &CtxswPreemptionRequest) -> Option<kayfabe_gsp::Reply> {
    let mut s = w::RpcScript::new();
    s.control(
        CLIENT.0,
        SUBDEVICE.0,
        NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE,
        &encode_ctxsw_preemption_mode(req),
    );
    let m = s.messages().into_iter().next().expect("one message");
    p.respond(&command(&m))
}

// =====================================================================================
// ★★★★★ THE RUNG — the mode word is the discriminator, and the reply never could be
// =====================================================================================

/// ★★★★★ **THE LOAD-BEARING TEST.** Hold all 32 bytes fixed except `cilpPreemptMode`, and
/// the answer must **change**.
///
/// ⊘ This is the only shape that can distinguish this port from an unconditional echo. The
/// reply body is the request either way; the *status* is not. A port that served the C's
/// behaviour — echo everything, `NV_OK` always — passes every other test in this file and
/// fails this one, which is the whole point of writing it first.
#[test]
fn the_mode_word_is_what_decides_and_the_reply_never_could() {
    let (gpu, h) = gpu_with_a_group();
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );

    // ---- WFI: the mode this port is unconditionally in -------------------------
    let wfi = answer(
        &mut p,
        &request(
            h.tsg,
            CTXSW_PREEMPTION_COMPUTE_WFI,
            CTXSW_PREEMPTION_GFX_WFI,
            CTXSW_PREEMPTION_FLAGS_CILP_SET,
        ),
    )
    .expect("★★★ the policy must CLAIM 0x20801210 — a None here is the s45/s46 wall");
    assert_eq!(
        wfi.rpc_result, 0,
        "★ the guest's own measured request asks for wait-for-idle, which is the state this \
         port's execution plane is in with no preemption machinery at all. Refusing it would \
         be refusing a postcondition that already holds"
    );

    // ---- CILP: the mode the C artifact asked for, and got NV_OK for ------------
    for cilp in [1, CTXSW_PREEMPTION_COMPUTE_CILP] {
        let asked = answer(
            &mut p,
            &request(
                h.tsg,
                cilp,
                CTXSW_PREEMPTION_GFX_WFI,
                CTXSW_PREEMPTION_FLAGS_CILP_SET,
            ),
        )
        .expect("still claimed — a refusal is a decision, not a decline");
        assert_eq!(
            asked.rpc_result, CTXSW_PREEMPTION_REFUSED_STATUS,
            "★★★★★ cilpPreemptMode={cilp} asks this port to preempt a compute context at \
             CTA/instruction level. It has no such machinery, and whether GA10x consumer \
             silicon has it at all is [unknown] from the open tree \
             (compute_limiting_and_priority.md 3.3). An NV_OK here is the C's behaviour and \
             it is a promise, not an answer"
        );
        assert!(
            asked.body.is_empty(),
            "a refusal carries no body — the caller's struct is left alone"
        );
    }

    // ---- and GFXP is checked independently of CILP -----------------------------
    let gfxp = answer(
        &mut p,
        &request(
            h.tsg,
            CTXSW_PREEMPTION_COMPUTE_WFI,
            1, // GFX_GFXP — preempt mid-triangle
            CTXSW_PREEMPTION_FLAGS_CILP_SET | CTXSW_PREEMPTION_FLAGS_GFXP_SET,
        ),
    )
    .expect("claimed");
    assert_eq!(
        gfxp.rpc_result, CTXSW_PREEMPTION_REFUSED_STATUS,
        "★ a request whose COMPUTE half is honest and whose GRAPHICS half is not is still a \
         request this port cannot satisfy. Reading only the field the measured payload \
         happens to exercise is how a gate ends up quantifying over one boot"
    );
}

/// ★★★★ **The C artifact's own request, verbatim, is REFUSED by this port — and our guest's,
/// verbatim, is SERVED.** The divergence, pinned as bytes.
///
/// ⊘ These two arrays differ in **one byte** (offset 12). If a later reader "restores parity
/// with the C" by dropping the classifier, this test names exactly what was given up.
#[test]
fn the_c_and_the_guest_ask_for_different_modes_and_get_different_answers() {
    assert_eq!(
        C_CAP3_REQUEST_BYTES.len(),
        CtxswPreemptionRequest::SIZE,
        "the measured paramsSize is 32 and the transcription must match it"
    );
    let differing: Vec<usize> = (0..CtxswPreemptionRequest::SIZE)
        .filter(|&i| C_CAP3_REQUEST_BYTES[i] != GUEST_S46_REQUEST_BYTES[i])
        .collect();
    assert_eq!(
        differing,
        vec![12],
        "★★★★ the brief for this rung said our request bytes match the C's byte-for-byte. \
         [measured] they differ in exactly one byte, and it is the low byte of \
         cilpPreemptMode — the only word that decides whether an NV_OK is true"
    );

    let c = decode_ctxsw_preemption_mode(&C_CAP3_REQUEST_BYTES).expect("well-formed");
    let ours = decode_ctxsw_preemption_mode(&GUEST_S46_REQUEST_BYTES).expect("well-formed");
    assert_eq!(c.flags, ours.flags);
    assert_eq!(c.h_channel, ours.h_channel, "both name TSG 0x5c000012");
    assert_eq!(c.gfxp_preempt_mode, ours.gfxp_preempt_mode);
    assert_eq!(
        c.asks_for(),
        CtxswPreemptionAsk::ComputePreemption {
            mode: CTXSW_PREEMPTION_COMPUTE_CILP
        },
        "the C's guest asked for instruction-level compute preemption"
    );
    assert_eq!(
        ours.asks_for(),
        CtxswPreemptionAsk::WaitForIdle,
        "ours asks for the mode we are already in"
    );

    // …and the two get different answers through the whole policy.
    let (gpu, h) = gpu_with_a_group();
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let re = |r: &CtxswPreemptionRequest| CtxswPreemptionRequest {
        h_channel: h.tsg.0,
        ..*r
    };
    assert_eq!(
        answer(&mut p, &re(&c)).expect("claimed").rpc_result,
        CTXSW_PREEMPTION_REFUSED_STATUS
    );
    assert_eq!(answer(&mut p, &re(&ours)).expect("claimed").rpc_result, 0);
}

/// ★★★ The `NV_OK` is about an object we can actually **see** — and the object is the
/// request's `hChannel` field, not the control's `hObject`.
///
/// ⊘ `[measured 2026-08-10, boot s46_1a9e93c_abi35 record 331]` `hObject` is the subdevice
/// (`0x5c000003`) and `hChannel` is the TSG (`0x5c000012`). A port that routed on `hObject` would be answering about the subdevice,
/// which nobody asked about, and would then answer `NV_OK` for **any** `hChannel` at all.
#[test]
fn the_answer_is_about_the_hchannel_field_and_a_context_we_can_see() {
    let (gpu, h) = gpu_with_a_group();

    // A handle nothing was ever allocated at.
    assert!(matches!(
        gpu.set_ctxsw_preemption_mode(CLIENT, HObject(0xdead_beef)),
        Err(CtxswPreemptionFault::UnknownContext { .. })
    ));
    // A live handle that is neither a group nor a channel — the process's own VA space.
    assert!(matches!(
        gpu.set_ctxsw_preemption_mode(CLIENT, h.vaspace),
        Err(CtxswPreemptionFault::NotAContext { .. })
    ));
    // The group: served, and reported AS a group.
    let ack = gpu
        .set_ctxsw_preemption_mode(CLIENT, h.tsg)
        .expect("the TSG resolves and its members are placed");
    assert!(
        ack.was_group,
        "hChannel carried a TSG handle and the ack must say so — the field's NAME is not the \
         field's type ({ack:?})"
    );
    // A bare member channel: also served, and reported as not-a-group.
    let ack = gpu
        .set_ctxsw_preemption_mode(CLIENT, h.gr_channel)
        .expect("a member channel resolves too");
    assert!(!ack.was_group, "{ack:?}");

    // …and through the policy, an unseeable context is a REFUSAL, not an NV_OK.
    let (gpu, _h) = gpu_with_a_group();
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let r = answer(
        &mut p,
        &request(
            HObject(0xdead_beef),
            CTXSW_PREEMPTION_COMPUTE_WFI,
            CTXSW_PREEMPTION_GFX_WFI,
            CTXSW_PREEMPTION_FLAGS_CILP_SET,
        ),
    )
    .expect("claimed");
    assert_eq!(
        r.rpc_result, CTXSW_PREEMPTION_REFUSED_STATUS,
        "★ a wait-for-idle request about an object this port cannot resolve is still a \
         promise about nothing"
    );
}

/// ⊘ The reply body is the request's own bytes — asserted **only** because a zero body would
/// rewrite the caller's struct, never as evidence that anything was performed.
///
/// `paramsSize` is 32, non-zero, so the GSP transport copies the reply's params over the
/// caller's own struct (`ogkm-580: rpc.c:11085-11090`).
/// `[measured 2026-08-10, cap3_matmul_forwarding]` the C does the same:
/// #453717 is the request element verbatim with only `checkSum`, `seqNum`,
/// `rpc_result` and `rpc_result_private` rewritten.
#[test]
fn the_served_reply_carries_the_requests_own_params_back() {
    let (gpu, h) = gpu_with_a_group();
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let req = request(
        h.tsg,
        CTXSW_PREEMPTION_COMPUTE_WFI,
        CTXSW_PREEMPTION_GFX_WFI,
        CTXSW_PREEMPTION_FLAGS_CILP_SET,
    );
    let reply = answer(&mut p, &req).expect("claimed");
    assert_eq!(reply.rpc_result, 0);
    let wanted = encode_ctxsw_preemption_mode(&req);
    assert!(
        reply
            .body
            .windows(wanted.len())
            .any(|w| w == wanted.as_slice()),
        "the request's 32 params bytes must appear in the reply; a zero-filled body would \
         clear the caller's flags and hChannel behind its back"
    );
}

// =====================================================================================
// The claim, the decode, and the non-vacuity
// =====================================================================================

/// The id is claimed by `ObjectPolicy`, by command id and not by the `RmControl` function.
#[test]
fn the_control_is_claimed_by_id() {
    assert!(
        OBJECT_CONTROLS.contains(&NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE),
        "★ §16.59 — 0x20801210 is CLAIMED. Before this rung it fell to the unserviced \
         ledger, which is what a 0x56 at record 331 means"
    );
}

/// ⊘ **An unknown `flags` bit is refused, not masked away.** `asks_for` would report
/// `WaitForIdle` for a request whose meaningful mode words we could not identify, and a
/// served `NV_OK` on that basis is the exact shape of the lie this rung exists to avoid.
#[test]
fn an_unknown_flag_bit_is_a_decode_failure_and_not_a_wait_for_idle() {
    let mut bytes = GUEST_S46_REQUEST_BYTES;
    bytes[0] = 0x05; // FLAGS_CILP_SET | an undocumented bit 2
    assert!(matches!(
        decode_ctxsw_preemption_mode(&bytes),
        Err(CtxswPreemptionError::UnknownFlags { flags: 0x5 })
    ));
    // …and short params are refused rather than zero-extended.
    assert!(matches!(
        decode_ctxsw_preemption_mode(&bytes[..31]),
        Err(CtxswPreemptionError::ShortParams { got: 31 })
    ));
}

/// ★ A mode word the `flags` mask says to IGNORE does not refuse the request. RM's own
/// prose: the flags *"tell callee which mode is valid in the call"*
/// (`ogkm-580: ctrl2080gr.h:797-800`).
#[test]
fn an_ignored_mode_word_is_ignored() {
    let r = request(
        HObject(0x1),
        CTXSW_PREEMPTION_COMPUTE_CILP,
        2,
        0, // neither FLAGS_CILP nor FLAGS_GFXP set
    );
    assert_eq!(
        r.asks_for(),
        CtxswPreemptionAsk::WaitForIdle,
        "★ both mode words carry non-WFI values and NEITHER is meaningful. Refusing here \
         would refuse on bytes RM itself says to disregard"
    );
}

/// ⊘ **Non-vacuity of the probe.** If `answer` could only ever produce one outcome, every
/// assertion above passes while checking nothing.
#[test]
fn the_probe_can_both_serve_and_refuse() {
    let (gpu, h) = gpu_with_a_group();
    let mut p = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    let ok = answer(
        &mut p,
        &request(
            h.tsg,
            CTXSW_PREEMPTION_COMPUTE_WFI,
            CTXSW_PREEMPTION_GFX_WFI,
            CTXSW_PREEMPTION_FLAGS_CILP_SET,
        ),
    )
    .expect("claimed");
    let no = answer(
        &mut p,
        &request(
            h.tsg,
            CTXSW_PREEMPTION_COMPUTE_CILP,
            CTXSW_PREEMPTION_GFX_WFI,
            CTXSW_PREEMPTION_FLAGS_CILP_SET,
        ),
    )
    .expect("claimed");
    assert_ne!(
        ok.rpc_result, no.rpc_result,
        "the probe must be able to say two different things"
    );
    assert_eq!(
        CTXSW_PREEMPTION_REFUSED_STATUS,
        kayfabe_abi::NV_ERR_NOT_SUPPORTED
    );
}
