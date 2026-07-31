//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` (0x20800a36): the identity this port serves,
//! pinned against an RTX 3060's own answer.
//!
//! ## ★★ Where the oracle bytes come from
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:3355-3364` declares `ctl_20800a36[]`, 88 bytes,
//! registered at `:6248` as `{0x20800a36u, 0x0u, 88u, 88u, ctl_20800a36}`. Those 88 bytes
//! appear **verbatim, exactly once** in the 360 725 records of
//! `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` — record **141991**, a 4096-byte
//! `GUEST_WR` to `0x1_2764_6000`, the blob at payload offset **120**, under an RPC envelope
//! reading `header_version=0x03000000 signature="VRPC" length=160 function=76
//! rpc_result=NV_OK sequence=3` and a control header carrying `hClient=0xc2000006
//! hObject=0xabcd2080 cmd=0x20800a36 status=NV_OK paramsSize=88 rmapiRpcFlags=0`.
//! `160 = 32 + 40 + 88` — envelope, `RpcControlReq::HEADER`, params.
//!
//! A `GUEST_WR` is the device writing into the guest's status queue, so this is the C's
//! *reply*, not the guest's request.
//!
//! ## ★★ What this file settles, and what it cannot
//!
//! It settles the **layout**: the three bytes of padding after `chipSubRev`, the three
//! after `isCmpSku`, `regBases[]` starting at 24, and — the one nobody could have read out
//! of the header — that `pciDeviceId` packs the **device** id in its high half and the
//! **vendor** id in its low.
//!
//! ⊘ It settles **nothing about which register groups to name**. The oracle's board named
//! four; this device names one, because it is the only one whose window the register plane
//! answers. [`oracle_with_only_the_group_we_serve`] withdraws the other three at literal
//! offsets, and the withdrawal is a visible, separate, *asserted* step.

use kayfabe_abi::chipinfo::{
    self, CHIP_INFO_PARAMS_SIZE, ChipIdentity, ChipInfoError, ChipInfoRow,
    NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO, REG_BASE_UNSUPPORTED, RegBaseRow, reg_base,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, ga10x};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `ctl_20800a36[]` — the 88 bytes an RTX 3060's GSP answered, as they sit in the capture.
const ORACLE_CHIP_INFO_PARAMS: &str = concat!(
    "000000000000000000000000de100425",
    "62147d39a1000000ffffffff00004000",
    "00900000000000000000bb00ffffffff",
    "ffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffff",
    "ffffffffffffffff",
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// The oracle's reply with the three register groups this device does not name withdrawn.
///
/// ★★★ The offsets are spelled as LITERALS — 28, 32, 36 — and not as
/// `REG_BASES_OFF + reg_base::GR * 4`. Deriving them from the constants under test is the
/// exact defect that made #125's and #126's own tests pass while measuring nothing: a wrong
/// `REG_BASES_OFF` would move both the encoder and the check, in step.
///
/// ★★ Each withdrawal asserts the oracle really carried a *servable* base there first, so
/// this function cannot quietly become a no-op. Note `NV_REG_BASE_MASTER` at 36: the oracle
/// sent **zero**, which is a perfectly legal offset (`NV_PMC` starts the aperture) — which
/// is precisely why a defaulted zero in that slot would have been unreadable as an
/// omission, and why the sentinel is the only honest way to decline one.
fn oracle_with_only_the_group_we_serve() -> Vec<u8> {
    let mut b = unhex(ORACLE_CHIP_INFO_PARAMS);
    assert_eq!(b.len(), 88, "the C's own array length");
    for (at, what) in [
        (28usize, "NV_REG_BASE_GR"),
        (32, "NV_REG_BASE_TIMER"),
        (36, "NV_REG_BASE_MASTER"),
    ] {
        assert_ne!(
            &b[at..at + 4],
            &[0xFFu8; 4],
            "the oracle really did name {what} at {at}"
        );
        b[at..at + 4].fill(0xFF);
    }
    b
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn identity() -> ChipIdentity {
    let id = kayfabe_device::identity_for(chip()).expect("the GA106 row resolves");
    ChipIdentity {
        pci_vendor_id: id.vendor_id,
        pci_device_id: id.device_id,
        pci_subsystem_vendor_id: id.subsystem_vendor_id,
        pci_subsystem_id: id.subsystem_id,
        pci_revision: id.revision,
    }
}

fn encode() -> Vec<u8> {
    chipinfo::encode_chip_info(&chip().chip_info, &identity(), chip().regs_aperture_len)
        .expect("the GA106 row encodes")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is filled with **0xAA, not zeros**, for the same reason
/// `tests/pci_bar_info.rs` does it: a reply that reflected any of it would put 0xAA on the
/// wire, and here that would land in `pGpu->idInfo.PCIDeviceID`.
fn chip_info_command(cmd: u32, params_size: u32, params_at: usize) -> RpcCommand {
    let mut payload = vec![0xAAu8; params_at + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes()); // hClient, as captured
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject, as captured
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..28].copy_from_slice(&0u32.to_le_bytes());
    payload[28..32].copy_from_slice(&0u32.to_le_bytes());
    payload[32..36].copy_from_slice(&0u32.to_le_bytes());
    payload[36..40].copy_from_slice(&0u32.to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 3,
        payload,
        elements: 1,
    }
}

/// `RpcControlReq::HEADER`, as the capture's own arithmetic gives it: the reply's declared
/// `length` was 160 and `160 - 32 - 88 = 40`.
const PARAMS_AT: usize = 40;

#[test]
fn the_encoder_reproduces_the_oracles_layout_field_by_field() {
    let got = encode();

    // ★★★ LITERALS, for the reason `oracle_with_only_the_group_we_serve` states.
    assert_eq!(got.len(), 88, "sizeof the params struct");
    assert_eq!(got[0], 0, "chipSubRev");
    assert_eq!(
        &got[1..4],
        &[0u8; 3],
        "the alignment hole after chipSubRev, which no field name mentions"
    );
    assert_eq!(&got[4..8], &0u32.to_le_bytes()[..], "emulationRev1");
    assert_eq!(got[8], 0, "isCmpSku");
    assert_eq!(&got[9..12], &[0u8; 3], "the alignment hole after isCmpSku");
    // ★★ Device id HIGH, vendor id LOW — the packing the capture is the third witness for
    // (`ogkm-580: gpu.c:6297-6299`).
    assert_eq!(
        &got[12..16],
        &0x2504_10deu32.to_le_bytes()[..],
        "pciDeviceId = (0x2504 << 16) | 0x10de"
    );
    assert_eq!(
        &got[16..20],
        &0x397d_1462u32.to_le_bytes()[..],
        "pciSubDeviceId = (0x397d << 16) | 0x1462"
    );
    assert_eq!(&got[20..24], &0xA1u32.to_le_bytes()[..], "pciRevisionId");

    // And the whole thing, against silicon, with the three withdrawals visible above.
    assert_eq!(
        got,
        oracle_with_only_the_group_we_serve(),
        "vs cap1b record 141991"
    );
}

#[test]
fn exactly_one_register_group_is_named_and_the_other_fifteen_are_rms_own_refusal() {
    let got = encode();

    // Literal byte offsets for `regBases[0..16]`: 24, 28, 32, …
    assert_eq!(
        &got[40..44],
        &0x00BB_0000u32.to_le_bytes()[..],
        "regBases[NV_REG_BASE_USERMODE] — the virtual-function aperture"
    );
    for (i, at) in (24usize..88).step_by(4).enumerate() {
        if at == 40 {
            continue;
        }
        assert_eq!(
            &got[at..at + 4],
            &0xFFFF_FFFFu32.to_le_bytes()[..],
            "regBases[{i}] must be RM's own NV_ERR_NOT_SUPPORTED, never a defaulted zero"
        );
    }
    // The index the served row occupies, stated independently of the byte offset above.
    assert_eq!(reg_base::USERMODE, 4);
    assert_eq!(chip().chip_info.reg_bases.len(), 1);
}

#[test]
fn the_reply_states_the_same_identity_as_configuration_space() {
    // ★★★ THE ANTI-DRIFT BITE. `_gpuInitChipInfo` OVERWRITES `pGpu->idInfo` with this
    // reply (`ogkm-580: gpu.c:891-893`), so if the reply and the PCI header ever disagreed,
    // RM would proceed believing a device it did not enumerate — and nothing logs. The two
    // are compared here through `identity_for`, which is the call the hypervisor shell
    // builds configuration space from.
    let cfg = kayfabe_device::identity_for(chip()).expect("the GA106 row resolves");
    let got = encode();

    let packed_device = u32::from_le_bytes([got[12], got[13], got[14], got[15]]);
    let packed_sub = u32::from_le_bytes([got[16], got[17], got[18], got[19]]);
    let revision = u32::from_le_bytes([got[20], got[21], got[22], got[23]]);

    assert_eq!(packed_device >> 16, u32::from(cfg.device_id));
    assert_eq!(packed_device & 0xFFFF, u32::from(cfg.vendor_id));
    assert_eq!(packed_sub >> 16, u32::from(cfg.subsystem_id));
    assert_eq!(packed_sub & 0xFFFF, u32::from(cfg.subsystem_vendor_id));
    assert_eq!(revision, u32::from(cfg.revision));
}

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let cmd = chip_info_command(NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO, 88, PARAMS_AT);
    let reply = p.respond(&cmd).expect("the policy answers 0x20800a36");

    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    // `status` and `paramsSize` are the two fields a GSP owns on the reply. Literal
    // offsets: 12 and 16 into the control header.
    assert_eq!(&reply.body[12..16], &0u32.to_le_bytes()[..], "status");
    assert_eq!(&reply.body[16..20], &88u32.to_le_bytes()[..], "paramsSize");
    // The guest's own `hClient`/`hObject`/`cmd` come back, as on a real reply.
    assert_eq!(&reply.body[8..12], &0x2080_0a36u32.to_le_bytes()[..], "cmd");

    // ★★★ THE BITE. A surviving 0xAA at offset 12 would become
    // `pGpu->idInfo.PCIDeviceID = 0xAAAAAAAA` — a device id RM would then carry for the
    // rest of its life, past every HAL decision that reads it.
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 88];
    assert!(
        !params.contains(&0xAA),
        "the reply carries bytes from the guest's request"
    );
    assert_eq!(params, &oracle_with_only_the_group_we_serve()[..]);
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    let mut p = policy();
    // One byte short of the struct: a guest whose layout is not our layout.
    let reply = p
        .respond(&chip_info_command(
            NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO,
            87,
            PARAMS_AT,
        ))
        .expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(reply.body.is_empty(), "a refusal carries no body");
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    let mut p = policy();
    let mut cmd = chip_info_command(NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO, 88, PARAMS_AT);
    let flags: u32 = 0x0000_0002; // `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`
    assert!(kayfabe_abi::rpc_params_are_serialized(flags));
    cmd.payload[20..24].copy_from_slice(&flags.to_le_bytes());
    let reply = p.respond(&cmd).expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
}

#[test]
fn the_classifier_names_this_control_and_its_size() {
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a36),
        Some(WantedTable::ChipInfo)
    );
    // 88, as a literal — the C's array length and the capture's own `paramsSize`.
    assert_eq!(WantedTable::ChipInfo.params_size(), 88);
    assert_eq!(CHIP_INFO_PARAMS_SIZE, 88);
    assert_eq!(NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO, 0x2080_0a36);
}

// ── The encoder's four refusals, each induced ──────────────────────────────────────

fn row(bases: &'static [RegBaseRow]) -> ChipInfoRow {
    ChipInfoRow {
        chip_sub_rev: 0,
        is_cmp_sku: false,
        reg_bases: bases,
    }
}

#[test]
fn a_register_base_past_the_array_is_refused() {
    static BAD: &[RegBaseRow] = &[RegBaseRow {
        index: 16,
        offset: 0x1000,
        name: "past the end",
    }];
    assert_eq!(
        chipinfo::encode_chip_info(&row(BAD), &identity(), 16 << 20),
        Err(ChipInfoError::RegBaseIndexOutOfRange {
            name: "past the end",
            index: 16,
            max: 16,
        })
    );
}

#[test]
fn two_rows_claiming_one_index_are_refused_rather_than_resolved_by_order() {
    static BAD: &[RegBaseRow] = &[
        RegBaseRow {
            index: reg_base::TIMER,
            offset: 0x9000,
            name: "first",
        },
        RegBaseRow {
            index: reg_base::TIMER,
            offset: 0x9400,
            name: "second",
        },
    ];
    assert_eq!(
        chipinfo::encode_chip_info(&row(BAD), &identity(), 16 << 20),
        Err(ChipInfoError::DuplicateRegBase {
            index: 2,
            first: "first",
            second: "second",
        })
    );
}

#[test]
fn a_row_whose_offset_is_the_sentinel_is_refused() {
    static BAD: &[RegBaseRow] = &[RegBaseRow {
        index: reg_base::MASTER,
        offset: REG_BASE_UNSUPPORTED,
        name: "advertised and denied at once",
    }];
    assert_eq!(
        chipinfo::encode_chip_info(&row(BAD), &identity(), 16 << 20),
        Err(ChipInfoError::RegBaseIsSentinel {
            name: "advertised and denied at once",
        })
    );
}

#[test]
fn a_register_base_outside_the_aperture_is_refused() {
    // ★★ The bite for [`RegBaseRow`]'s promise: the guest maps `BAR0 + offset`. This one
    // is one byte past the 16 MiB aperture the GA106 row declares.
    static BAD: &[RegBaseRow] = &[RegBaseRow {
        index: reg_base::USERMODE,
        offset: 16 << 20,
        name: "past the aperture",
    }];
    assert_eq!(
        chipinfo::encode_chip_info(&row(BAD), &identity(), 16 << 20),
        Err(ChipInfoError::RegBaseOutsideAperture {
            name: "past the aperture",
            offset: 16 << 20,
            aperture: 16 << 20,
        })
    );
    // And the one this device really serves is inside it, on the same yardstick.
    assert!(u64::from(0x00BB_0000u32) < chip().regs_aperture_len);
}

#[test]
fn the_served_register_base_is_a_window_this_devices_own_counter_lives_in() {
    // ★★ Not a restatement of the row: the *promise* is that BAR0 + 0x00BB_0000 is a
    // range this device answers, and the free-running nanosecond counter — the one source
    // in that window anything claims — is where that promise is cashed. The `ga10x` row
    // asserts the same relation at compile time; this states it where a reader looking at
    // the reply will find it.
    assert_eq!(ga10x::GA106.ptimer.lo_off, 0x00BB_0080);
    assert_eq!(ga10x::GA106.ptimer.hi_off, 0x00BB_0084);
    let base = 0x00BB_0000u64;
    let len = 0x1_0000u64; // DRF_SIZE(NVC361), `ogkm-580: clc361.h:29`
    assert!(ga10x::GA106.ptimer.lo_off >= base && ga10x::GA106.ptimer.hi_off < base + len);
}
