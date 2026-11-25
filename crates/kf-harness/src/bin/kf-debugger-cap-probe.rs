//! **How many host debugger sessions one process, and all processes together, can make before host RM
//! refuses** (decision A8; `traces/debugger_cap_20261125/PREREGISTRATION.md`). Host only, no VM.
//!
//! One process is one host RM client, device and subdevice, the shape kf3 holds per VM. It builds
//! `--grs` GR objects the way a twin does (bare VA space, GR channel group and channel, compute object),
//! then makes `GT200_DEBUGGER` sessions with `HostRm::alloc_debugger`, the call kf3 makes for a guest
//! alloc, round robin over its GR objects, until host RM refuses on every one, `--per-gr` sessions sit
//! on each, `--max-sessions` is reached or the controller says `stop`.
//!
//! ```text
//! kf-debugger-cap-probe [--gpu N] [--grs N] [--per-gr N] [--max-sessions N] [--progress-every N] [--no-wait]
//! ```
//!
//! Driven by `scripts/bench/debugger_cap_arms.py` over stdin (`go`, `stop`, `release`, `exit`; end of input
//! means `stop` and `exit`). Prints one line per event, `TAG key=value ...`: `SETUP`, `READY`, `PROGRESS`,
//! `REFUSED`, `ALLOC_DONE` (then waits for `release`), `FREE_DONE` (then waits for `exit`), `EXIT`.
//! Every object is this process's own; closing it frees the client. No FB is read here: the controller
//! samples `nvidia-smi`.

use kf_abi::submit::ENGINE_TYPE_GRAPHICS;
use kf_harness::dbgcap::{
    Cmd, Spread, latency, parse_args, parse_cmd, parse_rss_kib, refusal_name,
};
use kf_host::{HostRm, MapBacking, RingSpec, VaSpace};
use kf_linux_raw::DevDir;
use std::io::BufRead;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;

const RING_BYTES: u64 = 0x1_0000;
const USERD_AT: u64 = 0x8000;

struct Gr {
    space: VaSpace,
    ring: u32,
    chan: kf_host::Channel,
    object: u32,
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| parse_rss_kib(&s))
        .unwrap_or(0)
}

fn build_gr(rm: &HostRm) -> Result<Gr, String> {
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space {e:?}"))?;
    let ring = rm
        .alloc_device_local(RING_BYTES)
        .map_err(|e| format!("ring {e:?}"))?;
    let at = rm
        .map(
            space,
            ring,
            MapBacking::Dedicated,
            0,
            RING_BYTES,
            None,
            false,
        )
        .map_err(|e| format!("ring map {e:?}"))?;
    let chan = rm
        .birth_channel(
            space,
            ENGINE_TYPE_GRAPHICS,
            RingSpec {
                gp_fifo_va: at,
                gp_fifo_entries: 128,
                userd_memory: ring,
                userd_offset: USERD_AT,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("channel {e:?}"))?;
    let (object, _class) = rm
        .alloc_compute_object(chan)
        .map_err(|e| format!("compute {e:?}"))?;
    Ok(Gr {
        space,
        ring,
        chan,
        object,
    })
}

/// Block for the next controller command; end of input is `exit`.
fn wait_for(rx: &Receiver<Cmd>, want: &[Cmd]) -> Cmd {
    loop {
        match rx.recv() {
            Ok(c) if want.contains(&c) || c == Cmd::Exit => return c,
            Ok(_) => {}
            Err(_) => return Cmd::Exit,
        }
    }
}

fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            println!("SETUP_ERR error=args {e}");
            std::process::exit(2);
        }
    };
    let (tx, rx) = channel::<Cmd>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Some(c) = parse_cmd(&line)
                && tx.send(c).is_err()
            {
                break;
            }
        }
    });
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("SETUP_ERR error=open_dev {e:?}");
            std::process::exit(2);
        }
    };
    let rm = match HostRm::open(
        &dev,
        kf_arch::ids::GpuId(args.gpu),
        &kf_chip::choose_host_classes,
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("SETUP_ERR error=open {e}");
            std::process::exit(2);
        }
    };
    let setup = Instant::now();
    let mut grs: Vec<Gr> = Vec::new();
    for i in 0..args.grs {
        match build_gr(&rm) {
            Ok(g) => grs.push(g),
            Err(e) => {
                println!("SETUP_ERR error=gr gr={i} {e}");
                break;
            }
        }
    }
    println!(
        "SETUP driver={} grs_built={} grs_asked={} ms={} rss_kib={}",
        rm.driver_version(),
        grs.len(),
        args.grs,
        setup.elapsed().as_millis(),
        rss_kib()
    );
    if grs.is_empty() {
        std::process::exit(1);
    }
    println!("READY");
    let mut stopped = false;
    if args.wait_go {
        match wait_for(&rx, &[Cmd::Go]) {
            Cmd::Go => {}
            Cmd::Stop | Cmd::Release => stopped = true,
            Cmd::Exit => {
                finish(&rm, grs, Vec::new());
                return;
            }
        }
    }

    let mut spread = Spread::new(grs.len(), args.per_gr);
    let mut sessions: Vec<u32> = Vec::new();
    let mut window: Vec<u32> = Vec::new();
    let mut all: Vec<u32> = Vec::new();
    let t0 = Instant::now();
    let reason;
    loop {
        if stopped {
            reason = "stop";
            break;
        }
        match rx.try_recv() {
            Ok(Cmd::Stop) => {
                reason = "stop";
                break;
            }
            Ok(Cmd::Exit) | Err(TryRecvError::Disconnected) => {
                reason = "ctl_gone";
                break;
            }
            Ok(_) | Err(TryRecvError::Empty) => {}
        }
        if sessions.len() >= args.max_sessions {
            reason = "ceiling";
            break;
        }
        let Some(j) = spread.target() else {
            reason = if spread.refused_count() == grs.len() {
                "all_refused"
            } else {
                "per_gr_full"
            };
            break;
        };
        let t = Instant::now();
        let r = rm.alloc_debugger(grs[j].object);
        let us = u32::try_from(t.elapsed().as_micros()).unwrap_or(u32::MAX);
        match r {
            Ok(h) => {
                sessions.push(h);
                spread.made(j);
                window.push(us);
                all.push(us);
                if sessions.len().is_multiple_of(args.progress_every) {
                    let l = latency(&mut window);
                    println!(
                        "PROGRESS n={} t_ms={} rss_kib={} win_mean_us={} win_p99_us={} win_max_us={}",
                        sessions.len(),
                        t0.elapsed().as_millis(),
                        rss_kib(),
                        l.map_or(0, |l| l.mean),
                        l.map_or(0, |l| u64::from(l.p99)),
                        l.map_or(0, |l| u64::from(l.max)),
                    );
                    window.clear();
                }
            }
            Err(e) => {
                spread.refused(j);
                println!(
                    "REFUSED gr={j} on_gr={} total={} after_ms={} err={} raw={e:?}",
                    spread.on(j),
                    sessions.len(),
                    t0.elapsed().as_millis(),
                    refusal_name(&e),
                );
            }
        }
    }
    let l = latency(&mut all);
    let per: Vec<usize> = (0..grs.len()).map(|j| spread.on(j)).collect();
    println!(
        "ALLOC_DONE reason={reason} total={} refused_grs={} grs={} per_gr_min={} per_gr_max={} ms={} rss_kib={} mean_us={} p50_us={} p99_us={} max_us={}",
        sessions.len(),
        spread.refused_count(),
        grs.len(),
        per.iter().min().copied().unwrap_or(0),
        per.iter().max().copied().unwrap_or(0),
        t0.elapsed().as_millis(),
        rss_kib(),
        l.map_or(0, |l| l.mean),
        l.map_or(0, |l| u64::from(l.p50)),
        l.map_or(0, |l| u64::from(l.p99)),
        l.map_or(0, |l| u64::from(l.max)),
    );
    let exit_asked = wait_for(&rx, &[Cmd::Release]) == Cmd::Exit;
    let tf = Instant::now();
    let (mut ok, mut failed) = (0usize, 0usize);
    let mut first_fail = String::new();
    for h in sessions.drain(..) {
        match rm.free(h) {
            Ok(()) => ok += 1,
            Err(e) => {
                failed += 1;
                if first_fail.is_empty() {
                    first_fail = format!("{e:?}");
                }
            }
        }
    }
    println!(
        "FREE_DONE freed={ok} failed={failed} ms={} first_failure={}",
        tf.elapsed().as_millis(),
        if first_fail.is_empty() {
            "-"
        } else {
            &first_fail
        }
    );
    if !exit_asked {
        let _ = wait_for(&rx, &[]);
    }
    finish(&rm, grs, sessions);
}

fn finish(rm: &HostRm, grs: Vec<Gr>, sessions: Vec<u32>) {
    for h in sessions {
        let _ = rm.free(h);
    }
    for g in grs {
        let _ = rm.free_channel(g.chan);
        rm.free_vaspace(g.space);
        let _ = rm.free(g.ring);
    }
    println!("EXIT rc=0");
}
