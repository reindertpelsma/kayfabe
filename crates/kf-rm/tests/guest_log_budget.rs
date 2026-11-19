//! ★ 2026-11-18 (boundary fuzzing, finding BF-2; closes the `kf-rm` part of S1-86): every log line the
//! served chain writes for a guest command is rate-limited (`kf_arch::rate_eprintln!`: the first 8
//! events of a site, then powers of two, each line carrying the running count).
//!
//! Each class below is one command a guest can repeat at will (most are reachable from unprivileged
//! guest userspace: RM controls with a chosen id, size and flags). The child process sends it
//! [`REPEATS`] times through `kf_rm::served_policy`; the parent counts the class's lines on stderr.
//! Bound: `FIRST + log2(REPEATS) + 1` lines per site, a small number of sites per class. A class that
//! writes no line at all fails too (the test would be vacuous).
//!
//! A second property needs no list: one hostile fuzz session (the generator of
//! `boundary_fuzz_chain.rs`'s shape, fixed seed) replayed [`REPLAYS`] times writes at most a few
//! times the lines of one replay.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;
#[path = "support/windows_fn1.rs"]
mod support;

use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use std::sync::Arc;

const REPEATS: usize = 1024;
/// `ratelog::FIRST` (8) + powers of two up to 1024 (11), per site; a class may touch two sites.
const PER_CLASS: usize = 2 * (8 + 11);
const REPLAYS: usize = 64;
const CLIENT: u32 = 0xc1d0_0042;
const DEVICE: u32 = 0xff01_0001;
const SUBDEV: u32 = 0xff02_0001;

fn chain(windows: bool) -> Box<dyn CommandPolicy> {
    let driver = if windows {
        support::abi()
    } else {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench driver")
    };
    let os = if windows {
        kf_abi::GuestOs::Windows
    } else {
        kf_abi::GuestOs::Linux
    };
    let links = kf_rm::ObjectLinks {
        objects: Some(kf_rm::rmrpc::ObjectPolicy::over(
            &driver,
            os,
            Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
            Default::default(),
        )),
        memory: Some(kf_rm::MemoryLink {
            sink: Arc::new(|_| {}),
            guest_os: os,
        }),
        channels: Some(Arc::new(|_| kf_rm::chanlink::ChanAnswer::Token(1))),
        display: Some((&kf_chip::display::AMPERE).into()),
        console: None,
        windows: kf_rm::winprofile::WindowsProfile::on(),
    };
    let mut c = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver,
        Default::default(),
        Default::default(),
        links,
    );
    let fn1 = if windows {
        support::windows(&driver)
    } else {
        support::linux(&driver)
    };
    c.respond(&fn1);
    for w in world() {
        c.respond(&w);
    }
    c
}

fn cmd(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn world() -> Vec<RpcCommand> {
    let alloc = kf_rm::abi::FUNCTIONS.gsp_rm_alloc;
    vec![
        cmd(
            RpcFunction::RmAlloc,
            alloc,
            rpcwire::client_root_alloc_body(0x41, CLIENT, 1234),
        ),
        cmd(
            RpcFunction::RmAlloc,
            alloc,
            rpcwire::alloc_body(
                CLIENT,
                CLIENT,
                DEVICE,
                0x80,
                56,
                0,
                &rpcwire::device_params(0, 0, 0),
            ),
        ),
        cmd(
            RpcFunction::RmAlloc,
            alloc,
            rpcwire::alloc_body(CLIENT, DEVICE, SUBDEV, 0x2080, 4, 0, &[0; 4]),
        ),
    ]
}

fn control(object: u32, id: u32, params: usize, flags: u32) -> RpcCommand {
    cmd(
        RpcFunction::RmControl,
        kf_rm::abi::FUNCTIONS.gsp_rm_control,
        rpcwire::control_body(
            CLIENT,
            object,
            id,
            params as u32,
            flags,
            &vec![0xa5; params],
        ),
    )
}

/// `(class, Windows arm, the pattern its lines carry, the command)`.
fn classes() -> Vec<(&'static str, bool, &'static str, RpcCommand)> {
    let v = kf_abi::versions::BENCH_DRIVER;
    let latency = kf_abi::fifoctl::LatencyLayout::at(v).expect("measured").cmd;
    let codes = kf_rm::abi::FUNCTIONS;
    vec![
        (
            "gss-census",
            false,
            "W343GSSCENSUS",
            control(SUBDEV, 0x2080_beef, 16, 0),
        ),
        (
            "init-table-size",
            false,
            "why=size",
            control(SUBDEV, 0x2080_0a40, 4, 0),
        ),
        (
            "init-table-finn",
            false,
            "why=finn-serialized",
            control(SUBDEV, 0x2080_9001, 16, 3),
        ),
        (
            "static-info-size",
            false,
            "GET_GSP_STATIC_INFO refused",
            cmd(
                RpcFunction::GetGspStaticInfo,
                codes.get_gsp_static_info,
                vec![0; 56],
            ),
        ),
        (
            "display-finn",
            false,
            "arrived FINN-serialized",
            control(SUBDEV, 0x2080_0a01, 16, 2),
        ),
        (
            "bar-pde-short",
            false,
            "UPDATE_BAR_PDE refused",
            cmd(RpcFunction::UpdateBarPde, codes.update_bar_pde, vec![]),
        ),
        (
            "video-caps-size",
            false,
            "video caps",
            control(DEVICE, 0x0080_1c02, 4095, 0),
        ),
        (
            "fifo-latency-size",
            false,
            "FIFO_GET_LATENCY_BUFFER_SIZE REFUSED",
            control(DEVICE, latency, 1, 0),
        ),
        (
            "fn1-undecodable",
            false,
            "SET_GUEST_SYSTEM_INFO",
            cmd(
                RpcFunction::SetGuestSystemInfo,
                codes.set_guest_system_info,
                vec![],
            ),
        ),
    ]
}

#[test]
fn guest_log_child() {
    let Ok(mode) = std::env::var("KF_TEST_GUEST_LOG_CHILD") else {
        return;
    };
    if mode == "replay" {
        let session = hostile_session();
        let reps: usize = std::env::var("KF_TEST_GUEST_LOG_REPS")
            .unwrap()
            .parse()
            .unwrap();
        let mut c = chain(false);
        for _ in 0..reps {
            for s in &session {
                c.respond(s);
            }
        }
        eprintln!("CHILD-DONE");
        return;
    }
    let (_, windows, _, command) = classes()
        .into_iter()
        .find(|c| c.0 == mode)
        .expect("a class");
    let mut c = chain(windows);
    let reps: usize = std::env::var("KF_TEST_GUEST_LOG_REPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(REPEATS);
    eprintln!("CHILD-START");
    for _ in 0..reps {
        c.respond(&command);
    }
    eprintln!("CHILD-DONE");
}

fn child(env: &[(&str, String)]) -> String {
    let mut c = std::process::Command::new(std::env::current_exe().unwrap());
    c.args([
        "--exact",
        "guest_log_child",
        "--nocapture",
        "--test-threads=1",
    ]);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("CHILD-DONE"), "the child ran to its end");
    err
}

#[test]
fn every_guest_repeatable_refusal_line_is_rate_limited() {
    let mut over = Vec::new();
    for (name, _, pattern, _) in classes() {
        let err = child(&[("KF_TEST_GUEST_LOG_CHILD", name.to_string())]);
        let after = err.split("CHILD-START").nth(1).unwrap_or_default();
        let lines = after.lines().filter(|l| l.contains(pattern)).count();
        if lines == 0 || lines > PER_CLASS {
            over.push(format!(
                "{name}: {lines} lines matching {pattern:?} for {REPEATS} commands"
            ));
        }
    }
    assert!(
        over.is_empty(),
        "unbounded or vacuous classes:\n{}",
        over.join("\n")
    );
}

/// The second property of the same sites: a line that is written says how many events its site has
/// seen (`[#N, rate-limited]`), so a thinned log still states how often it happened.
#[test]
fn every_guest_repeatable_refusal_line_carries_its_running_count() {
    let mut bare = Vec::new();
    for (name, _, pattern, _) in classes() {
        let err = child(&[
            ("KF_TEST_GUEST_LOG_CHILD", name.to_string()),
            ("KF_TEST_GUEST_LOG_REPS", "3".into()),
        ]);
        let after = err.split("CHILD-START").nth(1).unwrap_or_default();
        let lines: Vec<&str> = after.lines().filter(|l| l.contains(pattern)).collect();
        if lines.is_empty() || lines.iter().any(|l| !l.ends_with(", rate-limited]")) {
            bare.push(format!("{name}: {lines:?}"));
        }
    }
    assert!(
        bare.is_empty(),
        "lines without a running count:\n{}",
        bare.join("\n")
    );
}

/// A fixed hostile session: controls of every id this crate and `kf-abi` name, at sizes and flags
/// that miss, against the session's objects.
fn hostile_session() -> Vec<RpcCommand> {
    let mut ids = std::collections::BTreeSet::new();
    for dir in [
        "src",
        "../kf-abi/src/inittables.rs",
        "../kf-abi/src/capability.rs",
    ] {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        let files: Vec<std::path::PathBuf> = if p.is_dir() {
            std::fs::read_dir(&p)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .collect()
        } else {
            vec![p]
        };
        for f in files {
            let t = std::fs::read_to_string(&f).unwrap_or_default();
            for tok in t.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                if let Some(h) = tok.strip_prefix("0x")
                    && let Ok(v) = u32::from_str_radix(&h.replace('_', ""), 16)
                    && v > 0xffff
                {
                    ids.insert(v);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (i, id) in ids.into_iter().enumerate() {
        let object = match id >> 16 {
            0x0000 => CLIENT,
            0x0080 => DEVICE,
            _ => SUBDEV,
        };
        out.push(control(
            object,
            id,
            [3usize, 17, 4095][i % 3],
            [0u32, 2, 3][i % 3],
        ));
    }
    out
}

#[test]
fn a_replayed_hostile_session_writes_a_bounded_number_of_lines() {
    let once = child(&[
        ("KF_TEST_GUEST_LOG_CHILD", "replay".into()),
        ("KF_TEST_GUEST_LOG_REPS", "1".into()),
    ])
    .lines()
    .count();
    let many = child(&[
        ("KF_TEST_GUEST_LOG_CHILD", "replay".into()),
        ("KF_TEST_GUEST_LOG_REPS", REPLAYS.to_string()),
    ])
    .lines()
    .count();
    assert!(
        many <= 4 * once + 256,
        "one replay wrote {once} lines, {REPLAYS} replays wrote {many}: the log grows with the guest's command count"
    );
}
