//! libcudart's clock pair `0x2080a084` / `0x2080a026` — ★★ v3-refusals: SERVED from the host's
//! realize-time answer (`kf_abi::gssreplay`), because refusing them sent cudart to its `0x2080a001`
//! fallback and the guest's `cudaDevAttrClockRate` read **420 MHz** for a **1695 MHz** die
//! (`[measured vrf, GA102 / 580.159.04, deviceQuery host vs kf3 guest]`,
//! `docs/design/V3_REFUSAL_AUDIT.md`).
//!
//! What this file pins, through `InitTablePolicy` (the link that answers GSS-legacy controls):
//! - a guest request shaped as bare-metal cudart builds it — the named input words, UNINITIALISED
//!   stack bytes everywhere else — is answered `NV_OK` with the host's clocks and whole-field
//!   writes, and the guest's bytes the host never writes survive;
//! - with no host answer recorded (the host refused the probe), the control is not answered by
//!   this path — a refusal, never a constant;
//! - the precursor `0x2080a084` is `NV_OK` with nothing written, exactly as the host answered.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::gssreplay::{Answer, GSS_CUDART_A084, GSS_CUDART_CLOCKS, PROBE_BACKGROUND, ROWS};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::inittables::InitTablePolicy;

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the control header.
const CONTROL_STATUS_OFF: usize = 12;

/// The host as measured on bare metal: a `u8` at 4 and whole `u32`s at 8, 0x0c, 0x1c and 0x2c.
fn host_reply(mut p: Vec<u8>) -> Vec<u8> {
    p[4] = 2;
    p[8..12].copy_from_slice(&4u32.to_le_bytes());
    p[12..16].copy_from_slice(&1u32.to_le_bytes());
    p[0x1c..0x20].copy_from_slice(&1_695_000u32.to_le_bytes());
    p[0x2c..0x30].copy_from_slice(&9_751_000u32.to_le_bytes());
    p
}

fn answers() -> Vec<Answer> {
    let row = |cmd| *ROWS.iter().find(|r| r.cmd == cmd).expect("row");
    let clocks = row(GSS_CUDART_CLOCKS);
    let a084 = row(GSS_CUDART_A084);
    vec![
        Answer::from_probes(clocks, &host_reply(clocks.request()), Some(&host_reply(clocks.request_over(PROBE_BACKGROUND)))).0,
        Answer::from_probes(a084, &a084.request(), Some(&a084.request_over(PROBE_BACKGROUND))).0,
    ]
}

fn policy(gss: Vec<Answer>) -> InitTablePolicy {
    let mut host = ga106::host_facts();
    host.gss_replay = gss;
    InitTablePolicy::new(ga106::board(), std::sync::Arc::new(host), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0245u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand { function: RpcFunction::RmControl, code: 0x4c, sequence: 103, payload, elements: 1, delivered: Vec::new() }
}

/// A request as a measured bare-metal cudart sample carries it: the constant words, garbage
/// (`0x5a`) in the bytes the caller never initialised.
fn cudart_request() -> Vec<u8> {
    let row = *ROWS.iter().find(|r| r.cmd == GSS_CUDART_CLOCKS).expect("row");
    let mut g = row.request_over(0x5a);
    for &(o, v) in row.inputs {
        g[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    g
}

#[test]
fn the_clock_query_is_answered_with_the_hosts_clocks_and_whole_fields() {
    let cmd = control(GSS_CUDART_CLOCKS, &cudart_request());
    let r = policy(answers()).respond(&cmd).expect("answered");
    assert_eq!(r.rpc_result, 0, "envelope NV_OK");
    let b = &r.body;
    assert_eq!(&b[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4], &0u32.to_le_bytes(), "control header NV_OK");
    let p = &b[PARAMS_AT..];
    assert_eq!(&p[0x1c..0x20], &1_695_000u32.to_le_bytes(), "GPC max clock, kHz — the 1695 MHz deviceQuery shows");
    assert_eq!(&p[0x2c..0x30], &9_751_000u32.to_le_bytes(), "MCLK, kHz");
    assert_eq!(&p[12..16], &1u32.to_le_bytes(), "a whole field: no stack garbage above its low byte");
    assert_eq!(p[4], 2);
    assert_eq!(&p[5..8], &[0x5a; 3], "bytes the host never writes stay the guest's, as on bare metal");
    assert_eq!(p[0x100], 0x5a);
}

#[test]
fn with_no_host_answer_the_path_answers_nothing() {
    let cmd = control(GSS_CUDART_CLOCKS, &cudart_request());
    let r = policy(Vec::new()).respond(&cmd);
    assert!(r.is_none_or(|r| r.rpc_result != 0), "never a constant where the host gave nothing");
}

#[test]
fn the_precursor_is_ok_with_nothing_written() {
    let cmd = control(GSS_CUDART_A084, &[0u8; 4]);
    let r = policy(answers()).respond(&cmd).expect("answered");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(&r.body[PARAMS_AT..PARAMS_AT + 4], &[0u8; 4]);
}
