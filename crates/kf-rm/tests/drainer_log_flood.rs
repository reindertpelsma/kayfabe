//! ★ Drainer verification 2026-10-09 (`docs/design/V3_NONSTALL_THREADS.md` §9, item L2): lines the
//! served policy chain writes on the register drainer that a GUEST can make it write again and again.
//!
//! `klog!` is one synchronous `write(2)` to stderr (`kf_util::log`; the control
//! `control_a_log_call_lasts_exactly_as_long_as_its_sink_blocks` there pins that a call lasts as long
//! as its sink blocks). The owner's rules: production is quiet, and a guest-reachable unbounded emit
//! is a security bug (`AGENTS.md`). So what must hold is that a guest repeating one RPC cannot make
//! the drainer write one line per repeat: a repeating line is `klog_limited!` (the first 4, then
//! powers of two) or `klog_trace!` (default off).
//!
//! Each test drives the real policy with the same request 100 times on a thread named as the drainer
//! and counts the drainer's log calls (`kf_util::log::stats`). They FAIL today: they are the
//! findings, kept `#[ignore]`d so CI stays green, and run with
//! `cargo test -p kf-rm --test drainer_log_flood -- --ignored`.
//!
//! Sites found by reading, not yet under a test: `kf-gsp/src/sysinfo.rs:94` (`log_nocat`, every
//! fn 72), `kf-rm/src/staticinfo.rs:268-307` (every fn 65 with a console seat),
//! `kf-gsp/src/boot.rs:1714` (`GSP-PUBLISH`, every boot-args publish), `kf-qemu/src/device.rs:3364`
//! (`GSP phase … -> …`, every phase change) and `:3370` (`WPR2 up`).

use kf_abi::DriverVersion;
use kf_abi::guestsysinfo::SET_GUEST_SYSTEM_INFO_SIZE;
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::guestsysinfo::GuestSystemInfoPolicy;
use kf_rm::{GuestDriverSource, ReselectAtFn1};
use kf_util::log::{ThreadClass, set_class, stats};

fn fn1(major: u32, minor: u32, version: &str) -> RpcCommand {
    let mut payload = vec![0xCDu8; SET_GUEST_SYSTEM_INFO_SIZE];
    payload[0..4].copy_from_slice(&major.to_le_bytes());
    payload[4..8].copy_from_slice(&minor.to_le_bytes());
    payload[24..24 + version.len()].copy_from_slice(version.as_bytes());
    payload[24 + version.len()] = 0;
    RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 1,
        sequence: 3,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn drainer_calls() -> u64 {
    stats().0[ThreadClass::Drainer as usize].calls
}

/// The counters are per thread CLASS and process-wide: the tests of this file take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What a bounded emitter may print for 100 repeats: the first 4 and the powers of two (7 lines).
const MAX_LINES_FOR_100_REPEATS: u64 = 8;

/// The same fn-1 request, 100 times, to the policy that answers it (`GuestSystemInfoPolicy`): one
/// "the guest says …" line each.
#[test]
#[ignore = "FINDING L2: GuestSystemInfoPolicy logs every fn 1 (guestsysinfo.rs:174-179) — one write(2) per guest repeat on the drainer"]
fn a_guest_repeating_fn1_cannot_make_the_drainer_write_a_line_each_time() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    set_class(ThreadClass::Drainer);
    let mut p = GuestSystemInfoPolicy::new(*table_for(BENCH_DRIVER).expect("a table row"));
    let req = fn1(0x2B, 0x13, "580.159.04");
    let before = drainer_calls();
    for _ in 0..100 {
        let _ = p.respond(&req);
    }
    let lines = drainer_calls() - before;
    assert!(
        lines <= MAX_LINES_FOR_100_REPEATS,
        "100 repeats of one fn 1 cost the drainer {lines} log calls"
    );
}

/// The chain's re-selecting wrapper (`ReselectAtFn1`, the first link every fn 1 meets): a device whose
/// driver was DECLARED refuses a guest of another version by name — and says so on every fn 1.
#[test]
#[ignore = "FINDING L2: ReselectAtFn1 logs the refusal on every fn 1 (lib.rs:408-420)"]
fn a_guest_repeating_a_refused_fn1_cannot_make_the_drainer_write_a_line_each_time() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    set_class(ThreadClass::Drainer);
    let declared: DriverVersion = BENCH_DRIVER;
    let table = *table_for(declared).expect("a table row");
    let mut p = ReselectAtFn1::new(
        table,
        GuestDriverSource::Declared,
        Box::new(|_t| Box::new(kf_gsp::EchoOk)),
    );
    // A guest that says it is another measured driver.
    let req = fn1(0x2B, 0x13, "580.65.06");
    let before = drainer_calls();
    for _ in 0..100 {
        let _ = p.respond(&req);
    }
    let lines = drainer_calls() - before;
    assert!(
        lines <= MAX_LINES_FOR_100_REPEATS,
        "100 repeats of one refused fn 1 cost the drainer {lines} log calls"
    );
}
