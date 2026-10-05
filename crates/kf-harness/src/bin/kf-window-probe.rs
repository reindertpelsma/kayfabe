//! Fault-aware, unprivileged USER-channel window-read research probe.
use std::path::PathBuf;

fn main() {
    match entry() {
        Ok(Some(pass)) => {
            println!(
                "WINDOW_PROBE_VERDICT={}",
                if pass { "PASS" } else { "FAIL" }
            );
            std::process::exit(if pass { 0 } else { 1 });
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("WINDOW_PROBE_ERROR={e}");
            println!("WINDOW_PROBE_VERDICT=FAIL");
            std::process::exit(1);
        }
    }
}

fn entry() -> Result<Option<bool>, String> {
    let mut args = std::env::args().skip(1);
    let mut selected = false;
    let mut manifest = None;
    let mut gpu = 0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--self-test" if !selected => selected = true,
            "--manifest" if !selected => {
                selected = true;
                manifest = Some(PathBuf::from(args.next().ok_or("missing manifest path")?));
            }
            "--gpu-minor" => {
                gpu = args
                    .next()
                    .ok_or("missing GPU minor")?
                    .parse::<u32>()
                    .map_err(|e| e.to_string())?
            }
            "--help" => {
                println!(
                    "kf-window-probe (--self-test | --manifest NEW_PATH) [--gpu-minor N]\n\
                    Run as a non-root user with CapEff=0, on an exclusive GPU.\n\
                    Intentionally faults one USER channel; never resets a GPU itself.\n\
                    Self-test creates/removes only its own alias and does not prove QEMU isolation.\n\
                    Manifest mode waits up to 60 seconds after WINDOW_READY for an atomic regular-file\n\
                    fixture describing this VM's own canary. See scripts/p1p2/WINDOW_PROBE.md."
                );
                return Ok(None);
            }
            _ => return Err(format!("unknown/repeated mode or argument {arg}")),
        }
    }
    if !selected {
        return Err("choose --self-test or --manifest; use --help".into());
    }
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    let uid = status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .ok_or("Uid missing")?;
    let ids = uid
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let cap = status
        .lines()
        .find_map(|l| l.strip_prefix("CapEff:"))
        .ok_or("CapEff missing")?;
    if ids.len() != 4
        || ids.contains(&0)
        || u64::from_str_radix(cap.trim(), 16).map_err(|e| e.to_string())? != 0
    {
        return Err("requires non-root real/effective/saved/filesystem UIDs and CapEff=0".into());
    }
    println!("PROBE_IDENTITY Uid:{uid} CapEff:{cap} gpu_minor={gpu}");
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = kf_host::HostRm::open(
        &dev,
        kf_arch::ids::GpuId(gpu),
        &kf_chip::choose_host_classes,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "PROBE_DEVICE driver={} arch={:?} bdf={} mode={}",
        rm.driver_version(),
        rm.arch_info(),
        rm.card().bdf(),
        if manifest.is_some() {
            "window-fixture"
        } else {
            "instrumentation-self-test"
        }
    );
    kf_harness::window_probe::run(&rm, manifest.as_deref()).map(Some)
}
