//! The dispatcher behind `main`: parse the command line, run the command, return the exit code. Kept out
//! of `main.rs` so the contract (what is printed where, which code is returned) is tested without a
//! process.

use crate::cli::{self, Command};
use crate::clock::{Clock, MonotonicClock};
use crate::hw::HwBackend;
use crate::ledger::{Sink, StdoutSink};
use crate::mode::Mode;
use crate::outcome::{EXIT_NOTRUN, EXIT_PASS, EXIT_USAGE, token};
use crate::registry::{self, ArmSpec};
use crate::stress::compare::{MAX_LOG_BYTES, Thresholds, compare, parse_log};
use crate::stress::params::StressParams;
use crate::stress::rng::mix;
use crate::stress::run::{StressEnv, run_stress};
use crate::watchdog::{Watchdog, exit_on_expiry};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

/// Run the program for `args` (without `argv[0]`), printing to `sink` (stdout) and to stderr for
/// refusals. Returns the process exit code.
pub fn main_with(args: &[String], sink: &mut dyn Sink) -> u8 {
    let cmd = match cli::parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kf-rawclient: {e}");
            eprintln!("{}", cli::usage());
            return EXIT_USAGE;
        }
    };
    match cmd {
        Command::Help => {
            for l in cli::usage().lines() {
                sink.line(l);
            }
            EXIT_PASS
        }
        Command::ListArms => {
            for l in registry::list_lines() {
                sink.line(&l);
            }
            EXIT_PASS
        }
        Command::Compare { native, guest } => run_compare(&native, &guest, sink),
        Command::Run {
            arm,
            gpu,
            mode,
            rest,
        } => run_arm(arm, gpu, mode, &rest, sink),
    }
}

fn read_bounded(path: &str) -> Result<String, &'static str> {
    let f = std::fs::File::open(path).map_err(|_| "unreadable-log")?;
    let mut text = String::new();
    f.take(MAX_LOG_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "unreadable-log")?;
    Ok(text)
}

fn run_compare(native: &str, guest: &str, sink: &mut dyn Sink) -> u8 {
    let unusable = |sink: &mut dyn Sink, why: &str| {
        sink.line(&format!(
            "STRESS_RATIO die=- drv=- rev=- critical_ratio=- map_p99_ratio=- state=unusable reason={}",
            token(why)
        ));
        EXIT_NOTRUN
    };
    let (n, g) = match (read_bounded(native), read_bounded(guest)) {
        (Ok(n), Ok(g)) => (n, g),
        (Err(e), _) | (_, Err(e)) => return unusable(sink, e),
    };
    let (n, g) = match (parse_log(&n), parse_log(&g)) {
        (Ok(n), Ok(g)) => (n, g),
        (Err(e), _) | (_, Err(e)) => return unusable(sink, e.0),
    };
    let th = Thresholds::CURRENT;
    let c = compare(&n, &g, th);
    sink.line(&c.line(th, Some(&n.cell)));
    c.exit_code()
}

fn run_arm(
    arm: &'static ArmSpec,
    gpu: u32,
    mode: Mode,
    rest: &[String],
    sink: &mut dyn Sink,
) -> u8 {
    match arm.name {
        "refresh-stress" => run_refresh_stress(arm, gpu, mode, rest, sink),
        other => {
            eprintln!("kf-rawclient: arm {other} is registered but has no runner");
            EXIT_USAGE
        }
    }
}

fn run_refresh_stress(
    arm: &'static ArmSpec,
    gpu: u32,
    mode: Mode,
    rest: &[String],
    sink: &mut dyn Sink,
) -> u8 {
    let params = match StressParams::parse(rest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("kf-rawclient: {e}");
            return EXIT_USAGE;
        }
    };
    let clock = Arc::new(MonotonicClock::new());
    let wd = Watchdog::start(
        clock.clone(),
        u64::from(params.phase_s) * 1_000_000_000,
        u64::from(params.hard_cap_s) * 1_000_000_000,
        Duration::from_millis(50),
        exit_on_expiry(),
    );
    let default_seed = {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        mix(t ^ u64::from(std::process::id()) << 32 ^ clock.now_ns())
    };
    let rev = std::env::var("KF_REV").ok().filter(|r| !r.is_empty());
    let mut backend = HwBackend::new(gpu);
    let mut env = StressEnv {
        clock: &*clock,
        progress: &wd,
        sink,
        mode,
        rev,
        default_seed,
    };
    run_stress(&params, arm, &mut backend, &mut env)
        .outcome
        .exit_code()
}

/// `main` for the binary.
#[must_use]
pub fn main_exit() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut sink = StdoutSink;
    std::process::ExitCode::from(main_with(&args, &mut sink))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::VecSink;

    fn run(a: &[&str]) -> (u8, Vec<String>) {
        let mut s = VecSink::default();
        let rc = main_with(
            &a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>(),
            &mut s,
        );
        (rc, s.lines)
    }

    #[test]
    fn list_arms_and_help_exit_zero_and_print_the_registry() {
        let (rc, l) = run(&["--list-arms"]);
        assert_eq!(rc, 0);
        assert!(l.iter().any(|x| x.starts_with("ARM name=refresh-stress ")));
        let (rc, l) = run(&["--help"]);
        assert_eq!(rc, 0);
        assert!(l.iter().any(|x| x.contains("--refresh-stress")));
    }

    #[test]
    fn a_refused_command_line_exits_2_and_prints_nothing_on_stdout() {
        for a in [
            vec![],
            vec!["--timer"],
            vec!["--refresh-stress", "--stress-batches", "0"],
            vec!["--refresh-stress", "--stress-bogus", "1"],
            vec!["--gpu", "999", "--refresh-stress"],
        ] {
            let (rc, l) = run(&a);
            assert_eq!(rc, 2, "{a:?}");
            assert!(l.is_empty(), "{a:?}: {l:?}");
        }
    }

    #[test]
    fn without_a_gpu_the_arm_is_notrun_not_a_failure() {
        // This host has no RM device in the sandbox of the unit tests (and on a GPU host the run would
        // not be a unit test), so the arm must come back NOTRUN with the reason. Skipped where a node exists.
        if std::path::Path::new("/dev/nvidiactl").exists() {
            return;
        }
        let (rc, l) = run(&[
            "--refresh-stress",
            "--mode",
            "bare",
            "--gpu",
            "0",
            "--stress-seed",
            "1",
        ]);
        assert_eq!(rc, 3, "{l:#?}");
        assert!(
            l.iter()
                .any(|x| x.starts_with("RUNG_refresh_stress=NOTRUN reason=no-rm-device")),
            "{l:#?}"
        );
        assert!(
            l.iter()
                .any(|x| x.starts_with("RAWCLIENT_JSON {") && x.contains("\"outcome\":\"NOTRUN\""))
        );
        assert!(
            l.iter()
                .any(|x| x.starts_with("STRESS_PARAMS ") && x.ends_with("seed=0x1"))
        );
    }

    #[test]
    fn compare_reads_two_logs_and_refuses_unreadable_or_mismatched_ones() {
        let dir = std::env::temp_dir().join(format!("kf-rawclient-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = |mode: &str, crit: f64, p99: f64| {
            format!(
                "RAWCLIENT_CELL die=GA106 family=Ampere mmu=Ver2 drv=580.159.04 kmod=open mode={mode} rev=r1\nSTRESS_REFRESH n=0 critical_ms={crit}\nSTRESS_REFRESH n=1 critical_ms={crit}\nSTRESS_HIST op=map n=2 p50_us=1 p90_us=1 p99_us={p99} max_us=9 log2_buckets=0:1\nRUNG_refresh_stress=PASS\n"
            )
        };
        let (n, g, bad) = (dir.join("n.log"), dir.join("g.log"), dir.join("bad.log"));
        std::fs::write(&n, log("bare", 10.0, 100.0)).unwrap();
        std::fs::write(&g, log("guest", 100.0, 1000.0)).unwrap();
        std::fs::write(&bad, "garbage\n").unwrap();
        let p = |x: &std::path::Path| x.to_str().unwrap().to_owned();
        let (rc, l) = run(&["--refresh-compare", &p(&n), &p(&g)]);
        assert_eq!(rc, 0, "{l:?}");
        assert!(
            l[0].contains("critical_ratio=10.000 map_p99_ratio=10.000")
                && l[0].contains("state=within")
        );
        let (rc, l) = run(&["--refresh-compare", &p(&g), &p(&n)]);
        assert_eq!(rc, 3);
        assert!(l[0].contains("state=unusable reason=native-log-mode-not-bare"));
        let (rc, l) = run(&["--refresh-compare", &p(&n), &p(&bad)]);
        assert_eq!(rc, 3);
        assert!(l[0].contains("reason=no-cell-line"));
        let (rc, l) = run(&["--refresh-compare", &p(&n), "/nonexistent/kf.log"]);
        assert_eq!(rc, 3);
        assert!(l[0].contains("reason=unreadable-log"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
