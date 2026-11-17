//! ★ Replay of the logged RUSD/utilisation request shapes (`docs/design/V3_REPLAY_TESTS.md` pattern; fixture
//! `tests/fixtures/utilization_logged_shapes.txt`, whose `logged-*` rows are verbatim lines of committed logs).
//!
//! What a replay can show here: given the logged requests, the served chain's DECISION (answer or refuse) and the
//! derived layout agree with what the real GSP and the pre-feature device logged. What it cannot show: timing,
//! what NVML does with the answer, or what the real GSP wrote into the page (the logs do not carry it).
//! `STATUS: LIVE, 2026-11-17`.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::utilization as ku;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::utilization::{Clock, UtilizationSeat, UtilizationState};
use kf_rm::{ChainLogs, ObjectLinks};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const FIXTURE: &str = include_str!("fixtures/utilization_logged_shapes.txt");

/// One `KAYFABE-RPC` line, parsed: `(cmd, psize, gspst, head bytes)`.
fn parse_kayfabe_rpc(line: &str) -> (u32, usize, u32, Vec<u8>) {
    let field = |key: &str| -> &str {
        let at = line.find(key).unwrap_or_else(|| panic!("{key} in {line}")) + key.len();
        line[at..].split_whitespace().next().unwrap()
    };
    let hex = |s: &str| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap();
    let head: Vec<u8> = line[line.find("head=").unwrap() + 5..]
        .split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect();
    (
        hex(field("cmd=")),
        field("psize=").parse().unwrap(),
        hex(field("gspst=")),
        head,
    )
}

fn logged(n: u32) -> &'static str {
    let key = format!("logged-{n} ");
    FIXTURE
        .lines()
        .find_map(|l| l.strip_prefix(key.as_str()))
        .unwrap_or_else(|| panic!("fixture row logged-{n}"))
}

#[derive(Debug, Default)]
struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

const GUEST: DriverVersion = DriverVersion {
    major: 580,
    minor: 65,
    patch: 6,
};

fn chain(on: bool) -> (Box<dyn CommandPolicy>, ChainLogs, Arc<UtilizationState>, kf_abi::versions::DriverAbiTable) {
    let abi = *kf_abi::versions::table_for(GUEST).expect("580.65.06");
    let state = UtilizationState::new(Arc::new(FakeClock(AtomicU64::new(1_000_000_000))));
    let logs = ChainLogs::default();
    let c = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        logs.clone(),
        Default::default(),
        ObjectLinks {
            utilization: on.then(|| UtilizationSeat {
                state: state.clone(),
            }),
            ..Default::default()
        },
    );
    (c, logs, state, abi)
}

fn control(abi: &kf_abi::versions::DriverAbiTable, cmd: u32, params: &[u8]) -> RpcCommand {
    let w = abi.rm_control_wire();
    let mut payload = vec![0u8; w.params_off + params.len()];
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4]
        .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
    payload[w.params_off..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The three logged requests as the guest sent them: params rebuilt from the logged head bytes and the two
/// labelled synthetic fields.
fn logged_requests() -> Vec<(u32, Vec<u8>)> {
    [1, 2, 3]
        .map(|n| {
            let (cmd, psize, _, head) = parse_kayfabe_rpc(logged(n));
            let mut p = vec![0u8; psize];
            p[..head.len()].copy_from_slice(&head);
            if cmd == 0x2080_0afe && head[0] == 1 {
                p[8..16].copy_from_slice(&0x0000_0001_2345_6000u64.to_le_bytes()); // synthetic-1
            }
            if cmd == 0x2080_0aff {
                p[8..12].copy_from_slice(&1000u32.to_le_bytes()); // synthetic-2
            }
            (cmd, p)
        })
        .to_vec()
}

/// ★ The replay starts from the logged state: the logged ids, sizes and the real GSP's status are what the
/// derived layout says, and the pre-feature device's logged refusal is what the property-OFF chain does.
#[test]
fn the_logged_shapes_are_the_derived_ones_and_off_reproduces_the_logged_refusal() {
    let l = ku::layout(GUEST).expect("a row for 580.65.06");
    let rows = [1, 2, 3].map(|n| parse_kayfabe_rpc(logged(n)));
    assert_eq!(rows[0].0 as u64, l.rusd_init_cmd, "INIT id");
    assert_eq!(rows[1].0 as u64, l.rusd_poll_cmd, "POLL id");
    assert_eq!(rows[2].0 as u64, l.rusd_init_cmd, "INIT id at teardown");
    assert_eq!(rows[0].1 as u64, l.init_size, "logged psize = generated INIT size");
    assert_eq!(rows[1].1 as u64, l.poll_size, "logged psize = generated POLL size");
    assert_eq!(rows[2].1 as u64, l.init_size);
    assert!(rows.iter().all(|r| r.2 == 0), "the REAL GSP answered every one with status 0");
    assert_eq!(rows[0].3[l.init_off_binit as usize], 1, "bInit at init");
    assert_eq!(rows[2].3[l.init_off_binit as usize], 0, "bInit at teardown");
    assert_eq!(rows[1].3[..8], [0u8; 8], "the first poll's mask is 0");

    // Property OFF: the chain refuses exactly the logged ids, the way the pre-feature kf3 log says.
    let (mut off, logs, _, abi) = chain(false);
    for (cmd, params) in logged_requests() {
        assert!(off.respond(&control(&abi, cmd, &params)).is_none());
    }
    let refused: Vec<String> = logs
        .unserviced
        .sample()
        .iter()
        .map(|u| format!("fn{}/{:#010x}=0x56", u.function, u.cmd.unwrap()))
        .collect();
    assert!(logged(5).contains(&refused[0]), "{} vs {}", refused[0], logged(5));
    assert!(logged(7).contains(&refused[1]), "{} vs {}", refused[1], logged(7));
    assert_eq!(refused.len(), 2, "the ledger keeps DISTINCT commands: INIT (twice) and POLL");
    assert_eq!(logs.unserviced.total(), 3, "INIT, POLL, INIT (teardown)");
}

/// ★ The logged sequence with the property ON: the three requests are answered `NV_OK`, as the real GSP
/// answered them, and the mask-0 poll asks for nothing (no record is wanted, nothing would be written).
#[test]
fn the_logged_init_poll_teardown_sequence_is_answered_ok_and_publishes_nothing_for_mask_zero() {
    let (mut on, logs, state, abi) = chain(true);
    let reqs = logged_requests();
    let ok = |r: Option<kf_gsp::Reply>| r.is_some_and(|r| r.rpc_result == 0);
    assert!(ok(on.respond(&control(&abi, reqs[0].0, &reqs[0].1))), "INIT (link)");
    assert_eq!(state.rusd_page(), Some(0x0000_0001_2345_6000));
    assert!(ok(on.respond(&control(&abi, reqs[1].0, &reqs[1].1))), "POLL mask 0");
    assert!(!state.rusd_wants_utilization(), "mask 0 asks for no record");
    state.push(kf_abi::utilization::GuestSample {
        timestamp_ns: 1_000_000_000,
        percents: Default::default(),
    });
    assert_eq!(
        state.publish(&mut |_, _| panic!("nothing is written for mask 0")),
        kf_rm::utilization::PublishOutcome::NotWanted
    );
    assert!(ok(on.respond(&control(&abi, reqs[2].0, &reqs[2].1))), "INIT (unlink)");
    assert_eq!(state.rusd_page(), None);
    assert_eq!(logs.unserviced.total(), 0, "nothing of the logged sequence is refused any more");
}

/// ★ Windows `utilization.gpu` (run51): the poll the query adds is answered; the three closed controls it also
/// sends have no derived layout and stay refused exactly as logged.
#[test]
fn the_windows_utilization_query_gets_the_poll_and_the_closed_controls_stay_refused() {
    let l = ku::layout(GUEST).unwrap();
    let (mut on, logs, state, abi) = chain(true);
    // the poll the query sends: the same 16-byte shape, with the PERF bit among the bits
    let mut poll = vec![0u8; 16];
    poll[..8].copy_from_slice(&(l.poll_perf).to_le_bytes());
    assert!(on.respond(&control(&abi, l.rusd_poll_cmd as u32, &poll)).is_some_and(|r| r.rpc_result == 0));
    assert!(state.rusd_wants_utilization());
    for (n, size) in [(9, 4usize), (10, 64), (11, 992)] {
        let line = logged(n);
        let id = u32::from_str_radix(line.split("0x").nth(1).unwrap().split_whitespace().next().unwrap(), 16).unwrap();
        assert!(line.ends_with("result=none"));
        assert!(
            on.respond(&control(&abi, id, &vec![0u8; size])).is_none(),
            "{id:#010x} has no derived layout: it stays refused"
        );
    }
    assert_eq!(logs.unserviced.total(), 3);
}
