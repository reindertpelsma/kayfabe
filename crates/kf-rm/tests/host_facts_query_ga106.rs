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
//! answered with a plausible body. So the whole-struct query on this replay alone cannot succeed,
//! and the first test states exactly which fields fail: that list IS the capture gap (no field
//! lacks a source any more — `kf_rm::authored`). A second host, `CompletedGa106`, answers the
//! uncaptured controls from the old rows so the whole query reaches `Ok`, and says which fields
//! that makes a layout round-trip rather than a hardware comparison.
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
const OUT_ONLY: &[u32] = &[0x2080_2a0a, 0x2080_1701, 0x2080_182b, 0x2080_0170, 0x2080_0110, 0x2080_0111, 0x2080_3601, 0x2080_122a, 0x2080_1227, 0x2080_121b];

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

/// ★★★ Over ONLY what a real GA106 replied, the query refuses EXACTLY the fields whose controls
/// were never captured — and nothing for want of a source: every field the host cannot state
/// is now authored (`kf_rm::authored`, w827 ruling).
#[test]
fn over_the_real_ga106_the_query_refuses_only_the_uncaptured() {
    let mut host = Ga106Replay::load();
    let refused = hostquery::query_host_facts(&mut host, Family::Ampere).expect_err("some controls have no capture");
    let by_field: BTreeMap<&str, &FieldCause> = refused.refusals.iter().map(|r| (r.field, &r.cause)).collect();
    let no_capture = |field: &str, cmd: u32| {
        matches!(by_field.get(field), Some(FieldCause::Host { cmd: c, refused }) if *c == cmd && refused.status.is_none())
    };
    // libcuda's cuInit never asks these, and no rmladder rung did:
    assert!(no_capture("intr_table", 0x2080_170e), "{refused}");
    assert!(no_capture("intr_subtree_map", 0x2080_170f), "{refused}");
    assert!(no_capture("gr_info", 0x2080_1228), "{refused}");
    assert!(no_capture("gr_context_buffers", 0x2080_122d), "{refused}");
    assert!(no_capture("gr_static", 0x2080_1237), "GR_GET_ZCULL_MASK was never captured: {refused}");
    assert!(no_capture("gr_zcull_info", 0x2080_1206), "GR_GET_ZCULL_INFO was never captured: {refused}");
    assert!(no_capture("zbc_table_sizes", 0x9096_0106), "GET_ZBC_CLEAR_TABLE_SIZE was never captured: {refused}");
    // ⊘ BIOS_GET_INFO_V2 was never captured — and a host that does not answer it is NOT a refused
    // field: the version is cosmetic (coordinator, 2026-09-26). It is asked, and `None`.
    assert!(host.asked.contains(&0x2080_0810));
    assert!(!refused.fields().contains(&"vbios_version"));
    assert_eq!(by_field.get("memory_system"), Some(&&FieldCause::DependsOn("gr_info")));
    assert_eq!(
        refused.fields(),
        ["intr_table", "intr_subtree_map", "memory_system", "gr_static", "gr_info", "gr_context_buffers", "gr_zcull_info", "zbc_table_sizes"],
        "{refused}"
    );
    assert!(!refused.refusals.iter().any(|r| matches!(r.cause, FieldCause::Unsourced(_))));
    // ★ Refusals come in PROVENANCE order, so the realize log reads like the table.
    let order: Vec<usize> = refused.fields().iter().map(|f| PROVENANCE.iter().position(|(p, _)| p == f).expect("a provenance row")).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    // ⊘ The kernel-only control is no longer asked at all.
    assert!(!host.asked.contains(&0x2080_2a08));
    // ★ CE_GET_ALL_CAPS IS captured (R18 / cuInit): `ce_caps` fills from the real reply, and a
    // PERF_GET_LEVEL_INFO_V2 the host does not answer is a VALUE (relayed), not a field refusal.
    assert!(host.asked.contains(&0x2080_2a0a) && host.asked.contains(&0x2080_200b));
}

/// ★ `ce_caps` from the REAL GA106's `CE_GET_ALL_CAPS` equals the fixture, and its GRCE set is
/// GA10x's `{LCE0, LCE1}` — the constant it replaced, reproduced on the part it was measured on.
#[test]
fn ce_caps_from_the_real_ga106_equal_the_captured_row() {
    let got = hostquery::query_ce_caps(&mut Ga106Replay::load()).expect("captured");
    assert_eq!(got, ga106::ce_caps());
    assert_eq!(got.grce_mask(), kf_abi::cecaps::GA10X_GRCE_LCE_MASK);
}

/// ★★★ **Realize succeeds on a GA106**: the replay, completed for the controls no capture holds
/// by answering them the way the old captured rows say a GA106 would (so those fields are a
/// LAYOUT round-trip here, not a hardware comparison — the test says which), fills EVERY field,
/// and each equals the old captured row except the enumerated, reasoned divergences.
#[test]
fn a_ga106_host_fills_every_field_and_each_equals_the_captured_row_or_a_stated_divergence() {
    let f = ga106::host_facts();
    let mut host = CompletedGa106(Ga106Replay::load());
    let got = hostquery::query_host_facts(&mut host, Family::Ampere).unwrap_or_else(|e| panic!("{e}"));

    // From real GA106 replies:
    assert_eq!(got.family, f.family);
    assert_eq!(got.has_c2c, f.has_c2c);
    assert_eq!(got.lce_pce_masks, f.lce_pce_masks);
    assert_eq!(got.chip_info, f.chip_info);
    assert_eq!(got.device_info, f.device_info);
    assert_eq!(got.forwarded_gpu_info, f.forwarded_gpu_info);
    assert_eq!(got.smc_mode, f.smc_mode);
    assert_eq!(got.pcie_max_gen, f.pcie_max_gen);
    assert_eq!(got.gsp_features, f.gsp_features);
    assert_eq!(got.gpu_name.map(|n| n.as_str()), Some("NVIDIA GeForce RTX 3060"));
    assert_eq!(got.ce_caps, f.ce_caps);
    // Layout round-trips through the completed controls (BIOS info / PERF level info):
    assert_eq!(got.vbios_version, f.vbios_version);
    assert_eq!(got.perf_level_info_v2, f.perf_level_info_v2);
    // Authored, and equal to what a stock GA106 GSP states (by choice — see authored.rs):
    assert_eq!(got.gmmu_static, f.gmmu_static);
    assert_eq!(got.ce_fault_method_buffer_size, f.ce_fault_method_buffer_size);
    // Fixed values:
    assert_eq!(got.user_register_access_map, f.user_register_access_map);
    assert_eq!(got.constructed_falcons, f.constructed_falcons);
    assert_eq!(got.conf_compute, f.conf_compute);
    assert_eq!(got.bif_static, f.bif_static);
    assert_eq!(got.fifo_channels, f.fifo_channels);
    // Layout round-trips through the completed controls (GR info / context props / subtree map):
    assert_eq!(got.gr_info.data, f.gr_info.data);
    assert_eq!(got.gr_context_buffers, f.gr_context_buffers);
    assert_eq!(got.gr_zcull_info, f.gr_zcull_info);
    assert_eq!(got.zbc_table_sizes, f.zbc_table_sizes);
    assert_eq!(got.intr_subtree_map, f.intr_subtree_map);
    assert_eq!(got.memory_system, f.memory_system);
    // gr_static: GPC mask, TPC masks, the logical→physical map (libcuda's own GRMGR batch), SM
    // order and caps from real replies; zcull, tpcCount, PES, the phys/gfx masks and the GR
    // litters from the completion; FECS / per-subctx AUTHORED — all equal. The optional GRMGR
    // batch (PPC/ROP/syspipe) has no capture: `None`, as the fixture states.
    assert_eq!(got.gr_static.gpcs, f.gr_static.gpcs);
    assert_eq!(got.gr_static.gfx_gpc_mask, f.gr_static.gfx_gpc_mask);
    assert_eq!(got.gr_static.num_gfx_tpc, f.gr_static.num_gfx_tpc);
    assert_eq!(got.gr_static.syspipe_masks, None);
    assert_eq!(got.gr_static.tpcs, f.gr_static.tpcs);
    assert_eq!(got.gr_static.sms_per_tpc, f.gr_static.sms_per_tpc);
    assert_eq!(got.gr_static.tpc_to_pes_map, f.gr_static.tpc_to_pes_map);
    assert_eq!(got.gr_static.caps, f.gr_static.caps);
    assert_eq!(got.gr_static.fecs_record_size, f.gr_static.fecs_record_size);
    assert_eq!(got.gr_static.per_subctx_header_supported, f.gr_static.per_subctx_header_supported);
    // intr_table: the host's STATIC rows and the authored GSP/DISP rows equal the captured table's
    // stall/static rows exactly. ⊘ The engine NON-STALL rows are authored for OUR runlists
    // (0x2080170d is NOT_SUPPORTED to usermode, measured f4b78ed9), so they are checked against
    // the rule, not the die: GR0 on 0, each async CE on its runlist, graphics CEs none.
    let engine_row = |e: &kf_abi::inittables::IntrTableEntry| {
        e.vector_stall == kf_abi::inittables::INTR_VECTOR_INVALID
            && e.vector_non_stall != kf_abi::inittables::INTR_VECTOR_INVALID
    };
    let captured_engine_row = |e: &kf_abi::inittables::IntrTableEntry| {
        e.vector_stall == kf_abi::inittables::INTR_VECTOR_INVALID && e.engine_idx < 156
    };
    let mut a: Vec<_> = got.intr_table.iter().filter(|e| !engine_row(e)).cloned().collect();
    let mut b: Vec<_> = f.intr_table.iter().filter(|e| !captured_engine_row(e)).cloned().collect();
    a.sort_by_key(|e| e.engine_idx);
    b.sort_by_key(|e| e.engine_idx);
    assert_eq!(a, b);
    let mut rows: Vec<(u16, u32)> =
        got.intr_table.iter().filter(|e| engine_row(e)).map(|e| (e.engine_idx, e.vector_non_stall)).collect();
    rows.sort_unstable();
    let mut vectors: Vec<u32> = rows.iter().map(|r| r.1).collect();
    vectors.dedup();
    assert_eq!(vectors.len(), rows.len(), "two engines on one non-stall vector: {rows:?}");
    assert!(rows.contains(&(84, 0)), "GR0 notifies on vector 0: {rows:?}");
    // engines: see the dedicated test.
    assert_engine_rows_match_except_stated(&got.engines, &f.engines);
}

/// ⊘ Every slot the authored engine layout does NOT reproduce, with the reason. Everything else
/// in the six rows equals the old captured table.
const ENGINE_DIVERGENCES: &[(&str, &str, &str)] = &[
    ("*", "RC_MASK", "no kernel-RM reader (only kfifoEngineInfoXlate's switch); authored 0"),
    ("CE1", "INTR", "capture noise (0x82300100): INTR is not stored on Ampere+ (kernel_fifo_ga100.c:52)"),
    ("CE2", "INTR", "capture noise (0x77f2058f)"),
    ("CE3", "INTR", "capture noise (0x018e0102)"),
    ("CE1", "pbdmaFaultIds", "capture says 0x20 for PBDMA 1, contradicting GR0's own 1 -> 0x21"),
    ("CE2", "pbdmaIds", "the die's PBDMA 5; our device numbers PBDMAs contiguously (2); fault id 0x22 agrees"),
    ("CE3", "pbdmaIds", "the die's PBDMA 6; ours is 3; fault id 0x23 agrees"),
    ("SOFTWARE", "*", "RM's pseudo-engine: the capture's RUNLIST/RESET/INTR/MC/INSTANCE/PRI-base/PBDMA are noise (kf_abi::deviceinfo)"),
];

fn assert_engine_rows_match_except_stated(got: &[kf_abi::inittables::FifoDeviceEntry], want: &[kf_abi::inittables::FifoDeviceEntry]) {
    use kf_rm::authored::slot;
    const NAMES: [(usize, &str); 15] = [
        (slot::ENG_DESC, "ENG_DESC"), (slot::FIFO_TAG, "FIFO_TAG"), (slot::RM_ENGINE_TYPE, "RM_ENGINE_TYPE"),
        (slot::RUNLIST, "RUNLIST"), (slot::MMU_FAULT_ID, "MMU_FAULT_ID"), (slot::RC_MASK, "RC_MASK"),
        (slot::RESET, "RESET"), (slot::INTR, "INTR"), (slot::MC, "MC"), (slot::DEV_TYPE_ENUM, "DEV_TYPE_ENUM"),
        (slot::INSTANCE_ID, "INSTANCE_ID"), (slot::RUNLIST_PRI_BASE, "RUNLIST_PRI_BASE"),
        (slot::IS_HOST_DRIVEN_ENGINE, "IS_HOST_DRIVEN_ENGINE"), (slot::RUNLIST_ENGINE_ID, "RUNLIST_ENGINE_ID"),
        (slot::CHRAM_PRI_BASE, "CHRAM_PRI_BASE"),
    ];
    let excused = |name: &str, what: &str| {
        ENGINE_DIVERGENCES.iter().any(|(n, w, _)| (*n == name || *n == "*") && (*w == what || *w == "*"))
    };
    assert_eq!(got.len(), want.len());
    let mut checked = 0;
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g.name, w.name);
        for (s, what) in NAMES {
            if !excused(g.name, what) {
                assert_eq!(g.engine_data[s], w.engine_data[s], "{} {what}", g.name);
                checked += 1;
            }
        }
        let n = w.num_pbdmas as usize;
        if !excused(g.name, "numPbdmas") {
            assert_eq!(g.num_pbdmas, w.num_pbdmas, "{} numPbdmas", g.name);
        }
        if !excused(g.name, "pbdmaIds") {
            assert_eq!(g.pbdma_ids[..n], w.pbdma_ids[..n], "{} pbdmaIds", g.name);
        }
        if !excused(g.name, "pbdmaFaultIds") {
            assert_eq!(g.pbdma_fault_ids[..n], w.pbdma_fault_ids[..n], "{} pbdmaFaultIds", g.name);
        }
    }
    assert_eq!(checked, 5 * 14 - 3, "5 hardware rows × 14 non-RC slots, minus CE1..3 INTR");
}

/// ★ The engine rows: authored over the host's REAL `GET_ENGINES_V2` list (captured), compared
/// slot by slot with the old captured table — equal except [`ENGINE_DIVERGENCES`].
#[test]
fn the_authored_engine_table_over_the_real_engine_list_equals_the_captured_rows_except_stated() {
    let f = ga106::host_facts();
    let all = hostquery::query_engine_list(&mut Ga106Replay::load()).expect("captured");
    println!("host engine list: {:?}", all.iter().map(|k| k.name()).collect::<Vec<_>>());
    // ★ The captured old row predates the video engines (it advertised none): compare the
    // non-video subset, and the video rows separately (`the_video_rows_...`).
    let kinds: Vec<_> = all.iter().copied().filter(|k| hostquery::video_eng_desc(*k).is_none()).collect();
    assert_eq!(kinds.iter().map(|k| k.name()).collect::<Vec<_>>(), ["GR0", "CE0", "CE1", "CE2", "CE3", "SOFTWARE"]);
    let rows = kf_rm::authored::engine_table(Family::Ampere, &kinds, f.ce_caps.grce_mask()).expect("Ampere has every constant");
    assert_engine_rows_match_except_stated(&rows, &f.engines);
    assert_eq!(hostquery::device_info_rule(&kinds, &[]), f.device_info);
    // ★ And the served CE geometry derived from the authored rows names the LCEs the real GA106
    // reports present (R18 CE_GET_ALL_CAPS: 0x0f).
    assert_eq!(kf_abi::cecaps::CeGeometry::from_engines(&rows, &f.ce_caps).expect("LCE rows").present, 0x0f);
}

/// A host of another family than realize chose is refused by name.
#[test]
fn a_host_of_another_family_is_refused_by_name() {
    let mut host = Ga106Replay::load();
    let refused = hostquery::query_host_facts(&mut host, Family::Hopper).expect_err("family");
    assert!(refused
        .refusals
        .iter()
        .any(|r| r.field == "family" && r.cause == FieldCause::FamilyMismatch { asked: Family::Hopper, host: Family::Ampere }));
}

/// ★ No field is left without a source any more: the four the first cut refused are
/// `Advertised` (gmmu, fault-method buffer) or host + authored (engines, gr_static).
#[test]
fn no_provenance_row_is_unsourced_and_the_authored_ones_say_so() {
    assert!(!PROVENANCE.iter().any(|(_, s)| matches!(s, Source::Unsourced(_))));
    for f in ["gmmu_static", "ce_fault_method_buffer_size"] {
        assert!(PROVENANCE.iter().any(|(n, s)| *n == f && matches!(s, Source::Advertised(_))), "{f}");
    }
}

/// The replay, completed for the controls no capture holds by answering them from the old
/// captured rows — a stand-in for the host, so the WHOLE query can be driven to `Ok`. ⚠ What
/// this proves for those controls is the request/reply LAYOUT, not a hardware value.
struct CompletedGa106(Ga106Replay);

impl HostControls for CompletedGa106 {
    fn control(&mut self, cmd: u32, p: &mut [u8]) -> Result<(), HostRefusal> {
        let f = ga106::host_facts();
        let put = |p: &mut [u8], at: usize, v: u32| p[at..at + 4].copy_from_slice(&v.to_le_bytes());
        match cmd {
            0x2080_1228 => {
                for (i, d) in f.gr_info.data.iter().enumerate() {
                    put(p, 8 + 8 * i, *d);
                }
                Ok(())
            }
            // GR_GET_ZCULL_MASK — asked by PHYSICAL gpcId: the row naming that physical GPC.
            0x2080_1237 => {
                let gpc = u32::from_le_bytes(p[0..4].try_into().expect("4"));
                let row = f.gr_static.gpcs.iter().find(|g| g.physical_id == gpc).expect("an enabled GPC");
                put(p, 4, row.zcull_mask);
                Ok(())
            }
            // ★ v3-gpcmask: the per-index floorsweeping controls the fixture's rows answer.
            // GR_GET_PHYS_GPC_MASK (syspipe 0).
            0x2080_1232 => {
                put(p, 4, f.gr_static.gpc_mask().expect("fixture"));
                Ok(())
            }
            // GR_GET_NUM_TPCS_FOR_GPC — LOGICAL gpcId.
            0x2080_1234 => {
                let gpc = u32::from_le_bytes(p[0..4].try_into().expect("4")) as usize;
                put(p, 4, f.gr_static.gpcs[gpc].tpc_count);
                Ok(())
            }
            // GPU_GET_PES_INFO — LOGICAL gpcId; numPesInGpc and the whole tpcToPesMap.
            0x2080_0168 => {
                let gpc = u32::from_le_bytes(p[0..4].try_into().expect("4")) as usize;
                put(p, 4, f.gr_static.gpcs[gpc].num_pes_per_gpc);
                for (i, m) in f.gr_static.tpc_to_pes_map.iter().enumerate() {
                    put(p, 16 + 4 * i, *m);
                }
                Ok(())
            }
            // GR_GET_GFX_GPC_AND_TPC_INFO.
            0x2080_1239 => {
                put(p, 16, f.gr_static.gfx_gpc_mask);
                put(p, 20, f.gr_static.num_gfx_tpc);
                Ok(())
            }
            0x2080_121b => {
                // The capture's entries (truncated at 4096 bytes) plus the counts it lost.
                let (_, _, prefix) = &self.0.ctrls[&cmd][0];
                p[..prefix.len()].copy_from_slice(prefix);
                let at = hostfacts::GR_GLOBAL_SM_ORDER_MAX_SM * hostfacts::GR_GLOBAL_SM_ENTRY_SIZE;
                let tpcs = f.gr_static.tpcs.len() as u16;
                p[at..at + 2].copy_from_slice(&(tpcs * f.gr_static.sms_per_tpc).to_le_bytes());
                p[at + 2..at + 4].copy_from_slice(&tpcs.to_le_bytes());
                Ok(())
            }
            // ★ v3-gfx: GR_GET_ZCULL_INFO — the fixture's row, or RM's "no zcull" (0x56).
            0x2080_1206 => match f.gr_zcull_info {
                Some(row) => {
                    for (i, w) in row.iter().enumerate() {
                        put(p, 4 * i, *w);
                    }
                    Ok(())
                }
                None => Err(HostRefusal { status: Some(0x56), detail: "no zcull".into() }),
            },
            // ★ v3-gfx: the fixture states no ZBC ranges ⇒ RM's own "no table" (0x56).
            0x9096_0106 => match f.zbc_table_sizes {
                Some(s) => {
                    let t = u32::from_le_bytes(p[8..12].try_into().expect("4")) as usize;
                    put(p, 0, s[t - 1].0);
                    put(p, 4, s[t - 1].1);
                    Ok(())
                }
                None => Err(HostRefusal { status: Some(0x56), detail: "no zbc".into() }),
            },
            0x2080_122d => {
                let id = u32::from_le_bytes(p[16..20].try_into().expect("4")) as usize;
                let b = f.gr_context_buffers[id];
                if b.size == kf_abi::grstatic::CONTEXT_BUFFER_ABSENT {
                    return Err(HostRefusal { status: Some(0x56), detail: "absent".into() });
                }
                put(p, 20, b.alignment);
                put(p, 24, b.size);
                p[28] = 1;
                Ok(())
            }
            0x2080_170f => {
                for (i, m) in f.intr_subtree_map.iter().enumerate() {
                    p[8 * i..8 * i + 8].copy_from_slice(&m.to_le_bytes());
                }
                Ok(())
            }
            0x2080_170e => {
                // The captured kernel table's static-type rows, re-keyed back to NV2080_INTR_TYPE.
                let rows: Vec<(u32, &kf_abi::inittables::IntrTableEntry)> = f
                    .intr_table
                    .iter()
                    .filter_map(|e| (1..=0x10u32).find(|&t| hostfacts::mc_engine_idx_of_intr_type(t) == Some(e.engine_idx)).map(|t| (t, e)))
                    .collect();
                put(p, 0, rows.len() as u32);
                for (i, (t, e)) in rows.iter().enumerate() {
                    let at = 4 + 16 * i;
                    put(p, at, *t);
                    put(p, at + 4, e.pmc_intr_mask);
                    put(p, at + 8, e.vector_stall);
                    put(p, at + 12, e.vector_non_stall);
                }
                Ok(())
            }
            0x2080_170d => {
                // The captured table's engine rows, keyed back to NV2080_ENGINE_TYPE.
                let rows: Vec<(u32, u32)> = f
                    .intr_table
                    .iter()
                    .filter_map(|e| (1..0x54u32).find(|&t| hostfacts::mc_engine_idx_of_engine_type(t) == Some(e.engine_idx)).map(|t| (t, e.vector_non_stall)))
                    .collect();
                put(p, 0, rows.len() as u32);
                for (i, (t, v)) in rows.iter().enumerate() {
                    put(p, 4 + 8 * i, *t);
                    put(p, 8 + 8 * i, *v);
                }
                Ok(())
            }
            // BIOS_GET_INFO_V2 [REVISION, OEM_REVISION] — the fixture's pair.
            0x2080_0810 => {
                let v = f.vbios_version.expect("the fixture answers it");
                put(p, 8, v.0);
                put(p, 16, u32::from(v.1));
                Ok(())
            }
            // PERF_GET_LEVEL_INFO_V2 — the fixture's reply (the GA106 oracle words).
            0x2080_200b => {
                let r = f.perf_level_info_v2.expect("the fixture answers it");
                p.copy_from_slice(&r);
                Ok(())
            }
            _ => self.0.control(cmd, p),
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
    // `GR_GET_TPC_MASK` takes the PHYSICAL gpcId: each row is asked by its own physical id.
    for row in f.gr_static.gpcs {
        let mut req = vec![0u8; hostfacts::GR_MASK_PARAMS_SIZE];
        req[16..20].copy_from_slice(&row.physical_id.to_le_bytes());
        assert_eq!(hostfacts::derive_tpc_mask(&ask(0x2080_122b, req), row.physical_id), Ok(row.tpc_mask));
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
    assert_eq!(hostquery::classify_engine(0x34), Some((kf_rm::authored::EngineKind::Copy(10), 0x13)), "COPY10: NV2080 0x34, RM 0x13");
    assert_eq!(hostquery::classify_engine(0x3d), Some((kf_rm::authored::EngineKind::Copy(19), 0x1c)));
    assert_eq!(
        hostquery::classify_engine(0x13),
        Some((kf_rm::authored::EngineKind::VideoDecode(0), 0x1d)),
        "NV2080 0x13 is NVDEC0 (RM 0x1d), not a copy engine"
    );
    assert_eq!(hostquery::classify_engine(0x08), Some((kf_rm::authored::EngineKind::Graphics(7), 0x08)), "GR7 (MIG parts)");
    assert_eq!(hostfacts::mc_engine_idx_of_engine_type(0x3d), Some(34), "CE19 = MC_ENGINE_IDX_CE19");
}

// =====================================================================================
// The authored rules, for the families a GA106 cannot exercise
// =====================================================================================

/// `tpc_to_pes_map` from the GA106's own GR litters (6 TPCs per GPC, 2 per PES) is the map the
/// real GA106 reported.
#[test]
fn the_tpc_to_pes_rule_reproduces_the_ga106_map() {
    let f = ga106::host_facts();
    let map = kf_rm::authored::tpc_to_pes_map(f.gr_info.data[0x17], f.gr_info.data[0x1e]).expect("usable litters");
    assert_eq!(map, kf_abi::grstatic::GA106_TPC_TO_PES_MAP);
    assert_eq!(kf_rm::authored::tpc_to_pes_map(6, 0), None, "zero TPCs per PES is refused");
}

/// ★ The engine layout for every family: Blackwell's fault ids are its own header's (GR 384,
/// CE0 65, HOST0 85); Turing has no Esched PRI bases; Hopper is refused BY NAME (no HOST0 in the
/// tree) and so is a MIG list with a second GR.
#[test]
fn the_engine_layout_answers_every_family_or_refuses_one_by_name() {
    use kf_rm::authored::{EngineKind as K, engine_table, slot};
    let list = [K::Graphics(0), K::Copy(0), K::Copy(1), K::Copy(2), K::Copy(12), K::Software];
    for fam in [Family::Turing, Family::Ampere, Family::Ada, Family::Blackwell] {
        let rows = engine_table(fam, &list, 0x3).unwrap_or_else(|e| panic!("{fam:?}: {e:?}"));
        let resets: Vec<u32> = rows[..5].iter().map(|r| r.engine_data[slot::RESET]).collect();
        let mut uniq = resets.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), resets.len(), "{fam:?}: reset bits collide: {resets:?}");
        assert!(resets.iter().all(|&b| b < 32));
    }
    let bw = engine_table(Family::Blackwell, &list, 0x3).expect("Blackwell");
    assert_eq!(bw[0].engine_data[slot::MMU_FAULT_ID], 384);
    assert_eq!(bw[4].engine_data[slot::MMU_FAULT_ID], 65 + 12);
    assert_eq!(bw[0].pbdma_fault_ids, [85, 86]);
    let tu = engine_table(Family::Turing, &list, 0x3).expect("Turing");
    assert_eq!(tu[3].engine_data[slot::RUNLIST_PRI_BASE], 0, "RUNLIST_PRI_BASE is valid only on Ampere+");
    assert_eq!(tu[3].engine_data[slot::RUNLIST], 1);
    let hopper = engine_table(Family::Hopper, &list, 0x3).expect_err("no HOST0");
    assert!(hopper.missing.contains("HOST0"));
    assert!(engine_table(Family::Ampere, &[K::Graphics(0), K::Graphics(1), K::Copy(0)], 0x3).is_err(), "MIG");
}

/// ★★ GB20x: four GRCEs (`kernel_ce_gb202.c:36`, `NV_CE_GRCE_ALLOWED_LCE_MASK 0x0F`). With the
/// host's GRCE bits on LCE0..3, all four share runlist 0 on GR's two PBDMAs (k mod 2) and the async
/// LCEs keep distinct runlists and PBDMAs — no PBDMA is shared between a GRCE and an async CE, and
/// the GRCEs get no non-stall row of their own (they notify through GR0).
#[test]
fn a_gb20x_host_with_four_grces_lays_them_all_on_runlist_zero() {
    use kf_rm::authored::{EngineKind as K, engine_notification_rows, engine_table, slot};
    let list = [K::Graphics(0), K::Copy(0), K::Copy(1), K::Copy(2), K::Copy(3), K::Copy(4), K::Copy(5), K::Software];
    let rows = engine_table(Family::Blackwell, &list, 0x0f).expect("Blackwell");
    for (i, r) in rows[1..5].iter().enumerate() {
        assert_eq!(r.engine_data[slot::RUNLIST], 0, "GRCE{i}");
        assert_eq!(r.engine_data[slot::RUNLIST_ENGINE_ID], i as u32 + 1, "GRCE{i}");
        assert_eq!(r.pbdma_ids[0], i as u32 % 2, "GRCE{i} rides GR's PBDMA");
    }
    assert_eq!((rows[5].engine_data[slot::RUNLIST], rows[5].pbdma_ids[0]), (1, 2));
    assert_eq!((rows[6].engine_data[slot::RUNLIST], rows[6].pbdma_ids[0]), (2, 3));
    let ns = engine_notification_rows(&list, 0x0f);
    assert_eq!(ns.len(), 3, "GR0 + the two async CEs: {ns:?}");
    // And with GA10x's two GRCEs the same list keeps its old shape.
    let ga = engine_table(Family::Ampere, &list, 0x03).expect("Ampere");
    assert_eq!(ga[3].engine_data[slot::RUNLIST], 1);
    assert_eq!(ga[3].pbdma_ids[0], 2);
}

/// The GSP/DISP rows are refused, never doubled, if a host row already sits on either vector.
#[test]
fn the_gsp_and_disp_rows_refuse_a_host_vector_collision() {
    use kf_abi::inittables::{INTR_VECTOR_INVALID, IntrTableEntry};
    let clash = vec![IntrTableEntry { engine_idx: 59, pmc_intr_mask: 0, vector_stall: 0x9b, vector_non_stall: INTR_VECTOR_INVALID }];
    assert_eq!(kf_rm::authored::with_gsp_and_disp_rows(clash), Err(0x9b));
    let rows = kf_rm::authored::with_gsp_and_disp_rows(Vec::new()).expect("no clash");
    assert_eq!(rows.iter().map(|e| (e.engine_idx, e.vector_stall)).collect::<Vec<_>>(), [(50, 0x9b), (2, 0x9a)]);
}

/// ★ v3-gfx: `gr_zcull_info` is the host's `GR_GET_ZCULL_INFO` reply word for word; the host's
/// `NV_ERR_NOT_SUPPORTED` is `None` (no zcull on the die), and any OTHER refusal is a field
/// refusal by name — never a default row.
#[test]
fn gr_zcull_info_is_the_hosts_reply_and_only_not_supported_means_none() {
    struct H(Result<[u32; 10], u32>);
    impl HostControls for H {
        fn control(&mut self, cmd: u32, p: &mut [u8]) -> Result<(), HostRefusal> {
            assert_eq!(cmd, 0x2080_1206);
            assert_eq!(p.len(), 40, "NV2080_CTRL_GR_GET_ZCULL_INFO_PARAMS is ten NvU32");
            match self.0 {
                Ok(row) => {
                    for (i, w) in row.iter().enumerate() {
                        p[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
                    }
                    Ok(())
                }
                Err(st) => Err(HostRefusal { status: Some(st), detail: "refused".into() }),
            }
        }
    }
    let row = [32, 16, 1024, 2048, 64, 16, 32, 16, 256, 128];
    assert_eq!(hostquery::query_gr_zcull_info(&mut H(Ok(row))).expect("served"), Some(row));
    assert_eq!(hostquery::query_gr_zcull_info(&mut H(Err(0x56))).expect("no zcull"), None);
    assert!(matches!(hostquery::query_gr_zcull_info(&mut H(Err(0x1b))), Err(FieldCause::Host { cmd: 0x2080_1206, .. })));
    // …and the internal control's reply carries it as engine 0, every other engine zero.
    let enc = kf_abi::grstatic::encode_zcull_info(&row);
    assert_eq!(enc.len(), 320);
    for (i, w) in row.iter().enumerate() {
        assert_eq!(u32::from_le_bytes(enc[4 * i..4 * i + 4].try_into().expect("4")), *w);
    }
    assert!(enc[40..].iter().all(|&b| b == 0));
}

/// ★ A host that refuses `BIOS_GET_INFO_V2` still realizes: `vbios_version` is `None`, not a
/// refused field (cosmetic — coordinator, 2026-09-26).
#[test]
fn a_host_refusing_bios_info_still_fills_every_field_with_no_vbios_version() {
    struct NoBios(CompletedGa106);
    impl HostControls for NoBios {
        fn control(&mut self, cmd: u32, p: &mut [u8]) -> Result<(), HostRefusal> {
            if cmd == 0x2080_0810 {
                return Err(HostRefusal { status: Some(0x56), detail: "refused".into() });
            }
            self.0.control(cmd, p)
        }
    }
    let got = hostquery::query_host_facts(&mut NoBios(CompletedGa106(Ga106Replay::load())), Family::Ampere)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got.vbios_version, None);
}

/// ★ v3-gpcmask: the logical → physical GPC map is the real GA106's OWN answer — the query asks
/// `GRMGR_GET_GR_FS_INFO` byte-identically to libcuda's `cuInit` batch (so the committed capture
/// answers it), and the answer is the identity on this board — the fixture's `physical_id`s.
#[test]
fn the_gpc_map_is_the_real_ga106s_own_grmgr_answer() {
    let f = ga106::host_facts();
    let mut host = CompletedGa106(Ga106Replay::load());
    let g = hostquery::query_gr_geometry(&mut host, Some(&f.gr_info)).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(g.chiplet_gpc_map, [0, 1, 2]);
    assert_eq!(g.chiplet_gpc_map, f.gr_static.gpcs.iter().map(|r| r.physical_id).collect::<Vec<_>>());
    assert_eq!(g.tpc_counts, [4, 5, 5]);
    assert_eq!(g.fs_extra, None, "the optional batch has no capture");
    assert!(host.0.asked.contains(&kf_abi::grfsinfo::NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO));
}
