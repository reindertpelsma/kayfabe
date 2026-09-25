//! ★★★★★ **On a GA106 host, derived must equal captured** (`V3_P2_PORT_MAP.md` §5).
//!
//! The old tree answered from captured GA106 rows (`kayfabe-device/src/ga10x.rs`, now the
//! fixture `support/ga106.rs`). v3 answers from [`HostFacts`], filled from the host. This file
//! feeds the `kf_rm::hostfacts::derive_*` functions the **real GA106's own reply bytes** —
//! copied verbatim from `traces/real_ga106/` (RTX 3060, 580.159.04, unprivileged ioctls) — and
//! asserts each derived fact equals the old captured row. A disagreement means one of the two
//! is wrong about the same physical part.
//!
//! It also holds the provenance gate: every [`HostFacts`] field has exactly one
//! [`hostfacts::PROVENANCE`] row.

#[path = "support/ga106.rs"]
mod ga106;

use kf_rm::HostFacts;
use kf_rm::hostfacts::{self, FactRefusal};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

/// `rmladder_r24_pcemask_real_ga106.txt`: LCE0..3 answer, LCE4 is refused `Other(86)`.
/// Each reply is `{ceEngineType, pceMask}`.
#[test]
fn lce_pce_masks_derived_from_the_real_ga106_equal_the_captured_row() {
    let lce = [
        unhex("0900000020000000"),
        unhex("0a00000010000000"),
        unhex("0b00000010000000"),
        unhex("0c00000020000000"),
    ];
    let replies: Vec<Option<&[u8]>> =
        lce.iter().map(|v| Some(v.as_slice())).chain(std::iter::once(None)).collect();
    let derived = hostfacts::derive_lce_pce_masks(&replies).expect("four LCEs answer");
    assert_eq!(derived, kf_abi::cepce::GA106_LCE_PCE_MASKS);
    assert_eq!(derived, ga106::host_facts().lce_pce_masks);
}

#[test]
fn a_host_that_refuses_lce0_is_a_named_refusal_not_an_empty_list() {
    assert_eq!(hostfacts::derive_lce_pce_masks(&[None]), Err(FactRefusal::NoCopyEngine));
}

/// `fmb_real_ga106.txt`: `0x20802a08 -> status=0x0 params.size=20480`.
#[test]
fn the_fault_method_buffer_size_derived_from_the_real_ga106_equals_the_captured_row() {
    let derived = hostfacts::derive_ce_fault_method_buffer_size(&20480u32.to_le_bytes()).expect("nonzero");
    assert_eq!(derived, kf_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE);
    assert_eq!(derived, ga106::host_facts().ce_fault_method_buffer_size);
}

#[test]
fn a_zero_fault_method_buffer_size_is_refused() {
    assert!(matches!(
        hostfacts::derive_ce_fault_method_buffer_size(&[0; 4]),
        Err(FactRefusal::Unservable { cmd: 0x2080_2a08, .. })
    ));
}

/// `rmladder_r18_cecaps_real_ga106.txt`: `0x20802a0a` NV_OK, 136 bytes, present mask `0x0f`.
/// The served chain derives its CE geometry from `engines`; that geometry must name exactly the
/// LCEs the host says are present.
#[test]
fn the_ce_geometry_from_the_engine_rows_names_the_lces_the_real_ga106_reports_present() {
    let mut reply = unhex("e303e303e203e203");
    reply.resize(kf_abi::cecaps::CE_GET_ALL_CAPS_PARAMS_SIZE, 0);
    reply[kf_abi::cecaps::PRESENT_OFF] = 0x0f;
    let host_present = hostfacts::derive_ce_present_mask(&reply).expect("136 bytes");
    let geometry = kf_abi::cecaps::CeGeometry::from_engines(&ga106::host_facts().engines).expect("engines decode");
    assert_eq!(host_present, 0x0f);
    assert_eq!(geometry.present, host_present);
}

/// `cuinit_ioctl_trace_real_ga106.txt`: `0x20803601` out word 0 = `0x00000001`.
#[test]
fn the_gsp_features_derived_from_the_real_ga106_equal_the_captured_row() {
    let out = unhex("0100000001013538302e3135392e3034");
    let derived = hostfacts::derive_gsp_features(&out).expect("known bits");
    assert_eq!(derived, kf_abi::gspfeatures::GspFeatures::GA106);
    assert_eq!(derived, ga106::host_facts().gsp_features);
}

/// `cuinit_ioctl_trace_real_ga106.txt`, `0x20801303` (7-entry and 3-entry requests):
/// `0x1b` L2 = `0x240000`, `0x0d` RAM type = `0x11` (GDDR6), `0x22` LTC count = 6.
#[test]
fn the_memory_geometry_derived_from_the_real_ga106_equals_the_captured_row() {
    let pairs = [
        (0x0b, 0xc0),
        (0x19, 3),
        (0x1b, 0x0024_0000),
        (0x18, 0),
        (0x0d, 0x11),
        (0x17, 0),
        (0x08, 0x00c0_0000),
        (0x1a, 7),
        (0x22, 6),
        (0x23, 0x12),
    ];
    let g = hostfacts::derive_fb_geometry(&pairs).expect("all three indices present");
    let row = ga106::host_facts().memory_system;
    assert_eq!(g.l2_cache_size, row.l2_cache_size);
    assert_eq!(g.ram_type, row.ram_type);
    assert_eq!(g.ltc_count, row.ltc_count);
}

#[test]
fn a_missing_fb_index_is_named() {
    assert_eq!(
        hostfacts::derive_fb_geometry(&[(0x1b, 1), (0x0d, 0x11)]).map(|_| ()),
        Err(FactRefusal::Missing { cmd: 0x2080_1303, index: 0x22 })
    );
}

/// `cuinit_ioctl_trace_real_ga106.txt`: `0x20801701` out = `{0x170, 0x6, 0xa1}` → Ampere.
#[test]
fn the_family_derived_from_the_real_ga106_is_ampere() {
    let out = unhex("7001000006000000a100000000000000");
    assert_eq!(hostfacts::derive_family(&out), Ok(ga106::host_facts().family));
}

/// `cuinit_ioctl_trace_real_ga106.txt`: `0x20800110` and `0x20800111`. ⊘ The old row served NO
/// name (the `nvidia-smi` `ERR!` defect), so there is no captured row to equal — the oracle is
/// the host's own string.
#[test]
fn the_name_derived_from_the_real_ga106_is_the_hosts_own_string() {
    let full = unhex("000000004e5649444941204765466f7263652052545820333036300000");
    let short = unhex("47413130362d410000");
    let n = hostfacts::derive_gpu_name(0x2080_0110, &full, 4).expect("ASCII");
    let sn = hostfacts::derive_gpu_name(0x2080_0111, &short, 0).expect("ASCII");
    assert_eq!(n.as_str(), "NVIDIA GeForce RTX 3060");
    assert_eq!(sn.as_str(), "GA106-A");
    assert!(hostfacts::derive_gpu_name(0x2080_0110, &[0x41; 8], 0).is_err(), "unterminated");
}

/// ★ Every field has exactly one provenance row. The destructure is exhaustive (no `..`), so a
/// new field fails to compile here until it is named — and then fails this test until it has a
/// source.
#[test]
fn every_host_fact_states_where_it_comes_from() {
    let HostFacts {
        family: _,
        has_c2c: _,
        engines: _,
        lce_pce_masks: _,
        intr_table: _,
        intr_subtree_map: _,
        chip_info: _,
        user_register_access_map: _,
        constructed_falcons: _,
        memory_system: _,
        device_info: _,
        conf_compute: _,
        bif_static: _,
        fifo_channels: _,
        gmmu_static: _,
        gr_static: _,
        gr_info: _,
        gr_context_buffers: _,
        gr_zcull_info: _,
        forwarded_gpu_info: _,
        smc_mode: _,
        pcie_max_gen: _,
        ce_fault_method_buffer_size: _,
        gsp_features: _,
        gpu_name: _,
        gpu_short_name: _,
    } = ga106::host_facts();
    let fields = [
        "family", "has_c2c", "engines", "lce_pce_masks", "intr_table", "intr_subtree_map",
        "chip_info", "user_register_access_map", "constructed_falcons", "memory_system",
        "device_info", "conf_compute", "bif_static", "fifo_channels", "gmmu_static", "gr_static",
        "gr_info", "gr_context_buffers", "gr_zcull_info", "forwarded_gpu_info", "smc_mode", "pcie_max_gen",
        "ce_fault_method_buffer_size", "gsp_features", "gpu_name", "gpu_short_name",
    ];
    for f in fields {
        let n = hostfacts::PROVENANCE.iter().filter(|(name, _)| *name == f).count();
        assert_eq!(n, 1, "{f} has {n} provenance rows");
    }
    assert_eq!(hostfacts::PROVENANCE.len(), fields.len(), "a provenance row names no field");
}
