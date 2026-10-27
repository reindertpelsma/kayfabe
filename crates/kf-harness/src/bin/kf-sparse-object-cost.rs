//! Explicitly authorized host-only RM object-cost arm. No VM or GPU workload is created.
//! Never run by default; the controller must own the host lock and a desktop watchdog.
use kf_host::HostRm;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn probe(program: &Path, out: &Path, label: &str) -> Result<(), String> {
    let file = std::fs::File::create(out.join(format!("watchdog-{label}.txt")))
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .stdout(file)
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("watchdog {status}"))
            };
        }
        if start.elapsed() >= Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("watchdog exceeded3s; stop this arm".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn snapshot(out: &Path, name: &str) -> Result<(), String> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    std::fs::write(out.join(format!("{name}-status.txt")), status).map_err(|e| e.to_string())?;
    for (label, path) in [
        ("meminfo", "/proc/meminfo"),
        ("rm-clients", "/proc/driver/nvidia/clients"),
    ] {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let _ = std::fs::write(out.join(format!("{name}-{label}.txt")), text);
            }
            Err(e) => eprintln!("{label} accounting unavailable: {e}"),
        }
    }
    // Optional accounting. Absence is evidence unavailable, not zero memory cost.
    let file = std::fs::File::create(out.join(format!("{name}-gpu-memory.txt")))
        .map_err(|e| e.to_string())?;
    let mut child = Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_gpu_memory",
            "--format=csv,noheader",
        ])
        .stdout(file)
        .spawn()
        .map_err(|e| format!("nvidia-smi unavailable: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("accounting {status}"))
            };
        }
        if start.elapsed() >= Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("accounting unavailable:3s timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 || args[0] != "--run-native" {
        return Err("usage: --run-native <1000|8000|32000|128000> \
            <sparse|normal> <fresh-output-dir> <desktop-watchdog-program>; \
            owner authorization and exclusive host lock are required"
            .into());
    }
    let n: usize = args[1].parse().map_err(|_| "invalid object count")?;
    if ![1000, 8000, 32000, 128000].contains(&n) {
        return Err("count outside protocol".into());
    }
    let sparse = match args[2].as_str() {
        "sparse" => true,
        "normal" => false,
        _ => return Err("mode must be sparse|normal".into()),
    };
    let out = PathBuf::from(&args[3]);
    let watchdog = PathBuf::from(&args[4]);
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    let uid = status.lines().find(|l| l.starts_with("Uid:")).unwrap_or("");
    if uid.split_whitespace().skip(1).any(|id| id == "0")
        || !status
            .lines()
            .any(|l| l.starts_with("CapEff:") && l.ends_with("0000000000000000"))
    {
        return Err("run under an unprivileged uid with zero effective capabilities".into());
    }
    if std::env::var("KF3_SPARSE_TWIN").ok().as_deref() == Some("1") {
        return Err("unset twin: this arm measures raw owned RM reservations".into());
    }
    std::fs::create_dir(&out).map_err(|e| e.to_string())?;
    probe(&watchdog, &out, "before")?;
    let _ = snapshot(&out, "before").map_err(|e| eprintln!("{e}"));
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let grain = if sparse {
        kf_host::sparse::SPARSE_GRAIN
    } else {
        kf_host::sparse::NORMAL_GRAIN
    };
    let mut handles = Vec::with_capacity(n);
    let mut samples = Vec::with_capacity(n.saturating_mul(2));
    let mut refusal = None;
    for i in 0..n {
        let va = (1u64 << 40)
            .checked_add((i as u64).checked_mul(grain).ok_or("VA offset wraps")?)
            .ok_or("VA wraps")?;
        let start = Instant::now();
        let result = rm.reserve_va_flags(
            space.space,
            va,
            grain,
            kf_host::channel::guest_reserve_flags(sparse),
        );
        samples.push(("alloc", i, start.elapsed().as_nanos()));
        match result {
            Ok(h) => handles.push(h),
            Err(e) => {
                refusal = Some(format!("first_refusal_index={i} status={e:?}"));
                break;
            }
        }
        if i.is_multiple_of(512)
            && let Err(e) = probe(&watchdog, &out, "during")
        {
            refusal = Some(e);
            break;
        }
    }
    let _ = snapshot(&out, "peak").map_err(|e| eprintln!("{e}"));
    let created = handles.len();
    let mut free_errors = 0usize;
    for (i, h) in handles.into_iter().enumerate().rev() {
        let start = Instant::now();
        let result = rm.free(h);
        samples.push(("free", i, start.elapsed().as_nanos()));
        if let Err(e) = result {
            free_errors += 1;
            eprintln!("own free {h:#x}: {e:?}");
        }
    }
    rm.free_vaspace(space);
    let _ = snapshot(&out, "after").map_err(|e| eprintln!("{e}"));
    let watchdog_after = probe(&watchdog, &out, "after");
    let mut csv = std::fs::File::create(out.join("samples.csv")).map_err(|e| e.to_string())?;
    writeln!(csv, "op,index,wall_ns").map_err(|e| e.to_string())?;
    for (op, i, ns) in samples {
        writeln!(csv, "{op},{i},{ns}").map_err(|e| e.to_string())?;
    }
    println!(
        "ENS5_RM_COST requested={n} created={created} sparse={sparse} grain={grain} \
        free_errors={free_errors} refusal={refusal:?} watchdog_after={watchdog_after:?}"
    );
    if refusal.is_some() || free_errors != 0 {
        return Err("arm stopped; inspect receipts".into());
    }
    watchdog_after
}
