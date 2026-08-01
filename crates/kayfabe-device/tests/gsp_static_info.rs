//! `GET_GSP_STATIC_INFO` (fn 65): the FB region table this port serves, pinned against an
//! RTX 3060's own answer.
//!
//! ## ★★ Where the oracle bytes come from
//!
//! `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` in the C research artifact,
//! record **141977** — a 4096-byte `GuestWrite` to `0x1_2764_4000`, decoded with
//! `scripts/mode2_diag/rec_dump.py`. The RPC envelope at payload offset 48 reads
//! `signature="VRPC"`, `length=1824`, `function=65`, so the body starts at payload offset
//! **80** and runs 1792 bytes. Those 1792 bytes are byte-identical to the C's
//! `src/qemu/mode2_gspstaticinfo_ga106.h` (sha256
//! `20a113b4…e608d92a`), which appears verbatim exactly once in the whole 14 MB capture —
//! so the header is not a transcription of a reply, it *is* the reply.
//!
//! [`ORACLE_FB_REGION_PARAMS`] below is bytes 344..632 of that body: `numFBRegions`, its
//! alignment hole, and the five populated `fbRegion[]` entries.
//!
//! ## ★★ What this file can and cannot settle
//!
//! It settles the **layout**: an encoder fed the oracle's rows must produce the oracle's
//! bytes, so every offset and stride is checked against silicon rather than against a
//! header read. It settles nothing about *which* rows are right — this port serves two
//! regions where the oracle served five, on purpose, and no capture can adjudicate a
//! choice the capture's hardware never had to make. That argument lives in
//! `kayfabe_device::ga10x::GA106_FB_REGIONS`'s doc, and the test that guards it here is
//! `the_regions_this_port_serves_are_the_ones_it_backs`, which is an assertion about
//! *this* device's own two publications agreeing — not about the oracle.

use kayfabe_abi::gspstaticinfo::{
    FbRegion, GSP_STATIC_CONFIG_INFO_SIZE, GspStaticInfo, GspStaticInfoError,
    encode_gsp_static_info,
};
use kayfabe_abi::versions::{BENCH_DRIVER, GspStaticInfoWire, table_for};
use kayfabe_device::staticinfo::StaticInfoPolicy;
use kayfabe_device::{ChipProfile, ga10x};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// Bytes 344..632 of the oracle's `GspStaticConfigInfo`: `numFBRegions = 5`, four bytes of
/// alignment padding, then `fbRegion[0..5]` at a stride of 56.
const ORACLE_FB_REGION_PARAMS: &str = concat!(
    "05000000000000000000000000000000ffff1003000000000000110300000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000110300000000ffffbdef0200000000000000000000000600000001010000",
    "0000000000000000000000000000000000000000000000000000beef02000000",
    "ffff38f30200000000007b030000000006000000010100000000000000000000",
    "00000000000000000000000000000000000039f302000000ffff3ff302000000",
    "0000070000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000040f302000000ffffffff020000000000c00c00000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
);

/// The five regions the oracle's GSP reported for a 12 GiB RTX 3060, decoded from
/// [`ORACLE_FB_REGION_PARAMS`] by hand so that feeding them back in is a real round trip
/// and not a copy of the same bytes.
const ORACLE_ROWS: [FbRegion; 5] = [
    FbRegion {
        base: 0x0,
        limit: 0x0310_FFFF,
        reserved: 0x0311_0000,
        performance: 0,
        support_compressed: false,
        support_iso: false,
        protected: false,
    },
    FbRegion {
        base: 0x0311_0000,
        limit: 0x2_EFBD_FFFF,
        reserved: 0,
        performance: 6,
        support_compressed: true,
        support_iso: true,
        protected: false,
    },
    FbRegion {
        base: 0x2_EFBE_0000,
        limit: 0x2_F338_FFFF,
        reserved: 0x037B_0000,
        performance: 6,
        support_compressed: true,
        support_iso: true,
        protected: false,
    },
    FbRegion {
        base: 0x2_F339_0000,
        limit: 0x2_F33F_FFFF,
        reserved: 0x0007_0000,
        performance: 0,
        support_compressed: false,
        support_iso: false,
        protected: false,
    },
    FbRegion {
        base: 0x2_F340_0000,
        limit: 0x2_FFFF_FFFF,
        reserved: 0x0CC0_0000,
        performance: 0,
        support_compressed: false,
        support_iso: false,
        protected: false,
    },
];

/// The framebuffer the oracle's board had, and the one this port advertises: 12 GiB.
const TWELVE_GIB: u64 = 0x3_0000_0000;

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> StaticInfoPolicy {
    StaticInfoPolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A fn-65 command with a body the size the guest's own request declares.
fn command(function: RpcFunction, code: u32, payload_len: usize) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 1,
        payload: vec![0u8; payload_len],
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_encoder_reproduces_the_oracles_own_fb_region_bytes() {
    let body = encode_gsp_static_info(
        &GspStaticInfo {
            fb_regions: &ORACLE_ROWS,
            fb_length: TWELVE_GIB,
        },
        GspStaticInfoWire::Pre610,
    )
    .expect("the oracle's own table encodes");

    // ★★★ Every number below is a LITERAL, deliberately, and the reason is MEASURED on
    // this branch (2026-07-31, `cargo test -p kayfabe-device --test gsp_static_info`).
    // `FB_REGION_INFO_PARAMS_OFF` was packed at 340 — dropping the 8-byte realignment,
    // which is the single most likely way to get this struct wrong — and a version of
    // this test that spelled every offset as the constant it was checking passed **all
    // seven tests green**. The reply would have been four bytes out, the guest would have
    // read `numFBRegions` out of `SKUInfo`'s padding, and the failure would have been the
    // one this whole module exists to fix. Only the literals fire: with 344 and 1352
    // written out, the same mutation is red on the first assertion.
    assert_eq!(body.len(), 1792);
    let want = unhex(ORACLE_FB_REGION_PARAMS);
    assert_eq!(want.len(), 8 + 5 * 56);
    assert_eq!(&body[344..632], &want[..], "fbRegionInfoParams");
    // The eleven unused rows are zero on the wire, and RM reads `numFBRegions` of them.
    assert!(body[632..1248].iter().all(|b| *b == 0), "fbRegion[5..16]");
    // `fb_length`, the second statement of the same 12 GiB.
    assert_eq!(&body[1352..1360], &0x3_0000_0000u64.to_le_bytes()[..]);
    // Bytes 0..344 are `grCapsBits`/`gidInfo`/`SKUInfo` — real in the capture, and left
    // zero here because this port does not advertise them. The oracle's are deliberately
    // NOT copied: a GID this device did not generate is an identity it cannot honour.
    assert!(body[..344].iter().all(|b| *b == 0));
}

#[test]
fn the_regions_this_port_serves_are_the_ones_it_backs() {
    let rows = chip().fb_regions;
    // Two, and the count is spelled out so that adding a third is a deliberate edit to a
    // test that says what backs it.
    assert_eq!(rows.len(), 2, "see GA106_FB_REGIONS for why not five");

    // The carve-out base, as a literal: 12 GiB minus the 0x1042_0000 the oracle's own
    // regions 2..4 spanned.
    let carve_out_base = 0x2_EFBE_0000u64;
    assert_eq!(
        rows[0],
        FbRegion {
            base: 0,
            limit: carve_out_base - 1,
            reserved: 0,
            performance: 6,
            support_compressed: true,
            support_iso: true,
            protected: false,
        }
    );
    assert_eq!(
        rows[1],
        FbRegion {
            base: carve_out_base,
            limit: 0x2_FFFF_FFFF,
            reserved: 0x1042_0000,
            performance: 0,
            support_compressed: false,
            support_iso: false,
            protected: false,
        }
    );
    // ★ A reserved region is one RM will not allocate from, and it reads only
    // `reserved != 0` — so the *size* being the region's own size is what makes RM's
    // `reservedMemSize` add up rather than under-count.
    assert_eq!(rows[1].reserved, rows[1].limit - rows[1].base + 1);
    // And the usable one is genuinely usable: `reserved == 0` is the whole test RM runs.
    assert_eq!(rows[0].reserved, 0);
}

#[test]
fn the_three_statements_of_the_framebuffer_size_agree() {
    // ⚠ RM reads the FB size from three places and believes each: the register, the last
    // region's limit, and `fb_length`. This is the only place they can be compared.
    let chip = chip();
    assert_eq!(chip.fb_length, 0x3_0000_0000);
    let last = chip.fb_regions.last().expect("non-empty");
    assert_eq!(last.limit + 1, chip.fb_length);
    let reg = chip
        .boot_regs
        .iter()
        .find(|r| r.off == 0x0011_83A4)
        .expect("NV_USABLE_FB_SIZE_IN_MB is a silicon constant on this row");
    assert_eq!(u64::from(reg.value) << 20, chip.fb_length);
}

#[test]
fn the_policy_answers_fn65_with_a_populated_table() {
    let mut p = policy();
    // 1792 is the body length the guest's own request declares — `rpc.length` 1824 less
    // the 32-byte RPC header, read from cap1b record 141976.
    let reply = p
        .respond(&command(RpcFunction::GetGspStaticInfo, 65, 1792))
        .expect("answered");
    assert_eq!(reply.rpc_result, 0);
    assert_eq!(reply.body.len(), 1792);
    assert_eq!(
        u32::from_le_bytes(reply.body[344..348].try_into().unwrap()),
        2,
        "numFBRegions — the whole point of the rung"
    );
    assert_eq!(
        u64::from_le_bytes(reply.body[1352..1360].try_into().unwrap()),
        0x3_0000_0000
    );
}

#[test]
fn every_other_command_falls_through_to_the_rest_of_the_chain() {
    let mut p = policy();
    for f in [
        RpcFunction::RmControl,
        RpcFunction::SetGuestSystemInfo,
        RpcFunction::Other(0x1234),
    ] {
        assert!(
            p.respond(&command(f, 0, 1792)).is_none(),
            "{f:?} is not this policy's to answer"
        );
    }
}

#[test]
fn the_two_installed_policies_do_not_both_claim_a_function() {
    // ★ `PolicyChain` cannot detect a contradiction — order silently decides it. This is
    // the crate that installs both, so this is where the disjointness is stated.
    let driver = *table_for(BENCH_DRIVER).expect("bench ABI");
    let mut tables = kayfabe_device::inittables::InitTablePolicy::new(chip(), driver);
    let mut static_info = StaticInfoPolicy::new(chip(), driver);
    assert!(
        tables
            .respond(&command(RpcFunction::GetGspStaticInfo, 65, 1792))
            .is_none(),
        "the init-table policy must not claim fn 65"
    );
    assert!(
        static_info
            .respond(&command(RpcFunction::RmControl, 76, 1792))
            .is_none(),
        "the static-info policy must not claim GSP_RM_CONTROL"
    );
}

#[test]
fn a_chip_whose_table_cannot_be_encoded_is_refused_in_the_envelope() {
    // ⊘ Induced, not observed in the wild: a row whose regions stop short of the
    // `fb_length` it also states. The point is that the failure is an envelope status the
    // guest logs by name, not a well-formed body that is quietly wrong.
    static BAD_REGIONS: &[FbRegion] = &[FbRegion {
        base: 0,
        limit: 0x0FFF_FFFF,
        reserved: 0,
        performance: 6,
        support_compressed: true,
        support_iso: true,
        protected: false,
    }];
    static BAD: ChipProfile = ChipProfile {
        fb_regions: BAD_REGIONS,
        fb_length: 0x3_0000_0000,
        ..copy_of_ga106()
    };
    let driver = *table_for(BENCH_DRIVER).expect("bench ABI");
    let p = StaticInfoPolicy::new(&BAD, driver);
    assert_eq!(
        p.body().expect_err("refuses"),
        GspStaticInfoError::RegionsDoNotSpanFb {
            top: 0x1000_0000,
            fb_length: 0x3_0000_0000
        }
    );
    let mut p = p;
    let reply = p
        .respond(&command(RpcFunction::GetGspStaticInfo, 65, 1792))
        .expect("answers, loudly");
    assert_ne!(reply.rpc_result, 0, "NV_ERR_NOT_SUPPORTED in the envelope");
    assert!(reply.body.is_empty());
}

/// A `ChipProfile` identical to GA106 in everything the test above does not override.
const fn copy_of_ga106() -> ChipProfile {
    ChipProfile {
        name: "TEST-BAD-FB",
        pci_device_id: 0x2504,
        pci_revision: 0xa1,
        pci_subsystem_vendor_id: 0x1462,
        pci_subsystem_id: 0x397D,
        regs_aperture_len: 0x0100_0000,
        pci_bars: &[kayfabe_abi::pcibars::PciBarRow {
            name: "registers",
            size_bytes: 0x0100_0000,
        }],
        boot_regs: &[],
        ptimer: kayfabe_device::PtimerRegs {
            lo_off: 0x9400,
            hi_off: 0x9410,
        },
        rom_window: kayfabe_device::RomWindow {
            base: 0x30_0000,
            len: 0x2_0000,
        },
        pramin_window: kayfabe_device::RegSpan {
            base: 0x0070_0000,
            len: 0x0010_0000,
        },
        bar0_window_reg: 0x0000_1700,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(ga10x::Ga10xGspModel::new()),
        engines: &[],
        intr_table: &[],
        intr_subtree_map: [0; kayfabe_abi::inittables::INTR_CATEGORY_COUNT],
        fb_regions: &[],
        // This test is about the framebuffer regions; the chip-identity reply is not
        // exercised, so the row names no register group at all — which the encoder
        // spells as sixteen `REG_BASE_UNSUPPORTED`s rather than a zero.
        chip_info: kayfabe_abi::chipinfo::ChipInfoRow {
            chip_sub_rev: 0,
            is_cmp_sku: false,
            reg_bases: &[],
        },
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: 0,
    }
}

#[test]
fn a_request_whose_body_is_not_the_struct_this_port_encodes_is_refused_in_the_envelope() {
    // ★★★ PC-D4. The failure this closes is **silent**, which is why the test has to state
    // what silence looked like: `RpcCommand::reply` clamps the body to the request's own
    // declared length, so a guest whose `sizeof(GspStaticConfigInfo)` differs from ours got
    // a TRUNCATED or ZERO-PADDED table — no fault, no counter, no refusal — copied straight
    // into `pGpu->pGspStaticInfo`. Every `InitTablePolicy` control has always refused the
    // same disagreement; fn 65 compared nothing.
    //
    // ⊘ Both directions, and both boundaries. A one-byte-short request is the truncation
    // case and a one-byte-long request is the padding case; either is a guest whose struct
    // is not the struct we encode.
    let mut p = policy();
    for len in [
        0,
        1,
        GSP_STATIC_CONFIG_INFO_SIZE - 1,
        GSP_STATIC_CONFIG_INFO_SIZE + 1,
        4096,
    ] {
        let reply = p
            .respond(&command(RpcFunction::GetGspStaticInfo, 65, len))
            .unwrap_or_else(|| panic!("len {len} must be REFUSED, not ignored"));
        assert_eq!(
            reply.rpc_result,
            kayfabe_abi::NV_ERR_NOT_SUPPORTED,
            "len {len} was answered rather than refused"
        );
        assert!(
            reply.body.is_empty(),
            "a refusal carries no body, least of all a clamped one"
        );
    }
    // Non-vacuity, and it is the whole assertion: the exact size still succeeds, so the
    // check is a size check and not a policy that has stopped answering.
    let ok = p
        .respond(&command(
            RpcFunction::GetGspStaticInfo,
            65,
            GSP_STATIC_CONFIG_INFO_SIZE,
        ))
        .expect("answered");
    assert_eq!(ok.rpc_result, 0);
    assert_eq!(ok.body.len(), GSP_STATIC_CONFIG_INFO_SIZE);
}
