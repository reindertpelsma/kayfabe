//! ★★★ **The video engines (NVENC / NVDEC) a device advertises — every statement of them.**
//!
//! A video engine is stated to the guest in FIVE places, and the guest cross-checks them:
//! the FIFO device-info row (`kfifoEngineInfoXlate`), the kernel interrupt table (the non-stall
//! vector its falcon registers on), `GspStaticConfigInfo.engineCaps[]` (the class-DB filter,
//! `gpuCheckEngineWithOrderList_KERNEL`), `GET_CONSTRUCTED_FALCON_INFO` (the falcon
//! `chandesConstruct` looks up) and the device-info PRI base. These tests pin each against the
//! source it is derived from, and — where one exists — against a real GA106's own answer.
//!
//! The GA106 oracle for the FIFO rows is the C tree's captured 11-row
//! `FIFO_GET_DEVICE_INFO_TABLE` (`C: src/qemu/mode2_initctrl_ga106.h:5054`, `ctl_20801112`),
//! decoded 2026-09-26: `NVENC0 = {ENG_DESC 0xe97b6c00, RM 0x25, fault 0xb, MC 0x26, DEV_TYPE 0xe,
//! INSTANCE 0}` and `NVDEC0 = {0x8f99e100, RM 0x1d, fault 0x19, MC 0x41, DEV_TYPE 0x10, 0}`.
//! Runlist, PBDMA and RESET are ours (the device's topology, `authored::ENGINE_LAYOUT_WHY`).

use kf_abi::falconinfo::{self, ConstructedFalcon, FalconInventoryRow};
use kf_abi::inittables::INTR_VECTOR_INVALID;
use kf_chip::Family;
use kf_rm::authored::{self, EngineKind, slot};
use kf_rm::hostquery::{self, HostControls, HostRefusal};

/// The host engine list a GA106 reports once video is kept: GR0, CE0..3, NVDEC0, NVENC0, SW.
fn ga106_kinds() -> Vec<EngineKind> {
    vec![
        EngineKind::Graphics(0),
        EngineKind::Copy(0),
        EngineKind::Copy(1),
        EngineKind::Copy(2),
        EngineKind::Copy(3),
        EngineKind::VideoDecode(0),
        EngineKind::VideoEncode(0),
        EngineKind::Software,
    ]
}

/// The two GA106 video falcons as the captured table reports them (`falconinfo.rs` table).
fn ga106_video_falcons() -> Vec<ConstructedFalcon> {
    vec![
        ConstructedFalcon { eng_desc: 0x8f99_e100, ctx_attr: 0, ctx_buffer_size: 0x1000, addr_space_list: 1, register_base: 0x0084_8000 },
        ConstructedFalcon { eng_desc: 0xe97b_6c00, ctx_attr: 0, ctx_buffer_size: 0x1000, addr_space_list: 1, register_base: 0x001c_8000 },
    ]
}

#[test]
fn the_video_rows_carry_the_real_ga106s_ogkm_constants() {
    let rows = authored::engine_table(Family::Ampere, &ga106_kinds()).expect("Ampere states every video constant");
    let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("{name} row"));
    let enc = row("NVENC0");
    let dec = row("NVDEC0");
    // ★ Exact values, each against the captured GA106 row.
    assert_eq!(enc.engine_data[slot::ENG_DESC], 0xe97b_6c00);
    assert_eq!(enc.engine_data[slot::RM_ENGINE_TYPE], 0x25);
    assert_eq!(enc.engine_data[slot::MMU_FAULT_ID], 0xb);
    assert_eq!(enc.engine_data[slot::MC], 0x26);
    assert_eq!(enc.engine_data[slot::DEV_TYPE_ENUM], 0xe);
    assert_eq!(enc.engine_data[slot::INSTANCE_ID], 0);
    assert_eq!(enc.engine_data[slot::IS_HOST_DRIVEN_ENGINE], 1);
    assert_eq!(dec.engine_data[slot::ENG_DESC], 0x8f99_e100);
    assert_eq!(dec.engine_data[slot::RM_ENGINE_TYPE], 0x1d);
    assert_eq!(dec.engine_data[slot::MMU_FAULT_ID], 0x19);
    assert_eq!(dec.engine_data[slot::MC], 0x41);
    assert_eq!(dec.engine_data[slot::DEV_TYPE_ENUM], 0x10);
    assert_eq!(dec.engine_data[slot::IS_HOST_DRIVEN_ENGINE], 1);
    // ★ Their own runlists, distinct from every other row's, each with one PBDMA.
    let runlists: Vec<u32> = rows.iter().filter(|r| r.engine_data[slot::IS_HOST_DRIVEN_ENGINE] == 1).map(|r| r.engine_data[slot::RUNLIST]).collect();
    for v in [enc, dec] {
        let rl = v.engine_data[slot::RUNLIST];
        assert_ne!(rl, 0, "{}: not on GR's runlist", v.name);
        assert_eq!(runlists.iter().filter(|r| **r == rl).count(), 1, "{}: runlist {rl} is its own", v.name);
        assert_eq!(v.num_pbdmas, 1);
        assert_eq!(v.pbdma_ids[0], rl + 1);
    }
    // ★ RESET bits are one 32-bit word and never collide.
    let mut resets: Vec<u32> = rows.iter().filter(|r| r.engine_data[slot::IS_HOST_DRIVEN_ENGINE] == 1).map(|r| r.engine_data[slot::RESET]).collect();
    assert!(resets.iter().all(|r| *r < 32));
    resets.sort_unstable();
    let n = resets.len();
    resets.dedup();
    assert_eq!(resets.len(), n, "RESET bits collide: {resets:?}");
}

#[test]
fn the_video_fault_ids_follow_each_familys_dev_fault_h() {
    let kinds = |enc: u32, dec: u32| {
        let mut k = vec![EngineKind::Graphics(0), EngineKind::Copy(0)];
        k.extend((0..enc).map(EngineKind::VideoEncode));
        k.extend((0..dec).map(EngineKind::VideoDecode));
        k
    };
    let fault = |rows: &[kf_abi::inittables::FifoDeviceEntry], name: &str| {
        rows.iter().find(|r| r.name == name).map(|r| r.engine_data[slot::MMU_FAULT_ID])
    };
    // Turing: NVDEC0 = 10, NVDEC1 = 25 — NOT contiguous (turing/tu102/dev_fault.h).
    let t = authored::engine_table(Family::Turing, &kinds(1, 2)).expect("Turing");
    assert_eq!((fault(&t, "NVENC0"), fault(&t, "NVDEC0"), fault(&t, "NVDEC1")), (Some(11), Some(10), Some(25)));
    // Ada: three encoders, four decoders (AD102 class list).
    let a = authored::engine_table(Family::Ada, &kinds(3, 4)).expect("Ada");
    assert_eq!((fault(&a, "NVENC2"), fault(&a, "NVDEC3")), (Some(13), Some(28)));
    // Blackwell: NVENC 44.., NVDEC 28.. (blackwell/gb202/dev_fault.h).
    let b = authored::engine_table(Family::Blackwell, &kinds(4, 4)).expect("Blackwell");
    assert_eq!((fault(&b, "NVENC3"), fault(&b, "NVDEC0")), (Some(47), Some(28)));
    // ⊘ An instance the family's header does not name is refused by name, never guessed.
    let e = authored::engine_table(Family::Ampere, &kinds(4, 0)).expect_err("Ampere names no NVENC3");
    assert!(e.missing.contains("NVENC"), "{e:?}");
}

#[test]
fn each_video_engine_notifies_on_its_own_vector_and_never_stalls() {
    let kinds = ga106_kinds();
    let table = authored::engine_notification_rows(&kinds);
    let enc = authored::non_stall_vector_for(&table, EngineKind::VideoEncode(0)).expect("NVENC0 has a vector");
    let dec = authored::non_stall_vector_for(&table, EngineKind::VideoDecode(0)).expect("NVDEC0 has a vector");
    assert_ne!(enc, dec);
    let rows = authored::engine_table(Family::Ampere, &kinds).expect("Ampere");
    // ★ The vector IS the engine's runlist number — the same rule, the same order, as the table.
    for (name, v) in [("NVENC0", enc), ("NVDEC0", dec)] {
        let r = rows.iter().find(|r| r.name == name).expect("row");
        assert_eq!(r.engine_data[slot::RUNLIST], v, "{name}");
    }
    for e in &table {
        if [38u16, 65].contains(&e.engine_idx) {
            assert_eq!(e.vector_stall, INTR_VECTOR_INVALID, "a video engine's stall interrupt is the GSP's");
        }
    }
    // Every vector distinct.
    let mut vs: Vec<u32> = table.iter().map(|e| e.vector_non_stall).collect();
    let n = vs.len();
    vs.sort_unstable();
    vs.dedup();
    assert_eq!(vs.len(), n);
    // ⊘ A decoder the table does not list has no vector — nothing is raised for it.
    assert_eq!(authored::non_stall_vector_for(&table, EngineKind::VideoDecode(1)), None);
}

#[test]
fn engine_caps_sets_the_nv2080_bits_of_exactly_the_served_engines() {
    let rows = authored::engine_table(Family::Ampere, &ga106_kinds()).expect("Ampere");
    let caps = authored::engine_caps(&rows);
    let bit = |t: u32| caps[t as usize / 32] & (1 << (t % 32)) != 0;
    // NV2080 space: GR0 1, COPY0..3 9..12, NVDEC0 0x13 (= _BSP), NVENC0 0x1b (= _MSENC).
    for t in [0x01, 0x09, 0x0a, 0x0b, 0x0c, 0x13, 0x1b] {
        assert!(bit(t), "bit {t:#x}");
    }
    // ⊘ Not the RM-space numbers (NVDEC0 = RM 0x1d is NV2080 NVENC2; NVENC0 = RM 0x25), and no SW.
    for t in [0x1d, 0x25, 0x22, 0x2d, 0x14, 0x1c] {
        assert!(!bit(t), "bit {t:#x} must be clear");
    }
    assert_eq!(caps.iter().map(|w| w.count_ones()).sum::<u32>(), 7);
    // A table without video sets no video bit — the pre-video device, byte for byte.
    let old = authored::engine_table(Family::Ampere, &ga106_kinds()[..5]).expect("Ampere");
    let c = authored::engine_caps(&old);
    assert_eq!(c[0] & (1 << 0x13 | 1 << 0x1b), 0);
}

#[test]
fn classify_engine_names_both_spaces_of_every_video_engine() {
    use kf_rm::hostquery::classify_engine;
    assert_eq!(classify_engine(0x13), Some((EngineKind::VideoDecode(0), 0x1d)));
    assert_eq!(classify_engine(0x1a), Some((EngineKind::VideoDecode(7), 0x24)));
    assert_eq!(classify_engine(0x1b), Some((EngineKind::VideoEncode(0), 0x25)));
    assert_eq!(classify_engine(0x1d), Some((EngineKind::VideoEncode(2), 0x27)), "NV2080 0x1d is NVENC2");
    assert_eq!(classify_engine(0x3f), Some((EngineKind::VideoEncode(3), 0x28)), "NVENC3 is out of line");
    // ⊘ Still NOT advertised: NVJPG (0x2b), OFA (0x33), SEC2 (0x1e).
    for t in [0x2b, 0x33, 0x1e] {
        assert_eq!(classify_engine(t), None, "{t:#x}");
    }
}

/// A host that answers only the falcon table.
struct FalconHost(Option<Vec<u8>>);
impl HostControls for FalconHost {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        match (&self.0, cmd) {
            (Some(r), falconinfo::NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO) => {
                params.copy_from_slice(r);
                Ok(())
            }
            _ => Err(HostRefusal { status: Some(0x56), detail: "not captured".into() }),
        }
    }
}

/// The host's falcon table as the C captured it on a GA106: FBFLCN, FECS, GPCCS, NVDEC0, PMU,
/// NVENC0, SEC2, OFA (`falconinfo.rs` module table).
fn ga106_host_falcon_reply() -> Vec<u8> {
    let all = [
        (0x8a20_bf00u32, 0x1000u32, 0x009a_4000u32),
        (0x5ee8_dc00, 0x1000, 0x0040_9000),
        (0x4781_e800, 0x1000, 0x0041_a000),
        (0x8f99_e100, 0x1000, 0x0084_8000),
        (0xf3d7_2200, 0x1000, 0x0010_a000),
        (0xe97b_6c00, 0x1000, 0x001c_8000),
        (0x28c4_0800, 0x10000, 0x0084_0000),
        (0xdd7b_ab00, 0x1000, 0x0084_4000),
    ];
    let falcons: Vec<ConstructedFalcon> = all
        .iter()
        .map(|&(eng_desc, ctx_buffer_size, register_base)| ConstructedFalcon { eng_desc, ctx_attr: 0, ctx_buffer_size, addr_space_list: 1, register_base })
        .collect();
    let row = FalconInventoryRow { falcons: Box::leak(falcons.into_boxed_slice()) };
    falconinfo::encode_constructed_falcon_info(&row, 16 << 20).expect("encodes")
}

#[test]
fn only_the_advertised_video_falcons_are_kept_from_the_hosts_table() {
    let mut host = FalconHost(Some(ga106_host_falcon_reply()));
    let got = hostquery::query_video_falcons(&mut host, &ga106_kinds()).expect("answered");
    assert_eq!(got, ga106_video_falcons(), "NVDEC0 and NVENC0 only, in the host's order");
    // ⊘ FECS/GPCCS/PMU/SEC2/OFA are never constructed: a named SEC2 would register an interrupt
    // service keyed by an engine the device does not list (`kernel_falcon.c:362-397`).
    let only_dec = [EngineKind::Graphics(0), EngineKind::VideoDecode(0)];
    let got = hostquery::query_video_falcons(&mut host, &only_dec).expect("answered");
    assert_eq!(got.iter().map(|f| f.eng_desc).collect::<Vec<_>>(), [0x8f99_e100]);
    // A refused host table is the host's refusal, by name.
    assert!(hostquery::query_video_falcons(&mut FalconHost(None), &ga106_kinds()).is_err());
    // And the reply round-trips through the guest's encoder byte for byte.
    let row = FalconInventoryRow { falcons: Box::leak(ga106_video_falcons().into_boxed_slice()) };
    let bytes = falconinfo::encode_constructed_falcon_info(&row, 16 << 20).expect("in BAR0");
    assert_eq!(falconinfo::decode_constructed_falcon_info(&bytes).expect("decodes"), ga106_video_falcons());
    assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 2);
}

#[test]
fn a_video_engines_pri_base_is_its_hosts_falcon_base() {
    use kf_abi::deviceinfo::DevicePriBase;
    let row = hostquery::device_info_rule(&ga106_kinds(), &ga106_video_falcons());
    let base = |n: &str| row.pri_bases.iter().find(|r| r.engine == n).map(|r| r.pri_base);
    assert_eq!(base("NVENC0"), Some(DevicePriBase::At(0x001c_8000)));
    assert_eq!(base("NVDEC0"), Some(DevicePriBase::At(0x0084_8000)));
    assert_eq!(base("GR0"), Some(DevicePriBase::At(0x0040_0000)));
}

#[test]
fn the_class_sets_are_derived_hopper_has_no_encoder() {
    use kf_chip::classes::{Kind, classes_for};
    assert_eq!(classes_for(Family::Ampere).kind_of(0xc7b7), Some(Kind::VideoEncoder));
    assert_eq!(classes_for(Family::Ampere).kind_of(0xc7b0), Some(Kind::VideoDecoder));
    assert_eq!(classes_for(Family::Turing).kind_of(0xc4b7), Some(Kind::VideoEncoder));
    assert_eq!(classes_for(Family::Ada).kind_of(0xc9b7), Some(Kind::VideoEncoder));
    assert_eq!(classes_for(Family::Blackwell).kind_of(0xcfb0), Some(Kind::VideoDecoder));
    // ⊘ GH100 lists NVDEC and no NVENC (`g_gpu_class_list.c`): derived, never assumed.
    assert!(classes_for(Family::Hopper).video_encoder.is_empty());
    assert_eq!(classes_for(Family::Hopper).kind_of(0xb8b0), Some(Kind::VideoDecoder));
}
