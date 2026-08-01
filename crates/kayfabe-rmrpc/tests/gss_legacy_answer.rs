//! **What this GSP answers when a rule — not a table row — let a control through.**
//!
//! The subject is `BridgeRefusal::GspRuleControlUnserviced`, and the property under test
//! is not "a refusal happens" but *"a plausible answer cannot happen by accident"*.
//!
//! ## The failure this file exists to keep red
//!
//! The C research artifact's default reply to an unmodelled control was the request echoed
//! back under `NV_OK`. For a control whose params are `[OUT]` that is a body of zeros, and
//! the CUDA runtime read those zeros as real data and aborted with
//! `cudaErrorInitializationError(3)` **silently** — no errno, no log line, the rejection
//! living entirely in the reply payload (`C: src/qemu/nvkvm_gpu_emul.c:3335-3360`). A wrong
//! number is worse than a crash because nothing reports it.
//!
//! Reintroducing that echo is a one-character change: `GraphPolicy::respond` returning
//! `None` instead of `Some(Reply)` makes the FSM post `RpcCommand::ack(0)`. So every test
//! below asserts the **envelope status word the guest actually branches on**, not merely
//! that a `Result` was `Err` — and the two `posts_*` tests assert it on the `OutgoingRpc`
//! the transport would really put on the ring.
//!
//! ## Why the envelope word and not the body
//!
//! `rpcRmApiControl_GSP` short-circuits on the envelope: `_issueRpcAndWait` returns
//! non-`NV_OK` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:1994`, `ogkm-610: :2012`) and
//! the whole post-RPC block is skipped — both the copy-out *and* the GSS-legacy control
//! cache that would otherwise persist our answer in the guest and stop the RPC ever
//! reaching us again (`ogkm-580: rpc.c:11098-11103`, `ogkm-610: :10903-10908`). The full
//! reading is on the refusal variant's own docs.

use kayfabe_abi::capability::{ControlPermit, PassthroughRule, RM_GSS_LEGACY_MASK};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_abi::{GuestOs, NV_ERR_NOT_SUPPORTED};
use kayfabe_arch::ids::ControlCmd;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kayfabe_mocks::{MockIsolateFactory, WireClassArch};
use kayfabe_rmrpc::{BridgeRefusal, GraphPolicy, Translation, translate};
use kayfabe_trace::{FaultTag, Faulted};

// =====================================================================================
// Harness
// =====================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

fn fresh_gpu() -> Gpu {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes")
}

/// ★ The **three commands the C measured**, read off its own source rather than invented:
/// the cudart initialisation-gate cluster at `C: src/qemu/nvkvm_gpu_emul.c:3328-3395`. They
/// are the concrete instance of this refusal — the CUDA runtime issues them near the end of its
/// lazy device enumeration, they are serviced entirely by GSP firmware, and they are the
/// commands the all-zeros echo actually broke.
///
/// Not a driver-version fact and not a chip fact: a command word is Axis-A-free, and the
/// point of the list is provenance, not coverage.
const CUDART_INIT_GATE: [u32; 3] = [0x2080_9009, 0x2080_9001, 0x2080_9064];

/// A `GSP_RM_CONTROL` payload: `rpc_gsp_rm_control_v03_00`, 40-byte fixed header
/// (`hClient`@0, `hObject`@4, `cmd`@8, `status`@12, `paramsSize`@16, `rmapiRpcFlags`@20,
/// `rmctrlFlags`@24, `rmctrlAccessRight`@28, `reserved0`@32) then `params[]`.
///
/// ★ `params` is **all zeros**, deliberately: that is what the guest sends for an `[OUT]`
/// control, and it is precisely the body an echo would hand back as an answer.
fn control_cmd(cmd: u32, params_len: usize) -> RpcCommand {
    let mut payload = vec![0u8; 40 + params_len];
    payload[0..4].copy_from_slice(&0x0000_c1d0u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0x0000_0b1eu32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params_len as u32).to_le_bytes()); // paramsSize
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

// =====================================================================================
// 1. The default is NAMED, and it names the rule
// =====================================================================================

/// The three commands the C measured are all GSS-legacy, and all **non-privileged**
/// (`RM_GSS_LEGACY_MASK_PRIVILEGED` = `0xC000`, `ogkm-580:
/// src/nvidia/interface/deprecated/rmapi_deprecated.h:41-43`, `ogkm-610: :41-43`).
///
/// Pins the premise the rest of the file rests on. Without it, a fixture that quietly
/// stopped having bit 15 set would make every assertion below pass for the wrong reason.
#[test]
fn the_c_measured_cluster_is_gss_legacy_and_reaches_the_rule() {
    for cmd in CUDART_INIT_GATE {
        assert_ne!(cmd & RM_GSS_LEGACY_MASK, 0, "{cmd:#010x} is not GSS-legacy");
        assert_ne!(
            cmd & 0xC000,
            0xC000,
            "{cmd:#010x} would be the PRIVILEGED form"
        );
        assert_eq!(
            abi().capabilities().control(ControlCmd(cmd)),
            ControlPermit::GssLegacyRule,
            "{cmd:#010x} must reach the gate by the RULE, not by a table row",
        );
        assert_eq!(
            abi().control_params(ControlCmd(cmd)),
            None,
            "{cmd:#010x} must be unmodelled — otherwise this file tests nothing",
        );
    }
}

/// The default itself: exact variant, carrying the command **and** the rule.
#[test]
fn an_unknown_gss_legacy_control_refuses_by_name() {
    for cmd in CUDART_INIT_GATE {
        assert_eq!(
            translate(abi(), GuestOs::Linux, &control_cmd(cmd, 8)),
            Err(BridgeRefusal::GspRuleControlUnserviced {
                cmd,
                rule: PassthroughRule::GssLegacy,
            }),
        );
    }
}

/// The binary-API class rule is the *other* rule-based passthrough and shares the
/// premise, so it shares the arm — and carries its own discriminator.
#[test]
fn the_binapi_rule_lands_in_the_same_arm_with_its_own_rule() {
    // NV2081_BINAPI: any command in the class, with bit 15 CLEAR so the GSS rule cannot
    // be the one that answers.
    let cmd = 0x2081_0042u32;
    assert_eq!(cmd & RM_GSS_LEGACY_MASK, 0);
    assert_eq!(
        abi().capabilities().control(ControlCmd(cmd)),
        ControlPermit::BinApiRule,
    );
    assert_eq!(
        translate(abi(), GuestOs::Linux, &control_cmd(cmd, 8)),
        Err(BridgeRefusal::GspRuleControlUnserviced {
            cmd,
            rule: PassthroughRule::BinApi,
        }),
    );
}

/// A control that is neither denied nor rule-admitted nor modelled keeps the OLD arm.
///
/// The non-vacuity instrument for the split: if `GspRuleControlUnserviced` had simply
/// replaced `UnknownControl`, every test above would still pass and the distinction would
/// be decoration.
#[test]
fn a_listed_but_unmodelled_control_is_still_unknown_control() {
    // On the allowlist (a row names it), bit 15 clear, no params decoder.
    let listed = abi()
        .capabilities()
        .all_controls()
        .map(|e| e.cmd)
        .find(|&c| {
            c & RM_GSS_LEGACY_MASK == 0
                && (c >> 16) & 0xffff != 0x2081
                && abi().control_params(ControlCmd(c)).is_none()
        })
        .expect("some allowlisted control has no params decoder");
    assert_eq!(
        abi()
            .capabilities()
            .control(ControlCmd(listed))
            .passthrough_rule(),
        None,
    );
    assert_eq!(
        translate(abi(), GuestOs::Linux, &control_cmd(listed, 8)),
        Err(BridgeRefusal::UnknownControl { cmd: listed }),
    );
}

/// The two rules are separately countable in the census.
///
/// ★ Distinct tags, and distinct from `UnknownControl`'s: a census that merged them could
/// not answer "which rule is the long tail arriving through?", which is the number that
/// decides what earns a forward arm first.
#[test]
fn each_rule_gets_its_own_census_tag() {
    let gss = BridgeRefusal::GspRuleControlUnserviced {
        cmd: CUDART_INIT_GATE[0],
        rule: PassthroughRule::GssLegacy,
    };
    let bin = BridgeRefusal::GspRuleControlUnserviced {
        cmd: 0x2081_0042,
        rule: PassthroughRule::BinApi,
    };
    let unknown = BridgeRefusal::UnknownControl { cmd: 0x1234 };
    assert_eq!(
        gss.fault_tag(),
        FaultTag("BridgeRefusal::GspRuleControlUnserviced::GssLegacy"),
    );
    assert_eq!(
        bin.fault_tag(),
        FaultTag("BridgeRefusal::GspRuleControlUnserviced::BinApi"),
    );
    assert_ne!(gss.fault_tag(), bin.fault_tag());
    assert_ne!(gss.fault_tag(), unknown.fault_tag());
}

// =====================================================================================
// 2. ★★★ The zeros cannot be handed over — asserted on the wire
// =====================================================================================

/// **The sharpest test in the file.** An unknown GSS-legacy control must not be
/// answerable with zeros by accident.
///
/// Asserts the `OutgoingRpc` the transport would actually post, and asserts the field the
/// guest branches on: a non-`NV_OK` envelope `rpc_result` makes `_issueRpcAndWait` return
/// early (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:1994`, `ogkm-610: :2012`), so neither
/// the copy-out nor the GSS-legacy control-cache write is reached, *whatever* our body
/// contains.
///
/// ★ Note what is deliberately **not** asserted: that the body is non-zero. The body IS
/// zeros — `RpcCommand::reply` zero-fills to the request's length. That is safe only
/// because the envelope says no, which is exactly why the envelope is what is pinned.
#[test]
fn posts_a_refusing_envelope_never_an_ok_echo() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);

    for cmd in CUDART_INIT_GATE {
        let c = control_cmd(cmd, 8);

        // What the FSM would post, built exactly as `GspFsm::answer` builds it.
        let out = match policy.respond(&c) {
            Some(r) => c.reply(r.rpc_result, &r.body),
            None => c.ack(0),
        };

        assert_eq!(
            out.rpc_result, NV_ERR_NOT_SUPPORTED,
            "{cmd:#010x}: the guest must short-circuit on the envelope",
        );
        assert_ne!(out.rpc_result, 0, "{cmd:#010x}: an NV_OK answer is the bug");
        assert_eq!(out.sequence, c.sequence);
        assert_eq!(out.function, c.code);
    }
    assert_eq!(policy.census().total(), 3);
    assert_eq!(
        policy.census().of(FaultTag(
            "BridgeRefusal::GspRuleControlUnserviced::GssLegacy"
        )),
        3,
    );
    // Non-vacuity: nothing was quietly accepted on the way.
    assert_eq!(policy.applied(), 0);
    assert_eq!(policy.inert(), 0);
    assert_eq!(policy.held(), 0);
}

/// The echo, built explicitly, so the thing this file forbids is *visible* in it.
///
/// `RpcCommand::ack(0)` is what `GraphPolicy::respond` returning `None` produces, and it
/// is the C's default. Asserting the policy's real answer differs from it in the status
/// word is the direct statement of the regression this file catches.
#[test]
fn the_c_echo_and_our_answer_are_different_replies() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    let c = control_cmd(CUDART_INIT_GATE[0], 8);

    let echo = c.ack(0);
    assert_eq!(echo.rpc_result, 0, "the C's default is NV_OK");
    assert_eq!(echo.payload, c.payload, "…with the request's own body");
    // ★ The zeros are in `params[]` — bytes 40.. — which is the only part the guest copies
    // back to its caller (`rpc_params->params`). The 40-byte fixed header in front is not
    // zero and never was; asserting on the whole payload asserts the wrong thing, and this
    // test failed the first time it ran for exactly that reason.
    assert!(
        echo.payload[40..].iter().all(|&b| b == 0),
        "…whose params[] for an [OUT] control is all zeros — the measured failure",
    );

    let r = policy.respond(&c).expect("a refusal is never a drop");
    let ours = c.reply(r.rpc_result, &r.body);
    assert_ne!(ours.rpc_result, echo.rpc_result);
    assert_eq!(ours.rpc_result, NV_ERR_NOT_SUPPORTED);
}

/// A refusal is **not** a drop.
///
/// The guest blocks in `_issueRpcAndWait` polling `(function, sequence)`, so withholding a
/// reply hangs it for the whole RPC timeout rather than failing it. `respond` must return
/// `Some`, and it must match the request.
#[test]
fn a_refused_control_is_still_answered() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    let c = control_cmd(CUDART_INIT_GATE[2], 520);
    let r = policy.respond(&c);
    assert!(r.is_some(), "an unanswered command hangs the guest");
    let out = c.reply(r.expect("some").rpc_result, &[]);
    assert_eq!(out.payload.len(), c.payload.len(), "M9 clamp");
    assert_eq!(out.rpc_result, NV_ERR_NOT_SUPPORTED);
}

// =====================================================================================
// 2b. ★★★ The ACCEPTED path — the half the envelope argument does not reach
// =====================================================================================

/// **The envelope short-circuit says nothing about a command this policy ACCEPTS**, and
/// until 2026-08-01 every sticky-answer sentence in this crate was attached to a refusal.
///
/// An accepted control leaves `GraphPolicy::respond` with `rpc_result: 0` and the request's
/// own body, so `_issueRpcAndWait` returns `NV_OK` and `rpcRmApiControl_GSP`'s post-RPC
/// block — copy-out **and** control cache — runs in full. What discharges the hazard is a
/// property of the body: the guest zeroes `rmctrlFlags`/`rmctrlAccessRight` in every request
/// it sends (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10994-10995`,
/// `ogkm-610: :10799-10800`), and `rmapiControlIsCacheable`'s first test is
/// `!(flags & RMCTRL_FLAGS_CACHEABLE_ANY) -> NV_FALSE`
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/rmapi_cache.c:152-158`). Reflecting the request
/// reflects **zero**, and zero means *"do not remember this"*.
///
/// ⚠ So this test asserts the accident, and then asserts that it IS an accident: a request
/// with the bits pre-set comes straight back. That second half is why
/// `kayfabe_device::sticky::StickyAnswerGuard` exists and why this policy must go inside it
/// the day anything installs it.
#[test]
fn an_accepted_answer_echoes_the_guests_own_cacheability_bits_verbatim() {
    const RMCTRL_FLAGS_CACHEABLE: u32 = 0x0000_0400;
    // On a `GSP_RM_CONTROL` payload these two words ARE `rmctrlFlags` and
    // `rmctrlAccessRight` (`rpc_gsp_rm_control_v03_00`, +24 and +28).
    const WINDOW: std::ops::Range<usize> = 24..32;

    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);

    // ★ A command this policy really ACCEPTS. `SET_GUEST_SYSTEM_INFO` translates to
    // `Translation::Inert` unconditionally (`kayfabe_rmrpc::translate`), so the accepted
    // arm is reached without building graph state — which is the point: the arm is what is
    // under test, not the object model.
    let mut payload = vec![0u8; 64];
    payload[WINDOW].copy_from_slice(&[0xaa; 8]);
    let accepted = kayfabe_gsp::RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 1,
        sequence: 3,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    let r = policy
        .respond(&accepted)
        .expect("an accepted command is acknowledged");
    assert_eq!(r.rpc_result, 0, "the premise: this command is ACCEPTED");
    assert_eq!(
        r.body, accepted.payload,
        "the accepted arm echoes the request VERBATIM — that is the mechanism under test",
    );
    assert_eq!(
        &r.body[WINDOW], &[0xaa; 8],
        "the accepted arm filters the two words that decide whether the guest keeps our \
         answer forever; it did not before, and the rustdoc argument rests on it not doing \
         so — see `kayfabe_device::sticky`",
    );

    // ⇒ For a `GSP_RM_CONTROL` this arm hands `rmctrlFlags` straight back. Against a stock
    // guest that is ZERO (`ogkm-580: rpc.c:10994-10995`) and therefore not cacheable
    // (`rmapiControlIsCacheable`'s first test, `ogkm-580: rmapi_cache.c:152-158`); against a
    // guest that sets the bit it is the bit. Both halves, so neither can be read as the
    // other.
    let mut crafted = control_cmd(CUDART_INIT_GATE[0], 8);
    crafted.payload[24..28].copy_from_slice(&RMCTRL_FLAGS_CACHEABLE.to_le_bytes());
    let r = policy.respond(&crafted).expect("a refusal is never a drop");
    assert_ne!(
        r.rpc_result, 0,
        "an UNMODELLED GSS-legacy control must still be refused — the envelope is what \
         keeps the crafted flags out of the guest's cache today",
    );
    assert!(r.body.is_empty());
}

// =====================================================================================
// 3. The structural guarantee behind all of the above
// =====================================================================================

/// **No refusal can ever carry `NV_OK`.**
///
/// `BridgeRefusal::rpc_result` ignores `self` and is `const`, so this holds for every
/// variant that exists and every variant anyone adds — the guarantee is a property of the
/// function, not of a list of cases. Pinned at **compile time** so it cannot be weakened
/// without the crate failing to build.
const _: () = {
    assert!(NV_ERR_NOT_SUPPORTED != 0);
    assert!(BridgeRefusal::ReservedClient.rpc_result() != 0);
    assert!(BridgeRefusal::ImplicitVaspace.rpc_result() != 0);
};

/// **No modelled control is admitted by a rule** — and this pins a fact about the *data*,
/// not about the code. Named that way on purpose.
///
/// The two are worth keeping apart. `translate_control` runs the params-table lookup
/// **before** it consults the rule, so the split can only refine an arm that was already a
/// refusal, and a modelled control would be decoded even if its command word had bit 15.
/// But that ordering has **no test that can see it fail today**, because no modelled
/// control carries the bit — the mutation "check the rule first" leaves this file green.
/// What this test actually catches is the day someone *adds* a modelled control that a
/// rule would also admit, which is the condition under which the ordering starts mattering.
/// Stated rather than dressed up as an ordering test it is not.
#[test]
fn no_modelled_control_is_admitted_by_a_rule() {
    let modelled: Vec<u32> = abi()
        .capabilities()
        .all_controls()
        .map(|e| e.cmd)
        .filter(|&c| abi().control_params(ControlCmd(c)).is_some())
        .collect();
    assert!(
        !modelled.is_empty(),
        "no modelled control found — this test would be vacuous",
    );
    for cmd in modelled {
        assert_eq!(
            abi()
                .capabilities()
                .control(ControlCmd(cmd))
                .passthrough_rule(),
            None,
            "{cmd:#010x} is modelled AND rule-admitted — the ordering in \
             `translate_control` is now load-bearing and needs a test that bites",
        );
        // Whatever it decodes to, it must NOT be the rule refusal: the decoder ran.
        match translate(abi(), GuestOs::Linux, &control_cmd(cmd, 8)) {
            Err(BridgeRefusal::GspRuleControlUnserviced { .. }) => {
                panic!("{cmd:#010x} is modelled but was diverted to the rule arm")
            }
            Err(BridgeRefusal::UnknownControl { .. }) => {
                panic!("{cmd:#010x} is modelled but reached the unmodelled arm")
            }
            Ok(Translation::Held) => panic!("{cmd:#010x}: translate never holds"),
            Ok(_) | Err(_) => {}
        }
    }
}
