//! ★ Lane C 2026-10-14 — a GPU-free ORACLE for the host-first cudart gate, from the measurement on
//! the home AD104 at host 595.91.07 (`traces/constants_night_20261014/host_cudart_probe_595.txt`,
//! `probe_8162_595.txt`): what libcudart's own requests got from the real host, word for word.
//!
//! The fake host below answers exactly those measured replies (and only for exactly those
//! requests). Through `query_gss_replay` → `InitTablePolicy`, the guest's identical requests must
//! come back byte-identical to what the host's own CUDA stack saw — on an AD104 die key, at a 580
//! guest branch, where the table has NO row (so the answer can only be the host's).

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_chip::{DieId, arch};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::hostquery::{HostControls, HostRefusal, query_gss_replay};
use kf_rm::inittables::InitTablePolicy;

const PARAMS_AT: usize = 40;

fn words_le(w: &[u32], size: usize) -> Vec<u8> {
    let mut b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
    b.resize(size, 0);
    b
}

/// CODE READ: historical text at commit 007ba1eb0:27;
/// BW9 quotes this statement without independently validating its evidence.
/// cudart's request for `0x20809064` (`[measured]` words 1, 2, 6).
fn req_9064() -> Vec<u8> {
    words_le(&[0, 2, 1, 0, 0, 0, 4], 520)
}

/// The measured AD104 / 595.91.07 host: the replies its own libcudart got.
struct Ad104At595;

impl HostControls for Ad104At595 {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        let refuse = || {
            Err(HostRefusal {
                status: Some(0x1f),
                detail: "not a measured request".into(),
            })
        };
        let reply = match cmd {
            0x2080_9009 if params == [0u8; 8].as_slice() => words_le(&[0, 0xd], 8),
            0x2080_9001 if params == [0u8; 8].as_slice() => words_le(&[0x03fc_007f, 0], 8),
            0x2080_9064 if params == req_9064().as_slice() => {
                words_le(&[0, 2, 1, 1, 1, 0x64, 4, 0x10, 1, 0x64], 520)
            }
            0x2080_8162 if params == [0u8].as_slice() => vec![0],
            _ => return refuse(),
        };
        params.copy_from_slice(&reply);
        Ok(())
    }
}

fn policy() -> InitTablePolicy {
    let mut host = ga106::host_facts();
    host.die = DieId::new(arch::AD100, 4);
    host.gss_replay = query_gss_replay(&mut Ad104At595);
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
        sequence: 21,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_guest_gets_exactly_what_the_hosts_own_cudart_got() {
    let mut p = policy();
    for (cmd, req, want) in [
        (0x2080_9009u32, vec![0u8; 8], words_le(&[0, 0xd], 8)),
        (0x2080_9001, vec![0u8; 8], words_le(&[0x03fc_007f, 0], 8)),
        (
            0x2080_9064,
            req_9064(),
            words_le(&[0, 2, 1, 1, 1, 0x64, 4, 0x10, 1, 0x64], 520),
        ),
        (0x2080_8162, vec![0u8], vec![0u8]),
    ] {
        let r = p.respond(&control(cmd, &req)).expect("answered");
        assert_eq!(r.rpc_result, 0, "{cmd:#x}");
        assert_eq!(
            &r.body[PARAMS_AT..PARAMS_AT + want.len()],
            &want[..],
            "{cmd:#x}"
        );
    }
}

/// ⊘ The host's `0x20808162` (`00`) wins over GA106's captured `01`, and with no host answer an
/// AD104 at a 580 guest branch has no table row: refused, never GA106's byte.
#[test]
fn the_captured_ga106_byte_is_never_served_to_the_ad104() {
    let r = policy()
        .respond(&control(0x2080_8162, &[0]))
        .expect("answered");
    assert_eq!(r.body[PARAMS_AT], 0, "the host's 00, not GA106's 01");
    let mut host = ga106::host_facts();
    host.die = DieId::new(arch::AD100, 4);
    let mut bare = InitTablePolicy::new(
        ga106::board(),
        std::sync::Arc::new(host),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
    );
    let r = bare.respond(&control(0x2080_8162, &[0]));
    assert!(
        r.is_none_or(|r| r.rpc_result != 0),
        "no host answer and no AD104/580 row: refused"
    );
}
