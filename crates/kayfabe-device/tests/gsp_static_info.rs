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
    FbRegion, GSP_STATIC_CONFIG_INFO_SIZE, GpuName, GspStaticInfo, GspStaticInfoError,
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

/// Bytes 24..36 of the oracle's `GspStaticConfigInfo`: `gidInfo.index`, `gidInfo.flags`
/// and `gidInfo.length`. Read from the same board's own reply
/// (`C: docs/research/captures/ga106_gspstaticinfo_580.log:3-4`, rows `0010` and `0020`).
/// ⊘ The shape only — the 16 identity bytes that follow are [`ORACLE_GID_UUID`], and this
/// port must not serve them.
const ORACLE_GID_INFO_HEADER: &str = "000000000200000010000000";

/// Bytes 36..52 of the oracle's body: the captured RTX 3060's **own** UUID. Present here
/// for exactly one purpose — so `the_encoder_reproduces_the_oracles_own_fb_region_bytes`
/// can assert this port does **not** serve it. ⊘ Never a default, never a fallback.
const ORACLE_GID_UUID: &str = "51b08678782840151962a65a7a488e3c";

/// A UUID for the layout tests. Any non-zero value: what is under test is where the bytes
/// land, never which bytes they are.
fn a_test_gid() -> kayfabe_abi::gspstaticinfo::GpuGid {
    kayfabe_abi::gspstaticinfo::GpuGid::derive(b"gsp_static_info layout test")
}

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
            bar1_pde_base: kayfabe_device::ga10x::GA106_BAR1_PDE_BASE,
            gid: a_test_gid(),
            // ⊘ The layout test states a name so the FB assertions below run against a
            // fully-populated body; the shipped device states none. See
            // `a_name_this_port_was_never_told_is_served_as_zero_for_every_chip_row`.
            name: Some(GpuName::declared("NVIDIA GeForce RTX 3060")),
            short_name: Some(GpuName::declared("GA106-A")),
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
    // Bytes 0..24 (`grCapsBits` + its alignment byte) and 292..344 (`SKUInfo`) are real in
    // the capture and left zero here, because this port does not advertise them.
    assert!(body[..24].iter().all(|b| *b == 0), "grCapsBits");
    assert!(body[292..344].iter().all(|b| *b == 0), "SKUInfo");

    // ★★★ `gidInfo`, and the two halves of it are treated DIFFERENTLY on purpose.
    //
    // - Its **shape** — `index`, `flags`, `length` — is a fact about what a GSP writes,
    //   and the capture is the oracle for it: bytes 24..36 of the oracle body read
    //   `00000000 02000000 10000000`, i.e. index 0, `FORMAT_BINARY | TYPE_SHA1`, and a
    //   length of 16. This port must write the same three words or the guest's
    //   `gidInfo.data` is not where it looks for it.
    // - Its **identity** — `data[0..16]` — is deliberately NOT the oracle's. A GID this
    //   device did not generate is an identity it cannot honour, and copying the C bench
    //   board's UUID would make every kayfabe GPU claim to be that specific RTX 3060.
    assert_eq!(
        &body[24..36],
        &unhex(ORACLE_GID_INFO_HEADER)[..],
        "gidInfo.{{index,flags,length}} — the oracle's own three words"
    );
    assert_eq!(
        &body[36..52],
        a_test_gid().as_bytes(),
        "gidInfo.data[0..16]"
    );
    assert_ne!(
        &body[36..52],
        &unhex(ORACLE_GID_UUID)[..],
        "this port must never ship the captured board's own UUID"
    );
    assert!(
        body[52..292].iter().all(|b| *b == 0),
        "gidInfo.data past the 16-byte SHA-1 GID"
    );
}

/// ★ **The default this port derives is not zero** — the one value
/// `gpuGenGidData_FWCLIENT` reads as *"GSP Static Info has not been initialized yet for
/// UUID"* (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_gspclient.c:152-156`).
///
/// `[measured 2026-08-08, boots p35_754e393 and p35_1f88649, revs 754e393 / 1f88649]` a
/// zero here is not a quiet field: it is `RmRegisterGpudb: Failed to get UUID` and
/// `RmInitAdapter failed! (0x43:0x59:2239)` — `osinit.c:2239`,
/// `RM_SET_ERROR(status, RM_GPUDB_REGISTER_FAILED)` — thirteen seconds and four subsystems
/// after the last thing that mentions the GSP. That distance is the whole reason this test
/// asserts on the *served body* rather than on the type.
#[test]
fn the_served_body_carries_a_non_zero_uuid_for_every_chip_row() {
    for chip in kayfabe_device::CHIPS {
        let body = StaticInfoPolicy::new(chip, *table_for(BENCH_DRIVER).expect("bench ABI"))
            .body()
            .expect("every shipped chip row encodes");
        assert!(
            body[36..52].iter().any(|b| *b != 0),
            "{}: gidInfo.data is all zero, which the guest reads as NV_ERR_INVALID_STATE",
            chip.name
        );
    }
}

/// ★★★ **The model name lands where the real board puts it — and its ABSENCE is loud.**
///
/// `[measured 2026-08-08, boot p35_8088019]` a zero `gpuNameString` is what `nvidia-smi`
/// prints as `Name: ERR!`. The field is read by `gpuGetNameString_FWCLIENT`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_gspclient.c:269-290`), which `portMemCopy`s
/// it straight out of `pGpu->pGspStaticInfo`; on a GSP client there is no other producer.
///
/// ## ⊘ What this test does NOT do, and it is the point
///
/// It does **not** assert that the shipped device serves a name. It cannot: there is no
/// `ChipProfile::name_string` and there must not be one — the value's source is the **host
/// GPU's own `NV2080_CTRL_CMD_GPU_GET_NAME_STRING`** under the owner's READ-NATIVE ruling,
/// and that query has an unsolved lifetime (see [`StaticInfoPolicy::with_name`]). What is
/// settled here is the **layout**: given a name, it lands at 1388/1452/1516 exactly where
/// a real GA106 puts its own; given none, all three arrays stay zero.
///
/// ★ The strings below come from the real board's own fn-65 reply
/// (`C: docs/research/captures/ga106_gspstaticinfo_580.log`) — a capture row **with a
/// body**, which is the half `CLAUDE.md` records as trustworthy byte-for-byte. They are
/// the oracle for *where the bytes go*, not a value this port ships.
#[test]
fn a_declared_name_lands_where_the_real_ga106_puts_its_own() {
    let body = policy()
        .with_name(
            GpuName::declared("NVIDIA GeForce RTX 3060"),
            GpuName::declared("GA106-A"),
        )
        .body()
        .expect("GA106 encodes");
    // Literals for the offsets, for the reason stated at length above: reading them back
    // off the constants under test would make the assertion agree with the arithmetic.
    assert_eq!(
        &body[1388..1388 + 23],
        b"NVIDIA GeForce RTX 3060",
        "gpuNameString — at the oracle's own offset"
    );
    assert_eq!(&body[1452..1452 + 7], b"GA106-A", "gpuShortNameString");
    // NUL-terminated, both: RM copies the whole fixed-width array out and treats it as a
    // C string.
    assert!(
        body[1388 + 23..1452].iter().all(|b| *b == 0),
        "gpuNameString is not NUL-padded to its 64 bytes"
    );
    assert!(body[1452 + 7..1516].iter().all(|b| *b == 0));
    // ★ And the UTF-16 array, which `gpuGetNameString_FWCLIENT` reads for every `type`
    // that is not `..._FLAGS_TYPE_ASCII`. Leaving it zero would answer a Unicode caller
    // with an empty string and no error.
    let unicode: Vec<u8> = b"NVIDIA GeForce RTX 3060"
        .iter()
        .flat_map(|c| [*c, 0])
        .collect();
    assert_eq!(
        &body[1516..1516 + 46],
        &unicode[..],
        "gpuNameString_Unicode"
    );
    assert!(body[1516 + 46..1516 + 128].iter().all(|b| *b == 0));
}

/// ★★★ **An unstated name is ZERO, not a stand-in** — `c_oracle_empty_rows_are_wrong`.
///
/// ⊘ This is the assertion that makes the missing chip-row constant a *decision* rather
/// than an omission. The tempting fix for `Name: ERR!` is a default string, and a default
/// would answer for a card nobody asked, on a host nobody queried, with the guest unable
/// to tell the difference. Zero is what this port knows, and `nvidia-smi` saying `ERR!` is
/// that fact reaching the operator.
///
/// ★ The precedent is not an aesthetic argument: `C: src/qemu/mode2_initctrl_ga106.h` has
/// 11 of 56 rows with `dlen = 0`, and every one checked against a real GA106 is
/// contradicted — `0x20802a08` decodes from its empty row as size 0 where the part
/// answers 20480, which is a buffer overrun with a hardware writer. An empty capture is
/// evidence of nothing; a default here would be the same mistake with a nicer string.
///
/// ⚠ Quantified over [`CHIPS`] (`gates_quantified_over_a_list`) so a chip row added later
/// cannot smuggle a name in by a route this test does not watch.
///
/// [`CHIPS`]: kayfabe_device::CHIPS
#[test]
fn a_name_this_port_was_never_told_is_served_as_zero_for_every_chip_row() {
    for chip in kayfabe_device::CHIPS {
        let body = StaticInfoPolicy::new(chip, *table_for(BENCH_DRIVER).expect("bench ABI"))
            .body()
            .expect("every shipped chip row encodes");
        assert!(
            body[1388..1516].iter().all(|b| *b == 0),
            "{}: a name was served without anyone declaring one — if a chip-row constant \
             has been added, read StaticInfoPolicy::with_name before deleting this test",
            chip.name
        );
        assert!(
            body[1516..1644].iter().all(|b| *b == 0),
            "{}: gpuNameString_Unicode",
            chip.name
        );
        // ⊘ Non-vacuity: the same policy DOES serve one when told, so this test is about
        // absence and not about an encoder that has stopped writing.
        let told = StaticInfoPolicy::new(chip, *table_for(BENCH_DRIVER).expect("bench ABI"))
            .with_name(GpuName::declared("X"), GpuName::declared("Y"))
            .body()
            .expect("encodes");
        assert_eq!(&told[1388..1390], b"X\0");
        assert_eq!(&told[1452..1454], b"Y\0");
    }
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
        has_c2c: false,
        lce_pce_masks: kayfabe_abi::cepce::GA106_LCE_PCE_MASKS,
        fb_regions: BAD_REGIONS,
        fb_length: 0x3_0000_0000,
        bar1_pde_base: kayfabe_device::ga10x::GA106_BAR1_PDE_BASE,
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
        has_c2c: false,
        lce_pce_masks: kayfabe_abi::cepce::GA106_LCE_PCE_MASKS,
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
        gr_info: kayfabe_abi::grinfo::GA106_GR_INFO,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        forwarded_gpu_info: kayfabe_abi::gpuinfo::GA106_FORWARDED_GPU_INFO,
        smc_mode: kayfabe_abi::smcmode::GA106_SMC_MODE,
        pcie_max_gen: kayfabe_abi::businfo::PcieGen::Gen4,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: 0,
        bar1_pde_base: kayfabe_device::ga10x::GA106_BAR1_PDE_BASE,
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
