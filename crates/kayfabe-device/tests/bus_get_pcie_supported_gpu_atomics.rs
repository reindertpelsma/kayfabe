//! `NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` (`0x2080182a`) — the control
//! `cuInit` stopped at in boot `gt1430_0dbbabc` (`execution_plane_increments.md` §14.30),
//! and ★★★ **the one whose refusal on a real part was the INSTRUMENT'S OWN SEED**.
//!
//! ## ⊘ What these tests are NOT
//!
//! They are not proof that `cuInit` succeeds. Only a boot is (`only_live_boots_are_proof`).
//! What they pin is the shape that makes the value usable at all, and on this control there
//! are **three** ways to be wrong that no assertion on the value could see:
//!
//! 1. **The correct answer is zeros.** Thirteen `bSupported = FALSE, attributes = 0` is what
//!    a real GA106 writes and also what an unwritten buffer looks like — `fmbsize`'s
//!    polarity inverted, exactly as in `internal_gpu_get_smc_mode.rs`. Only a poison fill
//!    separates them.
//! 2. ★★ **`capType` is an `[IN]` field.** A port that ignored it would answer `NV_OK` to
//!    `_CAPTYPE_GPU` and `_CAPTYPE_P2P`, which a real GA106 **refuses** with `0x56`
//!    (`[measured 2026-08-08, R23]`). That is a *stronger* claim than hardware makes and no
//!    happy-path test on the SYSMEM reply can see it — so the refusals are quantified over
//!    a list here (`gates_quantified_over_a_list`, `mutate_the_refusals_not_the_mechanism`).
//! 3. ★★ **Three of every entry's eight bytes are padding RM does not own.** `[measured]` a
//!    real part leaves them exactly as they arrived — R23's `0xCD`-seeded arm reads them
//!    back as `0xCD` while `bSupported` and `attributes` are written. A port writing an
//!    8-byte zeroed entry per op produces a byte-identical reply to libcuda's zeroed request
//!    and a **different** one to any other caller's. The padding assertions are the only
//!    thing that can catch it.

use kayfabe_abi::gpuatomics::{
    self, CAPTYPE_GPU, CAPTYPE_P2P, CAPTYPE_SYSMEM, GpuAtomicOp,
    NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS, PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT,
    PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying the guest's `0x2080182a`, over a chosen params fill and a
/// chosen `capType`.
///
/// ★★ The fill is the instrument. libcuda really does send this struct **zeroed** — which
/// is precisely why the committed real-hardware trace could not settle what the reply says
/// (`traces/real_ga106/README.md`: *"libcuda hands RM zeroed buffers, so an all-zero pair is
/// ambiguous"*). Every test below that asserts on a *value* sends a poisoned struct instead,
/// and the one test that replays libcuda's request byte for byte says in its own name that
/// it is a compatibility check and not a value measurement.
fn atomics_command(params_size: u32, fill: u8, cap_type: Option<u32>) -> RpcCommand {
    let mut payload = vec![fill; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12]
        .copy_from_slice(&NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    // ⚠ Guarded, because `params_size` is a variable here: the wrong-size test asks for a
    // 0-byte params block and an unguarded write would panic in the HARNESS and be read as
    // the port refusing to serve it (`suspect_the_instrument_first` — this exact panic
    // happened once). A request too short to hold `capType` simply does not carry one.
    if let Some(cap) = cap_type
        && payload.len() >= PARAMS_AT + 4
    {
        payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&cap.to_le_bytes());
    }
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x2a,
        sequence: 31,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS),
        Some(WantedTable::BusGetPcieSupportedGpuAtomics),
    );
    assert_eq!(
        WantedTable::BusGetPcieSupportedGpuAtomics.cmd_id(),
        0x2080_182a,
        "the id in the `unserviced fn 76` line of boot gt1430_0dbbabc"
    );
    assert_eq!(
        WantedTable::BusGetPcieSupportedGpuAtomics.params_size(),
        PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE,
    );
    assert_eq!(
        PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE, 112,
        "size on the wire"
    );
}

#[test]
fn sysmem_is_served_with_the_body_a_real_ga106_wrote_into_a_poisoned_buffer() {
    // ★★★ `[measured 2026-08-08, real GA106 `GPU-d0913685`, `rmladder --atomics-probe`
    // (R23), `traces/real_ga106/rmladder_r23_atomics_real_ga106.txt`]`: `capType = 0` into a
    // `0xCD`-seeded buffer comes back `NV_OK` with all thirteen ops
    // `bSupported=0x00 attributes=0x00000000` and every entry's three padding bytes still
    // `0xCD`. This test is that measurement, with `0xAA` for the seed.
    let cmd = atomics_command(
        PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE as u32,
        0xAA,
        Some(CAPTYPE_SYSMEM),
    );
    let reply = policy()
        .respond(&cmd)
        .expect("this port serves 0x2080182a since the §14.31 rung");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(status, 0, "and so does the inner control status");

    let params = &reply.body[PARAMS_AT..PARAMS_AT + PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
    assert_eq!(
        &params[0..4],
        &CAPTYPE_SYSMEM.to_le_bytes(),
        "capType echoed"
    );
    // ⊘ `dbdf` is `[IN]` and RM does not touch it — `[measured]` R23's poisoned-`dbdf` arm
    // read `0xCDCDCDCD` back out. The poison surviving here IS the assertion.
    assert_eq!(&params[4..8], &[0xAA; 4], "dbdf echoed untouched");

    for i in 0..PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT {
        let at = 8 + 8 * i;
        // ⊘⊘ THE ASSERTION THAT CARRIES THE FILE, twice over. Zero is the right answer AND
        // what an unwritten buffer holds, so only the disappearance of the poison from
        // exactly these bytes says the arm ran.
        assert_eq!(params[at], 0x00, "op {i} bSupported was not written");
        assert_eq!(
            &params[at + 4..at + 8],
            &[0, 0, 0, 0],
            "op {i} attributes were not written"
        );
        // ★★ And the poison SURVIVING in the padding is the second half: it says the arm
        // wrote five bytes per entry and not eight, which is what a real part does.
        assert_eq!(
            &params[at + 1..at + 4],
            &[0xAA, 0xAA, 0xAA],
            "op {i} padding is not this port's to write"
        );
    }
    assert_eq!(
        gpuatomics::decode_gpu_atomics(params),
        Ok(GpuAtomicOp::none_supported()),
        "and it decodes to thirteen NAMED denials, not to a bare run of zeros"
    );
}

#[test]
fn libcuda_zeroed_request_reproduces_the_committed_real_hardware_reply() {
    // ⚠ Named for what it is: a **compatibility** check against
    // `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:48` (112 zero bytes in, 112 zero
    // bytes out, `NV_OK`) — and NOT a measurement, because an all-zero reply to an all-zero
    // request cannot distinguish a served answer from an unwritten buffer. The test above
    // is the measurement; this one proves we match the byte stream libcuda will actually
    // see.
    let cmd = atomics_command(PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE as u32, 0x00, None);
    let reply = policy().respond(&cmd).expect("served");
    assert_eq!(reply.rpc_result, 0);
    let params = &reply.body[PARAMS_AT..PARAMS_AT + PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE];
    assert_eq!(params, &[0u8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE]);
}

#[test]
fn every_captype_but_sysmem_is_refused_because_a_real_ga106_refuses_it() {
    // ★★★ `[measured 2026-08-08, real GA106, R23]`: `_CAPTYPE_GPU(1)`, `_CAPTYPE_P2P(2)`,
    // `3` and `0xCDCDCDCD` were each refused `NV_ERR_NOT_SUPPORTED` on the same bare
    // Subdevice that answered `_CAPTYPE_SYSMEM(0)` `NV_OK` seconds earlier.
    //
    // ⊘ Quantified over a list, and the list deliberately includes the two captypes the
    // HEADER DECLARES. A port that read `capType` only to reject values outside `0..=2`
    // would pass a test over `{3, 0xCDCDCDCD}` alone and still answer `NV_OK` where
    // hardware says `0x56` — a smaller universe is a smaller true statement.
    for cap in [CAPTYPE_GPU, CAPTYPE_P2P, 3, 4, 0xCDCD_CDCD, u32::MAX] {
        let cmd = atomics_command(
            PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE as u32,
            0xAA,
            Some(cap),
        );
        let reply = policy().respond(&cmd);
        match reply {
            None => panic!("capType {cap:#x} must be refused, not left unclassified"),
            Some(r) => {
                assert_ne!(r.rpc_result, 0, "capType {cap:#x} must not be served NV_OK");
                assert!(
                    r.body.is_empty(),
                    "capType {cap:#x} was refused but still carried a {}-byte body",
                    r.body.len()
                );
            }
        }
    }
}

#[test]
fn the_probe_ctrl_seed_is_refused_on_the_captype_that_actually_refused_it() {
    // ★★★ The §14.30 request, verbatim: `0xCD` in all 112 bytes, which is what `rmladder
    // --probe-ctrl 0x2080182a:112` sent and what a real GA106 refused twice. It must be
    // refused HERE for the same reason it was refused THERE — the captype — and not by
    // accident of length. If this ever goes green while
    // `every_captype_but_sysmem_is_refused...` still passes, the two are reading different
    // mechanisms.
    let cmd = atomics_command(PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE as u32, 0xCD, None);
    let reply = policy().respond(&cmd).expect("classified");
    assert_ne!(reply.rpc_result, 0);
    assert!(reply.body.is_empty());
    // And the ABI layer names the cause, so the refusal is attributable rather than merely
    // present.
    assert_eq!(
        gpuatomics::answer_bus_get_pcie_supported_gpu_atomics(
            &[0xCDu8; PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE],
            &GpuAtomicOp::none_supported()
        ),
        Err(gpuatomics::GpuAtomicsError::UnansweredCapType {
            cap_type: 0xCDCD_CDCD
        })
    );
}

#[test]
fn a_wrongly_sized_request_is_refused_rather_than_answered() {
    // RM allocates exactly `sizeof(NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS_PARAMS)`;
    // a request claiming any other size is not this control. ⊘ Serving a short one would
    // write up to 112 bytes into a buffer the guest sized smaller.
    //
    // ⚠ The refusal is signalled in the **RPC envelope** over an empty body, so this reads
    // `rpc_result` and never `body[12..16]` — an inner-status read panics on the empty
    // vector and would be the instrument failing, not the port.
    for bad in [0u32, 4, 8, 104, 111, 113, 420] {
        let reply = policy().respond(&atomics_command(bad, 0x00, Some(CAPTYPE_SYSMEM)));
        match reply {
            None => {}
            Some(r) => {
                assert_ne!(
                    r.rpc_result, 0,
                    "params_size {bad} must not be served as NV_OK"
                );
                assert!(
                    r.body.is_empty(),
                    "params_size {bad} was refused but still carried a {}-byte body",
                    r.body.len()
                );
            }
        }
    }
}

#[test]
fn the_served_answer_is_derived_and_carries_no_chip_row() {
    // ★ Not a tautology: the shape is the claim. `none_supported()` takes **no chip
    // argument**, because whether a GPU atomic completes to coherent sysmem depends on the
    // ROOT COMPLEX being a PCIe AtomicOp completer — `PCIE_GEN_INFO`'s species, not
    // `GPU_GEN`'s. This test fails to compile the day someone gives it one, which is the
    // only durable way to state "no per-chip table here"
    // (`derive_what_you_cannot_query_then_oracle_it`).
    let ops = GpuAtomicOp::none_supported();
    assert_eq!(ops.len(), PCIE_SUPPORTED_GPU_ATOMICS_OP_TYPE_COUNT);
    assert!(
        ops.iter().all(|o| !o.supported && o.attributes == 0),
        "the link this port presents supports no PCIe GPU atomics — `[measured]` on a real \
         GA106 and `[src]` RM's own vGPU-guest arm (ogkm-580: kern_bus_ctrl.c:693-707)"
    );
    // ⊘ And nothing may quietly turn a denial into a claim: a `TRUE` here would tell the
    // guest driver a PCIe capability exists that the emulated link does not have.
    assert!(ops.iter().all(|o| !o.supported));
}
