//! ★★★ **The optical-flow engine (OFA) a device advertises — v3-gfxset, 2026-09-26.**
//!
//! `[measured gs1, RTX 3070 GA104, kf3 ce20926e]` a kf3 guest's Vulkan driver advertised every
//! extension bare metal does EXCEPT `VK_NV_optical_flow`, and every queue family except the sixth
//! (`VK_QUEUE_OPTICAL_FLOW_BIT_NV`): nvkvm-pv's "RDR2 check" (`vkCreateDevice` with seven RT/NVX
//! extensions) failed on it. The engine was never stated: `classify_engine` dropped `OFA0`.
//! OFA is stated in the same five places as NVENC/NVDEC (`tests/video_engines.rs`), and these tests
//! pin each against its source and against a real GA106's own answer — the C tree's captured
//! `FIFO_GET_DEVICE_INFO_TABLE` (`C: src/qemu/mode2_initctrl_ga106.h:5054`, `ctl_20801112`), row 9:
//! `OFA = {ENG_DESC 0xdd7bab00, RM 0x3e, fault 0xa, MC 0x51, DEV_TYPE 0x16, INSTANCE 0}`.
//! Runlist, PBDMA and RESET are ours (the device's topology, `authored::ENGINE_LAYOUT_WHY`).

use kf_abi::falconinfo::ConstructedFalcon;
use kf_abi::inittables::INTR_VECTOR_INVALID;
use kf_chip::Family;
use kf_rm::authored::{self, EngineKind, slot};
use kf_rm::hostquery;

const GRCE: u64 = kf_abi::cecaps::GA10X_GRCE_LCE_MASK;

/// A GA106's host engine list with the video engines and OFA: GR0, CE0..3, NVDEC0, NVENC0, OFA0, SW.
fn ga106_kinds() -> Vec<EngineKind> {
    vec![
        EngineKind::Graphics(0),
        EngineKind::Copy(0),
        EngineKind::Copy(1),
        EngineKind::Copy(2),
        EngineKind::Copy(3),
        EngineKind::VideoDecode(0),
        EngineKind::VideoEncode(0),
        EngineKind::OpticalFlow(0),
        EngineKind::Software,
    ]
}

/// The three GA106 video-class falcons as the captured table reports them (`falconinfo.rs` table).
fn ga106_falcons() -> Vec<ConstructedFalcon> {
    vec![
        ConstructedFalcon { eng_desc: 0x8f99_e100, ctx_attr: 0, ctx_buffer_size: 0x1000, addr_space_list: 1, register_base: 0x0084_8000 },
        ConstructedFalcon { eng_desc: 0xe97b_6c00, ctx_attr: 0, ctx_buffer_size: 0x1000, addr_space_list: 1, register_base: 0x001c_8000 },
        ConstructedFalcon { eng_desc: 0xdd7b_ab00, ctx_attr: 0, ctx_buffer_size: 0x1000, addr_space_list: 1, register_base: 0x0084_4000 },
    ]
}

#[test]
fn the_ofa_row_carries_the_real_ga106s_ogkm_constants() {
    let rows = authored::engine_table(Family::Ampere, &ga106_kinds(), GRCE).expect("Ampere states OFA0");
    let ofa = rows.iter().find(|r| r.name == "OFA").expect("an `OFA` row (the captured table's own name)");
    // ★ Exact values, each against the captured GA106 row 9.
    assert_eq!(ofa.engine_data[slot::ENG_DESC], 0xdd7b_ab00);
    assert_eq!(ofa.engine_data[slot::RM_ENGINE_TYPE], 0x3e);
    assert_eq!(ofa.engine_data[slot::MMU_FAULT_ID], 0xa);
    assert_eq!(ofa.engine_data[slot::MC], 0x51);
    assert_eq!(ofa.engine_data[slot::DEV_TYPE_ENUM], 0x16);
    assert_eq!(ofa.engine_data[slot::INSTANCE_ID], 0);
    assert_eq!(ofa.engine_data[slot::IS_HOST_DRIVEN_ENGINE], 1);
    // ★ Its own runlist (as the video engines'), one PBDMA, a RESET bit nobody else holds.
    let host_driven: Vec<_> = rows.iter().filter(|r| r.engine_data[slot::IS_HOST_DRIVEN_ENGINE] == 1).collect();
    let rl = ofa.engine_data[slot::RUNLIST];
    assert_ne!(rl, 0);
    assert_eq!(host_driven.iter().filter(|r| r.engine_data[slot::RUNLIST] == rl).count(), 1);
    assert_eq!((ofa.num_pbdmas, ofa.pbdma_ids[0]), (1, rl + 1));
    let mut resets: Vec<u32> = host_driven.iter().map(|r| r.engine_data[slot::RESET]).collect();
    assert!(resets.iter().all(|r| *r < 32));
    resets.sort_unstable();
    let n = resets.len();
    resets.dedup();
    assert_eq!(resets.len(), n, "RESET bits collide");
}

#[test]
fn the_ofa_fault_id_follows_each_familys_dev_fault_h() {
    let kinds = [EngineKind::Graphics(0), EngineKind::Copy(0), EngineKind::OpticalFlow(0)];
    let fault = |f: Family| {
        authored::engine_table(f, &kinds, GRCE).ok().and_then(|rows| rows.iter().find(|r| r.name == "OFA").map(|r| r.engine_data[slot::MMU_FAULT_ID]))
    };
    assert_eq!(fault(Family::Ampere), Some(10), "ampere/ga100/dev_fault.h:57");
    assert_eq!(fault(Family::Ada), Some(10), "ada/ad102/dev_fault.h:56");
    assert_eq!(fault(Family::Blackwell), Some(48), "blackwell/gb202/dev_fault.h:100");
    assert_eq!(fault(Family::Hopper), Some(53), "kernel-open/nvidia-uvm/hwref/hopper/gh100/dev_fault.h:81");
    // ⊘ Turing states no OFA fault id: the whole table is refused by name if one is forced in —
    // hostquery never forces it (`an_unstatable_ofa_is_dropped_not_the_device`, below).
    let e = authored::engine_table(Family::Turing, &kinds, GRCE).expect_err("Turing names no OFA");
    assert!(e.missing.contains("OFA"), "{e:?}");
}

#[test]
fn ofa_notifies_on_its_own_vector_and_never_stalls() {
    let kinds = ga106_kinds();
    let table = authored::engine_notification_rows(&kinds, GRCE);
    let v = authored::non_stall_vector_for(&table, EngineKind::OpticalFlow(0)).expect("OFA0 has a vector");
    let rows = authored::engine_table(Family::Ampere, &kinds, GRCE).expect("Ampere");
    let r = rows.iter().find(|r| r.name == "OFA").expect("row");
    // ★ The vector IS the engine's runlist number — the same rule as every async engine.
    assert_eq!(r.engine_data[slot::RUNLIST], v);
    let e = table.iter().find(|e| e.engine_idx == 81).expect("an MC_ENGINE_IDX_OFA0 row");
    assert_eq!(e.vector_stall, INTR_VECTOR_INVALID, "OFA's stall interrupt is the GSP's");
    let mut vs: Vec<u32> = table.iter().map(|e| e.vector_non_stall).collect();
    let n = vs.len();
    vs.sort_unstable();
    vs.dedup();
    assert_eq!(vs.len(), n, "every vector distinct");
    assert_eq!(authored::non_stall_vector_for(&table, EngineKind::OpticalFlow(1)), None, "OFA1 not listed ⇒ no vector");
}

#[test]
fn engine_caps_sets_the_nv2080_ofa_bit_not_the_rm_one() {
    let rows = authored::engine_table(Family::Ampere, &ga106_kinds(), GRCE).expect("Ampere");
    let caps = authored::engine_caps(&rows);
    let bit = |t: u32| caps[t as usize / 32] & (1 << (t % 32)) != 0;
    // ★ NV2080 OFA0 = 0x33. ⊘ RM OFA0 = 0x3e is NV2080 OFA1 — must stay clear.
    assert!(bit(0x33), "NV2080_ENGINE_TYPE_OFA0");
    assert!(!bit(0x3e), "0x3e is RM OFA0 = NV2080 OFA1: the spaces were not converted");
    assert_eq!(caps.iter().map(|w| w.count_ones()).sum::<u32>(), 8, "GR0, COPY0..3, NVDEC0, NVENC0, OFA0");
}

#[test]
fn classify_engine_names_both_spaces_of_ofa() {
    use kf_rm::hostquery::classify_engine;
    assert_eq!(classify_engine(0x33), Some((EngineKind::OpticalFlow(0), 0x3e)));
    assert_eq!(classify_engine(0x3e), Some((EngineKind::OpticalFlow(1), 0x3f)), "NV2080 0x3e is OFA1, out of line");
    assert_eq!(kf_abi::submit::engine_type_ofa(1), Some(0x3e));
    assert!(kf_abi::submit::is_video_engine_type(0x33), "a passthrough twin + a promote satisfied by the twin");
}

#[test]
fn the_ofa_falcon_is_kept_and_its_pri_base_is_the_hosts() {
    use kf_abi::deviceinfo::DevicePriBase;
    assert_eq!(hostquery::video_eng_desc(EngineKind::OpticalFlow(0)), Some(0xdd7b_ab00), "ENG_OFA(0) = OBJOFA << 8");
    let row = hostquery::device_info_rule(&ga106_kinds(), &ga106_falcons());
    let base = |n: &str| row.pri_bases.iter().find(|r| r.engine == n).map(|r| r.pri_base);
    assert_eq!(base("OFA"), Some(DevicePriBase::At(0x0084_4000)), "the host falcon table's OFA registerBase");
    // ⊘ No falcon ⇒ no PRI base invented.
    let row = hostquery::device_info_rule(&ga106_kinds(), &ga106_falcons()[..2]);
    assert_eq!(row.pri_bases.iter().find(|r| r.engine == "OFA").map(|r| r.pri_base), Some(DevicePriBase::NotADevice));
}

#[test]
fn the_ofa_class_sets_are_derived() {
    use kf_chip::classes::{Kind, classes_for};
    assert_eq!(classes_for(Family::Ampere).kind_of(0xc7fa), Some(Kind::OpticalFlow), "NVC7FA_VIDEO_OFA (GA10x)");
    assert_eq!(classes_for(Family::Ampere).kind_of(0xc6fa), Some(Kind::OpticalFlow), "NVC6FA_VIDEO_OFA (GA100)");
    assert_eq!(classes_for(Family::Ada).kind_of(0xc9fa), Some(Kind::OpticalFlow));
    assert_eq!(classes_for(Family::Blackwell).kind_of(0xcdfa), Some(Kind::OpticalFlow));
    assert!(classes_for(Family::Turing).optical_flow.is_empty());
}

#[test]
fn an_unstatable_ofa_is_dropped_not_the_device() {
    let with = |f: Family, n: u32| {
        let mut k = vec![EngineKind::Graphics(0), EngineKind::Copy(0)];
        k.extend((0..n).map(EngineKind::OpticalFlow));
        hostquery::keep_statable_ofa(f, k)
    };
    // GB100 lists two OFAs; only OFA0's fault id is in any header ⇒ OFA1 is not advertised, and the
    // table over what remains is laid out, not refused.
    let b = with(Family::Blackwell, 2);
    assert_eq!(b, [EngineKind::Graphics(0), EngineKind::Copy(0), EngineKind::OpticalFlow(0)]);
    assert!(authored::engine_table(Family::Blackwell, &b, GRCE).is_ok());
    // A Turing host listing an OFA keeps its GR and CE, loses nothing else.
    let t = with(Family::Turing, 1);
    assert_eq!(t, [EngineKind::Graphics(0), EngineKind::Copy(0)]);
    assert!(authored::engine_table(Family::Turing, &t, GRCE).is_ok());
    assert_eq!(with(Family::Ampere, 1).len(), 3, "Ampere states OFA0");
}

#[test]
fn software_is_the_last_row_whatever_the_hosts_order() {
    // ★ The host's GET_ENGINES_V2 order on a GA10x (measured diag2): SW (0x22) BEFORE OFA0 (0x33).
    let host_order = vec![
        EngineKind::Graphics(0),
        EngineKind::Copy(0),
        EngineKind::Copy(1),
        EngineKind::Copy(2),
        EngineKind::Copy(3),
        EngineKind::VideoDecode(0),
        EngineKind::VideoEncode(0),
        EngineKind::Software,
        EngineKind::OpticalFlow(0),
    ];
    let k = hostquery::software_last(host_order);
    assert_eq!(k.last(), Some(&EngineKind::Software), "RM counts engineInfoListSize-1 engines: SW must be last");
    assert_eq!(k.iter().filter(|x| **x == EngineKind::Software).count(), 1);
    let rows = authored::engine_table(Family::Ampere, &k, GRCE).expect("Ampere");
    assert_eq!(rows.last().map(|r| r.name), Some("SOFTWARE"));
    // every row RM counts (all but the last) is host-driven — nothing it walks is a non-host engine
    assert!(rows[..rows.len() - 1].iter().all(|r| r.engine_data[slot::IS_HOST_DRIVEN_ENGINE] == 1));
    // the served order is otherwise the host's: OFA keeps its place after NVENC0
    let names: Vec<&str> = rows.iter().map(|r| r.name).collect();
    assert_eq!(names, ["GR0", "CE0", "CE1", "CE2", "CE3", "NVDEC0", "NVENC0", "OFA", "SOFTWARE"]);
}
