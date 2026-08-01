//! The four controls the first **batched** rung serves — `0x20800af3` (ConfidentialCompute),
//! `0x20800aac` (KernelBif), `0x20800a61` (KernelFifo's channel count) and `0x20800a59`
//! (KernelGmmu's fault-buffer sizes).
//!
//! ## ★★★ Why one file and not four
//!
//! Because they are one *decision*. `docs/design/preinit_sweep_loop.md` §4.3 is explicit
//! that past `gpuPreInit` the unit of progress stopped being a control: the sweep walks the
//! whole engine list in one boot, so the pre-flight either answers everything it will reach
//! or the boot confirms nothing. These four are what the pre-flight of `cap1b`'s measured
//! prefix said to serve, and they were built and are tested together.
//!
//! ## ★★ The evidence each one rests on, and it is not the same evidence
//!
//! | control | oracle bytes | what refusing does |
//! |---|---|---|
//! | `0x20800af3` | empty (all-zero), `psize` 2 | nothing — the guest state is identical |
//! | `0x20800aac` | empty (all-zero), `psize` 4 | nothing — the guest state is identical |
//! | `0x20800a61` | `00000000 00080000`, `psize` 8 | **halts** `gpuStateInit` at a named statement |
//! | `0x20800a59` | 16 bytes, nothing trimmed | leaves a **freed** `pStaticInfo` in `KernelGmmu` |
//!
//! ⊘ **The first two rows are stated plainly rather than dressed up.** Serving them changes
//! no guest state at all: `ccStaticInfo` is a zeroed NVOC member and `kbifStaticInfoInit`'s
//! params are `portMemSet` to zero, so a refusal and a served all-zero reply are the same
//! bytes. What changes is the RPC **envelope** — `NV_OK` where we used to send
//! `NV_ERR_NOT_SUPPORTED` — which is what a real GA106's GSP sends, and which stops RM
//! printing a `LEVEL_ERROR` the next boot's `dmesg` would have to be read past. That is a
//! diagnostic argument, and it is the honest one.
//!
//! `[measured]`, and every oracle row's run is the **C artifact's rather than this port's**:
//! `C: src/qemu/mode2_initctrl_ga106.h`, `nvidia-gpu-passthrough` rev `018e492`.

use kayfabe_abi::bifstatic::{
    self, BIF_STATIC_INFO_PARAMS_SIZE, BifStaticError, C2C_LINK_UP_OFF, DEVICE_MULTI_FUNCTION_OFF,
    GCX_PMU_CFG_SPACE_RESTORE_OFF, NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
    PCIE_GEN4_CAPABLE_OFF,
};
use kayfabe_abi::confcompute::{
    self, BAR1_TRUSTED_OFF, CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE, ConfComputeError,
    NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO, PCIE_TRUSTED_OFF,
};
use kayfabe_abi::fifochannels::{
    self, FIFO_NUM_CHANNELS_PARAMS_SIZE, FifoChannelsError, NUM_CHANNELS_OFF,
    NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS, RUNLIST_ID_OFF,
};
use kayfabe_abi::gmmustatic::{
    self, FAULT_PACKET_SIZE, FaultBufferKind, GMMU_STATIC_INFO_PARAMS_SIZE, GmmuStaticError,
    NON_REPLAYABLE_SIZE_OFF, NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO, REPLAYABLE_SIZE_OFF,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::ga10x;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 42 - psize 2 = 40`.
const PARAMS_AT: usize = 40;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is `0xAA`, and here that is load-bearing for three of the four:
/// their callers hand RM a **zeroed** destination, so on the bench a reply that merely
/// reflected the request would be indistinguishable from one that answered zeros. Only a
/// poisoned request can tell an answer from an echo.
///
/// ⊘ For `0x20800a61` the poison is overwritten deliberately by
/// [`fifo_command`], because that control's `runlistId` is an `[IN]` field.
fn control(cmd: u32, params_size: u32, serialized: bool) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes());
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    let flags: u32 = if serialized { 1 << 1 } else { 0 };
    payload[20..24].copy_from_slice(&flags.to_le_bytes());
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 13,
        payload,
        elements: 1,
    }
}

/// `0x20800a61` with the guest's `[IN]` `runlistId` actually set.
fn fifo_command(runlist_id: u32) -> RpcCommand {
    let mut c = control(
        NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
        FIFO_NUM_CHANNELS_PARAMS_SIZE as u32,
        false,
    );
    let at = PARAMS_AT + RUNLIST_ID_OFF;
    c.payload[at..at + 4].copy_from_slice(&runlist_id.to_le_bytes());
    c
}

fn params_of(reply: &kayfabe_gsp::Reply, len: usize) -> Vec<u8> {
    assert_eq!(reply.rpc_result, 0, "served, not refused");
    reply.body[PARAMS_AT..PARAMS_AT + len].to_vec()
}

// ══════════════════ 0x20800af3 — ConfidentialCompute ══════════════════

#[test]
fn the_conf_compute_reply_is_the_oracles_two_bytes() {
    // ★★★ The provenance test, and its shape is unusual because the oracle's answer is two
    // zero bytes: `{0x20800af3u, 0x0u, 2u, 0u, ctl_20800af3}` with an EMPTY `ctl_20800af3[]`.
    // A capture that trims trailing zeros and kept nothing means the reply was all zero.
    //
    // ⊘ So this test can only be meaningful if the request is poisoned — which it is, with
    // 0xAA — because a policy that echoed the request would produce 0xAA 0xAA here.
    let ours =
        confcompute::encode_conf_compute_static_info(&ga10x::GA106_CONF_COMPUTE).expect("encodes");
    assert_eq!(ours, vec![0u8, 0u8]);
    assert_eq!(ours.len(), CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE);

    let mut p = policy();
    let reply = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
            CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE as u32,
            false,
        ))
        .expect("the policy serves this control");
    assert_eq!(
        params_of(&reply, CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE),
        vec![0u8, 0u8],
        "the poisoned request bytes were replaced, not reflected"
    );
    assert_eq!(reply.body[PARAMS_AT + BAR1_TRUSTED_OFF], 0);
    assert_eq!(reply.body[PARAMS_AT + PCIE_TRUSTED_OFF], 0);
}

#[test]
fn a_trust_claim_this_port_cannot_back_is_unencodable() {
    // ★★★ The one direction this encoder can forbid. `mapping_cpu.c:227-235` refuses to map
    // CPR vidmem through BAR1 while BOTH bits are clear and stops refusing the moment
    // either is set — so a `true` deletes a guest-side check and backs it with nothing.
    for (bar1, pcie) in [(true, false), (false, true), (true, true)] {
        let row = kayfabe_abi::confcompute::ConfComputeRow {
            bar1_trusted: bar1,
            pcie_trusted: pcie,
        };
        assert_eq!(
            confcompute::encode_conf_compute_static_info(&row),
            Err(ConfComputeError::TrustedWithoutCprPlane {
                bar1_trusted: bar1,
                pcie_trusted: pcie,
            })
        );
    }
    // Non-vacuity: the combination this device actually claims does encode.
    assert!(confcompute::encode_conf_compute_static_info(&ga10x::GA106_CONF_COMPUTE).is_ok());
}

// ══════════════════ 0x20800aac — KernelBif ══════════════════

#[test]
fn the_bif_reply_is_the_oracles_four_bytes() {
    let ours = bifstatic::encode_bif_static_info(&ga10x::GA106_BIF_STATIC).expect("encodes");
    assert_eq!(ours, vec![0u8; BIF_STATIC_INFO_PARAMS_SIZE]);

    let mut p = policy();
    let reply = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
            BIF_STATIC_INFO_PARAMS_SIZE as u32,
            false,
        ))
        .expect("the policy serves this control");
    assert_eq!(
        params_of(&reply, BIF_STATIC_INFO_PARAMS_SIZE),
        vec![0u8; BIF_STATIC_INFO_PARAMS_SIZE],
        "the poisoned request bytes were replaced, not reflected"
    );
}

#[test]
fn the_two_bif_bits_that_point_rm_at_absent_hardware_are_unencodable() {
    // ★★ Each variant stands in front of a *specific* guest path, and they are checked
    // separately so that removing one check is one red test rather than none.
    let base = ga10x::GA106_BIF_STATIC;
    assert_eq!(
        bifstatic::encode_bif_static_info(&kayfabe_abi::bifstatic::BifStaticRow {
            c2c_link_up: true,
            ..base
        }),
        Err(BifStaticError::C2cLinkUpWithoutC2cPlane)
    );
    assert_eq!(
        bifstatic::encode_bif_static_info(&kayfabe_abi::bifstatic::BifStaticRow {
            device_multi_function: true,
            ..base
        }),
        Err(BifStaticError::MultiFunctionWithoutSecondFunction)
    );
    // ⊘ And the two fields that are descriptions rather than directions DO encode — a
    // blanket "anything true is refused" would pass the two assertions above for the wrong
    // reason.
    let gen4 = bifstatic::encode_bif_static_info(&kayfabe_abi::bifstatic::BifStaticRow {
        pcie_gen4_capable: true,
        gcx_pmu_cfg_space_restore: true,
        ..base
    })
    .expect("a description this port has no reason to forbid");
    assert_eq!(gen4[PCIE_GEN4_CAPABLE_OFF], 1);
    assert_eq!(gen4[C2C_LINK_UP_OFF], 0);
    assert_eq!(gen4[DEVICE_MULTI_FUNCTION_OFF], 0);
    assert_eq!(gen4[GCX_PMU_CFG_SPACE_RESTORE_OFF], 1);
}

// ══════════════════ 0x20800a61 — KernelFifo's channel count ══════════════════

#[test]
fn the_channel_count_is_the_oracles_and_the_runlist_id_is_the_guests() {
    // ★★★ The one served reply in this file that is a function of the REQUEST as well as
    // the chip row. `runlistId` is `[IN]`; overwriting it with a number of our own would be
    // answering a question the guest did not ask.
    for runlist in [0u32, 1, 2, 7] {
        let mut p = policy();
        let reply = p
            .respond(&fifo_command(runlist))
            .expect("the policy serves this control");
        let params = params_of(&reply, FIFO_NUM_CHANNELS_PARAMS_SIZE);
        assert_eq!(
            u32::from_le_bytes(
                params[RUNLIST_ID_OFF..RUNLIST_ID_OFF + 4]
                    .try_into()
                    .unwrap()
            ),
            runlist,
            "the guest's own [IN] runlistId, echoed"
        );
        assert_eq!(
            u32::from_le_bytes(
                params[NUM_CHANNELS_OFF..NUM_CHANNELS_OFF + 4]
                    .try_into()
                    .unwrap()
            ),
            0x0800,
            "2048, which is what a real GA106's GSP answered"
        );
    }
}

#[test]
fn a_zero_channel_count_is_unencodable() {
    // ★★ `kfifoChidMgrConstruct` reads a zero as `NV_ERR_INVALID_STATE`
    // (`ogkm-580: kernel_fifo.c:300-308`). Encoding it would wrap the content of a refusal
    // in an envelope that says the answer is good — strictly worse than refusing.
    assert_eq!(
        fifochannels::encode_fifo_num_channels(
            &kayfabe_abi::fifochannels::FifoChannelsRow {
                channels_per_runlist: 0
            },
            0
        ),
        Err(FifoChannelsError::NoChannels)
    );
    assert!(fifochannels::encode_fifo_num_channels(&ga10x::GA106_FIFO_CHANNELS, 0).is_ok());
}

// ══════════════════ 0x20800a59 — KernelGmmu's fault-buffer sizes ══════════════════

/// The sixteen bytes an RTX 3060's GSP answered (`C: mode2_initctrl_ga106.h:5418-5419`).
const GMMU_ORACLE: [u8; 16] = [
    0x00, 0x10, 0x03, 0x00, // replayableFaultBufferSize = 0x0003_1000
    0x00, 0x00, 0x00, 0x00, // replayableShadowFaultBufferMetadataSize = 0
    0x20, 0x0c, 0x12, 0x00, // nonReplayableFaultBufferSize = 0x0012_0c20
    0x00, 0x00, 0x00, 0x00, // nonReplayableShadowFaultBufferMetadataSize = 0
];

#[test]
fn the_gmmu_static_reply_is_the_oracles_sixteen_bytes() {
    let ours = gmmustatic::encode_gmmu_static_info(&ga10x::GA106_GMMU_STATIC).expect("encodes");
    assert_eq!(ours, GMMU_ORACLE.to_vec(), "byte for byte");
    assert_eq!(ours.len(), GMMU_STATIC_INFO_PARAMS_SIZE);

    let mut p = policy();
    let reply = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
            GMMU_STATIC_INFO_PARAMS_SIZE as u32,
            false,
        ))
        .expect("the policy serves this control");
    assert_eq!(
        params_of(&reply, GMMU_STATIC_INFO_PARAMS_SIZE),
        GMMU_ORACLE.to_vec()
    );
}

#[test]
fn the_captured_sizes_are_whole_fault_packets_which_is_what_pins_the_field_order() {
    // ★★ An independent check on the layout rather than a restatement of it: RM divides
    // both sizes by `NVC369_BUF_SIZE` to get a queue capacity
    // (`ogkm-580: kern_gmmu.c:1725`), so a transposed field order would put a zero where a
    // size belongs and this property would be vacuously true instead of confirmatory.
    let replayable = u32::from_le_bytes(
        GMMU_ORACLE[REPLAYABLE_SIZE_OFF..REPLAYABLE_SIZE_OFF + 4]
            .try_into()
            .unwrap(),
    );
    let non_replayable = u32::from_le_bytes(
        GMMU_ORACLE[NON_REPLAYABLE_SIZE_OFF..NON_REPLAYABLE_SIZE_OFF + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(replayable, 0x0003_1000);
    assert_eq!(non_replayable, 0x0012_0c20);
    assert_eq!(replayable % FAULT_PACKET_SIZE, 0);
    assert_eq!(non_replayable % FAULT_PACKET_SIZE, 0);
    assert_eq!(replayable / FAULT_PACKET_SIZE, 6272);
    assert_eq!(non_replayable / FAULT_PACKET_SIZE, 36961);
    // ⊘ Non-vacuity for "confirmatory": neither is zero, so the modulo is a real check.
    assert!(replayable > 0 && non_replayable > 0);
}

#[test]
fn a_fault_buffer_geometry_rm_would_reject_is_unencodable() {
    // ★★★ Three variants, each in front of a different consequence. The first is an
    // invariant RM asserts **on itself**, which is the same shape as
    // `ComptagAllocationPolicy` having no *neither* variant.
    let base = ga10x::GA106_GMMU_STATIC;
    assert_eq!(
        gmmustatic::encode_gmmu_static_info(&kayfabe_abi::gmmustatic::GmmuStaticRow {
            non_replayable_size: 0,
            ..base
        }),
        Err(GmmuStaticError::NonReplayableSizeZero)
    );
    assert_eq!(
        gmmustatic::encode_gmmu_static_info(&kayfabe_abi::gmmustatic::GmmuStaticRow {
            replayable_size: 0,
            ..base
        }),
        Err(GmmuStaticError::ReplayableSizeZero)
    );
    assert_eq!(
        gmmustatic::encode_gmmu_static_info(&kayfabe_abi::gmmustatic::GmmuStaticRow {
            replayable_size: 0x0003_1001,
            ..base
        }),
        Err(GmmuStaticError::SizeNotPacketAligned {
            which: FaultBufferKind::Replayable,
            size: 0x0003_1001,
        })
    );
    assert_eq!(
        gmmustatic::encode_gmmu_static_info(&kayfabe_abi::gmmustatic::GmmuStaticRow {
            non_replayable_size: 0x0012_0c21,
            ..base
        }),
        Err(GmmuStaticError::SizeNotPacketAligned {
            which: FaultBufferKind::NonReplayable,
            size: 0x0012_0c21,
        })
    );
}

// ══════════════════ the policy's own refusals, for all four ══════════════════

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    // ★ A guest whose struct is not the struct we encode gets a loud envelope refusal
    // rather than a table read at the wrong strides. Quantified over all four so a new
    // control cannot join the file without inheriting the check.
    for (cmd, right) in [
        (
            NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
            CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
            BIF_STATIC_INFO_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
            FIFO_NUM_CHANNELS_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
            GMMU_STATIC_INFO_PARAMS_SIZE,
        ),
    ] {
        for size in [0usize, 1, 2, 4, 8, 16, 40] {
            if size == right {
                continue;
            }
            let mut p = policy();
            let reply = p
                .respond(&control(cmd, size as u32, false))
                .expect("answers");
            assert_ne!(
                reply.rpc_result, 0,
                "{cmd:#x}: a guest declaring {size} bytes is not a guest whose struct we \
                 encode"
            );
            assert!(reply.body.is_empty(), "and no body to misread");
        }
    }
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    for (cmd, size) in [
        (
            NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
            CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
            BIF_STATIC_INFO_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
            FIFO_NUM_CHANNELS_PARAMS_SIZE,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
            GMMU_STATIC_INFO_PARAMS_SIZE,
        ),
    ] {
        let mut p = policy();
        let reply = p
            .respond(&control(cmd, size as u32, true))
            .expect("answers");
        assert_ne!(reply.rpc_result, 0, "{cmd:#x}: FINN-serialized is not flat");
    }
}

#[test]
fn all_four_are_in_the_served_universe_and_classify_to_themselves() {
    // ★★ The round trip through `WantedTable::ALL`, which is the single list that decides
    // both "served" and "covered by the gates" since a96f867.
    for (cmd, want) in [
        (
            NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
            WantedTable::ConfComputeStaticInfo,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
            WantedTable::BifStaticInfo,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
            WantedTable::FifoNumChannels,
        ),
        (
            NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
            WantedTable::GmmuStaticInfo,
        ),
    ] {
        assert_eq!(WantedTable::from_cmd(cmd), Some(want));
        assert_eq!(want.cmd_id(), cmd);
        assert!(WantedTable::ALL.contains(&want));
    }
}
