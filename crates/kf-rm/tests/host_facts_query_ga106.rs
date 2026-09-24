//! ★★★★★ **The host-query half, run over a REAL GA106's own replies** (`kf_rm::hostquery`).
//!
//! `tests/host_facts_ga106.rs` checks the `derive_*` functions on hand-copied reply bytes. This
//! file checks the WHOLE request path a device runs at realize — the same
//! [`kf_rm::hostquery`] code `kf-qemu`'s `rmfacts` drives over the real session — against a
//! [`HostControls`] that answers ONLY from what a real GA106 (RTX 3060, 580.159.04, unprivileged
//! ioctls) actually replied, read at test time from `traces/real_ga106/`:
//!
//! | file | what it answers |
//! |---|---|
//! | `cuinit_ioctl_trace_real_ga106.txt` | every `CTRL` libcuda issued in `cuInit`, reply bytes verbatim |
//! | `rmladder_r21_gpuinfo_sweep_real_ga106.txt` | `GPU_GET_INFO_V2`, one index per call, all 70 |
//! | `rmladder_r22_businfo_sweep_real_ga106.txt` | `BUS_GET_INFO_V2`, one index per call |
//! | `rmladder_r24_pcemask_real_ga106.txt` | `CE_GET_CE_PCE_MASK` for LCE0..4 (LCE4 refused) |
//!
//! ⊘ A control no capture holds is REFUSED by the replay (`status: None`, "no capture") — never
//! answered with a plausible body. So the whole-struct query on this replay cannot succeed, and
//! the first test states exactly which fields fail and why: that list IS the capture gap plus
//! the provenance gap, measured rather than asserted.
//!
//! ★ Every field the captures DO cover is then asserted equal to the old tree's captured GA106
//! row (`support/ga106.rs`) — *"on a GA106 host, derived must equal captured"*. Where a field
//! has no capture, the test says so and checks only what can be checked without one.

#[path = "support/ga106.rs"]
mod ga106;

use std::collections::BTreeMap;

use kf_chip::Family;
use kf_rm::hostfacts::{self, PROVENANCE, Source};
use kf_rm::hostquery::{self, FieldCause, HostControls, HostRefusal};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex")).collect()
}

fn trace(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../traces/real_ga106").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(key))
}

fn hexword(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex word")
}

/// A sweep line — `R21 0x2a  NV_OK  data=0x…` or `R21 0x05  refused  Other(86)` — as
/// `(index, Some(data) | None)`.
fn sweep(text: &str, tag: &str) -> BTreeMap<u32, Option<u32>> {
    let mut out = BTreeMap::new();
    for l in text.lines() {
        let mut w = l.split_whitespace().skip_while(|w| *w != tag).skip(1);
        let (Some(idx), Some(status)) = (w.next(), w.next()) else { continue };
        if !idx.starts_with("0x") || idx.len() != 4 {
            continue;
        }
        let v = match status {
            "NV_OK" => Some(hexword(field(l, "data=").expect("data="))),
            "refused" => None,
            _ => continue,
        };
        out.entry(hexword(idx)).or_insert(v);
    }
    out
}

/// ★ The replay: a [`HostControls`] that knows only what a real GA106 said.
struct Ga106Replay {
    /// `cmd → [(in, status, out)]`, from the `cuInit` trace.
    ctrls: BTreeMap<u32, Vec<(Vec<u8>, u32, Vec<u8>)>>,
    /// `GPU_GET_INFO_V2` index → value (`None` = refused).
    gpu_info: BTreeMap<u32, Option<u32>>,
    /// `BUS_GET_INFO_V2` index → value.
    bus_info: BTreeMap<u32, Option<u32>>,
    /// `NV2080_ENGINE_TYPE_COPY(i)` → PCE mask (`None` = refused).
    pce: BTreeMap<u32, Option<u32>>,
    /// Every control asked, in order — so a test can say what the query issued.
    asked: Vec<u32>,
}

/// A status no RM returns, marking a capture the interposer truncated.
const TRUNCATED: u32 = u32::MAX;

/// The controls whose params carry no `[IN]` field this replay needs to match: any capture of
/// the command answers them (libcuda seeds `GET_ENGINES_V2`'s count with `0x54`; RM overwrites it).
const OUT_ONLY: &[u32] = &[0x2080_1701, 0x2080_182b, 0x2080_0170, 0x2080_0110, 0x2080_0111, 0x2080_3601, 0x2080_122a, 0x2080_1227, 0x2080_121b];

impl Ga106Replay {
    fn load() -> Ga106Replay {
        let mut ctrls: BTreeMap<u32, Vec<(Vec<u8>, u32, Vec<u8>)>> = BTreeMap::new();
        for l in trace("cuinit_ioctl_trace_real_ga106.txt").lines().filter(|l| l.starts_with("CTRL ")) {
            let (Some(cmd), Some(st), Some(i), Some(o)) = (field(l, "cmd="), field(l, "status="), field(l, "in="), field(l, "out=")) else {
                continue;
            };
            // ⊘ The interposer keeps 4096 bytes and marks the rest `..TRUNC`. A truncated reply
            // is NOT a reply (`dlen < psize` is the oracle's dangerous row): it is kept only so
            // the replay can refuse it by name instead of treating the command as uncaptured.
            let truncated = o.ends_with("..TRUNC") || i.ends_with("..TRUNC");
            let bytes = |s: &str| if s == "-" { Vec::new() } else { unhex(s.trim_end_matches("..TRUNC")) };
            let status = if truncated { TRUNCATED } else { hexword(st) };
            ctrls.entry(hexword(cmd)).or_default().push((bytes(i), status, bytes(o)));
        }
        let mut pce = BTreeMap::new();
        for l in trace("rmladder_r24_pcemask_real_ga106.txt").lines().filter(|l| l.contains("R24 LCE")) {
            let Some(t) = l.split("(type ").nth(1).and_then(|s| s.split(')').next()) else { continue };
            let v = field(l, "pceMask=").map(hexword);
            pce.insert(hexword(t), v);
        }
        Ga106Replay {
            ctrls,
            gpu_info: sweep(&trace("rmladder_r21_gpuinfo_sweep_real_ga106.txt"), "R21"),
            bus_info: sweep(&trace("rmladder_r22_businfo_sweep_real_ga106.txt"), "R22"),
            pce,
            asked: Vec::new(),
        }
    }

    fn no_capture(cmd: u32) -> HostRefusal {
        HostRefusal { status: None, detail: format!("no real-GA106 capture holds {cmd:#010x}") }
    }

    /// `*_GET_INFO_V2` answered index by index from a one-index-per-call sweep.
    fn info_list(cmd: u32, table: &BTreeMap<u32, Option<u32>>, params: &mut [u8]) -> Result<(), HostRefusal> {
        let n = u32::from_le_bytes(params[0..4].try_into().expect("4")) as usize;
        for i in 0..n {
            let at = 4 + 8 * i;
            let index = u32::from_le_bytes(params[at..at + 4].try_into().expect("4"));
            match table.get(&index) {
                Some(Some(v)) => params[at + 4..at + 8].copy_from_slice(&v.to_le_bytes()),
                Some(None) => return Err(HostRefusal { status: Some(0x56), detail: format!("{cmd:#x}[{index:#x}] refused in the sweep") }),
                None => return Err(Self::no_capture(cmd)),
            }
        }
        Ok(())
    }
}

impl HostControls for Ga106Replay {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        self.asked.push(cmd);
        match cmd {
            0x2080_0102 => return Self::info_list(cmd, &self.gpu_info, params),
            0x2080_1823 => return Self::info_list(cmd, &self.bus_info, params),
            0x2080_2a02 => {
                let t = u32::from_le_bytes(params[0..4].try_into().expect("4"));
                return match self.pce.get(&t) {
                    Some(Some(m)) => {
                        params[4..8].copy_from_slice(&m.to_le_bytes());
                        Ok(())
                    }
                    Some(None) => Err(HostRefusal { status: Some(0x56), detail: "LCE refused in R24".into() }),
                    None => Err(Self::no_capture(cmd)),
                };
            }
            0x2080_1303 => {
                // FB_GET_INFO_V2: every captured reply's pairs, index-wise.
                let mut known: BTreeMap<u32, Option<u32>> = BTreeMap::new();
                for (_, st, out) in self.ctrls.get(&cmd).into_iter().flatten() {
                    if *st == 0 {
                        for (i, d) in kf_abi::fbinfo::decode_fb_info_pairs(out).expect("captured reply decodes") {
                            known.insert(i, Some(d));
                        }
                    }
                }
                return Self::info_list(cmd, &known, params);
            }
            _ => {}
        }
        let caps = self.ctrls.get(&cmd).ok_or_else(|| Self::no_capture(cmd))?;
        let hit = caps
            .iter()
            .find(|(i, _, _)| i.as_slice() == &*params)
            .or_else(|| OUT_ONLY.contains(&cmd).then(|| &caps[0]))
            .ok_or_else(|| Self::no_capture(cmd))?;
        if hit.1 == TRUNCATED {
            return Err(HostRefusal { status: None, detail: format!("the only capture of {cmd:#010x} is truncated at 4096 bytes") });
        }
        if hit.1 != 0 {
            return Err(HostRefusal { status: Some(hit.1), detail: "captured refusal".into() });
        }
        if hit.2.len() != params.len() {
            return Err(HostRefusal { status: None, detail: format!("capture is {} bytes, request {}", hit.2.len(), params.len()) });
        }
        params.copy_from_slice(&hit.2);
        Ok(())
    }
}

// =====================================================================================
// The whole struct
// =====================================================================================

/// ★★★ The query over the real GA106 refuses EXACTLY these fields — and each for the stated
/// reason. Every other field is filled from a real reply (checked equal to the fixture below).
#[test]
fn the_whole_query_over_the_real_ga106_refuses_exactly_the_uncaptured_and_the_unsourced() {
    let mut host = Ga106Replay::load();
    let refused = hostquery::query_host_facts(&mut host, Family::Ampere).expect_err("some fields have no capture or no source");
    let by_field: BTreeMap<&str, &FieldCause> = refused.refusals.iter().map(|r| (r.field, &r.cause)).collect();
    let no_capture = |cmd: u32| matches!(by_field.get(hostquery_field_for(cmd)), Some(FieldCause::Host { cmd: c, refused }) if *c == cmd && refused.status.is_none());

    // No capture exists (libcuda's `cuInit` never asks, and no rmladder rung did):
    assert!(no_capture(0x2080_017a), "engines: GET_HW_ENGINE_ID was never captured: {refused}");
    assert!(no_capture(0x2080_170e), "intr_table: MC_GET_STATIC_INTR_TABLE was never captured: {refused}");
    assert!(no_capture(0x2080_170f), "intr_subtree_map: never captured: {refused}");
    assert!(no_capture(0x2080_1228), "gr_info: GR_GET_INFO_V2 was never captured: {refused}");
    assert!(no_capture(0x2080_122d), "gr_context_buffers: never captured: {refused}");
    assert!(no_capture(0x2080_2a08), "ce_fault_method_buffer_size: the only capture is a KERNEL-client probe: {refused}");
    // Refused because a field they are derived from was:
    assert_eq!(by_field.get("device_info"), Some(&&FieldCause::DependsOn("engines")));
    assert_eq!(by_field.get("memory_system"), Some(&&FieldCause::DependsOn("gr_info")));
    // The only SM-order capture is truncated at 4096 of 9240 bytes (`numSm`/`numTpc` lost):
    assert!(
        matches!(by_field.get("gr_static"), Some(FieldCause::Host { cmd: 0x2080_121b, refused }) if refused.detail.contains("truncated")),
        "{refused}"
    );
    // ⊘ No source at all — this one fails on EVERY host, captured or not:
    assert!(matches!(by_field.get("gmmu_static"), Some(FieldCause::Unsourced(t)) if t.contains("GSP firmware")));

    let expected = [
        "engines", "intr_table", "intr_subtree_map", "memory_system", "device_info", "gmmu_static", "gr_static",
        "gr_info", "gr_context_buffers", "ce_fault_method_buffer_size",
    ];
    assert_eq!(refused.fields(), expected, "{refused}");
    // ★ Refusals come in PROVENANCE order, so the realize log reads like the table.
    let order: Vec<usize> = refused.fields().iter().map(|f| PROVENANCE.iter().position(|(p, _)| p == f).expect("a provenance row")).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
}

/// Which field a control's refusal lands on, for the assertion above.
fn hostquery_field_for(cmd: u32) -> &'static str {
    match cmd {
        0x2080_017a => "engines",
        0x2080_170e => "intr_table",
        0x2080_170f => "intr_subtree_map",
        0x2080_1228 => "gr_info",
        0x2080_122d => "gr_context_buffers",
        0x2080_2a08 => "ce_fault_method_buffer_size",
        _ => "",
    }
}

/// ★ The two stated gaps refuse the field even on a host that answers everything they ask:
/// `engines` and `gr_static` name the members with no unprivileged source.
#[test]
fn engines_and_gr_static_refuse_the_members_with_no_unprivileged_source_by_name() {
    for (text, members) in [
        (hostquery::ENGINE_SLOTS_UNSOURCED, &["RUNLIST", "RUNLIST_PRI_BASE", "RESET", "INTR", "RC_MASK", "pbdmaIds"][..]),
        (hostquery::GR_STATIC_MEMBERS_UNSOURCED, &["mmu_per_gpc", "num_pes_per_gpc", "tpc_to_pes_map", "zcull_mask", "fecs_record_size"][..]),
    ] {
        for m in members {
            assert!(text.contains(m), "{m} missing from {text}");
        }
    }
}

#[test]
fn a_host_of_another_family_is_refused_by_name() {
    let mut host = Ga106Replay::load();
    let refused = hostquery::query_host_facts(&mut host, Family::Hopper).expect_err("family");
    assert!(refused
        .refusals
        .iter()
        .any(|r| r.field == "family" && r.cause == FieldCause::FamilyMismatch { asked: Family::Hopper, host: Family::Ampere }));
}

/// Every [`Source::Unsourced`] row is refused by the query with the table's own text.
#[test]
fn every_unsourced_provenance_row_is_refused_with_its_own_text() {
    let mut host = Ga106Replay::load();
    let refused = hostquery::query_host_facts(&mut host, Family::Ampere).expect_err("gaps");
    for (name, src) in PROVENANCE {
        if let Source::Unsourced(text) = src {
            let r = refused.refusals.iter().find(|r| r.field == *name).expect("an unsourced field is refused");
            assert_eq!(r.cause, FieldCause::Unsourced(text));
        }
    }
}

// =====================================================================================
// Field by field: derived from the real GA106 == the old captured row
// =====================================================================================

#[test]
fn family_and_sub_revision_equal_the_captured_row() {
    let f = ga106::host_facts();
    let (family, sub) = hostquery::query_arch(&mut Ga106Replay::load()).expect("captured");
    assert_eq!(family, f.family);
    assert_eq!(sub, f.chip_info.chip_sub_rev);
}

#[test]
fn has_c2c_equals_the_captured_row() {
    assert_eq!(hostquery::query_has_c2c(&mut Ga106Replay::load()), Ok(ga106::host_facts().has_c2c));
}

#[test]
fn lce_pce_masks_equal_the_captured_row_and_the_query_stops_at_the_first_refused_lce() {
    let mut host = Ga106Replay::load();
    assert_eq!(hostquery::query_lce_pce_masks(&mut host), Ok(ga106::host_facts().lce_pce_masks));
    assert_eq!(host.asked, [0x2080_2a02; 5], "LCE0..3 answered, LCE4 refused, nothing asked after");
}

/// `chip_info`: the sub-revision (arch info), `isCmpSku` (R21 `0x3c`), and the USERMODE base
/// rule — all three equal the old row.
#[test]
fn chip_info_equals_the_captured_row() {
    let f = ga106::host_facts();
    let c = hostquery::query_chip_info(&mut Ga106Replay::load(), f.chip_info.chip_sub_rev).expect("R21 0x3c");
    assert_eq!(c, f.chip_info);
}

#[test]
fn forwarded_gpu_info_smc_mode_and_pcie_gen_equal_the_captured_rows() {
    let f = ga106::host_facts();
    let mut host = Ga106Replay::load();
    assert_eq!(hostquery::query_forwarded_gpu_info(&mut host), Ok(f.forwarded_gpu_info));
    assert_eq!(hostquery::query_smc_mode(&mut host), Ok(f.smc_mode));
    assert_eq!(hostquery::query_pcie_max_gen(&mut host), Ok(f.pcie_max_gen));
}

#[test]
fn gsp_features_equal_the_captured_row() {
    assert_eq!(hostquery::query_gsp_features(&mut Ga106Replay::load()), Ok(ga106::host_facts().gsp_features));
}

/// ⊘ The old row served NO name (`nvidia-smi`'s `ERR!`), so there is no row to equal; the oracle
/// is the host's own string, now carried end to end through the request path.
#[test]
fn the_names_are_the_real_ga106s_own_strings() {
    let mut host = Ga106Replay::load();
    assert_eq!(hostquery::query_gpu_name(&mut host).expect("captured").as_str(), "NVIDIA GeForce RTX 3060");
    assert_eq!(hostquery::query_gpu_short_name(&mut host).expect("captured").as_str(), "GA106-A");
    assert_eq!(ga106::host_facts().gpu_name, None, "the fixture is faithful to the old defect");
}

/// `memory_system`: L2, RAM type and LTC count come from the captured `FB_GET_INFO_V2`
/// replies. ⚠ `lts_per_ltc_count` comes from `GR_GET_INFO_V2`, which has NO capture — so it is
/// fed the fixture's `gr_info` here, and this test checks the FB half and the authored half, not
/// the slices-per-LTC reading.
#[test]
fn memory_system_equals_the_captured_row() {
    let f = ga106::host_facts();
    let m = hostquery::query_memory_system(&mut Ga106Replay::load(), Some(&f.gr_info)).expect("FB captured");
    assert_eq!(m, f.memory_system);
}

/// ★ `gr_static`'s host-sourced half, from the real replies the query asks for: the GPC mask,
/// each GPC's TPC mask (asked by id, exactly as libcuda asked), and the caps table all equal
/// the old row.
#[test]
fn gpc_mask_tpc_masks_and_caps_equal_the_captured_row() {
    let f = ga106::host_facts();
    let mut host = Ga106Replay::load();
    let mut ask = |cmd: u32, mut p: Vec<u8>| {
        host.control(cmd, &mut p).expect("captured");
        p
    };
    let gpc_mask = hostfacts::derive_gpc_mask(&ask(0x2080_122a, vec![0; hostfacts::GR_MASK_PARAMS_SIZE])).expect("nonzero");
    assert_eq!(gpc_mask, f.gr_static.gpc_mask().expect("fixture"));
    for (gpc, row) in f.gr_static.gpcs.iter().enumerate() {
        let mut req = vec![0u8; hostfacts::GR_MASK_PARAMS_SIZE];
        req[16..20].copy_from_slice(&(gpc as u32).to_le_bytes());
        assert_eq!(hostfacts::derive_tpc_mask(&ask(0x2080_122b, req), gpc as u32), Ok(row.tpc_mask));
    }
    let caps = hostfacts::derive_gr_caps(&ask(0x2080_1227, vec![0; hostfacts::GR_CAPS_V2_PARAMS_SIZE])).expect("populated");
    assert_eq!(caps, f.gr_static.caps);
}

/// ★ The SM order: the real GA106's `globalSmId[]` entries derive to the old row's TPC rows.
/// ⊘ The capture is TRUNCATED at 4096 of 9240 bytes, which keeps all 28 entries (504 bytes) but
/// loses `numSm`/`numTpc` at byte 9216 — so the query refuses it (see the whole-struct test),
/// and HERE the two counts are supplied from the fixture's own shape (14 TPC rows × 2). What is
/// checked is therefore the entries and the pairing rule, not the counts.
#[test]
fn the_captured_sm_order_entries_derive_to_the_captured_tpc_rows() {
    let f = ga106::host_facts();
    let host = Ga106Replay::load();
    let (_, status, prefix) = &host.ctrls[&0x2080_121b][0];
    assert_eq!(*status, TRUNCATED);
    let mut reply = vec![0u8; hostfacts::GR_GLOBAL_SM_ORDER_PARAMS_SIZE];
    reply[..prefix.len()].copy_from_slice(prefix);
    let num_tpc = f.gr_static.tpcs.len() as u16;
    let at = hostfacts::GR_GLOBAL_SM_ORDER_MAX_SM * hostfacts::GR_GLOBAL_SM_ENTRY_SIZE;
    reply[at..at + 2].copy_from_slice(&(num_tpc * f.gr_static.sms_per_tpc).to_le_bytes());
    reply[at + 2..at + 4].copy_from_slice(&num_tpc.to_le_bytes());
    let (tpcs, sms_per_tpc) = hostfacts::derive_sm_order(&reply).expect("entries pair up");
    assert_eq!(tpcs, f.gr_static.tpcs);
    assert_eq!(sms_per_tpc, f.gr_static.sms_per_tpc);
}

/// `engines`: `GET_ENGINES_V2` IS captured; `GET_HW_ENGINE_ID` and `GET_ENGINE_FAULT_INFO` are
/// not. So this checks what the capture can: the advertised list (GR, the CEs, `SW` — the
/// host's NVDEC/NVENC/OFA read and deliberately dropped) names the fixture's rows, in order,
/// with the fixture's `RM_ENGINE_TYPE`s; and `device_info` built over it equals the fixture's.
#[test]
fn the_advertised_engine_list_and_device_info_equal_the_captured_rows() {
    use kf_abi::inittables::engine_info_type::RM_ENGINE_TYPE;
    let f = ga106::host_facts();
    let host = Ga106Replay::load();
    let list = hostfacts::derive_engine_list(&host.ctrls[&0x2080_0170][0].2).expect("captured");
    assert_eq!(list.len(), 9, "the real GA106 lists nine engines");
    let ids: Vec<hostquery::EngineIdentity> = list
        .iter()
        .filter_map(|&t| {
            hostquery::classify_engine(t).map(|(kind, rm)| hostquery::EngineIdentity {
                kind,
                nv2080_engine_type: t,
                rm_engine_type: rm,
                fifo_tag: 0,
                mmu_fault_id: None,
            })
        })
        .collect();
    let names: Vec<String> = ids.iter().map(|e| e.name()).collect();
    let fixture_names: Vec<&str> = f.engines.iter().map(|e| e.name).collect();
    assert_eq!(names, fixture_names);
    let rm: Vec<u32> = ids.iter().map(|e| e.rm_engine_type).collect();
    let fixture_rm: Vec<u32> = f.engines.iter().map(|e| e.engine_data[RM_ENGINE_TYPE]).collect();
    assert_eq!(rm, fixture_rm);
    assert_eq!(hostquery::device_info_rule(&ids), f.device_info);
}

/// ⊘ `gr_info` — NO capture of `GR_GET_INFO_V2` exists. What CAN be checked is the layout: the
/// fixture's table, laid out as `GR_GET_INFO_V2`'s reply, must derive back to itself.
#[test]
fn gr_info_has_no_capture_so_only_its_layout_round_trips() {
    let f = ga106::host_facts();
    let mut reply = vec![0u8; hostfacts::GR_GET_INFO_V2_PARAMS_SIZE];
    reply[0..4].copy_from_slice(&(kf_abi::grinfo::GR_INFO_MAX_SIZE as u32).to_le_bytes());
    for (i, d) in f.gr_info.data.iter().enumerate() {
        reply[4 + 8 * i..8 + 8 * i].copy_from_slice(&(i as u32).to_le_bytes());
        reply[8 + 8 * i..12 + 8 * i].copy_from_slice(&d.to_le_bytes());
    }
    assert_eq!(hostfacts::derive_gr_info(&reply).expect("well formed").data, f.gr_info.data);
}

/// ⊘ `gr_context_buffers` — NO capture. Checked: the query's mapping of RM's own
/// `NV_ERR_NOT_SUPPORTED` to ABSENT, over a host answering the fixture's rows the way
/// `subdeviceCtrlCmdKGrGetEngineContextProperties_IMPL` would.
#[test]
fn gr_context_buffers_have_no_capture_so_only_the_absent_mapping_is_checked() {
    struct FromFixture;
    impl HostControls for FromFixture {
        fn control(&mut self, cmd: u32, p: &mut [u8]) -> Result<(), HostRefusal> {
            assert_eq!(cmd, 0x2080_122d);
            let id = u32::from_le_bytes(p[16..20].try_into().expect("4")) as usize;
            let b = kf_abi::grstatic::GA106_CONTEXT_BUFFERS[id];
            if b.size == kf_abi::grstatic::CONTEXT_BUFFER_ABSENT {
                return Err(HostRefusal { status: Some(0x56), detail: "NV_U32_MAX".into() });
            }
            p[20..24].copy_from_slice(&b.alignment.to_le_bytes());
            p[24..28].copy_from_slice(&b.size.to_le_bytes());
            p[28] = 1;
            Ok(())
        }
    }
    assert_eq!(hostquery::query_gr_context_buffers(&mut FromFixture), Ok(ga106::host_facts().gr_context_buffers));
}

/// ⊘ `intr_table` / `intr_subtree_map` — NO capture. Checked: the re-keying. Every static
/// `NV2080_INTR_TYPE` and every engine type the fixture's kernel table has a row for maps to
/// that row's `MC_ENGINE_IDX`. ⚠ The fixture ALSO carries `GSP` (50) and `DISP` (2) stall rows,
/// which neither host control reports — a derived table will not have them.
#[test]
fn intr_table_has_no_capture_so_only_the_mc_engine_idx_keying_is_checked() {
    let f = ga106::host_facts();
    let has = |idx: u16| f.intr_table.iter().any(|e| e.engine_idx == idx);
    for t in [0x4u32, 0x5, 0x6, 0x8] {
        let idx = hostfacts::mc_engine_idx_of_intr_type(t).expect("keyed");
        assert!(has(idx), "INTR_TYPE {t:#x} -> {idx} not in the captured table");
    }
    for t in 0x9..=0x10u32 {
        assert!(has(hostfacts::mc_engine_idx_of_intr_type(t).expect("FECS")));
    }
    // GR0, CE0..CE4, NVDEC0, NVENC0, OFA0, SEC2 — the engine rows the captured table carries.
    for (t, idx) in [(0x01, 84), (0x09, 15), (0x0a, 16), (0x0b, 17), (0x0c, 18), (0x0d, 19), (0x13, 65), (0x1b, 38), (0x33, 81), (0x26, 47)] {
        assert_eq!(hostfacts::mc_engine_idx_of_engine_type(t), Some(idx));
        assert!(has(idx));
    }
    // ⊘ Unknown keys are refused, never dropped.
    let mut s = vec![0u8; hostfacts::MC_STATIC_INTR_TABLE_PARAMS_SIZE];
    s[0] = 1;
    s[4] = 0x7f;
    let n = vec![0u8; hostfacts::MC_ENGINE_NOTIFICATION_PARAMS_SIZE];
    assert!(hostfacts::derive_intr_table(&s, &n).is_err());
}

/// The family rules take no family argument — each is one statement for all five families,
/// cited to headers of more than one generation — and they cover the parts GA106 lacks: the
/// second copy decade (Hopper/Blackwell have >10 LCEs), whose NV2080 and RM spaces differ.
#[test]
fn the_family_rules_cover_what_a_ga106_does_not_have() {
    assert_eq!(hostquery::USERMODE_REG_BASE, 0x00BB_0000, "tu102/gb100 dev_vm.h: 0xB80000 + 0x30000");
    assert_eq!(hostquery::classify_engine(0x34), Some((hostquery::EngineKind::Copy(10), 0x13)), "COPY10: NV2080 0x34, RM 0x13");
    assert_eq!(hostquery::classify_engine(0x3d), Some((hostquery::EngineKind::Copy(19), 0x1c)));
    assert_eq!(hostquery::classify_engine(0x13), None, "NV2080 0x13 is NVDEC0, not a copy engine");
    assert_eq!(hostquery::classify_engine(0x08), Some((hostquery::EngineKind::Graphics(7), 0x08)), "GR7 (MIG parts)");
    assert_eq!(hostfacts::mc_engine_idx_of_engine_type(0x3d), Some(34), "CE19 = MC_ENGINE_IDX_CE19");
}
