//! `kayfabe_device::faultbuffer` — step **5a** and only step 5a: write down where the
//! guest put its replayable fault buffer, and change nothing else.
//!
//! ## ⊘⊘ §14.41 REFUTED this file's old premise
//!
//! `[measured 2026-08-09, boot `pu1448` at `ef20ccc`]`.
//!
//! It used to say that answering `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`
//! "would change what the guest's UVM bring-up does … on a path this port cannot service",
//! and treat `the_recorder_answers_nothing` as the load-bearing test. Boot `pu1448` @
//! `ef20ccc` measured what NOT answering does: `kgmmuFaultBufferReplayableAllocate_IMPL`
//! propagates the `0x56` (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1261-1272`),
//! `faultbufConstruct_IMPL` re-returns it, `UVM_REGISTER_GPU` fails, **`cuInit` dies**. The
//! mechanism was right and the conclusion was backwards.
//!
//! `kayfabe_device::inittables` now answers it; this type only records, from an observer
//! seat whose `observe` returns nothing — so *"it changes no reply byte"* is rustc's
//! guarantee and no longer a test's. What is left here is the **non-vacuity**: that the
//! recorder really sees the traffic it claims to record.
//!
//! ## ⊘ What is NOT here, deliberately
//!
//! No fault buffer. Nothing writes a 32-byte packet, moves `PUT`, or pulses an interrupt
//! leaf. `docs/design/resume_from_fault.md` §7 step 0 is explicit that steps 5b–5d are
//! gated on a decision this must not pre-empt — and serving the *registration* does not
//! pre-empt it: 5a is the step that says "receive and record", and the guest's own bring-up
//! is what turned "record" into "record and acknowledge".

use kayfabe_abi::faultbuffer::{
    FaultBufferRegistration, NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
    REGISTER_FAULT_BUFFER_PARAMS_SIZE,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::faultbuffer::{
    FAULT_BUFFER_SAMPLE_MAX, FaultBufferLog, FaultBufferNote, FaultBufferRecorder,
};
use kayfabe_gsp::{CommandObserver, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — the control header the params follow.
const CTRL_HEADER: usize = 40;

/// A `GSP_RM_CONTROL` naming `cmd`, carrying `params` as its declared window.
fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; CTRL_HEADER];
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    // `paramsSize` @ +16 — the guest's own declaration, which the recorder bounds.
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload.extend_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 3,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The params a stock guest sends: three 4 KiB pages of replayable fault buffer.
fn registration(size: u32, pages: &[u64]) -> Vec<u8> {
    let mut b = vec![0u8; REGISTER_FAULT_BUFFER_PARAMS_SIZE];
    b[0..4].copy_from_slice(&0xc1d0_0069u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x5c00_0031u32.to_le_bytes());
    b[8..12].copy_from_slice(&size.to_le_bytes());
    for (i, p) in pages.iter().enumerate() {
        let o = 16 + i * 8;
        b[o..o + 8].copy_from_slice(&p.to_le_bytes());
    }
    b
}

fn recorder() -> (FaultBufferRecorder, FaultBufferLog) {
    let log = FaultBufferLog::new();
    let driver = *table_for(BENCH_DRIVER).expect("bench ABI");
    (FaultBufferRecorder::new(driver, log.clone()), log)
}

/// ★★★ It records its own control and ignores everything else — and it CANNOT answer.
///
/// ⊘ The name changed at §14.41 with the type. `the_recorder_answers_nothing` asserted a
/// property that a `CommandObserver` cannot violate; keeping the old name would have left a
/// test whose title claims a guard it no longer provides.
#[test]
fn the_recorder_records_its_own_control_and_ignores_the_rest() {
    let (mut r, log) = recorder();
    let pages = [0x1_0000_0000u64, 0x1_0000_1000, 0x1_0000_2000];
    r.observe(&control(
        NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
        &registration(3 * 4096, &pages),
    ));
    r.observe(&control(0x2080_1803, &[]));
    r.observe(&RpcCommand {
        function: RpcFunction::Other(228),
        code: 228,
        sequence: 4,
        payload: Vec::new(),
        elements: 1,
        delivered: Vec::new(),
    });
    // ★ The non-vacuity, which is now the WHOLE of what this test can assert: it really did
    // see the first command. That it answered none of the three is no longer assertable
    // here, because `CommandObserver::observe` has no return value to assert about — the
    // property became rustc's at §14.41 and this test stopped being its guard.
    assert_eq!(log.total(), 1);
}

/// ★★ The address the guest declared is what comes out — the whole deliverable of 5a.
#[test]
fn the_declared_buffer_pages_are_what_is_recorded() {
    let (mut r, log) = recorder();
    let pages = [0x1_0000_0000u64, 0x1_0000_1000, 0x1_0000_2000];
    r.observe(&control(
        NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
        &registration(3 * 4096, &pages),
    ));
    assert_eq!(
        log.sample(),
        vec![FaultBufferNote::Registered(FaultBufferRegistration {
            h_client: 0xc1d0_0069,
            h_object: 0x5c00_0031,
            size: 3 * 4096,
            pages: pages.to_vec(),
        })],
        "★ 'where is the buffer' is now a fact, not an unknown"
    );
}

/// Only this command is recorded. A ledger that recorded neighbours would answer a
/// different question from the one it is named after.
#[test]
fn no_other_control_reaches_the_log() {
    let (mut r, log) = recorder();
    for cmd in [
        0x2080_1803u32,
        // The UNREGISTER neighbour, one command away — the most plausible mis-match.
        0x2080_0a9c,
        // The other GMMU internal in the same block.
        0x2080_0a9f,
        NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER ^ 1,
    ] {
        r.observe(&control(cmd, &registration(4096, &[0x2000])));
    }
    assert_eq!(log.total(), 0, "neighbours are not this command");
    assert!(log.sample().is_empty());
}

/// ★ A malformed registration is recorded **as malformed**, not dropped.
///
/// *"The guest never asked"* and *"the guest asked in a shape we could not read"* are
/// different findings, and the second is the one that means this port's layout is wrong.
#[test]
fn a_short_params_window_is_recorded_as_malformed() {
    let (mut r, log) = recorder();
    r.observe(&control(
        NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
        &[0u8; 64],
    ));
    assert_eq!(
        log.sample(),
        vec![FaultBufferNote::Malformed { len: 64 }],
        "the ask is visible even when the params are not"
    );
    assert_eq!(log.total(), 1);
}

/// ★★ Two registrations at two addresses stay **two** records.
///
/// De-duplicating would hide a re-registration, which is exactly what an
/// unregister/register cycle looks like from here — and knowing the buffer moved is the
/// only thing that makes a recorded address trustworthy later.
#[test]
fn a_re_registration_is_not_collapsed_into_the_first() {
    let (mut r, log) = recorder();
    for base in [0x1_0000_0000u64, 0x2_0000_0000] {
        r.observe(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &registration(4096, &[base]),
        ));
    }
    let s = log.sample();
    assert_eq!(s.len(), 2);
    assert_ne!(s[0], s[1], "the second address is not the first");
}

/// ⊘ The remembered list is capped and the counter is not — a guest that loops on a
/// refused control must not be able to grow a host allocation.
#[test]
fn the_list_is_capped_and_the_count_is_not() {
    let (mut r, log) = recorder();
    let n = FAULT_BUFFER_SAMPLE_MAX * 4;
    for i in 0..n {
        r.observe(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &registration(4096, &[0x1_0000_0000 + (i as u64) * 0x1000]),
        ));
    }
    assert_eq!(log.sample().len(), FAULT_BUFFER_SAMPLE_MAX);
    assert_eq!(log.total(), n as u64, "the count saw every one of them");
}
