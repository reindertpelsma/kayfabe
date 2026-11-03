//! ★★ Lane C 2026-10-14 — libcudart's init gate (`0x20809009`, `0x20809001`, `0x20809064`,
//! `0x2080a001`, `0x20802209`) is no longer ONE GA106 capture served to every die
//! (`docs/design/V3_CONSTANT_MATRIX_20261014.md`, `kf_rm::consttab`). Through `InitTablePolicy`:
//!
//! - the host's realize-time answer to cudart's own request wins (tier 2, `kf_abi::gssreplay`);
//! - with no host answer, the PUBLISHED table answers — only on the die and guest branch a row was
//!   measured on (GA106 at 580 here), and only for the measured request;
//! - every other die is REFUSED by name (never a neighbour's row): AD104/TU116/GB205 at 580 with
//!   no host answer get `NV_ERR_NOT_SUPPORTED`, not GA106's bytes;
//! - `0x20802209` carries a mechanism tag (RM software state) and answers on every die.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::gssreplay::{Answer, PROBE_BACKGROUND, ROWS};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_chip::{DieId, arch};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::inittables::InitTablePolicy;

const PARAMS_AT: usize = 40;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

const GA106: DieId = DieId::new(arch::GA100, 6);
const AD104: DieId = DieId::new(arch::AD100, 4);
const TU116: DieId = DieId::new(arch::TU100, 8);
const GB205: DieId = DieId::new(arch::GB200, 5);
/// Same family and branch as the GA106 rows: the neighbour a family rule would wrongly serve.
const GA104: DieId = DieId::new(arch::GA100, 4);

fn policy(die: DieId, gss: Vec<Answer>) -> InitTablePolicy {
    let mut host = ga106::host_facts();
    host.die = die;
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
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// cudart's own 520-byte `0x20809064` request (`[measured AD104/595.91.07, libcudart 13.3]`).
fn cudart_9064() -> Vec<u8> {
    let mut p = vec![0u8; 520];
    p[4..8].copy_from_slice(&2u32.to_le_bytes());
    p[8..12].copy_from_slice(&1u32.to_le_bytes());
    p[0x18..0x1c].copy_from_slice(&4u32.to_le_bytes());
    p
}

fn words(b: &[u8], n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
        .collect()
}

fn served(r: &kf_gsp::Reply) -> bool {
    r.rpc_result == 0
}

/// No host answer, the measured die and branch: the table's GA106 rows.
#[test]
fn the_measured_die_is_answered_from_its_own_rows() {
    let mut p = policy(GA106, Vec::new());
    let r = p.respond(&control(0x2080_9009, &[0; 8])).expect("answered");
    assert!(served(&r));
    assert_eq!(words(&r.body[PARAMS_AT..], 2), vec![0, 0xd]);
    let r = p
        .respond(&control(0x2080_9064, &cudart_9064()))
        .expect("answered");
    assert!(served(&r));
    assert_eq!(
        words(&r.body[PARAMS_AT..], 10),
        vec![0, 2, 1, 1, 1, 0x64, 4, 0x10, 1, 0x64]
    );
}

/// ⊘ The core rule: with no host answer, a die with no measured row is REFUSED — GA106's bytes
/// are never served to an AD104, TU116 or GB205.
#[test]
fn a_neighbour_die_is_refused_not_served_ga106s_bytes() {
    for die in [GA104, AD104, TU116, GB205] {
        let mut p = policy(die, Vec::new());
        for (cmd, req) in [
            (0x2080_9009u32, vec![0u8; 8]),
            (0x2080_9001, vec![0u8; 8]),
            (0x2080_9064, cudart_9064()),
            (0x2080_a001, vec![0u8; 16]),
        ] {
            let r = p.respond(&control(cmd, &req));
            assert!(
                r.as_ref()
                    .is_none_or(|r| r.rpc_result == NV_ERR_NOT_SUPPORTED),
                "{die} {cmd:#x}: refused, never a neighbour's row ({:?})",
                r.map(|r| r.rpc_result)
            );
        }
    }
}

/// ⊘ The request is part of the question: the encoder's `{0, 1, 8}` is not cudart's, even on
/// the die that has the row.
#[test]
fn another_request_is_refused_from_cudarts_row() {
    let mut enc = vec![0u8; 520];
    enc[4..8].copy_from_slice(&1u32.to_le_bytes());
    enc[8..12].copy_from_slice(&8u32.to_le_bytes());
    let r = policy(GA106, Vec::new()).respond(&control(0x2080_9064, &enc));
    assert!(r.is_none_or(|r| r.rpc_result == NV_ERR_NOT_SUPPORTED));
}

/// ★ Tier 2 first: when the host answered cudart's request at realize, ANY die is answered with
/// the host's own bytes — no table involved.
#[test]
fn the_hosts_answer_serves_any_die() {
    let row = *ROWS
        .iter()
        .find(|r| r.cmd == 0x2080_9009)
        .expect("cudart 9009 row");
    let host = |mut b: Vec<u8>| {
        b[4..8].copy_from_slice(&0xdu32.to_le_bytes());
        b[0..4].copy_from_slice(&0u32.to_le_bytes());
        b
    };
    let (a, both) = Answer::from_probes(
        row,
        &host(row.request()),
        Some(&host(row.request_over(PROBE_BACKGROUND))),
    );
    assert!(both);
    for die in [AD104, TU116, GB205, DieId::new(arch::AD100, 1)] {
        let r = policy(die, vec![a.clone()])
            .respond(&control(0x2080_9009, &[0; 8]))
            .expect("answered");
        assert!(served(&r), "{die}");
        assert_eq!(words(&r.body[PARAMS_AT..], 2), vec![0, 0xd]);
    }
}

/// ★ A mechanism row (RM software state, not a die fact) answers every die.
#[test]
fn the_watchdog_state_is_answered_on_every_die() {
    for die in [GA106, AD104, TU116, GB205] {
        let r = policy(die, Vec::new())
            .respond(&control(0x2080_2209, &[0; 4]))
            .expect("answered");
        assert!(served(&r), "{die}");
        assert_eq!(words(&r.body[PARAMS_AT..], 1), vec![0x5]);
    }
}
