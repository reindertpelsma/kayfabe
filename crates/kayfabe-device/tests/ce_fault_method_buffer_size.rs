//! `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE` (`0x20802a08`) — the control that ended
//! the `irq1` boot, and the first one this port answers with a number **measured on real
//! silicon**.
//!
//! ## What these tests are for, and what they cannot be for
//!
//! The value itself is not derivable, so no test here can *check* it — only pin it, and pin
//! the two properties that would make a right value useless:
//!
//! 1. the reply body is the four bytes the guest reads back (the transport copies the reply
//!    over the caller's own struct and the caller reads it on the next line —
//!    `ogkm-580: kernel_ce.c:846`, `rpc.c:11085-11090`), and
//! 2. **zero is never served**, because a served zero is not a weaker answer — it is the
//!    original `RmInitAdapter failed! (0x25:0x1f:1249)` wearing an answer's clothes.
//!
//! ⊘ The number's *provenance* is the load-bearing part and it lives in
//! `kayfabe_abi::fmbsize` with the run that produced it. A test that asserted `20480` while
//! `20480` was a guess would be green and worthless; what makes it worth something is that a
//! real RTX 3060 was asked.

use kayfabe_abi::fmbsize::{
    self, CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE, FaultMethodBufferSizeError,
    GA106_CE_FAULT_METHOD_BUFFER_SIZE, NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipError, ChipProfile, identity_for};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 44 - 4 = 40`.
const PARAMS_AT: usize = 40;

/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy_for(c: &'static ChipProfile) -> InitTablePolicy {
    InitTablePolicy::new(c, *table_for(BENCH_DRIVER).expect("bench ABI"))
}

fn policy() -> InitTablePolicy {
    policy_for(chip())
}

/// A `GSP_RM_CONTROL` carrying the guest's `0x20802a08`.
///
/// ★★ Over a `0xAA` fill, so every byte the reply does not define is poison. The guest sends
/// this struct **zeroed** (`NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS params = {0}`,
/// `ogkm-580: kernel_ce.c:830`) — the poison is here to catch an echo, not to model the
/// guest.
fn fmb_command(params_size: u32) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 30,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    // ★ The two facts a served control must state before anything else: which command it
    // answers, and how many bytes RM allocated for the answer. Both come from the SDK
    // header, and both are confirmed on the wire by the real part — `psize=4`, RTX 3060 on
    // open 580.159.04, 2026-08-01.
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE),
        Some(WantedTable::CeFaultMethodBufferSize),
    );
    assert_eq!(
        WantedTable::CeFaultMethodBufferSize.cmd_id(),
        0x2080_2a08,
        "the id in the log line that ended boot irq1"
    );
    assert_eq!(
        WantedTable::CeFaultMethodBufferSize.params_size(),
        CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE,
    );
}

#[test]
fn the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire() {
    // ★★★ The end-to-end assertion, through the same policy a guest drives. The expected
    // bytes are transcribed from the RAW RPC reply captured on the real part —
    // `cmd=0x20802a08 psize=4 gspst=0x0 head=00 50 00 00` — not from the decoded integer, so
    // a byte-order mistake anywhere in the chain is caught here rather than by a boot.
    let cmd = fmb_command(CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE as u32);
    let reply = policy()
        .respond(&cmd)
        .expect("this port serves 0x20802a08 since the fmb rung");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(status, 0, "and so does the inner control status");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE];
    assert_eq!(
        params,
        &[0x00, 0x50, 0x00, 0x00],
        "the four bytes a real RTX 3060 returned"
    );
    // ⊘ And not the poison: an echo would bring `0xAA` back, which would ALSO be non-zero
    // and would ALSO clear `bufSizeInBytes > 0`. Passing that assert is not the bar.
    assert!(
        !params.contains(&0xAA),
        "the reply is an answer, not an echo"
    );
    assert_eq!(fmbsize::decode_fault_method_buffer_size(params), Ok(20480));
}

#[test]
fn the_answer_comes_from_the_chip_row_and_not_from_this_crate() {
    // ⊘ The property that keeps the measurement attributable: the value is the chip's, so a
    // second generation that has never been asked cannot be answered with GA106's number by
    // accident. If this ever reads a constant instead, the test fails.
    assert_eq!(
        chip().ce_fault_method_buffer_size,
        GA106_CE_FAULT_METHOD_BUFFER_SIZE,
    );
    assert_eq!(chip().ce_fault_method_buffer_size, 20480);
}

#[test]
fn a_chip_that_states_no_size_is_refused_at_realize_not_served_a_zero() {
    // ★★★ The falsifier for the whole rung. A zero is the exact input `memdescCreate`
    // rejects (`ogkm-580: mem_desc.c:239-241`), so a port that served it would reproduce
    // `0x25:0x1f:1249` while every gate here stayed green. It is refused at the assembly
    // point instead, where an operator can see it.
    let mut bad = *chip();
    bad.ce_fault_method_buffer_size = 0;
    assert_eq!(
        identity_for(&bad),
        Err(ChipError::NoFaultMethodBufferSize {
            device_id: bad.pci_device_id
        }),
        "a chip with no measured size must not realize"
    );
    // ⊘ And the refusal must SAY so — a diagnostic that named nothing would cost the bench
    // cycle this check exists to save.
    let msg = format!(
        "{}",
        ChipError::NoFaultMethodBufferSize {
            device_id: bad.pci_device_id
        }
    );
    assert!(msg.contains("fault method buffer"), "{msg}");
    assert!(msg.contains("0x25:0x1f:1249"), "{msg}");
    // The real row still realizes, so the check above is not passing for the wrong reason.
    assert!(identity_for(chip()).is_ok());
}

#[test]
fn the_encoder_refuses_a_zero_even_if_a_chip_row_smuggles_one_past() {
    // ⊘ Belt and braces, deliberately: the realize check and the encoder check are two
    // independent statements of one rule, so a future path that builds a policy without
    // going through `identity_for` still cannot serve a zero.
    assert_eq!(
        fmbsize::encode_fault_method_buffer_size(0),
        Err(FaultMethodBufferSizeError::Zero),
    );
    let mut bad = *chip();
    bad.ce_fault_method_buffer_size = 0;
    let leaked: &'static ChipProfile = Box::leak(Box::new(bad));
    let cmd = fmb_command(CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE as u32);
    // ⊘⊘ The observation is the RAW ENVELOPE, deliberately, and this line is the whole
    // reason the test is worth having.
    //
    // ★★★ It used to end `…decode_fault_method_buffer_size(params).ok()` — asking the
    // decoder whether a servable number came back. That is a **broken instrument**: the
    // decoder refuses zero, so "the policy refused" and "the policy cheerfully answered
    // four zero bytes" both came out `None` and the assertion passed either way. A planted
    // mutation replacing this arm's `return refuse()` with `.unwrap_or(vec![0u8; 4])`
    // SURVIVED (`scripts/bite_fmb_size.py`, 2026-08-01) — the exact defect this file exists
    // to catch, waved through by this file. Suspect the instrument first.
    //
    // So: an accepted answer is `rpc_result == 0` AND inner `status == 0`, whatever bytes
    // follow. A chip with no measured size must not produce one.
    let accepted: bool = policy_for(leaked).respond(&cmd).is_some_and(|reply| {
        let Some(st) = reply.body.get(CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4) else {
            return false;
        };
        let status = u32::from_le_bytes(st.try_into().expect("4 bytes"));
        reply.rpc_result == 0 && status == 0
    });
    assert!(
        !accepted,
        "a chip with no size must be REFUSED, never answered with four zero bytes"
    );
    // And the positive control, so the assertion above is not passing because the harness
    // is broken: the real chip DOES come away with a number through the same path.
    assert!(
        policy()
            .respond(&fmb_command(CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE as u32))
            .is_some()
    );
}
