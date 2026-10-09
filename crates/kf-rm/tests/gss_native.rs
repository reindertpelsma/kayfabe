//! ★ EXPERIMENT `KF3_GSS_NATIVE` (`docs/design/V3_GSS_NATIVE.md`, STATUS: LIVE experiment, default
//! off, 2026-10-09), through the WHOLE served chain (census → sticky guard → links → object seat →
//! ledger), against a FAKE host and a FAKE act thread named like the plane's.
//!
//! What this file pins:
//! - a non-privileged `0x2080xxxx` GSS-legacy control on the guest's subdevice is forwarded; the
//!   host's status and reply bytes are what the guest's reply carries (and are never invented);
//! - the host is called from the act thread, never from the drainer (the thread that calls
//!   `respond`): the host here BLOCKS until the test lets it go, and `respond` returns regardless;
//! - a `0xC000` command, a display control, an oversized or short params window, a serialized
//!   control, a control on a non-subdevice object and the (cap+1)th forward are NOT forwarded and
//!   are answered exactly as with the experiment off;
//! - the experiment off changes nothing: the chain without the seat answers every control of the
//!   table with the same bytes as the chain with the seat, but for the forwardable ones.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, Deferred, RpcCommand, RpcFunction};
use kf_rm::gssnative::{
    ACT_THREAD, BOOT_CAP, GssAnswer, GssHost, GssRequest, GssSeat, MAX_PARAMS, Stats, execute,
};

const PARAMS_AT: usize = 40;
const NV_OK: u32 = 0;
const NV_ERR_INVALID_STATE: u32 = 0x40;
const CLIENT: u32 = 0xc1d0_0002;
const DEVICE: u32 = 0xff01_0000;
const SUBDEVICE: u32 = 0xff03_0000;
/// `RMAPI_RPC_FLAGS_SERIALIZED`.
const SERIALIZED: u32 = 1 << 1;

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

fn control_with(object: u32, cmd: u32, flags: u32, declared: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&CLIENT.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&declared.to_le_bytes());
    payload[20..24].copy_from_slice(&flags.to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    rpc(RpcFunction::RmControl, 0x4c, payload)
}

fn control(object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    control_with(object, cmd, 0, params.len() as u32, params)
}

fn alloc(parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<u8> = [CLIENT, parent, handle, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend_from_slice(params);
    rpc(RpcFunction::RmAlloc, 0x67, payload)
}

/// What the fake host does with a call.
#[derive(Clone)]
struct Script {
    status: u32,
    reply: Vec<u8>,
    err: Option<String>,
}

#[derive(Default)]
struct HostLog {
    /// `(thread name, cmd, params the host was handed)`.
    calls: Vec<(Option<String>, u32, Vec<u8>)>,
}

/// The fake host: records who called, optionally BLOCKS until released, answers per script.
struct FakeHost {
    log: Arc<Mutex<HostLog>>,
    script: Arc<Mutex<Script>>,
    gate: Mutex<Option<mpsc::Receiver<()>>>,
}

impl GssHost for FakeHost {
    fn control(&self, cmd: u32, params: &mut [u8]) -> Result<u32, String> {
        self.log.lock().unwrap().calls.push((
            std::thread::current().name().map(str::to_owned),
            cmd,
            params.to_vec(),
        ));
        if let Some(rx) = self.gate.lock().unwrap().as_ref() {
            rx.recv_timeout(Duration::from_secs(10))
                .expect("the test released the host");
        }
        let s = self.script.lock().unwrap().clone();
        if let Some(e) = s.err {
            return Err(e);
        }
        if s.status == 0 {
            let n = params.len().min(s.reply.len());
            params[..n].copy_from_slice(&s.reply[..n]);
        }
        Ok(s.status)
    }
}

struct Rig {
    chain: Box<dyn CommandPolicy>,
    stats: Arc<Stats>,
    log: Arc<Mutex<HostLog>>,
    script: Arc<Mutex<Script>>,
    release: Option<mpsc::Sender<()>>,
    unserviced: kf_rm::unserviced::UnservicedLog,
}

/// The chain with the object seat; `seat` on or off. The act thread is named like the plane's.
fn rig(seat: bool, gated: bool) -> Rig {
    let stats = Arc::new(Stats::new());
    let log = Arc::new(Mutex::new(HostLog::default()));
    let script = Arc::new(Mutex::new(Script {
        status: 0,
        reply: Vec::new(),
        err: None,
    }));
    let (rel_tx, rel_rx) = mpsc::channel();
    let host = Arc::new(FakeHost {
        log: log.clone(),
        script: script.clone(),
        gate: Mutex::new(gated.then_some(rel_rx)),
    });
    // The fake act thread: statement order, named like the plane's.
    let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
    std::thread::Builder::new()
        .name(ACT_THREAD.into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                job();
            }
        })
        .unwrap();
    let tx = Mutex::new(tx);
    let st = stats.clone();
    let sink: kf_rm::gssnative::GssSink = Arc::new(move |req: GssRequest| {
        let d = Deferred::new();
        let (d2, st2, host2) = (d.clone(), st.clone(), host.clone());
        let sent = tx
            .lock()
            .unwrap()
            .send(Box::new(move || execute(&st2, &*host2, req, &d2)));
        if sent.is_ok() {
            GssAnswer::Deferred(d)
        } else {
            GssAnswer::Refused {
                why: "no act thread".into(),
            }
        }
    });
    let unserviced = kf_rm::unserviced::UnservicedLog::new();
    let mut objects = kf_rm::rmrpc::ObjectPolicy::over(
        &driver(),
        kf_abi::GuestOs::Linux,
        Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
        Default::default(),
    );
    if seat {
        objects = objects.with_gss_native(GssSeat {
            sink,
            stats: stats.clone(),
        });
    }
    let links = kf_rm::ObjectLinks {
        objects: Some(objects),
        memory: None,
        channels: None,
        display: None,
        console: None,
    };
    let mut chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs {
            unserviced: unserviced.clone(),
            ..Default::default()
        },
        kf_rm::census::ControlCensusLog::new(),
        links,
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
        stats,
        log,
        script,
        release: gated.then_some(rel_tx),
        unserviced,
    }
}

impl Rig {
    /// The chain's reply and the cell it deferred (the FSM's two questions, in its order).
    fn ask(&mut self, c: &RpcCommand) -> (Option<kf_gsp::Reply>, Option<Deferred>) {
        let r = self.chain.respond(c);
        let d = self.chain.defers(c);
        (r, d)
    }

    fn wait(d: &Deferred) -> u32 {
        let t0 = Instant::now();
        loop {
            if let Some(o) = d.outcome() {
                return o;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(10),
                "the act never resolved"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn host_calls(&self) -> usize {
        self.log.lock().unwrap().calls.len()
    }
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

#[test]
fn a_gss_legacy_subdevice_control_is_forwarded_and_the_hosts_answer_reaches_the_guest() {
    let mut r = rig(true, true);
    let host_reply = pattern(2024, 0x5a);
    r.script.lock().unwrap().reply = host_reply.clone();
    let guest_params = pattern(2024, 0x11);
    let c = control(SUBDEVICE, 0x2080_a0d1, &guest_params);
    let ledger_before = r.unserviced.total();

    // The drainer's half: classify, copy, queue. The host is BLOCKED (gated) and respond returned
    // anyway — the drainer did not wait on, and did not make, the host call.
    let t0 = Instant::now();
    let (reply, cell) = r.ask(&c);
    assert!(t0.elapsed() < Duration::from_secs(5));
    let reply = reply.expect("held reply");
    let cell = cell.expect("the reply is held on a cell");
    assert_eq!(reply.rpc_result, NV_OK);
    assert_eq!(
        cell.outcome(),
        None,
        "the host has not answered: the reply waits"
    );
    assert_eq!(cell.reply_patch(), None);

    // Let the host go: it answers on the ACT thread.
    r.release.as_ref().unwrap().send(()).unwrap();
    assert_eq!(Rig::wait(&cell), NV_OK);
    let patch = cell.reply_patch().expect("the host's bytes");
    assert_eq!(patch.at, PARAMS_AT);
    assert_eq!(
        patch.bytes, host_reply,
        "exactly the host's bytes, paramsSize of them"
    );

    let calls = r.log.lock().unwrap().calls.clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0.as_deref(),
        Some(ACT_THREAD),
        "the host call is the act thread's"
    );
    assert_ne!(
        calls[0].0.as_deref(),
        std::thread::current().name(),
        "never the drainer's thread"
    );
    assert_eq!(calls[0].1, 0x2080_a0d1);
    assert_eq!(
        calls[0].2, guest_params,
        "the guest's params, opaque and whole"
    );
    assert_eq!(r.stats.off_act.load(Ordering::Relaxed), 0);
    assert_eq!(
        r.stats.status(),
        "gss[fwd=1 ok=1 host_err=0 refused=0 top=0x2080a0d1:1]"
    );
    // Not the ledger's: the ledger saw nothing of it.
    assert_eq!(
        r.unserviced.total(),
        ledger_before,
        "the ledger saw nothing of it"
    );
}

#[test]
fn the_hosts_refusal_is_the_guests_status_and_is_never_turned_into_ok() {
    for host_status in [0x56u32, 0x1d, 0x1f, 0x3] {
        let mut r = rig(true, false);
        r.script.lock().unwrap().status = host_status;
        // The host would write a reply too, but only a successful call's bytes are carried.
        r.script.lock().unwrap().reply = pattern(528, 9);
        let c = control(SUBDEVICE, 0x2080_852e, &pattern(528, 1));
        let (_, cell) = r.ask(&c);
        let cell = cell.unwrap();
        assert_eq!(Rig::wait(&cell), host_status);
        assert_eq!(cell.reply_patch(), None, "no bytes with a refusal");
        assert_eq!(
            r.stats.status(),
            "gss[fwd=1 ok=0 host_err=1 refused=0 top=0x2080852e:1]"
        );
    }
}

#[test]
fn a_host_that_cannot_be_asked_is_invalid_state_not_ok() {
    let mut r = rig(true, false);
    r.script.lock().unwrap().err = Some("ioctl EBADF".into());
    let (_, cell) = r.ask(&control(SUBDEVICE, 0x2080_9004, &pattern(1544, 3)));
    assert_eq!(Rig::wait(&cell.unwrap()), NV_ERR_INVALID_STATE);
    assert_eq!(r.stats.host_err.load(Ordering::Relaxed), 1);
}

/// What the chain does with a control with the experiment off: nothing answers it (`None`: the FSM
/// then posts `NV_ERR_NOT_SUPPORTED` with an empty body, `kf_gsp::boot`), no cell, the ledger
/// notes it once.
fn off_reply(c: &RpcCommand) -> (Option<kf_gsp::Reply>, bool, u64) {
    let mut off = rig(false, false);
    let before = off.unserviced.total();
    let (r, d) = off.ask(c);
    (r, d.is_some(), off.unserviced.total() - before)
}

#[test]
fn what_is_not_forwardable_is_answered_exactly_as_with_the_experiment_off() {
    let p = pattern(64, 2);
    let table: Vec<(&str, RpcCommand)> = vec![
        (
            "privileged 0xC000 pattern",
            control(SUBDEVICE, 0x2080_c0a8, &p),
        ),
        ("privileged 0xe123", control(SUBDEVICE, 0x2080_e123, &p)),
        (
            "display 0x730285 on the subdevice handle",
            control(SUBDEVICE, 0x0073_0285, &p),
        ),
        ("display 0x730288", control(SUBDEVICE, 0x0073_0288, &p)),
        ("display 0x7302a5", control(SUBDEVICE, 0x0073_02a5, &p)),
        ("display 0x50700000", control(SUBDEVICE, 0x5070_0000, &p)),
        ("device-class GSS bit", control(SUBDEVICE, 0x0080_8123, &p)),
        ("no bit 15", control(SUBDEVICE, 0x2080_0123, &p)),
        ("a non-subdevice object", control(DEVICE, 0x2080_9004, &p)),
        ("an unknown object", control(0xdead_0001, 0x2080_9004, &p)),
        (
            "serialized",
            control_with(SUBDEVICE, 0x2080_9004, SERIALIZED, p.len() as u32, &p),
        ),
        (
            "declared bigger than present",
            control_with(SUBDEVICE, 0x2080_9004, 0, 4096, &p),
        ),
        ("oversized", {
            let big = vec![0u8; MAX_PARAMS + 1];
            control(SUBDEVICE, 0x2080_a0d1, &big)
        }),
        ("unparseable (short)", {
            let mut c = control(SUBDEVICE, 0x2080_9004, &p);
            c.payload.truncate(12);
            c
        }),
    ];
    for (name, c) in table {
        let mut on = rig(true, false);
        let before = on.unserviced.total();
        let (reply, cell) = on.ask(&c);
        let (off_answer, off_cell, off_noted) = off_reply(&c);
        assert_eq!(reply, off_answer, "{name}: the reply");
        assert_eq!(cell.is_some(), off_cell, "{name}: cell");
        assert_eq!(
            on.unserviced.total() - before,
            off_noted,
            "{name}: the ledger"
        );
        assert_eq!(on.host_calls(), 0, "{name}: the host was not called");
        assert_eq!(on.stats.fwd.load(Ordering::Relaxed), 0, "{name}");
        assert_eq!(
            off_answer, None,
            "{name}: unanswered, so the FSM refuses it 0x56 as today"
        );
        assert_eq!(off_noted, 1, "{name}: the ledger noted it");
    }
}

#[test]
fn refusals_are_counted_only_for_gss_legacy_commands_and_never_for_display() {
    let mut r = rig(true, false);
    let p = pattern(16, 1);
    for (cmd, counted) in [
        (0x0073_0285u32, false),
        (0x0073_0288, false),
        (0x0073_02a5, false),
        (0x5070_0000, false),
        (0x2080_0123, false),
        (0x2080_c0a8, true),
        (0x0080_8123, true),
    ] {
        let before = r.stats.refused.load(Ordering::Relaxed);
        let _ = r.ask(&control(SUBDEVICE, cmd, &p));
        let after = r.stats.refused.load(Ordering::Relaxed);
        assert_eq!(after - before, u64::from(counted), "{cmd:#010x}");
    }
}

#[test]
fn the_size_cap_is_inclusive() {
    let mut r = rig(true, false);
    r.script.lock().unwrap().reply = vec![0xee; 8];
    let c = control(SUBDEVICE, 0x2080_a0d1, &vec![7u8; MAX_PARAMS]);
    let (_, cell) = r.ask(&c);
    let cell = cell.expect("exactly the cap is forwarded");
    assert_eq!(Rig::wait(&cell), NV_OK);
    assert_eq!(cell.reply_patch().unwrap().bytes.len(), MAX_PARAMS);
    assert_eq!(r.stats.fwd.load(Ordering::Relaxed), 1);
}

#[test]
fn a_zero_length_control_is_forwarded_with_no_params() {
    let mut r = rig(true, false);
    let (_, cell) = r.ask(&control(SUBDEVICE, 0x2080_a084, &[]));
    assert_eq!(Rig::wait(&cell.unwrap()), NV_OK);
    let calls = r.log.lock().unwrap().calls.clone();
    assert_eq!(calls[0].2.len(), 0);
}

#[test]
fn the_boot_wide_cap_refuses_afterwards_as_today() {
    let mut r = rig(true, false);
    let c = control(SUBDEVICE, 0x2080_a0d1, &pattern(32, 4));
    // One forward below the cap goes through...
    r.stats.fwd.store(BOOT_CAP - 1, Ordering::Relaxed);
    let (_, cell) = r.ask(&c);
    assert_eq!(Rig::wait(&cell.expect("the last one under the cap")), NV_OK);
    assert_eq!(r.stats.fwd.load(Ordering::Relaxed), BOOT_CAP);
    // ...and the next is refused 0x56 by the ledger, not forwarded, and counted.
    let calls = r.host_calls();
    let before = r.unserviced.total();
    let (reply, cell) = r.ask(&c);
    assert!(cell.is_none());
    assert_eq!(reply, None, "unanswered: the FSM posts 0x56");
    assert_eq!(r.unserviced.total() - before, 1, "the ledger noted it");
    assert_eq!(r.host_calls(), calls);
    assert_eq!(r.stats.refused.load(Ordering::Relaxed), 1);
    assert_eq!(r.stats.fwd.load(Ordering::Relaxed), BOOT_CAP);
}

#[test]
fn a_repeat_while_the_first_is_pending_waits_behind_it() {
    // The act thread is one queue: the second host call cannot start before the first returned.
    let mut r = rig(true, true);
    let c = control(SUBDEVICE, 0x2080_a0d1, &pattern(32, 4));
    let (_, first) = r.ask(&c);
    let (_, second) = r.ask(&c);
    let (first, second) = (first.unwrap(), second.unwrap());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(first.outcome(), None);
    assert_eq!(second.outcome(), None);
    assert_eq!(r.host_calls(), 1, "only the first reached the host");
    r.release.as_ref().unwrap().send(()).unwrap();
    assert_eq!(Rig::wait(&first), NV_OK);
    r.release.as_ref().unwrap().send(()).unwrap();
    assert_eq!(Rig::wait(&second), NV_OK);
    assert_eq!(r.host_calls(), 2);
}

#[test]
fn a_host_call_off_the_act_thread_is_counted() {
    // The instrument works: `execute` called from a thread that is not the act thread says so.
    let stats = Stats::new();
    let host = FakeHost {
        log: Arc::default(),
        script: Arc::new(Mutex::new(Script {
            status: 0,
            reply: vec![],
            err: None,
        })),
        gate: Mutex::new(None),
    };
    let d = Deferred::new();
    execute(
        &stats,
        &host,
        GssRequest {
            client: 1,
            object: 2,
            cmd: 0x2080_8000,
            params_at: PARAMS_AT,
            params: vec![0; 4],
        },
        &d,
    );
    assert_eq!(stats.off_act.load(Ordering::Relaxed), 1);
    assert!(stats.status().contains("off_act=1"));
}

#[test]
fn a_dead_act_thread_refuses_as_today() {
    // The sink says the act thread is not running: the link falls through to the ledger.
    let stats = Arc::new(Stats::new());
    let sink: kf_rm::gssnative::GssSink = Arc::new(|_| GssAnswer::Refused { why: "down".into() });
    let objects = kf_rm::rmrpc::ObjectPolicy::over(
        &driver(),
        kf_abi::GuestOs::Linux,
        Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
        Default::default(),
    )
    .with_gss_native(GssSeat {
        sink,
        stats: stats.clone(),
    });
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
    assert_eq!(chain.respond(&root).unwrap().rpc_result, NV_OK);
    let dev = rpcwire::device_params(0, 0, 0);
    assert_eq!(
        chain
            .respond(&alloc(CLIENT, DEVICE, 0x80, &dev))
            .unwrap()
            .rpc_result,
        NV_OK
    );
    assert_eq!(
        chain
            .respond(&alloc(DEVICE, SUBDEVICE, 0x2080, &[0; 4]))
            .unwrap()
            .rpc_result,
        NV_OK
    );
    let c = control(SUBDEVICE, 0x2080_9004, &pattern(8, 1));
    assert_eq!(chain.respond(&c), None, "unanswered: the FSM posts 0x56");
    assert!(chain.defers(&c).is_none());
    assert_eq!(stats.fwd.load(Ordering::Relaxed), 0);
    assert_eq!(stats.refused.load(Ordering::Relaxed), 1);
}
