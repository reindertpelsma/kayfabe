//! `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` (`0x20801115`) and the two `SET_INTERLEAVE_LEVEL`
//! controls — the queue-priority path of `VK_KHR_global_priority` (STATUS: LIVE, 2026-11-18; a replay
//! test in the sense of `docs/design/V3_REPLAY_TESTS.md`, design note `docs/design/V3_QUEUE_PRIORITY.md`).
//!
//! ## The logged failure
//!
//! `[measured]` (`tests/fixtures/queue_priority_refusal.txt`): in 4 of 4 guest boots whose
//! `vkCreateDevice` failed with `-3` for a non-default queue priority, the kf3 log holds
//! `GSP REFUSED fn76/0x20801115=0x56`; the neighbour boot that only ran info cases has no such line.
//!
//! ## The invariant (what a log line can falsify)
//!
//! A `SET_SCHED_POLICY` the guest sends through `GSP_RM_CONTROL` is answered `NV_OK` from kayfabe's own
//! per-VM state, and NOTHING reaches the host: no channel statement, no memory statement, no ledger
//! entry. A replay shows a decision, given a request. It cannot show that the Vulkan driver then
//! succeeds, nor which params the closed-source driver sends (INFERRED; the three policy values and the
//! restore flag are all covered).
//!
//! The literals below are written from the ogkm headers, not from `kf_rm`'s constants: an oracle may
//! not take the value under test from the artifact under test.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::barpde::MemStatement;
use kf_rm::chanlink::{ChanAnswer, ChanStatement};
use std::sync::{Arc, Mutex};

const FIXTURE: &str = include_str!("fixtures/queue_priority_refusal.txt");

/// `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` (`ctrl2080fifo.h:776`).
const SET_SCHED_POLICY: u32 = 0x2080_1115;
/// `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`ctrla06c.h:268`).
const TSG_SET_INTERLEAVE: u32 = 0xa06c_0107;
/// `NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`ctrla06fgpfifo.h:144`).
const CHAN_SET_INTERLEAVE: u32 = 0xa06f_0109;
/// Neighbours that are NOT served: they change GPU preemption or the runlist itself.
const TSG_GET_INTERLEAVE: u32 = 0xa06c_0108;
const MAKE_REALTIME: u32 = 0xa06c_0110;
const RESTART_RUNLIST: u32 = 0xa06f_0111;
const UPDATE_CHANNEL_INFO: u32 = 0x2080_1116;

const NV_OK: u32 = 0;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
const NV_ERR_INVALID_PARAM_STRUCT: u32 = 0x3a;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const PARAMS_AT: usize = 40;
const STATUS_AT: usize = 12;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// A `GSP_RM_CONTROL` with `declared` as the params size the guest wrote (a lie can be built).
fn control_declared(
    object: u32,
    cmd: u32,
    params: &[u8],
    declared: u32,
    rpc_flags: u32,
) -> RpcCommand {
    let abi = driver();
    let w = abi.rm_control_wire();
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    // a stale status the reply must overwrite
    payload[STATUS_AT..STATUS_AT + 4].copy_from_slice(&0x1111_1111u32.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4].copy_from_slice(&declared.to_le_bytes());
    payload[w.rpc_flags_off..w.rpc_flags_off + 4].copy_from_slice(&rpc_flags.to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 9,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn control(object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    control_declared(object, cmd, params, params.len() as u32, 0)
}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

const SUBDEVICE: u32 = 0xff03_0000;
const TSG: u32 = 0xcaf0_0001;
const CHANNEL: u32 = 0xcaf0_0002;

#[derive(Default)]
struct Seen {
    chan: Mutex<Vec<String>>,
    mem: Mutex<Vec<String>>,
}

/// The whole served chain with BOTH host-facing planes seated and recording: whatever a control
/// causes on the host is a statement in `Seen`.
fn chain(log: &kf_rm::unserviced::UnservicedLog) -> (Box<dyn CommandPolicy>, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let (c, m) = (seen.clone(), seen.clone());
    let links = kf_rm::ObjectLinks {
        objects: None,
        memory: Some(kf_rm::MemoryLink {
            sink: Arc::new(move |st: MemStatement| m.mem.lock().unwrap().push(format!("{st:?}"))),
            guest_os: kf_abi::GuestOs::Linux,
        }),
        channels: Some(Arc::new(move |st: ChanStatement| {
            c.chan.lock().unwrap().push(format!("{st:?}"));
            ChanAnswer::NotOurs
        })),
        display: None,
        console: None,
        windows: Default::default(),
        utilization: None,
    };
    let chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs {
            unserviced: log.clone(),
            ..Default::default()
        },
        kf_rm::census::ControlCensusLog::new(),
        links,
    );
    (chain, seen)
}

fn assert_no_host_action(seen: &Seen) {
    assert_eq!(
        seen.chan.lock().unwrap().as_slice(),
        &[] as &[String],
        "no channel statement"
    );
    assert_eq!(
        seen.mem.lock().unwrap().as_slice(),
        &[] as &[String],
        "no memory statement"
    );
}

/// The envelope AND the control header's own status word.
fn statuses(r: &kf_gsp::Reply) -> (u32, u32) {
    let st = r
        .body
        .get(STATUS_AT..STATUS_AT + 4)
        .map_or(0xdead_beef, |b| u32::from_le_bytes(b.try_into().unwrap()));
    (r.rpc_result, st)
}

fn fixture_values(kind: &str) -> Vec<(String, u32, u32)> {
    FIXTURE
        .lines()
        .filter_map(|l| l.strip_prefix("synthetic: "))
        .filter(|l| !l.is_empty())
        .filter(|l| kind.is_empty() || l.starts_with(kind))
        .map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next().unwrap().to_string();
            let num = |p: &str, s: Option<&str>| {
                let s = s.unwrap().strip_prefix(p).unwrap();
                u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
            };
            (
                name,
                num("flags=", it.next()),
                num("schedPolicy=", it.next()),
            )
        })
        .collect()
}

/// The replay starts from the logged state: the fixture names this control and this status, and four
/// boots; a drifted fixture fails here, before it can make the acceptance test pass for the wrong reason.
#[test]
fn the_fixture_is_the_logged_refusal() {
    let logged: Vec<&str> = FIXTURE
        .lines()
        .filter(|l| l.starts_with("logged-line: "))
        .collect();
    assert_eq!(logged.len(), 4);
    for l in &logged {
        assert!(l.ends_with("GSP REFUSED fn76/0x20801115=0x56"), "{l}");
    }
    assert!(FIXTURE.contains("measured: control=0x20801115 status=0x56"));
    assert_eq!(fixture_values("").len(), 4);
}

/// ★ THE REPLAY. Each params shape the closed-source driver may send is answered `NV_OK` in the
/// envelope and in the control header, the params come back as sent (no `[OUT]` field), nothing reaches
/// the ledger and nothing reaches the host. RED before the fix: the ledger refused it `0x56`.
#[test]
fn the_logged_refused_control_is_served_ok_and_nothing_reaches_the_host() {
    for (name, flags, policy) in fixture_values("") {
        let log = kf_rm::unserviced::UnservicedLog::new();
        let (mut c, seen) = chain(&log);
        let params = words(&[flags, policy]);
        let r = c
            .respond(&control(SUBDEVICE, SET_SCHED_POLICY, &params))
            .expect("answered");
        assert_eq!(
            statuses(&r),
            (NV_OK, NV_OK),
            "{name}: served, not refused 0x56"
        );
        assert_eq!(
            &r.body[PARAMS_AT..],
            params.as_slice(),
            "{name}: params as sent"
        );
        assert_eq!(
            log.total(),
            0,
            "{name}: the control no longer reaches the ledger"
        );
        assert_no_host_action(&seen);
    }
}

/// The three-step sequence a Vulkan device lifetime makes: set, (second device) set again, restore.
/// Each step is OK; repeating it any number of times stays OK.
#[test]
fn a_device_lifetime_set_set_restore_repeated() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(&log);
    for _ in 0..200 {
        for p in [
            words(&[0, 1]),
            words(&[0, 1]),
            words(&[1, 0]),
            words(&[0, 2]),
            words(&[1, 0xffff_ffff]),
        ] {
            let r = c
                .respond(&control(SUBDEVICE, SET_SCHED_POLICY, &p))
                .expect("answered");
            assert_eq!(statuses(&r), (NV_OK, NV_OK), "{p:?}");
        }
    }
    assert_eq!(log.total(), 0);
    assert_no_host_action(&seen);
}

/// ⊘ Hostile params: a wrong size (declared or actual), a policy that is not one of the three, a
/// serialized envelope. Refused by name, never decoded past the size check, nothing reaches the host.
#[test]
fn hostile_set_sched_policy_is_refused_by_name_and_reaches_no_host() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(&log);
    let mut refusals = 0;
    for (params, declared) in [
        (vec![], 0u32),
        (vec![0u8; 4], 4),
        (vec![0u8; 7], 7),
        (vec![0u8; 9], 9),
        (vec![0u8; 4096], 4096),
        (vec![0u8; 8], 7),
        (vec![0u8; 8], 9),
        (vec![0u8; 8], u32::MAX),
        (vec![0u8; 8], 0),
    ] {
        let r = c
            .respond(&control_declared(
                SUBDEVICE,
                SET_SCHED_POLICY,
                &params,
                declared,
                0,
            ))
            .expect("answered");
        assert_eq!(
            r.rpc_result,
            NV_ERR_INVALID_PARAM_STRUCT,
            "len {} declared {declared}",
            params.len()
        );
        refusals += 1;
    }
    // restore not set and a policy that does not exist
    for policy in [3u32, 0x8000_0000, u32::MAX] {
        let r = c
            .respond(&control(SUBDEVICE, SET_SCHED_POLICY, &words(&[0, policy])))
            .expect("answered");
        assert_eq!(r.rpc_result, NV_ERR_INVALID_ARGUMENT, "policy {policy:#x}");
        refusals += 1;
    }
    // FINN-serialized params are not decoded
    let serialized = (0..32)
        .map(|b| 1u32 << b)
        .find(|f| kf_abi::rpc_params_are_serialized(*f))
        .expect("a serialized flag bit");
    let r = c
        .respond(&control_declared(
            SUBDEVICE,
            SET_SCHED_POLICY,
            &words(&[0, 1]),
            8,
            serialized,
        ))
        .expect("answered");
    assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED);
    refusals += 1;
    // a truncated envelope (header cut short) is not a control any link can read
    let mut short = control(SUBDEVICE, SET_SCHED_POLICY, &words(&[0, 1]));
    short.payload.truncate(10);
    assert!(c.respond(&short).is_none_or(|r| r.rpc_result != NV_OK));
    assert!(refusals > 10);
    assert_no_host_action(&seen);
}

/// The garbage-flags case: only bit 0 is a flag (`..._FLAGS_RESTORE 0:0`); the other 31 bits do not
/// change the answer, and `schedPolicy` is ignored when RESTORE is set (header text).
#[test]
fn only_the_restore_bit_of_flags_is_read() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(&log);
    for flags in [0u32, 2, 0x8000_0000, 0xffff_fffe] {
        let r = c
            .respond(&control(SUBDEVICE, SET_SCHED_POLICY, &words(&[flags, 1])))
            .expect("answered");
        assert_eq!(statuses(&r), (NV_OK, NV_OK), "set with flags {flags:#x}");
    }
    for flags in [1u32, 0xffff_ffff] {
        let r = c
            .respond(&control(
                SUBDEVICE,
                SET_SCHED_POLICY,
                &words(&[flags, 0xdead_beef]),
            ))
            .expect("answered");
        assert_eq!(
            statuses(&r),
            (NV_OK, NV_OK),
            "restore with flags {flags:#x}"
        );
    }
    assert_no_host_action(&seen);
}

/// The two interleave SETs ride the same path (INFERRED reached next by the Vulkan driver; the guest
/// answers the GETs itself, `kernel_channel_group_api.c:1431-1451`). Levels LOW/MEDIUM/HIGH are served;
/// anything else is `INVALID_ARGUMENT`; neither reaches the host.
#[test]
fn interleave_level_sets_are_served_and_validated() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(&log);
    for (object, cmd) in [(TSG, TSG_SET_INTERLEAVE), (CHANNEL, CHAN_SET_INTERLEAVE)] {
        for level in [0u32, 1, 2] {
            let r = c
                .respond(&control(object, cmd, &words(&[level])))
                .expect("answered");
            assert_eq!(statuses(&r), (NV_OK, NV_OK), "{cmd:#x} level {level}");
            assert_eq!(&r.body[PARAMS_AT..], words(&[level]).as_slice());
        }
        for level in [3u32, 0x8000_0000, u32::MAX] {
            let r = c
                .respond(&control(object, cmd, &words(&[level])))
                .expect("answered");
            assert_eq!(
                r.rpc_result, NV_ERR_INVALID_ARGUMENT,
                "{cmd:#x} level {level:#x}"
            );
        }
        for (params, declared) in [
            (vec![], 0u32),
            (vec![0u8; 8], 8),
            (vec![0u8; 4], 8),
            (vec![0u8; 4], u32::MAX),
        ] {
            let r = c
                .respond(&control_declared(object, cmd, &params, declared, 0))
                .expect("answered");
            assert_eq!(
                r.rpc_result, NV_ERR_INVALID_PARAM_STRUCT,
                "{cmd:#x} declared {declared}"
            );
        }
    }
    assert_eq!(log.total(), 0);
    assert_no_host_action(&seen);
}

/// ⊘ Everything next to these controls stays refused by name exactly as before: they change GPU
/// preemption or the runlist itself (`MAKE_REALTIME`, `RESTART_RUNLIST`), or the guest never sends them
/// (the interleave GET is answered by the guest's own RM).
#[test]
fn the_neighbours_stay_refused() {
    for cmd in [TSG_GET_INTERLEAVE, MAKE_REALTIME, RESTART_RUNLIST] {
        let log = kf_rm::unserviced::UnservicedLog::new();
        let (mut c, seen) = chain(&log);
        // The ledger answers nothing (`None`), which the FSM turns into the refusal the guest reads.
        let r = c.respond(&control(TSG, cmd, &words(&[1, 0, 0, 0])));
        assert!(r.is_none_or(|r| r.rpc_result != NV_OK), "{cmd:#x}");
        assert_eq!(log.total(), 1, "{cmd:#x} reaches the ledger as before");
        assert_no_host_action(&seen);
    }
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, _) = chain(&log);
    let r = c.respond(&control(SUBDEVICE, UPDATE_CHANNEL_INFO, &words(&[0; 8])));
    assert!(r.is_none_or(|r| r.rpc_result != NV_OK));
    assert_eq!(log.total(), 1);
}

/// Another RPC function carrying the same bytes is not a control: the link claims `GSP_RM_CONTROL` only.
#[test]
fn another_function_with_the_same_id_is_not_claimed() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(&log);
    let mut alloc = control(SUBDEVICE, SET_SCHED_POLICY, &words(&[0, 1]));
    alloc.function = RpcFunction::RmAlloc;
    assert!(c.respond(&alloc).is_none_or(|r| r.rpc_result != NV_OK));
    assert_no_host_action(&seen);
}
