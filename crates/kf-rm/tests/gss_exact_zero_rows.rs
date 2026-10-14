//! ★★ Lane C 2026-10-14 — Windows-only GSS-legacy queries answered by the HOST (tier 2), for the
//! controls whose real-driver request is measured all-zero (`kf_abi::gssreplay` `exact` rows:
//! `0x2080852a`, `0x20808524`, `0x2080a70a`, `0x2080a088`, `0x2080852e`, `0x2080853a`,
//! `0x2080a618`). Pinned here, through `query_gss_replay` and `InitTablePolicy`:
//!
//! - the host is asked ONCE per exact row, with the all-zero request only (never a background
//!   fill: that would be a different, unmeasured question about a control with no open header);
//! - an all-zero guest request is answered with exactly the host's bytes;
//! - one non-zero byte makes it another question: not answered from the host's reply;
//! - a host that refused the row leaves the guest's request unanswered by this path.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::gssreplay::{Answer, ROWS};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::hostquery::{HostControls, HostRefusal, query_gss_replay};
use kf_rm::inittables::InitTablePolicy;

const PARAMS_AT: usize = 40;

/// The zero-request rows: the 580 branch's sizes (AD103/GA106 Windows 580.88 captures) and, below them, the 595 branch's
/// (GA104 Windows 596.51 / GSP 595.76 capture): the same controls, a different `paramsSize` per firmware branch.
const EXACT: [(u32, usize); 11] = [
    (0x2080_852a, 1784),
    (0x2080_8524, 4),
    (0x2080_a70a, 26),
    (0x2080_a088, 12),
    (0x2080_852e, 528),
    (0x2080_853a, 1456),
    (0x2080_a618, 8620),
    (0x2080_852e, 596),
    (0x2080_853a, 15352),
    (0x2080_a618, 8632),
    (0x2080_852e, 1432),
];

/// `0x2080853b` at the 595 branch: 14320 B, request word 1 = 1, the rest zero (the 580 capture issued it once at 1832).
const R853B: (u32, usize) = (0x2080_853b, 14320);

/// A host that answers every exact row with a recognisable pattern and records each request.
struct Recorder {
    asked: Vec<(u32, Vec<u8>)>,
    refuse: Option<u32>,
}

fn pattern(cmd: u32, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7) ^ (cmd as u8) | 1)
        .collect()
}

impl HostControls for Recorder {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        self.asked.push((cmd, params.to_vec()));
        if Some(cmd) == self.refuse
            || !(EXACT.iter().any(|(c, n)| *c == cmd && *n == params.len())
                || (cmd, params.len()) == R853B)
        {
            return Err(HostRefusal {
                status: Some(0x56),
                detail: "not this test's row".into(),
            });
        }
        params.copy_from_slice(&pattern(cmd, params.len()));
        Ok(())
    }
}

fn policy(gss: Vec<Answer>) -> InitTablePolicy {
    let mut host = ga106::host_facts();
    host.gss_replay = gss;
    InitTablePolicy::new(
        ga106::board(),
        std::sync::Arc::new(host),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
    )
}

fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0245u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 11,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn every_exact_row_is_in_rows_with_its_measured_size() {
    for (cmd, size) in EXACT {
        let r = ROWS
            .iter()
            .find(|r| r.cmd == cmd && r.size == size)
            .unwrap_or_else(|| panic!("{cmd:#x}/{size} row"));
        assert!(r.exact && r.inputs.is_empty(), "{cmd:#x}/{size}");
    }
    let r = ROWS
        .iter()
        .find(|r| r.cmd == R853B.0 && r.size == R853B.1)
        .expect("853b row");
    assert!(r.exact && r.inputs == [(4, 1)], "853b asks {{0, 1}} and nothing else");
}

#[test]
fn the_host_is_asked_once_with_zeros_only() {
    let mut h = Recorder {
        asked: Vec::new(),
        refuse: None,
    };
    let answers = query_gss_replay(&mut h);
    for (cmd, size) in EXACT {
        let asks: Vec<&Vec<u8>> = h
            .asked
            .iter()
            .filter(|(c, p)| *c == cmd && p.len() == size)
            .map(|(_, p)| p)
            .collect();
        assert_eq!(
            asks.len(),
            1,
            "{cmd:#x}/{size}: asked exactly once (no background probe)"
        );
        assert!(
            asks[0].len() == size && asks[0].iter().all(|&b| b == 0),
            "{cmd:#x}: zero request"
        );
        assert!(
            answers.iter().any(|a| a.row.cmd == cmd && a.row.size == size),
            "{cmd:#x}/{size} kept"
        );
    }
    // 853b: asked once, the request is word 1 = 1 and zeros.
    let asks: Vec<&Vec<u8>> = h
        .asked
        .iter()
        .filter(|(c, p)| *c == R853B.0 && p.len() == R853B.1)
        .map(|(_, p)| p)
        .collect();
    assert_eq!(asks.len(), 1);
    assert_eq!(&asks[0][4..8], &1u32.to_le_bytes());
    assert!(asks[0].iter().enumerate().all(|(i, &b)| (4..8).contains(&i) || b == 0));
}

#[test]
fn a_zero_guest_request_gets_the_hosts_bytes_and_any_other_does_not() {
    let mut h = Recorder {
        asked: Vec::new(),
        refuse: None,
    };
    let answers = query_gss_replay(&mut h);
    let mut p = policy(answers);
    for (cmd, size) in EXACT {
        let r = p
            .respond(&control(cmd, &vec![0u8; size]))
            .expect("answered");
        assert_eq!(r.rpc_result, 0, "{cmd:#x}");
        assert_eq!(
            &r.body[PARAMS_AT..PARAMS_AT + size],
            &pattern(cmd, size)[..],
            "{cmd:#x}"
        );
        // One non-zero byte: another question.
        let mut other = vec![0u8; size];
        other[size - 1] = 1;
        let r = p.respond(&control(cmd, &other));
        assert!(
            r.as_ref()
                .is_none_or(|r| r.rpc_result != 0 || r.body[PARAMS_AT..] != pattern(cmd, size)[..]),
            "{cmd:#x}: a different request is not answered from the host's zero reply"
        );
    }
}

#[test]
fn a_refused_row_is_not_answered() {
    let mut h = Recorder {
        asked: Vec::new(),
        refuse: Some(0x2080_852a),
    };
    let answers = query_gss_replay(&mut h);
    assert!(!answers.iter().any(|a| a.row.cmd == 0x2080_852a));
    let r = policy(answers).respond(&control(0x2080_852a, &[0u8; 1784]));
    assert!(
        r.is_none_or(|r| r.rpc_result != 0),
        "never a constant where the host gave nothing"
    );
}

#[test]
fn the_595_853b_request_is_answered_only_when_word_one_is_one() {
    let mut h = Recorder {
        asked: Vec::new(),
        refuse: None,
    };
    let answers = query_gss_replay(&mut h);
    let mut p = policy(answers);
    let (cmd, size) = R853B;
    let mut real = vec![0u8; size];
    real[4..8].copy_from_slice(&1u32.to_le_bytes());
    let r = p.respond(&control(cmd, &real)).expect("answered");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(&r.body[PARAMS_AT..PARAMS_AT + size], &pattern(cmd, size)[..]);
    // The all-zero request of the same size is another question.
    let r = p.respond(&control(cmd, &vec![0u8; size]));
    assert!(
        r.is_none_or(|r| r.rpc_result != 0 || r.body[PARAMS_AT..] != pattern(cmd, size)[..]),
        "word 1 = 0 is not the measured question"
    );
}
