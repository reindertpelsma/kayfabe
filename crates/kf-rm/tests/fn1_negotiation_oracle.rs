//! kayfabe's fn-1 (`SET_GUEST_SYSTEM_INFO`) handling against NVIDIA's own guest-side code.
//!
//! The oracle (`kf-abi/tests/oracle/fn1_negotiation_oracle.c`) is ogkm 595.84's real
//! `RmRpcSetGuestSystemInfo`, driven as the GUEST. Here the HOST end of every exchange is kayfabe's
//! real `GuestSystemInfoPolicy`: each request the guest issues is answered by `respond`, converted to
//! the wire exactly as `GspFsm` does (`RpcCommand::reply`: envelope `rpc_result` and
//! `rpc_result_private`, body clamped to the request), and handed back to the guest.
//!
//! ## What is compared
//!
//! For every driver tag D kayfabe serves (the `guest-driver=` rows, `MEASURED`) and every guest VGX
//! pair g in the oracle's universe (the ends of each internal->external bucket of the ogkm tree's own
//! mapping table, plus every served row's pair), with a guest that says it IS driver D:
//!
//! * the guest's decision against kayfabe, and
//! * the guest's decision against the oracle's reference host that serves D's pair (`steer-to-host`:
//!   OK + a private status + the host's pair in the body when the guest's pair differs, OK/OK when it
//!   matches).
//!
//! must be identical: status, exchanges issued, agreed pair, the RPC version table selected, the fn-64
//! tail call. No version number is typed here; the pairs come from the oracle and the rows.
//!
//! Skips (`FN1-ORACLE-GATE: SKIPPED ...`) when the ogkm 595.84 tree is absent.

#[path = "../../kf-abi/tests/support/fn1_oracle.rs"]
mod fn1_oracle;

use fn1_oracle::{Done, Guest, HostReply, cell, consts, drive, hexnum, oracle};
use kf_abi::DriverVersion;
use kf_abi::generated::matrix::MEASURED;
use kf_abi::versions::table_for;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::guestsysinfo::GuestSystemInfoPolicy;
use std::collections::BTreeSet;

/// A served row and the pair its layouts speak.
fn served_rows() -> Vec<(DriverVersion, (u32, u32))> {
    MEASURED
        .iter()
        .filter_map(|v| {
            let t = table_for(*v).ok()?;
            let p = t.vgx_version()?;
            Some((*v, (p.major, p.minor)))
        })
        .collect()
}

/// Answer one guest request with the real policy, converted to the wire as the FSM does.
fn kayfabe_answers(
    policy: &mut GuestSystemInfoPolicy,
    seq: &mut u32,
    req: &[u8],
) -> HostReply {
    *seq += 1;
    let cmd = RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 1,
        sequence: *seq,
        payload: req.to_vec(),
        elements: 1,
        delivered: Vec::new(),
    };
    let reply = policy.respond(&cmd).expect("fn 1 is always answered");
    let out = wire(&cmd, &reply);
    HostReply {
        rpc_result: out.rpc_result,
        rpc_result_private: out.rpc_result_private,
        body: out.payload,
    }
}

/// `Reply` -> wire envelope, the way `kf_gsp::GspFsm::answer` does it.
fn wire(cmd: &RpcCommand, r: &kf_gsp::Reply) -> kf_gsp::OutgoingRpc {
    cmd.reply(r.rpc_result, &r.body)
}

/// The universe of guest pairs: everything the ogkm tree names, plus every served row's pair.
fn pair_universe(bin: &std::path::Path, rows: &[(DriverVersion, (u32, u32))]) -> BTreeSet<(u32, u32)> {
    let c = consts(bin);
    let mut u: BTreeSet<(u32, u32)> = rows.iter().map(|(_, p)| *p).collect();
    for m in &c["map"] {
        // "2d.00-2d.03"
        let (lo, hi) = m.split_once('-').unwrap();
        for e in [lo, hi] {
            let (a, b) = e.split_once('.').unwrap();
            u.insert((hexnum(a), hexnum(b)));
        }
    }
    u
}

fn describe(d: &Done) -> String {
    format!(
        "status={:#x} issued={} agreed={:?} ip_version={:?} ext_calls={}",
        d.status, d.issued, d.agreed, d.ip_version, d.ext_calls
    )
}

#[test]
fn kayfabe_and_the_real_guest_agree_on_every_pair_of_every_served_row() {
    let Some(bin) = oracle("kayfabe_and_the_real_guest_agree") else {
        return;
    };
    let rows = served_rows();
    assert!(rows.len() >= 5, "the served rows: {rows:?}");
    let universe = pair_universe(&bin, &rows);
    let mut divergences = Vec::new();
    let mut exact = 0;
    for (driver, host_pair) in &rows {
        let said = driver.to_string();
        for &guest_pair in &universe {
            let mut seq = 0;
            // A fresh policy per exchange: no state may carry a verdict from one guest to the next.
            let mut policy = GuestSystemInfoPolicy::new(*table_for(*driver).unwrap());
            let (got, _) = drive(
                &bin,
                &Guest {
                    major: guest_pair.0,
                    minor: guest_pair.1,
                    driver: &said,
                    branch: "rel/gpu_drv/oracle",
                    cl: 0,
                },
                |req| kayfabe_answers(&mut policy, &mut seq, req),
            );
            let want = cell(&bin, "steer-to-host", guest_pair, *host_pair).done;
            if guest_pair == *host_pair {
                exact += 1;
                assert!(got.accepted(), "exact match must keep working: {driver} {got:?}");
                assert_eq!(got.issued, 1, "{driver}");
            }
            if got != want {
                divergences.push(format!(
                    "driver {driver} (row pair {:02x}.{:02x}) vs guest pair {:02x}.{:02x}:\n    kayfabe : {}\n    oracle  : {}",
                    host_pair.0, host_pair.1, guest_pair.0, guest_pair.1,
                    describe(&got), describe(&want)
                ));
            }
        }
    }
    assert!(exact >= rows.len());
    assert!(
        divergences.is_empty(),
        "{} divergence(s) from the real guest's handshake:\n  {}",
        divergences.len(),
        divergences.join("\n  ")
    );
}
