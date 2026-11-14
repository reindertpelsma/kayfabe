//! Security review 1 (2026-11-13), F-06 / X-01: the `KF3_GSS_NATIVE` experiment carried a guest's
//! GSS-legacy control parameters UNCHANGED to the host RM (a guest-authored host action, owner
//! principle A.2). The owner ruled "fix it"; nothing in the gates, the CI or the docs depended on
//! it, so the forwarding path is REMOVED (`docs/design/V3_GSS_NATIVE.md`, SUPERSEDED).
//!
//! What this file pins (each test is a hostile caller that the old code obeyed):
//! - with a seat installed, every control the experiment used to forward is answered exactly as
//!   with no seat at all (unanswered: the FSM posts `NV_ERR_NOT_SUPPORTED`), no cell is held, the
//!   sink is never called and the host is never called;
//! - `execute`, called directly, never reaches the host and resolves the cell with a refusal;
//! - `KF3_GSS_NATIVE=1` in the environment does not turn anything on.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, Deferred, RpcCommand, RpcFunction};
use kf_rm::gssnative::{GssAnswer, GssHost, GssRequest, GssSeat, MAX_PARAMS, Stats, execute};

const PARAMS_AT: usize = 40;
const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const CLIENT: u32 = 0xc1d0_0002;
const DEVICE: u32 = 0xff01_0000;
const SUBDEVICE: u32 = 0xff03_0000;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

fn rpc(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn control(object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&CLIENT.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    rpc(RpcFunction::RmControl, 0x4c, payload)
}

fn alloc(parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<u8> = [CLIENT, parent, handle, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend_from_slice(params);
    rpc(RpcFunction::RmAlloc, 0x67, payload)
}

/// A host that records every call and answers `NV_OK` with a recognisable body.
#[derive(Default)]
struct CountingHost {
    calls: Mutex<Vec<(u32, Vec<u8>)>>,
}

impl GssHost for CountingHost {
    fn control(&self, cmd: u32, params: &mut [u8]) -> Result<u32, String> {
        self.calls.lock().unwrap().push((cmd, params.to_vec()));
        params.fill(0xab);
        Ok(NV_OK)
    }
}

struct Rig {
    chain: Box<dyn CommandPolicy>,
    sink_calls: Arc<AtomicUsize>,
    host: Arc<CountingHost>,
}

/// The served chain with the object seat; `seat` installs the (former) experiment's seat with a
/// sink that would run the host call inline if it were ever handed a request.
fn rig(seat: bool) -> Rig {
    let sink_calls = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(CountingHost::default());
    let stats = Arc::new(Stats::new());
    let mut objects = kf_rm::rmrpc::ObjectPolicy::over(
        &driver(),
        kf_abi::GuestOs::Linux,
        Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
        Default::default(),
    );
    if seat {
        let (calls, h, st) = (sink_calls.clone(), host.clone(), stats.clone());
        let sink: kf_rm::gssnative::GssSink = Arc::new(move |req: GssRequest| {
            calls.fetch_add(1, Ordering::SeqCst);
            let d = Deferred::new();
            execute(&st, &*h, req, &d);
            GssAnswer::Deferred(d)
        });
        objects = objects.with_gss_native(GssSeat { sink, stats });
    }
    let mut chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs::default(),
        kf_rm::census::ControlCensusLog::new(),
        kf_rm::ObjectLinks {
            objects: Some(objects),
            memory: None,
            channels: None,
            display: None,
            console: None,
        },
    );
    let root = rpc(
        RpcFunction::RmAlloc,
        0x67,
        rpcwire::client_root_alloc_body(0x41, CLIENT, 1234),
    );
    assert_eq!(chain.respond(&root).expect("client").rpc_result, NV_OK);
    let dev = rpcwire::device_params(0, 0, 0);
    assert_eq!(
        chain
            .respond(&alloc(CLIENT, DEVICE, 0x80, &dev))
            .expect("device")
            .rpc_result,
        NV_OK
    );
    assert_eq!(
        chain
            .respond(&alloc(DEVICE, SUBDEVICE, 0x2080, &[0; 4]))
            .expect("subdevice")
            .rpc_result,
        NV_OK
    );
    Rig {
        chain,
        sink_calls,
        host,
    }
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

/// Every id the experiment forwarded (`docs/design/V3_REFUSAL_AUDIT.md`), plus the edges of its old
/// rule (zero-length, exactly the 64 KiB cap).
fn formerly_forwarded() -> Vec<(String, RpcCommand)> {
    let mut v: Vec<(String, RpcCommand)> = [
        (0x2080_9004u32, 1544usize),
        (0x2080_b201, 2188),
        (0x2080_852e, 528),
        (0x2080_852f, 776),
        (0x2080_a0d1, 2024),
        (0x2080_a0a8, 64),
        (0x2080_9037, 64),
        (0x2080_8539, 64),
        (0x2080_9038, 64),
        (0x2080_a080, 64),
        (0x2080_a097, 64),
        (0x2080_8163, 4),
        (0x2080_a084, 0),
        (0x2080_a0d1, MAX_PARAMS),
    ]
    .into_iter()
    .map(|(cmd, n)| {
        (
            format!("{cmd:#010x}/{n}"),
            control(SUBDEVICE, cmd, &pattern(n, 3)),
        )
    })
    .collect();
    v.push((
        "a GSS-legacy id on a non-subdevice object".into(),
        control(DEVICE, 0x2080_9004, &pattern(16, 1)),
    ));
    v
}

#[test]
fn a_seat_installed_forwards_nothing_and_answers_as_with_no_seat() {
    for (name, c) in formerly_forwarded() {
        let mut on = rig(true);
        let mut off = rig(false);
        let on_reply = on.chain.respond(&c);
        let on_cell = on.chain.defers(&c);
        let off_reply = off.chain.respond(&c);
        let off_cell = off.chain.defers(&c);
        assert!(
            on_cell.is_none(),
            "{name}: no reply may be held on a host act"
        );
        assert!(off_cell.is_none(), "{name}");
        assert_eq!(on_reply, off_reply, "{name}: exactly the no-seat answer");
        assert_eq!(
            on_reply, None,
            "{name}: unanswered, so the FSM refuses it (0x56)"
        );
        assert_eq!(
            on.sink_calls.load(Ordering::SeqCst),
            0,
            "{name}: no request"
        );
        assert!(
            on.host.calls.lock().unwrap().is_empty(),
            "{name}: the guest's bytes never reached the host"
        );
    }
}

#[test]
fn execute_never_reaches_the_host_and_refuses() {
    let host = CountingHost::default();
    let stats = Stats::new();
    let d = Deferred::new();
    execute(
        &stats,
        &host,
        GssRequest {
            client: CLIENT,
            object: SUBDEVICE,
            cmd: 0x2080_a0d1,
            params_at: PARAMS_AT,
            params: pattern(2024, 9),
        },
        &d,
    );
    assert!(
        host.calls.lock().unwrap().is_empty(),
        "a hostile request handed straight to `execute` must not reach the host"
    );
    assert_eq!(
        d.outcome(),
        Some(NV_ERR_NOT_SUPPORTED),
        "a refusal, never NV_OK"
    );
}

/// `KF3_GSS_NATIVE=1` cannot turn the path back on. Run in a child process (the workspace forbids
/// the `unsafe` `set_var`).
#[test]
fn the_environment_flag_turns_nothing_on() {
    if std::env::var_os("KF_GSS_REMOVED_CHILD").is_some() {
        assert!(
            !kf_rm::gssnative::enabled(),
            "the removed experiment stays off"
        );
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "the_environment_flag_turns_nothing_on",
            "--test-threads=1",
        ])
        .env("KF_GSS_REMOVED_CHILD", "1")
        .env(kf_rm::gssnative::FLAG, "1")
        .output()
        .expect("the test binary runs");
    assert!(
        out.status.success(),
        "child: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
