//! ★★★★★ **A floor-swept die, end to end, over its OWN replies** (v3-gpcmask, 2026-09-26).
//!
//! `traces/real_ga104/grfs_probe_real_ga104_3060ti.txt` is an RTX 3060 Ti (GA104, 580.159.04,
//! open module) answering an UNPRIVILEGED client — `scripts/bench/probes/grfs_probe.c`, which
//! issues exactly the request shapes `kf_rm::hostquery` issues and logs every
//! `CTRL cmd= status= size= in= out=`. Its GPC mask is `0x3e` (physical GPC 0 fused): the die on
//! which realize refused, before this branch, with *"a non-contiguous GPC mask: GrStaticProfile
//! states GPCs 0..n"*.
//!
//! [`ProbeReplay`] answers a control ONLY from that file, by exact request bytes — a request the
//! probe never made is refused, never invented. Over it this file runs the realize path
//! (`query_gr_info`, `query_gr_geometry`, `gr_static_from`) and then checks the property the fix
//! exists for: ★ **what the guest's own RM would answer its userspace, from the replies we
//! serve, equals what the host's RM answered ours** — control by control, index by index,
//! including the out-of-range statuses.

use std::collections::BTreeMap;

use kf_abi::grfsinfo::{self, GrFsGeometry};
use kf_abi::grstatic::{self, FLOORSWEEPING_ROW_SIZE, GR_MAX_GPC, MAX_TPC_PER_GPC};
use kf_rm::hostfacts;
use kf_rm::hostquery::{self, HostControls, HostRefusal};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex")).collect()
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(key))
}

/// Every logged control: `(cmd, in, status, out)`, in file order.
fn probe() -> Vec<(u32, Vec<u8>, u32, Vec<u8>)> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../traces/real_ga104/grfs_probe_real_ga104_3060ti.txt");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let rows: Vec<_> = text
        .lines()
        .filter(|l| l.starts_with("CTRL "))
        .map(|l| {
            let w = |k| u32::from_str_radix(field(l, k).expect(k).trim_start_matches("0x"), 16).expect("hex word");
            let (i, o) = (unhex(field(l, "in=").expect("in=")), unhex(field(l, "out=").expect("out=")));
            let size: usize = field(l, "size=").expect("size=").parse().expect("size");
            assert_eq!((i.len(), o.len()), (size, size), "a complete record: {}", &l[..60]);
            (w("cmd="), i, w("status="), o)
        })
        .collect();
    assert!(rows.len() > 100, "the probe logged {} controls — the file is not the capture", rows.len());
    assert!(text.contains("GRFS_PROBE_DONE"), "the probe ran to its end");
    rows
}

/// ★ The host, as the probe recorded it — and nothing else.
struct ProbeReplay {
    ctrls: BTreeMap<u32, Vec<(Vec<u8>, u32, Vec<u8>)>>,
    asked: Vec<u32>,
}

impl ProbeReplay {
    fn load() -> ProbeReplay {
        let mut ctrls: BTreeMap<u32, Vec<(Vec<u8>, u32, Vec<u8>)>> = BTreeMap::new();
        for (cmd, i, st, o) in probe() {
            ctrls.entry(cmd).or_default().push((i, st, o));
        }
        ProbeReplay { ctrls, asked: Vec::new() }
    }

    /// The host's reply to exactly `input`, if the probe asked it.
    fn answer(&self, cmd: u32, input: &[u8]) -> Option<&(Vec<u8>, u32, Vec<u8>)> {
        self.ctrls.get(&cmd)?.iter().find(|(i, _, _)| i.as_slice() == input)
    }
}

impl HostControls for ProbeReplay {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        self.asked.push(cmd);
        let (_, st, out) = self
            .answer(cmd, params)
            .ok_or_else(|| HostRefusal { status: None, detail: format!("the probe never asked {cmd:#010x} with these bytes") })?;
        if *st != 0 {
            return Err(HostRefusal { status: Some(*st), detail: "the host refused it".into() });
        }
        params.copy_from_slice(out);
        Ok(())
    }
}

fn w32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4"))
}

/// The 3060 Ti's GR profile, through the realize path.
fn realize() -> (kf_abi::grinfo::GrInfoProfile, hostquery::GrGeometry, grstatic::GrStaticProfile) {
    let mut host = ProbeReplay::load();
    let info = hostquery::query_gr_info(&mut host).expect("GR_GET_INFO_V2 captured");
    let geo = hostquery::query_gr_geometry(&mut host, Some(&info)).unwrap_or_else(|e| panic!("{e:?}"));
    let p = hostquery::gr_static_from(&geo, &info).unwrap_or_else(|e| panic!("{e:?}"));
    (info, geo, p)
}

/// ★★★ Realize SUCCEEDS on the die it refused — and states the die's own geometry.
#[test]
fn a_3060ti_realizes_with_its_own_non_contiguous_mask() {
    let (info, g, p) = realize();
    assert_eq!(g.gpc_mask, 0x3e, "physical GPC 0 fused");
    assert_eq!(g.tpc_masks, [(1, 0xe), (2, 0xf), (3, 0xf), (4, 0xf), (5, 0xf)]);
    assert_eq!(g.chiplet_gpc_map, [1, 2, 3, 4, 5], "the host's own CHIPLET_GPC_MAP");
    assert_eq!(g.tpc_counts, [3, 4, 4, 4, 4]);
    let (pes, t2p) = g.pes.clone().expect("GPU_GET_PES_INFO answered");
    assert_eq!(pes, [2, 2, 2, 2, 2], "two PES per GPC on GA104 — NOT the litter's 3");
    assert_eq!(t2p, [0, 0, 1, 1, 2, 2, 0, 0, 0, 0]);
    assert_ne!(info.data[hostquery::GR_INFO_IDX_LITTER_NUM_PES_PER_GPC], 2, "the litter would have been wrong here");
    assert_eq!(g.gfx, (0x3e, 19));
    let extra = g.fs_extra.clone().expect("the optional GRMGR batch was answered");
    assert_eq!(extra.per_gpc, vec![(Some(3), Some(3)); 5]);
    assert_eq!(extra.syspipe, grstatic::GrSyspipeMasks { syspipe: 1, graphics_syspipe: 0 });
    assert_eq!((g.tpcs.len(), g.sms_per_tpc), (19, 2));

    assert_eq!(p.gpc_mask(), Ok(0x3e));
    p.validate().expect("validates");
    info.validate_against(&p).expect("GR info agrees with the GR rows");
}

/// ★★★ **The guest sees what the host sees.** The guest's RM answers its userspace's GR
/// floorsweeping controls from its cached copy of the `0x20800a26` reply WE serve; each handler
/// is re-stated here from `ogkm-580` and run over our encoded bytes, and compared — for EVERY
/// index 0..16 the probe swept — with the host RM's own answer to the same control.
#[test]
fn the_guests_gr_controls_answer_exactly_what_the_hosts_answered() {
    let (info, _, p) = realize();
    let fs = grstatic::encode_floorsweeping_masks(&p).expect("encodes");
    let row = &fs[..FLOORSWEEPING_ROW_SIZE];
    // Field offsets, ctrl2080internal.h:297-332 (kf_abi::grstatic's encoder).
    let arr = |base: usize, i: usize| w32(row, 4 * (base + i));
    let (gpc_mask, tpc_mask, tpc_count) = (w32(row, 0), |i| arr(1, i), |i| arr(1 + GR_MAX_GPC, i));
    let num_pes = |i| arr(2 + 3 * GR_MAX_GPC + MAX_TPC_PER_GPC, i);
    let zcull = |i| arr(2 + 4 * GR_MAX_GPC + MAX_TPC_PER_GPC, i);
    let litter_gpcs = info.data[kf_abi::grinfo::IDX_LITTER_NUM_GPCS];
    let host = ProbeReplay::load();
    let host_says = |cmd: u32, req: Vec<u8>| host.answer(cmd, &req).map(|(_, st, out)| (*st, out.clone())).expect("the probe asked it");
    const INVALID_ARGUMENT: u32 = 0x1f;
    const NOT_SUPPORTED: u32 = 0x56;

    // GR_GET_GPC_MASK — kgrmgrGetLegacyGpcMask: floorsweepingMasks.gpcMask.
    let (st, out) = host_says(hostfacts::NV2080_CTRL_CMD_GR_GET_GPC_MASK, vec![0; 24]);
    assert_eq!((st, w32(&out, 16)), (0, gpc_mask), "GR_GET_GPC_MASK");
    for g in 0..16u32 {
        let gi = g as usize;
        // GR_GET_TPC_MASK(g) — kgrmgrGetLegacyTpcMask: tpcMask[g] for g < LITTER_NUM_GPCS, else 0
        // with NV_OK (kernel_graphics_manager.c:612-628). g is PHYSICAL.
        let mut req = vec![0u8; 24];
        req[16..20].copy_from_slice(&g.to_le_bytes());
        let (st, out) = host_says(hostfacts::NV2080_CTRL_CMD_GR_GET_TPC_MASK, req);
        let ours = if g < litter_gpcs { tpc_mask(gi) } else { 0 };
        assert_eq!((st, w32(&out, 20)), (0, ours), "GR_GET_TPC_MASK({g})");
        // GR_GET_NUM_TPCS_FOR_GPC(l) — tpcCount[l] for l < popcount(gpcMask), else
        // NV_ERR_INVALID_ARGUMENT (kernel_graphics.c:3555-3562). l is LOGICAL.
        let (st, out) = host_says(hostfacts::NV2080_CTRL_CMD_GR_GET_NUM_TPCS_FOR_GPC, [g.to_le_bytes(), [0; 4]].concat());
        let ours = if g < gpc_mask.count_ones() { (0, tpc_count(gi)) } else { (INVALID_ARGUMENT, 0) };
        assert_eq!((st, w32(&out, 4)), ours, "GR_GET_NUM_TPCS_FOR_GPC({g})");
        // GR_GET_ZCULL_MASK(g) — zcullMask[g] for g < LITTER_NUM_GPCS (NV_U32_MAX → NOT_SUPPORTED),
        // else NV_ERR_INVALID_ARGUMENT (kernel_graphics.c:3808-3823). g is PHYSICAL.
        let (st, out) = host_says(hostquery::NV2080_CTRL_CMD_GR_GET_ZCULL_MASK, [g.to_le_bytes(), [0; 4]].concat());
        let ours = match (g < litter_gpcs, zcull(gi)) {
            (false, _) => (INVALID_ARGUMENT, 0),
            (true, u32::MAX) => (NOT_SUPPORTED, 0),
            (true, z) => (0, z),
        };
        assert_eq!((st, w32(&out, 4)), ours, "GR_GET_ZCULL_MASK({g})");
        // GPU_GET_PES_INFO(g).numPesInGpc — numPesPerGpc[g] for g < LITTER_NUM_GPCS
        // (subdevice_ctrl_gpu_kernel.c:1959-1965). ⚠ The guest's own call then returns
        // NOT_SUPPORTED (we refuse the PPC-mask control, so its pPpcMasks is NULL), so only the
        // array is compared here, where the host answered.
        let (st, out) = host_says(hostfacts::NV2080_CTRL_CMD_GPU_GET_PES_INFO, [g.to_le_bytes().to_vec(), vec![0; 52]].concat());
        if g < litter_gpcs {
            assert_eq!((st, w32(&out, 4)), (0, num_pes(gi)), "GPU_GET_PES_INFO({g}).numPesInGpc");
        } else {
            assert_eq!(st, INVALID_ARGUMENT, "GPU_GET_PES_INFO({g})");
        }
    }
    // GR_GET_PHYS_GPC_MASK and GR_GET_GFX_GPC_AND_TPC_INFO — the scalar fields.
    let (_, out) = host_says(hostfacts::NV2080_CTRL_CMD_GR_GET_PHYS_GPC_MASK, vec![0; 8]);
    assert_eq!(w32(&out, 4), w32(row, 4 * (1 + 2 * GR_MAX_GPC)), "physGpcMask");
    let (_, out) = host_says(hostfacts::NV2080_CTRL_CMD_GR_GET_GFX_GPC_AND_TPC_INFO, vec![0; 24]);
    let gfx_at = 4 * (2 + 5 * GR_MAX_GPC + MAX_TPC_PER_GPC);
    assert_eq!((w32(&out, 16), w32(&out, 20)), (w32(row, gfx_at), w32(row, gfx_at + 4)), "physGfxGpcMask, numGfxTpc");
}

/// ★★★ `GRMGR_GET_GR_FS_INFO` — which the guest kernel does NOT answer but forwards to US
/// (`kf_abi::grfsinfo`) — byte for byte against the host's reply to every batch the probe sent:
/// libcuda's map batch, kayfabe's optional batch, and the two exploratory sweeps past the GPC
/// count (whose per-query `0x1f` statuses are the measurement this port now serves).
#[test]
fn every_grmgr_batch_the_host_answered_is_answered_byte_for_byte() {
    let (_, _, p) = realize();
    let geom = GrFsGeometry::from_profile(&p);
    let host = ProbeReplay::load();
    let batches = &host.ctrls[&grfsinfo::NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO];
    assert_eq!(batches.len(), 4, "map, optional, and two exploratory batches");
    for (i, (input, st, out)) in batches.iter().enumerate() {
        assert_eq!(*st, 0, "batch {i}: the call succeeds; refusals are per query");
        let ours = grfsinfo::answer_gr_fs_info(input, &geom).unwrap_or_else(|e| panic!("batch {i}: {e}"));
        assert_eq!(&ours, out, "batch {i}");
    }
}

/// ⊘ The request order is the contract with the GA106 replay (`host_facts_query_ga106.rs`): a
/// host that cannot answer `GR_GET_ZCULL_MASK` is refused on that control. Here every control
/// the realize path asks is one the probe logged — so the probe IS the request shape.
#[test]
fn the_realize_path_asks_nothing_the_probe_did_not() {
    let mut host = ProbeReplay::load();
    let info = hostquery::query_gr_info(&mut host).expect("captured");
    hostquery::query_gr_geometry(&mut host, Some(&info)).expect("every request was one the probe made");
    let asked: std::collections::BTreeSet<u32> = host.asked.iter().copied().collect();
    assert_eq!(
        asked.into_iter().collect::<Vec<_>>(),
        [0x2080_0168, 0x2080_121b, 0x2080_1227, 0x2080_1228, 0x2080_122a, 0x2080_122b, 0x2080_1232, 0x2080_1234, 0x2080_1237, 0x2080_1239, 0x2080_3801]
    );
}
