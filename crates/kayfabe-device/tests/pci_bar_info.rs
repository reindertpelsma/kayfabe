//! `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO` (0x20801803): the BAR table this port serves,
//! pinned against an RTX 3060's own answer — and against the guest-kernel page fault that
//! echoing it produced.
//!
//! ## ★★ Where the oracle bytes come from
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:5396-5409` declares `ctl_20801803[]`, 200 bytes,
//! registered at `:6256` as `{0x20801803u, 0x0u, 200u, 200u, ctl_20801803}`. Those 200
//! bytes appear **verbatim, exactly once** in the whole 14 MB of
//! `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` — record **142070**, a 4096-byte
//! `GUEST_WR` to `0x1_2765_7000`, the blob at payload offset **120**, under an RPC envelope
//! reading `header_version=0x03000000 signature="VRPC" length=272 function=76 sequence=12`
//! with `rpc_result = NV_OK`, and a control header carrying `cmd=0x20801803 status=0
//! paramsSize=200`. `272 = 32 + 40 + 200` — envelope, `RpcControlReq::HEADER`, params.
//!
//! A `GUEST_WR` is the device writing into the guest's status queue, so this is the C's
//! *reply*, not the guest's request. The header is not a transcription of one; it **is**
//! one.
//!
//! ## ★★ What this file settles, and what it cannot
//!
//! It settles the **layout** — `pciBarCount` at 0, the four bytes of alignment padding the
//! `NV_DECLARE_ALIGNED(..., 8)` on `pciBarInfo[]` creates, the 24-byte stride, and which
//! word inside an entry is megabytes and which is bytes. Every one of those is checked
//! against silicon rather than against a header read.
//!
//! ⊘ It settles **nothing about the values**. The oracle's `barOffset` fields are a
//! *host* board's physical addresses and this device is not that board, so
//! [`oracle_without_offsets`] blanks them at literal offsets before any comparison and the
//! blanking is a visible, separate step. The sizes agree because this device really
//! presents those apertures — which is a fact about `GA106_PCI_BARS` and the hypervisor
//! shell's realize-time check, not something the capture could adjudicate.

use kayfabe_abi::pcibars::{
    self, NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO, PCI_BAR_INFO_PARAMS_SIZE, PciBarError, PciBarRow,
    bus_bar,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, ga10x};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `ctl_20801803[]` — the 200 bytes an RTX 3060's GSP answered, as they sit in the capture.
const ORACLE_PCI_BAR_PARAMS: &str = concat!(
    "040000000000000000000000100000000000000100000000000000c000000000",
    "0000000000010000000000100000000000000000100000000000000020000000",
    "0000000200000000000000101000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000",
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// The oracle's reply with its three `barOffset` fields blanked.
///
/// ★★★ The offsets are spelled as LITERALS — 24, 48, 72 — and not as
/// `PCI_BAR_INFO_OFF + i * PCI_BAR_ENTRY_SIZE + 16`. Deriving them from the constants
/// under test is the exact defect that made #125's and #126's own tests pass while
/// measuring nothing: a wrong stride would move both the encoder and the check, in step.
fn oracle_without_offsets() -> Vec<u8> {
    let mut b = unhex(ORACLE_PCI_BAR_PARAMS);
    assert_eq!(b.len(), 200, "the C's own array length");
    for at in [24usize, 48, 72] {
        assert_ne!(
            &b[at..at + 8],
            &[0u8; 8],
            "the oracle really did carry a host physical address at {at}"
        );
        b[at..at + 8].fill(0);
    }
    b
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is filled with **0xAA, not zeros**. That is the whole point of this
/// rung: `kbusInitBarsSize_KERNEL` sends an uninitialised stack struct
/// (`ogkm-580: kern_bus.c:585`), so a policy that reflected any part of the request would
/// put 0xAA on the wire and every assertion below would see it.
fn bar_info_command(cmd: u32, params_size: u32, params_at: usize) -> RpcCommand {
    let mut payload = vec![0xAAu8; params_at + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
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
        sequence: 12,
        payload,
        elements: 1,
    }
}

/// `RpcControlReq::HEADER`, as the capture's own arithmetic gives it: the reply's declared
/// `length` was 272 and `272 - 32 - 200 = 40`.
const PARAMS_AT: usize = 40;

#[test]
fn the_encoder_reproduces_the_oracles_own_bar_table() {
    let got = pcibars::encode_pci_bar_info(chip().pci_bars).expect("the GA106 rows encode");

    // ★★★ LITERALS, for the reason `oracle_without_offsets` states. 200 is the struct's
    // size; 0 is where `pciBarCount` lives; 8, 32, 56, 80 are the four entries.
    assert_eq!(got.len(), 200);
    assert_eq!(&got[0..4], &4u32.to_le_bytes()[..], "pciBarCount");
    assert_eq!(
        &got[4..8],
        &[0u8; 4],
        "the alignment hole before pciBarInfo[]"
    );

    // Entry 0: the register aperture — flags, megabytes, bytes, offset.
    assert_eq!(&got[8..12], &0u32.to_le_bytes()[..], "bar0 flags");
    assert_eq!(&got[12..16], &16u32.to_le_bytes()[..], "bar0 barSize (MB)");
    assert_eq!(
        &got[16..24],
        &0x0100_0000u64.to_le_bytes()[..],
        "bar0 bytes"
    );
    assert_eq!(&got[24..32], &[0u8; 8], "bar0 barOffset — not sourced");
    // Entry 1: the framebuffer window.
    assert_eq!(&got[36..40], &256u32.to_le_bytes()[..], "bar1 barSize (MB)");
    assert_eq!(
        &got[40..48],
        &0x1000_0000u64.to_le_bytes()[..],
        "bar1 bytes"
    );
    // Entry 2: the instance window.
    assert_eq!(&got[60..64], &32u32.to_le_bytes()[..], "bar2 barSize (MB)");
    assert_eq!(
        &got[64..72],
        &0x0200_0000u64.to_le_bytes()[..],
        "bar2 bytes"
    );
    // Entry 3: the I/O BAR, which this device does not have. Present and zero, which is
    // RM's own encoding for an absent BAR — not omitted, because `pciBarCount` is 4.
    assert_eq!(&got[80..104], &[0u8; 24], "bar3 — absent, and stated so");
    // Rows 4..8 are the tail RM clears before sending.
    assert!(got[104..200].iter().all(|b| *b == 0), "pciBarInfo[4..8]");

    // And the whole thing, against silicon.
    assert_eq!(got, oracle_without_offsets(), "vs cap1b record 142070");
}

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let cmd = bar_info_command(NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO, 200, PARAMS_AT);
    let reply = p.respond(&cmd).expect("the policy answers 0x20801803");

    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    // `status` and `paramsSize` are the two fields a GSP owns on the reply. Literal
    // offsets: 12 and 16 into the control header.
    assert_eq!(&reply.body[12..16], &0u32.to_le_bytes()[..], "status");
    assert_eq!(&reply.body[16..20], &200u32.to_le_bytes()[..], "paramsSize");

    // ★★★ THE BITE. The request's params were 0xAA. If any of them survived into the
    // reply, RM would loop to whatever `pciBarCount` that produced — which is run
    // `t126b`'s guest-kernel page fault (a stock 580.159.04 guest at `f2acb89`, twice on
    // two fresh boots; `kayfabe_abi::pcibars` carries the dmesg).
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 200];
    assert!(
        !params.contains(&0xAA),
        "the reply carries bytes from the guest's uninitialised stack"
    );
    assert_eq!(params, &oracle_without_offsets()[..]);
    // And `pciBarCount` is 4, spelled out, not 0xAAAAAAAA.
    assert_eq!(&params[0..4], &4u32.to_le_bytes()[..]);
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    let mut p = policy();
    // One byte short of the struct: a guest whose layout is not our layout.
    let reply = p
        .respond(&bar_info_command(
            NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO,
            199,
            PARAMS_AT,
        ))
        .expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(reply.body.is_empty(), "a refusal carries no body");
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    let mut p = policy();
    let mut cmd = bar_info_command(NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO, 200, PARAMS_AT);
    // `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`, spelled as the literal the ABI crate's
    // own `rpc_params_are_serialized` tests use. The encoders here produce a flat struct
    // and a FINN payload is not one.
    let flags: u32 = 0x0000_0002;
    assert!(kayfabe_abi::rpc_params_are_serialized(flags));
    cmd.payload[20..24].copy_from_slice(&flags.to_le_bytes());
    let reply = p.respond(&cmd).expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
}

#[test]
fn the_classifier_names_this_control_and_its_size() {
    assert_eq!(
        WantedTable::from_cmd(0x2080_1803),
        Some(WantedTable::PciBarInfo)
    );
    // 200, as a literal — the C's array length and the capture's own `paramsSize`.
    assert_eq!(WantedTable::PciBarInfo.params_size(), 200);
    assert_eq!(PCI_BAR_INFO_PARAMS_SIZE, 200);
}

#[test]
fn the_chip_row_states_the_apertures_this_device_actually_presents() {
    let bars = chip().pci_bars;
    // Four, and the count is spelled out: it is what RM's own physical side sends for a
    // classic dGPU (`ogkm-580: kern_bus_gm107.c:4715-4718`) and what the capture carries.
    assert_eq!(bars.len(), 4);
    assert_eq!(bars[bus_bar::REGS].size_bytes, 16 * 1024 * 1024);
    assert_eq!(bars[bus_bar::FB].size_bytes, 256 * 1024 * 1024);
    assert_eq!(bars[bus_bar::INST].size_bytes, 32 * 1024 * 1024);
    assert_eq!(bars[bus_bar::IO].size_bytes, 0, "a GA106 has no I/O BAR");

    // ★★ The identity a shell realizes against carries the same two numbers, so a
    // hypervisor that registers a different aperture is refused rather than silently
    // disagreeing with what the guest is told.
    let id = kayfabe_device::identity_for(chip()).expect("GA106 has an identity");
    assert_eq!(id.fb_window_len, 256 * 1024 * 1024);
    assert_eq!(id.inst_window_len, 32 * 1024 * 1024);
    assert_eq!(chip().regs_aperture_len, 16 * 1024 * 1024);
}

#[test]
fn a_row_the_wire_cannot_carry_is_refused_by_name() {
    let nine = [PciBarRow {
        name: "x",
        size_bytes: 1 << 20,
    }; 9];
    assert_eq!(
        pcibars::encode_pci_bar_info(&nine),
        Err(PciBarError::TooManyBars { len: 9, max: 8 })
    );

    // Not a whole number of megabytes: `barSize` and `barSizeBytes` would disagree.
    let odd = [PciBarRow {
        name: "odd",
        size_bytes: 4096,
    }];
    assert_eq!(
        pcibars::encode_pci_bar_info(&odd),
        Err(PciBarError::SizeNotWholeMegabytes {
            name: "odd",
            size_bytes: 4096
        })
    );

    // A whole number of megabytes that no base-address register can have.
    let three_mb = [PciBarRow {
        name: "three",
        size_bytes: 3 << 20,
    }];
    assert_eq!(
        pcibars::encode_pci_bar_info(&three_mb),
        Err(PciBarError::SizeNotPowerOfTwo {
            name: "three",
            size_bytes: 3 << 20
        })
    );
}

#[test]
fn the_ga106_rows_are_reachable_by_name_and_are_the_chips_own() {
    // The public row and the profile's field are one table, not two.
    assert_eq!(ga10x::GA106_PCI_BARS.len(), chip().pci_bars.len());
    assert_eq!(
        ga10x::GA106_PCI_BARS[bus_bar::FB].size_bytes,
        chip().pci_bar_len(bus_bar::FB)
    );
    // An index the chip does not declare reads zero — the protocol's own "not present".
    assert_eq!(chip().pci_bar_len(7), 0);
}
