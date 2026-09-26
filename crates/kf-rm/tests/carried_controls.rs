//! ★★★ The served controls CARRIED to another guest version's layout (`respond_transcoded`,
//! `docs/design/V3_DRIVER_MATRIX.md` §4.5), end to end through `InitTablePolicy`.
//!
//! For every reviewed control and every measured version whose params layout differs from the
//! bench's, the same logical request is served twice — by a policy for the bench driver and by
//! one for the guest's version — and the guest's reply must be EXACTLY the bench reply carried
//! by the measured transcoder, inside a correctly re-sized control header. A carry that dropped,
//! shifted or re-sized anything the encoder wrote shows here as a byte diff, per version.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::{ALL_STRUCTS, MEASURED};
use kf_abi::matrix::{Resolved, transcode};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::inittables::{InitTablePolicy, WantedTable, layout_differs_from_bench};

fn policy(v: DriverVersion) -> InitTablePolicy {
    InitTablePolicy::new(ga106::board(), ga106::host(), *table_for(v).unwrap_or_else(|e| panic!("{v}: {e}")))
}

/// A `GSP_RM_CONTROL` in version `v`'s RM-control wire, carrying `params`.
fn command(v: DriverVersion, cmd_id: u32, params: &[u8]) -> RpcCommand {
    let w = table_for(v).expect("table").rm_control_wire();
    let mut payload = vec![0u8; w.params_off + params.len()];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&cmd_id.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[w.params_off..].copy_from_slice(params);
    RpcCommand { function: RpcFunction::RmControl, code: 0x23, sequence: 40, payload, elements: 1, delivered: Vec::new() }
}

fn runs(ct: &str) -> &'static kf_abi::matrix::StructRuns {
    ALL_STRUCTS.iter().find(|r| r.name == ct).unwrap_or_else(|| panic!("{ct} not in the matrix"))
}

/// The controls whose request carries INPUT the encoder reads (an index list): the same logical
/// request is stated at both versions. Every other reviewed control is `[OUT]`-only (zero request).
fn request(ct: &str, lay: &Resolved) -> Vec<u8> {
    let mut p = vec![0u8; lay.size()];
    if ct == "NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS" {
        // GPC_COUNT, then CHIPLET_GPC_MAP for logical GPCs 0 and 1 (`kf_abi::grfsinfo`).
        let n = lay.maybe("numQueries").expect("numQueries").off();
        p[n..n + 2].copy_from_slice(&3u16.to_le_bytes());
        let (q0, stride) = (lay.maybe("queries[]").expect("queries[]").off(), lay.maybe("queries[]").expect("q").bytes().expect("elem"));
        let t = lay.maybe("queries[].queryType").expect("queryType").off() - q0;
        let d = lay.maybe("queries[].queryData").expect("queryData").off() - q0;
        for (k, (qt, gpc)) in [(1u16, 0u32), (2, 0), (2, 1)].iter().enumerate() {
            let at = q0 + k * stride;
            p[at + t..at + t + 2].copy_from_slice(&qt.to_le_bytes());
            p[at + d..at + d + 4].copy_from_slice(&gpc.to_le_bytes());
        }
        return p;
    }
    let list = match ct {
        // The one index a GSP answers (0x11, forwarded by the guest kernel with bit 31 set —
        // `kf_abi::gpuinfo`), beside indices the kernel answered itself (passed through).
        "NV2080_CTRL_GPU_GET_INFO_V2_PARAMS" => {
            Some(("gpuInfoListSize", "gpuInfoList[].index", "gpuInfoList[]", [0x8000_0011u32, 0x1, 0x2, 0x3, 0x12]))
        }
        // BUS_WIDTH, RAM_TYPE, FBP_COUNT, FBP_MASK, L2CACHE_SIZE — indices the GSP answers (`kf_abi::fbinfo`).
        "NV2080_CTRL_FB_GET_INFO_V2_PARAMS" => Some(("fbInfoListSize", "fbInfoList[].index", "fbInfoList[]", [0x0bu32, 0x0d, 0x19, 0x1a, 0x1b])),
        _ => None,
    };
    if let Some((count, idx, el, indices)) = list {
        let c = lay.maybe(count).expect("count field").off();
        p[c..c + 4].copy_from_slice(&(indices.len() as u32).to_le_bytes());
        let (i0, stride) = (lay.maybe(idx).expect("index field").off(), lay.maybe(el).expect("element").bytes().expect("elem"));
        for (k, ix) in indices.iter().enumerate() {
            p[i0 + k * stride..i0 + k * stride + 4].copy_from_slice(&ix.to_le_bytes());
        }
    }
    p
}

#[test]
fn every_carried_control_is_the_bench_answer_carried_by_name() {
    let carried = [
        WantedTable::GrFloorsweepingMasks,
        WantedTable::GrGlobalSmOrder,
        WantedTable::GrInfo,
        WantedTable::GpuInfoV2,
        WantedTable::FbGetInfoV2,
        WantedTable::GrmgrGetGrFsInfo,
        WantedTable::C2cInfo,
        WantedTable::InternalDeviceInfo,
        WantedTable::UserRegisterAccessMap,
    ];
    let bench_policy_reply = |w: WantedTable, ct: &str, blay: &Resolved| -> Vec<u8> {
        let mut p = policy(BENCH_DRIVER);
        let r = p.respond(&command(BENCH_DRIVER, w.cmd_id(), &request(ct, blay))).expect("served at the bench");
        assert_eq!(r.rpc_result, 0, "{w:?} at the bench");
        let at = table_for(BENCH_DRIVER).expect("t").rm_control_wire().params_off;
        r.body[at..at + blay.size()].to_vec()
    };
    let mut checked = 0;
    for w in carried {
        let ct = w.c_type().expect("c_type");
        let rr = runs(ct);
        let blay = Resolved::of(rr, BENCH_DRIVER).expect("bench layout");
        let bench = bench_policy_reply(w, ct, &blay);
        for &v in MEASURED {
            // Versions the guest axis serves (a capability row) where this layout moved.
            if layout_differs_from_bench(ct, v).is_none() && Resolved::of(rr, v).map(|g| g.layout == blay.layout).unwrap_or(true) {
                continue;
            }
            let Ok(t) = table_for(v) else { continue };
            let Ok(glay) = Resolved::of(rr, v) else { continue };
            let mut p = InitTablePolicy::new(ga106::board(), ga106::host(), *t);
            let r = p
                .respond(&command(v, w.cmd_id(), &request(ct, &glay)))
                .unwrap_or_else(|| panic!("{w:?} declined at {v}"));
            assert_eq!(r.rpc_result, 0, "{w:?} refused at {v}");
            let wire = t.rm_control_wire();
            let size = u32::from_le_bytes(r.body[wire.params_size_off..wire.params_size_off + 4].try_into().expect("4"));
            assert_eq!(size as usize, glay.size(), "{w:?} at {v}: paramsSize in the reply");
            let got = &r.body[wire.params_off..wire.params_off + glay.size()];
            let truncatable: &[&str] = if w == WantedTable::GrInfo { &["engineInfo[].infoList"] } else { &[] };
            let (want, _) = transcode(&blay, &glay, &bench, truncatable).unwrap_or_else(|e| panic!("{w:?} to {v}: {e}"));
            assert_eq!(got, want.as_slice(), "{w:?} at {v}: the reply is not the bench answer carried by name");
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} (control, version) carries were checked");
}
