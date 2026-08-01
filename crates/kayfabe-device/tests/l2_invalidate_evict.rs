//! `NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT` (`0x20800a6c`) — the fourteenth
//! control this port serves, and ★★★ **the first that is an INSTRUCTION TO ACT on hardware
//! this device does not have.**
//!
//! ## What this file establishes, in the order it matters
//!
//! 1. **The rung.** `kbusVerifyBar2_GM107:4110-4115` turns a refusal into
//!    `NV_PRINTF("L2 evict failed")` and a `goto`, so this control stands between the boot
//!    and the rest of `kbusVerifyBar2` — the BAR2 sub-test at `:4155-4200`, which is where
//!    an MMU translation is exercised for the first time. `[measured]`
//!    `docs/design/boot_measured_2026_08_01.md` §21 and §24, boot `bar0win` (rev `f43668b`).
//! 2. **The `0x20800301` trap is checked and is NOT present here.** That control had to
//!    re-encode its request because the transport copies the reply's params back over the
//!    caller's struct (`ogkm-580: rpc.c:11085-11090`) and the caller then reads its own
//!    fields. `kmemsysSendL2InvalidateEvict_IMPL` does not: its params are a stack local and
//!    it `return`s the transport's status directly (`ogkm-580: kern_mem_sys.c:1079-1093`).
//!    ⇒ the two controls take **opposite** answers to the same question, and
//!    [`the_reply_is_four_zeros_which_is_the_opposite_of_the_event_control`] pins that they
//!    do.
//! 3. **The reply is not a fall-through.** `#127`'s default is a named refusal, so an
//!    `NV_OK` has to be earned per operation. It is earned for the six flag bits the 580 SDK
//!    names and refused for anything else —
//!    [`a_flag_bit_the_sdk_does_not_name_is_refused_rather_than_blanket_accepted`].
//!
//! ## ⚠ What it does NOT establish
//!
//! ⊘ **Nothing about a real L2.** This device has none. The whole licence is that the
//! postcondition — *"the next read does not hit a stale copy"* — holds structurally, because
//! [`kayfabe_device::fbwin`]'s store **is** the framebuffer rather than a cache over one.
//! `kayfabe_abi::l2evict` names the three futures that falsify that, and the first of them
//! (real host-GPU forwarding) is on this project's road. ⊘ No test here can see it coming;
//! only a re-decision of the row can.
//!
//! ⊘ **No boot is claimed by this file.** That serving it advances the boot is `[inferred]`
//! from `ogkm-580: kern_bus_gm107.c:4106-4118` until a boot says otherwise, and §23 of the
//! boot doc is the specific warning: the `bar0win` boot reaches `gpuStateLoad` partly
//! because `gpuStateInit_IMPL` maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and `KernelBus` is
//! amputated. Serving this control may **re-expose** what that amputation hides, which would
//! surface as a *new* failure that is progress.

use kayfabe_abi::l2evict::{
    self, FLAGS_ALL, FLAGS_CLEAN, FLAGS_CLEAN_VERIFY_BAR2, FLAGS_FIRST, FLAGS_LAST, FLAGS_NORMAL,
    FLAGS_OFF, FLAGS_WAIT_FB_PULL, L2_INVALIDATE_EVICT_FLAGS_KNOWN,
    L2_INVALIDATE_EVICT_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::sweep::{SWEEP_TRIAGE, SweepDisposition};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 44 - 4 = 40`.
const PARAMS_AT: usize = 40;

/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

/// Byte offset of `paramsSize` in the reply's control header.
const CONTROL_PARAMS_SIZE_OFF: usize = 16;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying an L2 invalidate/evict.
///
/// ★★ Laid down over a `0xAA` fill, so every byte the struct does not define is poison. An
/// echo would bring `0xAA` back; this port's reply must not.
fn evict_command(flags: u32, params_size: u32) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12]
        .copy_from_slice(&NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    if params_size as usize >= L2_INVALIDATE_EVICT_PARAMS_SIZE {
        let at = PARAMS_AT + FLAGS_OFF;
        payload[at..at + 4].copy_from_slice(&flags.to_le_bytes());
    }
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 30,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The params half of a served reply, or `None` if the policy refused.
fn served_params(flags: u32) -> Option<Vec<u8>> {
    let cmd = evict_command(flags, L2_INVALIDATE_EVICT_PARAMS_SIZE as u32);
    let reply = policy().respond(&cmd)?;
    if reply.rpc_result != 0 {
        return None;
    }
    Some(reply.body[PARAMS_AT..PARAMS_AT + L2_INVALIDATE_EVICT_PARAMS_SIZE].to_vec())
}

// ── The rung ──────────────────────────────────────────────────────────────────────────

#[test]
fn the_evict_kbusverifybar2_actually_sends_is_served() {
    // ★★★ The rung, at the exact value the guest sends. `flagsClean` on a GA106 is
    // `ALL | CLEAN | WAIT_FB_PULL` — `WAIT_FB_PULL` is included because `bL2CleanFbPull` is
    // NV_TRUE for this chip in the NVOC HAL initialiser
    // (`ogkm-580: g_kern_mem_sys_nvoc.c:256-262`), which the registry-override code in
    // `kern_mem_sys.c:44-53` does not reveal.
    let cmd = evict_command(
        FLAGS_CLEAN_VERIFY_BAR2,
        L2_INVALIDATE_EVICT_PARAMS_SIZE as u32,
    );
    let reply = policy().respond(&cmd).expect("this port serves 0x20800a6c");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    // ⊘ And the inner status too. RM reads `rpc_params->status` before it copies params out
    // (`ogkm-580: rpc.c:11061-11065`), and `kbusVerifyBar2_GM107:4111` tests the value that
    // comes back out of `pRmApi->Control`. An NV_OK envelope around a non-zero inner status
    // is still "L2 evict failed".
    assert_eq!(
        u32::from_le_bytes(
            reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
                .try_into()
                .expect("four bytes")
        ),
        0,
        "the control header's own status says NV_OK"
    );
    // The reply describes itself: four bytes of params, which is what the oracle's captured
    // row declares (`C: mode2_initctrl_ga106.h:6245`, `psize = 4`).
    assert_eq!(
        u32::from_le_bytes(
            reply.body[CONTROL_PARAMS_SIZE_OFF..CONTROL_PARAMS_SIZE_OFF + 4]
                .try_into()
                .expect("four bytes")
        ),
        L2_INVALIDATE_EVICT_PARAMS_SIZE as u32
    );
    assert_eq!(reply.body.len(), cmd.payload.len());
}

#[test]
fn the_control_is_in_the_served_universe_every_gate_quantifies_over() {
    // ★★ "In `ALL`" and "served" are one fact, by construction — `from_cmd` is a lookup
    // through `ALL`. Asserted from the outside anyway, because that construction is what a
    // future refactor would break silently.
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT),
        Some(WantedTable::MemsysL2InvalidateEvict)
    );
    assert!(WantedTable::ALL.contains(&WantedTable::MemsysL2InvalidateEvict));
    assert_eq!(
        WantedTable::MemsysL2InvalidateEvict.params_size(),
        L2_INVALIDATE_EVICT_PARAMS_SIZE
    );
}

// ── The `0x20800301` trap, checked explicitly ─────────────────────────────────────────

#[test]
fn the_reply_is_four_zeros_which_is_the_opposite_of_the_event_control() {
    // ★★★ The file's headline. `0x20800301` MUST echo its request because the caller reads
    // `pSetEventParams->action` after the RPC returns; `0x20800a6c` must NOT, because its
    // caller's params are a stack local that dies at the `return`. Same transport, same
    // "zero [OUT] fields", opposite answers — so the rule cannot be "does the struct have
    // [OUT] fields?" and has to be "does the caller read its own params afterwards?".
    //
    // ⊘ And the oracle agrees from the other side: `{0x20800a6cu, 0x0u, 4u, 0u}` with an
    // EMPTY `ctl_20800a6c[]`, where `dlen` is trailing-zero-trimmed
    // (`C: mode2_initctrl_ga106.h:6245, :3346`; `C: src/qemu/nvkvm_gpu_emul.c:3422-3425`).
    // A real GA106's GSP returned four zeros, not the flags it was sent.
    for flags in [
        0,
        FLAGS_ALL,
        FLAGS_ALL | FLAGS_CLEAN,
        FLAGS_CLEAN_VERIFY_BAR2,
        L2_INVALIDATE_EVICT_FLAGS_KNOWN,
    ] {
        let params = served_params(flags).unwrap_or_else(|| panic!("{flags:#x} must be served"));
        assert_eq!(
            params,
            vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE],
            "flags {flags:#x} came back in the reply"
        );
    }
}

#[test]
fn not_one_poison_byte_of_the_request_survives_into_the_reply_params() {
    // ⊘ The request's params are laid over a `0xAA` fill. An echo would bring `0xAA` back
    // for a params size larger than the struct; a re-encode of the wrong length would leave
    // a tail. Neither is allowed.
    let params = served_params(FLAGS_CLEAN_VERIFY_BAR2).expect("served");
    assert!(
        !params.contains(&0xAA),
        "a byte of the request's poison fill reached the reply: {params:02x?}"
    );
}

// ── The answer is EARNED per operation, not a fall-through ────────────────────────────

#[test]
fn a_flag_bit_the_sdk_does_not_name_is_refused_rather_than_blanket_accepted() {
    // ★★★ The gate that keeps this from being a generic `NV_OK`. The licence enumerates six
    // operations and says why each is already true on this device; a seventh, named by a bit
    // the 580 SDK does not define, has no such argument.
    for bad in [
        0x0000_0040, // the first bit past the named set
        0x0000_0080,
        0x8000_0000,                              // the one a sign-extension bug sets
        L2_INVALIDATE_EVICT_FLAGS_KNOWN | 0x0100, // a legal request plus one unnamed bit
    ] {
        let cmd = evict_command(bad, L2_INVALIDATE_EVICT_PARAMS_SIZE as u32);
        let reply = policy()
            .respond(&cmd)
            .unwrap_or_else(|| panic!("{bad:#x} must be answered, not ignored"));
        assert_ne!(
            reply.rpc_result, 0,
            "flags {bad:#x} set a bit this port cannot name and was answered NV_OK anyway"
        );
        assert!(reply.body.is_empty(), "a refusal carries no body");
    }
}

#[test]
fn every_named_flag_subset_is_served_including_the_empty_request() {
    // ⊘ Quantified over the whole 64-element subset lattice rather than the two values the
    // driver happens to send: the licence is per-bit, so the gate is too. `flags == 0` is
    // served rather than refused — refusing it would be inventing a rule RM does not have.
    let mut served = 0usize;
    for flags in 0..=L2_INVALIDATE_EVICT_FLAGS_KNOWN {
        if flags & !L2_INVALIDATE_EVICT_FLAGS_KNOWN != 0 {
            continue;
        }
        assert!(
            served_params(flags).is_some(),
            "flags {flags:#x} is a subset of the named bits and was refused"
        );
        served += 1;
    }
    assert_eq!(served, 64, "the six named bits have 64 subsets");
}

#[test]
fn a_clean_and_a_plain_invalidate_get_the_same_answer_because_nothing_is_dirty() {
    // ★★ The vacuity argument, made checkable rather than only narrated. On real silicon
    // `CLEAN` writes dirty lines back before dropping them and `NORMAL` does not, so the two
    // are different operations. On this device there are no lines and no dirt, so both
    // postconditions hold identically — and the replies are identical too.
    assert_eq!(
        served_params(FLAGS_ALL | FLAGS_CLEAN),
        served_params(FLAGS_ALL | FLAGS_NORMAL)
    );
    assert_eq!(
        served_params(FLAGS_ALL | FLAGS_WAIT_FB_PULL),
        served_params(FLAGS_ALL)
    );
    assert_eq!(
        served_params(FLAGS_FIRST | FLAGS_LAST),
        served_params(FLAGS_ALL)
    );
}

// ── The guest's own assertions are still checked ──────────────────────────────────────

#[test]
fn a_declared_size_that_is_not_the_structs_is_refused() {
    // ⊘ The guest's `paramsSize` is the guest's assertion about its own struct. A mismatch
    // means the struct is not the one this port encodes, and answering anyway would hand RM
    // a well-formed read at the wrong stride.
    for size in [0u32, 2, 3, 5, 8, 20] {
        let cmd = evict_command(FLAGS_CLEAN_VERIFY_BAR2, size);
        let reply = policy().respond(&cmd).expect("answered");
        assert_ne!(
            reply.rpc_result, 0,
            "a declared paramsSize of {size} was served as if it were 4"
        );
    }
}

#[test]
fn a_finn_serialized_payload_is_refused_because_this_encoder_produces_a_flat_struct() {
    // ⊘ `RMAPI_RPC_FLAGS_SERIALIZED` sends the reply down `serverDeserializeCtrlUp` instead
    // of the flat copy (`ogkm-580: rpc.c:11072-11078`). Four flat zeros are not that.
    let mut cmd = evict_command(
        FLAGS_CLEAN_VERIFY_BAR2,
        L2_INVALIDATE_EVICT_PARAMS_SIZE as u32,
    );
    // `RMAPI_RPC_FLAGS_SERIALIZED` is private to `kayfabe-abi`; bit 1 is the value its own
    // `rpc_params_are_serialized` tests, and this asserts that rather than assuming it.
    cmd.payload[20..24].copy_from_slice(&(1u32 << 1).to_le_bytes());
    assert!(kayfabe_abi::rpc_params_are_serialized(1 << 1));
    let reply = policy().respond(&cmd).expect("answered");
    assert_ne!(reply.rpc_result, 0);
}

// ── The triage table and this file agree ──────────────────────────────────────────────

#[test]
fn the_triage_row_is_kept_and_says_this_control_is_served() {
    // ★★ The row is corrected rather than deleted, so the argument for refusing it and the
    // argument that overturned it sit side by side — the discipline `0x20800301`'s row
    // established. Its disposition stays `RefusalHalts`, which describes what happens if we
    // *stop* serving it and is orthogonal to whether we do.
    let row = SWEEP_TRIAGE
        .iter()
        .find(|c| c.cmd == NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT)
        .expect("0x20800a6c is triaged");
    assert_eq!(row.disposition, SweepDisposition::RefusalHalts);
    assert_eq!(row.engine, "KernelMemorySystem");
    assert!(
        row.why.contains("SERVED"),
        "the row must say the decision was overturned, not merely carry the old argument"
    );
    assert!(
        row.why.contains("C:"),
        "the corroborating oracle row is what makes NV_OK more than an argument"
    );
}

#[test]
fn the_sysmembar_beside_it_was_decided_separately_and_is_still_refused() {
    // ★★★ ⊘ The one this rung must NOT silently generalise to. `0x20800a70` was triaged in
    // the same breath as `0x20800a6c` and its answer is DIFFERENT — see its `why`.
    //
    // Two facts, and they are independent. (a) It is still refused: a sysmembar's
    // postcondition is about the write path crossing to system memory, which this port
    // genuinely acquires the day a real host GPU DMAs into guest RAM. (b) Its *stated
    // reason* was wrong and is corrected: `kbusFlush_GM107` overwrites its status only for
    // `NV_ERR_TIMEOUT` (`ogkm-580: kern_bus_gm107.c:3384-3405`), and GA106 dispatches
    // `kbusFlush` there (`g_kern_bus_nvoc.c:1871-1881`), so the refusal is INVISIBLE rather
    // than halting — including at `kbusVerifyBar2_GM107:4218-4221`, the one site that checks
    // a flush.
    assert_eq!(WantedTable::from_cmd(0x2080_0a70), None, "still refused");
    let row = SWEEP_TRIAGE
        .iter()
        .find(|c| c.cmd == 0x2080_0a70)
        .expect("0x20800a70 is triaged");
    assert_eq!(row.disposition, SweepDisposition::RefusalIsInvisible);
    assert!(
        row.why.contains("CORRECTED"),
        "a disposition that changed must say so, or the reader trusts the old reading"
    );
}

#[test]
fn the_decoder_and_the_policy_refuse_the_same_requests() {
    // ⊘ Two implementations of "may this be served?" would drift. The policy's serve
    // decision is the decoder's `Ok`, and nothing else — asserted over the whole low byte,
    // which covers every named bit and the first two unnamed ones.
    for flags in 0u32..=0xff {
        let decodes = l2evict::decode_l2_invalidate_evict(&flags.to_le_bytes()).is_ok();
        let serves = served_params(flags).is_some();
        assert_eq!(
            decodes, serves,
            "flags {flags:#x}: decode {decodes}, serve {serves}"
        );
    }
}
