//! `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` (`0x20800a1c`) — the seventh
//! control this port serves, and the first whose **refusal** is the dangerous outcome.
//!
//! ## What this file establishes, in the order it matters
//!
//! 1. **The layout is right**, checked against silicon: the encoder, driven with the GA106
//!    row, reproduces a real RTX 3060's forty-byte reply byte for byte — including both pad
//!    runs the FINN alignment inserts, which is the only reason to trust the offsets past
//!    `l2CacheSize`.
//! 2. **The all-zero answer is unencodable**, in three independent ways, each standing in
//!    front of a *different* guest-side fault. This is the point of the file. RM pre-zeroes
//!    the params (`ogkm-580: kern_mem_sys.c:114`), so a
//!    [`kayfabe_device::inert`]-style reply and an all-zero served reply are the same forty
//!    bytes — and unlike every earlier control, those bytes are not merely uninformative:
//!    they violate an invariant RM asserts on itself and divide-by-zero in the heap path.
//! 3. **`comprPageShift` cannot disagree with `comprPageSize`**, because it is not a field.
//!
//! ## Provenance
//!
//! `[measured]` [`ORACLE`] is the C artifact's `ctl_20800a1c`
//! (`C: src/qemu/mode2_initctrl_ga106.h:5391`, registered at `:6255` as
//! `{0x20800a1cu, 0x0u, 40u, 40u, …}` — a forty-byte capture for a forty-byte struct, with
//! nothing trimmed and nothing zero-extended), which is in turn a real RTX 3060's own
//! answer captured through the same control
//! (`C: docs/research/captures/ga106_initctrl_580.log`). The guest asks for it at
//! `rpc.sequence` 11 of `traces/cap1b_coldboot_hermetic_d6.rec`, one queue element,
//! `paylen 80 = 40 + 40`.

use kayfabe_abi::memsysconfig::{
    self, COMPR_PAGE_SHIFT_OFF, COMPR_PAGE_SIZE_OFF, ComptagAllocationPolicy, ENABLED_ECC_FBPA_OFF,
    FBPA_PRESENT_OFF, L2_CACHE_SIZE_OFF, LTC_COUNT_OFF, LTS_PER_LTC_COUNT_OFF,
    MEMSYS_STATIC_CONFIG_PARAMS_SIZE, MemorySystemError, MemorySystemRow,
    NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG, ONE_TO_FOUR_COMPTAG_OFF,
    ONE_TO_ONE_COMPTAG_OFF, RAM_TYPE_GDDR6, RAM_TYPE_OFF, RAW_MODE_COMPTAG_OFF,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::ga10x;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// The forty bytes an RTX 3060's GSP answered.
const ORACLE: &str = concat!(
    "0000010000000000", // policy bits: raw mode only; then one pad byte
    "0000240000000000", // l2CacheSize = 0x24_0000
    "0100000000000100", // bFbpaPresent = 1, three pad bytes, comprPageSize = 0x1_0000
    "1000000011000000", // comprPageShift = 16, ramType = 0x11 (GDDR6)
    "0600000004000000", // ltcCount = 6, ltsPerLtcCount = 4
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// `RpcControlReq::HEADER`, as `cap1b`'s own arithmetic gives it: the request's `paylen`
/// was 80 and `80 - 40 = 40`.
const PARAMS_AT: usize = 40;

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is `0xAA`, not zeros, and here that is load-bearing rather than
/// tidy: `kmemsysStatePreInitLocked_IMPL` `portMemSet`s its config to zero before sending
/// (`ogkm-580: kern_mem_sys.c:114`), so on the bench a reply that merely *reflected* the
/// request would be indistinguishable from one that answered zeros. Only a poisoned request
/// can tell an answer from an echo.
fn memsys_command(cmd: u32, params_size: u32, serialized: bool) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes()); // hClient, as captured
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject, as captured
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    let flags: u32 = if serialized { 1 << 1 } else { 0 };
    payload[20..24].copy_from_slice(&flags.to_le_bytes());
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 11,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The GA106 row with one field replaced — for the refusal tests.
fn row() -> MemorySystemRow {
    ga10x::GA106_MEMORY_SYSTEM
}

// ── The layout, against silicon ────────────────────────────────────────────────────

#[test]
fn the_encoder_reproduces_the_oracles_reply_byte_for_byte() {
    // ★★★ The provenance test. Two sides that share no source: one is a real GA106's GSP
    // through the C artifact's capture, the other is this crate's row and encoder.
    let oracle = unhex(ORACLE);
    assert_eq!(
        oracle.len(),
        MEMSYS_STATIC_CONFIG_PARAMS_SIZE,
        "the capture is a whole struct, not a prefix"
    );
    let ours = memsysconfig::encode_memsys_static_config(&row()).expect("the GA106 row encodes");
    assert_eq!(ours, oracle, "byte for byte");
}

#[test]
fn the_pad_bytes_the_finn_alignment_inserts_are_zero_and_are_where_the_capture_says() {
    // ★★ The offsets past `l2CacheSize` are only trustworthy if the padding is. A layout
    // that packed `l2CacheSize` at 7 instead of 8 would shift every later field by one and
    // still be a plausible-looking struct; the capture is what rules it out.
    let oracle = unhex(ORACLE);
    assert_eq!(oracle[7], 0, "the pad byte after seven NvBools");
    assert_eq!(
        &oracle[17..20],
        &[0, 0, 0],
        "the pad run after bFbpaPresent"
    );
    // And the fields either side land on their declared offsets.
    assert_eq!(
        u64::from_le_bytes(
            oracle[L2_CACHE_SIZE_OFF..L2_CACHE_SIZE_OFF + 8]
                .try_into()
                .unwrap()
        ),
        0x0024_0000,
    );
    assert_eq!(oracle[FBPA_PRESENT_OFF], 1);
    assert_eq!(
        u32::from_le_bytes(
            oracle[COMPR_PAGE_SIZE_OFF..COMPR_PAGE_SIZE_OFF + 4]
                .try_into()
                .unwrap()
        ),
        0x0001_0000,
    );
    assert_eq!(
        u32::from_le_bytes(oracle[RAM_TYPE_OFF..RAM_TYPE_OFF + 4].try_into().unwrap()),
        RAM_TYPE_GDDR6,
    );
}

#[test]
fn the_captures_four_facts_agree_with_each_other() {
    // ★★ Why this capture is trusted and not merely copied. Four fields that a
    // transcription error would desynchronise:
    let o = unhex(ORACLE);
    let size = u32::from_le_bytes(
        o[COMPR_PAGE_SIZE_OFF..COMPR_PAGE_SIZE_OFF + 4]
            .try_into()
            .unwrap(),
    );
    let shift = u32::from_le_bytes(
        o[COMPR_PAGE_SHIFT_OFF..COMPR_PAGE_SHIFT_OFF + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(1u32 << shift, size, "comprPageShift is log2(comprPageSize)");

    let l2 = u64::from_le_bytes(
        o[L2_CACHE_SIZE_OFF..L2_CACHE_SIZE_OFF + 8]
            .try_into()
            .unwrap(),
    );
    let ltc = u32::from_le_bytes(o[LTC_COUNT_OFF..LTC_COUNT_OFF + 4].try_into().unwrap());
    let lts = u32::from_le_bytes(
        o[LTS_PER_LTC_COUNT_OFF..LTS_PER_LTC_COUNT_OFF + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        l2,
        u64::from(ltc) * u64::from(lts) * 96 * 1024,
        "24 slices at 96 KiB each is the L2 size two fields above"
    );
}

#[test]
fn the_layout_constants_are_the_structs_own_arithmetic() {
    assert_eq!(ONE_TO_ONE_COMPTAG_OFF, 0);
    assert_eq!(ONE_TO_FOUR_COMPTAG_OFF, 1);
    assert_eq!(RAW_MODE_COMPTAG_OFF, 2);
    assert_eq!(ENABLED_ECC_FBPA_OFF, 5);
    assert_eq!(L2_CACHE_SIZE_OFF, 8, "8-aligned NvU64 after seven NvBools");
    assert_eq!(FBPA_PRESENT_OFF, 16);
    assert_eq!(COMPR_PAGE_SIZE_OFF, 20, "4-aligned NvU32 after one NvBool");
    assert_eq!(MEMSYS_STATIC_CONFIG_PARAMS_SIZE, 40);
}

// ── The combinations that fail open ────────────────────────────────────────────────

#[test]
fn the_zero_filled_reply_rm_pre_zeroes_for_us_is_unencodable() {
    // ★★★ The whole reason this control is not inert-eligible. RM zeroes the params before
    // the call, so "answer nothing" and "answer zeros" are the same forty bytes on the
    // wire. This test says those bytes cannot be built here — and the three tests below
    // say what each of them would have done to the guest.
    //
    // ⊘ There is no `MemorySystemRow` whose encoding is all zeros: `ComptagAllocationPolicy`
    // has no *neither* variant, so byte 0 or byte 2 is always set. That half is closed by
    // the type and cannot be tested by constructing a counterexample — so this test asserts
    // the consequence instead, over BOTH policies.
    for policy in [
        ComptagAllocationPolicy::OneToOne,
        ComptagAllocationPolicy::Raw,
    ] {
        let p = memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            comptag_policy: policy,
            ..row()
        })
        .expect("both policies encode");
        assert!(
            p[ONE_TO_ONE_COMPTAG_OFF] != 0 || p[RAW_MODE_COMPTAG_OFF] != 0,
            "RM asserts this disjunction on itself (ogkm-580: kern_mem_sys.c:422)"
        );
        assert_ne!(p, vec![0u8; MEMSYS_STATIC_CONFIG_PARAMS_SIZE]);
    }
}

#[test]
fn the_one_to_four_bit_rm_would_reject_alone_is_never_set() {
    // ★★ The field exists in the ABI and RM's own disjunction does not accept it: a config
    // whose only policy bit is the one-to-four bit fails `kmemsysAllocComprResources_KERNEL`
    // exactly as an all-zero config does. `ComptagAllocationPolicy` has no variant for it,
    // so the encoder can never write byte 1.
    for policy in [
        ComptagAllocationPolicy::OneToOne,
        ComptagAllocationPolicy::Raw,
    ] {
        let p = memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            comptag_policy: policy,
            ..row()
        })
        .expect("encodes");
        assert_eq!(
            p[ONE_TO_FOUR_COMPTAG_OFF], 0,
            "the bit RM's disjunction does not accept"
        );
    }
}

#[test]
fn a_zero_compr_page_size_is_unencodable_because_rm_divides_by_it() {
    // ★★★ `mem_mgr_gm107.c:210-211` divides an allocation size by this field with no
    // guard. A zero here is a guest-kernel divide-by-zero, not a dull answer.
    assert_eq!(
        memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            compr_page_size: 0,
            ..row()
        }),
        Err(MemorySystemError::ComprPageSizeZero)
    );
}

#[test]
fn a_compr_page_size_that_is_not_a_power_of_two_is_unencodable() {
    // ★★ The same field is used as `comprPageSize - 1`, an alignment MASK
    // (`ogkm-580: mem_mgr_gm107.c:216`, `mem_mgr_ga100.c:93`) — which masks nothing unless
    // the value is a power of two. It also has no exact `comprPageShift`.
    for bad in [3u32, 0x1_0001, 0xFFFF, 96 * 1024] {
        assert_eq!(
            memsysconfig::encode_memsys_static_config(&MemorySystemRow {
                compr_page_size: bad,
                ..row()
            }),
            Err(MemorySystemError::ComprPageSizeNotPowerOfTwo {
                compr_page_size: bad
            }),
            "{bad:#x}"
        );
    }
}

#[test]
fn a_zero_ltc_factor_is_unencodable_because_ampere_multiplies_and_branches_on_it() {
    // ★★ `kern_mem_sys_ga100.c:332-345` forms `… * ltcCount * ltsPerLtcCount >> 4` and
    // branches on the product; `kern_mem_sys_ga102.c:66-120` matches it against 48/40/4x8/3x8.
    assert_eq!(
        memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            ltc_count: 0,
            ..row()
        }),
        Err(MemorySystemError::NoLtcSlices {
            ltc_count: 0,
            lts_per_ltc_count: 4
        })
    );
    assert_eq!(
        memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            lts_per_ltc_count: 0,
            ..row()
        }),
        Err(MemorySystemError::NoLtcSlices {
            ltc_count: 6,
            lts_per_ltc_count: 0
        })
    );
}

#[test]
fn claiming_no_framebuffer_partitions_is_unencodable_because_it_moves_rms_bar0_window() {
    // ★★ `kbusInitBar0Window` re-derives the window from `l2CacheSize` when
    // `!bFbpaPresent` (`ogkm-580: kern_bus_gm107.c:230-247`). This device decodes one fixed
    // PRAMIN span, so it may only advertise the value that selects that placement.
    assert_eq!(
        memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            fbpa_present: false,
            ..row()
        }),
        Err(MemorySystemError::FbpaAbsent)
    );
}

#[test]
fn a_zero_l2_is_unencodable() {
    assert_eq!(
        memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            l2_cache_size: 0,
            ..row()
        }),
        Err(MemorySystemError::L2CacheSizeZero)
    );
}

#[test]
fn the_shift_cannot_disagree_with_the_size_because_it_is_not_a_field() {
    // ★★★ The `FalconInventoryRow`-has-no-count discipline, applied to a second pair. Two
    // fields on the wire, one field on the row: the encoder derives the shift, so a row
    // that stated them inconsistently cannot be written.
    for size in [4096u32, 0x1_0000, 0x2_0000, 1 << 20] {
        let p = memsysconfig::encode_memsys_static_config(&MemorySystemRow {
            compr_page_size: size,
            ..row()
        })
        .expect("a power of two encodes");
        let got_size = u32::from_le_bytes(
            p[COMPR_PAGE_SIZE_OFF..COMPR_PAGE_SIZE_OFF + 4]
                .try_into()
                .unwrap(),
        );
        let got_shift = u32::from_le_bytes(
            p[COMPR_PAGE_SHIFT_OFF..COMPR_PAGE_SHIFT_OFF + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(got_size, size);
        assert_eq!(1u32 << got_shift, got_size, "derived, so it agrees");
    }
}

// ── The serve site ─────────────────────────────────────────────────────────────────

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let reply = p
        .respond(&memsys_command(
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
            40,
            false,
        ))
        .expect("this port serves the memory-system config");
    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 40];
    assert_eq!(params, &unhex(ORACLE)[..], "a real GA106's own forty bytes");
    assert!(
        !params.contains(&0xAA),
        "not one poisoned request byte survived into the reply"
    );
    assert_eq!(
        &reply.body[12..16],
        &0u32.to_le_bytes()[..],
        "status = NV_OK"
    );
    assert_eq!(
        &reply.body[16..20],
        &40u32.to_le_bytes()[..],
        "paramsSize, rewritten to what we encoded"
    );
}

#[test]
fn the_serve_site_refuses_when_the_encoder_declines() {
    // ★★★ The fail-open guard at the layer that would actually widen — and the one place
    // in this port where refusing is known to be the WORSE guest outcome (it amputates
    // KernelMemorySystem and the boot dies later in the heap path). It is still right: the
    // rows the encoder declines are each a guest-kernel fault of their own, so answering
    // anyway would trade a NULL dereference for a divide-by-zero.
    static BAD: ChipProfile = ChipProfile {
        has_c2c: false,
        lce_pce_masks: kayfabe_abi::cepce::GA106_LCE_PCE_MASKS,
        memory_system: MemorySystemRow {
            compr_page_size: 0,
            ..ga10x::GA106_MEMORY_SYSTEM
        },
        ..copy_of_ga106()
    };
    let mut p = InitTablePolicy::new(&BAD, *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p
        .respond(&memsys_command(
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
            40,
            false,
        ))
        .expect("answers, loudly");
    assert_ne!(reply.rpc_result, 0, "NV_ERR_NOT_SUPPORTED in the envelope");
    assert!(reply.body.is_empty(), "and no body to misread");
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    for size in [0u32, 4, 36, 44, 48, 1284] {
        let mut p = policy();
        let reply = p
            .respond(&memsys_command(
                NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
                size,
                false,
            ))
            .expect("answers");
        assert_ne!(
            reply.rpc_result, 0,
            "a guest declaring {size} bytes is not a guest whose struct we encode"
        );
        assert!(reply.body.is_empty());
    }
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    let mut p = policy();
    let reply = p
        .respond(&memsys_command(
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
            40,
            true,
        ))
        .expect("answers");
    assert_ne!(
        reply.rpc_result, 0,
        "FINN-serialized is not our flat layout"
    );
    assert!(reply.body.is_empty());
}

#[test]
fn the_classifier_names_this_control_and_its_size() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG),
        Some(WantedTable::MemorySystemStaticConfig)
    );
    assert_eq!(
        WantedTable::MemorySystemStaticConfig.cmd_id(),
        0x2080_0a1c,
        "literal — the id the capture carries"
    );
    assert_eq!(WantedTable::MemorySystemStaticConfig.params_size(), 40);
    assert!(
        WantedTable::ALL.contains(&WantedTable::MemorySystemStaticConfig),
        "and it is in the universe every coverage gate quantifies over"
    );
}

/// A `ChipProfile` identical to GA106 in everything the test above does not override.
///
/// ⊘ A `const fn` literal rather than `..ga10x::GA106`: a `static` fixture needs a const
/// expression and `ChipProfile` cannot be moved out of a `static`.
const fn copy_of_ga106() -> ChipProfile {
    ChipProfile {
        has_c2c: false,
        lce_pce_masks: kayfabe_abi::cepce::GA106_LCE_PCE_MASKS,
        name: "TEST-BAD-MEMSYS",
        pci_device_id: 0x2504,
        pci_revision: 0xa1,
        pci_subsystem_vendor_id: 0x1462,
        pci_subsystem_id: 0x397D,
        regs_aperture_len: 16 << 20,
        pci_bars: &[kayfabe_abi::pcibars::PciBarRow {
            name: "registers",
            size_bytes: 16 << 20,
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
        chip_info: kayfabe_abi::chipinfo::ChipInfoRow {
            chip_sub_rev: 0,
            is_cmp_sku: false,
            reg_bases: &[],
        },
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: ga10x::GA106_MEMORY_SYSTEM,
        device_info: ga10x::GA106_DEVICE_INFO,
        conf_compute: ga10x::GA106_CONF_COMPUTE,
        bif_static: ga10x::GA106_BIF_STATIC,
        fifo_channels: ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: ga10x::GA106_GMMU_STATIC,
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
