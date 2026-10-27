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
/// Refuse to run privileged, with the twin env set, or into an existing output directory.
fn guard(out: &Path) -> Result<(), String> {
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
    std::fs::create_dir(out).map_err(|e| e.to_string())
}

/// ENS6 churn arm: `N` live background 2 MiB reservations (the saturated tree of 7330), then the
/// recycling pattern ENS5 introduced: release two adjacent 2 MiB blocks and reserve the merged
/// 4 MiB at the same VA (3 verbs), then the split back (3 verbs). Reports the first refusal.
fn churn(args: &[String]) -> Result<(), String> {
    let usage = "usage: --run-churn <background-N> <cycles> <fresh-output-dir> <watchdog>";
    if args.len() != 4 {
        return Err(usage.into());
    }
    let n: usize = args[0].parse().map_err(|_| usage)?;
    let cycles: usize = args[1].parse().map_err(|_| usage)?;
    if n > 128_000 || cycles > 20_000 {
        return Err("count outside protocol".into());
    }
    let out = PathBuf::from(&args[2]);
    let watchdog = PathBuf::from(&args[3]);
    guard(&out)?;
    probe(&watchdog, &out, "before")?;
    let _ = snapshot(&out, "before").map_err(|e| eprintln!("{e}"));
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let grain = kf_host::sparse::NORMAL_GRAIN;
    let flags = kf_host::channel::guest_reserve_flags(false);
    let base = 1u64 << 40;
    let mut samples: Vec<(&str, usize, u128)> = Vec::new();
    let mut background = Vec::with_capacity(n);
    let mut refusal = None;
    for i in 0..n {
        let start = Instant::now();
        let r = rm.reserve_va_flags(space.space, base + i as u64 * grain, grain, flags);
        samples.push(("alloc", i, start.elapsed().as_nanos()));
        match r {
            Ok(h) => background.push(h),
            Err(e) => {
                refusal = Some(format!("background_refusal_index={i} status={e:?}"));
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
    let _ = snapshot(&out, "peak-background").map_err(|e| eprintln!("{e}"));
    // 64 adjacent pairs just above the background.
    let region = base + (n as u64 + 16) * grain;
    let mut pairs: Vec<[u32; 2]> = Vec::new();
    for p in 0..64u64 {
        let mut pair = [0u32; 2];
        for (k, slot) in pair.iter_mut().enumerate() {
            match rm.reserve_va_flags(
                space.space,
                region + (2 * p + k as u64) * grain,
                grain,
                flags,
            ) {
                Ok(h) => *slot = h,
                Err(e) => {
                    refusal.get_or_insert(format!("pair_setup_refusal={p}/{k} status={e:?}"));
                }
            }
        }
        pairs.push(pair);
    }
    let started = Instant::now();
    let (mut done, mut merge_refusals, mut split_refusals) = (0usize, 0usize, 0usize);
    let mut first_merge_refusal = None;
    for c in 0..cycles {
        if started.elapsed() >= Duration::from_secs(90) {
            break;
        }
        let p = c % 64;
        let at = region + 2 * p as u64 * grain;
        if pairs[p].contains(&0) {
            continue;
        }
        for (k, h) in pairs[p].into_iter().enumerate() {
            let start = Instant::now();
            let r = rm.free(h);
            samples.push(("merge_free", c, start.elapsed().as_nanos()));
            if r.is_err() {
                return Err(format!("free {k} failed in cycle {c}"));
            }
        }
        let start = Instant::now();
        let merged = rm.reserve_va_flags(space.space, at, 2 * grain, flags);
        samples.push(("merge_reserve", c, start.elapsed().as_nanos()));
        match merged {
            Ok(m) => {
                let start = Instant::now();
                let r = rm.free(m);
                samples.push(("split_free", c, start.elapsed().as_nanos()));
                if r.is_err() {
                    return Err(format!("merged free failed in cycle {c}"));
                }
            }
            Err(e) => {
                merge_refusals += 1;
                first_merge_refusal.get_or_insert(format!("cycle={c} status={e:?}"));
            }
        }
        for k in 0..2u64 {
            let start = Instant::now();
            let r = rm.reserve_va_flags(space.space, at + k * grain, grain, flags);
            samples.push(("split_reserve", c, start.elapsed().as_nanos()));
            match r {
                Ok(h) => pairs[p][k as usize] = h,
                Err(e) => {
                    split_refusals += 1;
                    pairs[p][k as usize] = 0;
                    first_merge_refusal.get_or_insert(format!("split cycle={c} status={e:?}"));
                }
            }
        }
        done += 1;
        if c.is_multiple_of(256)
            && let Err(e) = probe(&watchdog, &out, "during")
        {
            refusal = Some(e);
            break;
        }
    }
    let _ = snapshot(&out, "peak-churn").map_err(|e| eprintln!("{e}"));
    let mut free_errors = 0usize;
    for h in pairs
        .into_iter()
        .flatten()
        .filter(|&h| h != 0)
        .chain(background)
    {
        let start = Instant::now();
        if rm.free(h).is_err() {
            free_errors += 1;
        }
        samples.push(("final_free", 0, start.elapsed().as_nanos()));
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
        "ENS6_RM_CHURN background={n} cycles_done={done} merge_refusals={merge_refusals} \
         split_refusals={split_refusals} first_refusal={first_merge_refusal:?} \
         setup_refusal={refusal:?} free_errors={free_errors} watchdog_after={watchdog_after:?}"
    );
    if free_errors != 0 {
        return Err("arm stopped; inspect receipts".into());
    }
    watchdog_after
}

/// ENS6 roots arm: the cost of the space-creation reservations (`GUEST_VA_RANGES`, up to
/// 127 TiB) that every twin space makes before the guest runs. 7330 and 7350 both show one
/// single RM call of ~191 ms on the VA thread in their first slow apply; this measures it.
fn roots(args: &[String]) -> Result<(), String> {
    if args.len() != 2 {
        return Err("usage: --run-roots <fresh-output-dir> <watchdog>".into());
    }
    let out = PathBuf::from(&args[0]);
    let watchdog = PathBuf::from(&args[1]);
    guard(&out)?;
    probe(&watchdog, &out, "before")?;
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let flags = kf_host::channel::guest_reserve_flags(false);
    let mut samples: Vec<(&str, usize, u128)> = Vec::new();
    let mut note = Vec::new();
    for round in 0..5usize {
        let start = Instant::now();
        let space = rm
            .alloc_vaspace_bare()
            .map_err(|e| format!("space: {e:?}"))?;
        samples.push(("space_create", round, start.elapsed().as_nanos()));
        let mut handles = Vec::new();
        for (k, &(lo, hi)) in kf_host::channel::GUEST_VA_RANGES.iter().enumerate() {
            let start = Instant::now();
            let r = rm.reserve_va_flags(space.space, lo, hi - lo, flags);
            samples.push((if k == 0 { "root_low" } else { "root_high" }, round, start.elapsed().as_nanos()));
            match r {
                Ok(h) => handles.push(h),
                Err(e) => note.push(format!("round {round} root {k} [{lo:#x},{hi:#x}): {e:?}")),
            }
        }
        for h in handles {
            let start = Instant::now();
            let _ = rm.free(h);
            samples.push(("root_free", round, start.elapsed().as_nanos()));
        }
        let start = Instant::now();
        rm.free_vaspace(space);
        samples.push(("space_free", round, start.elapsed().as_nanos()));
        probe(&watchdog, &out, "during")?;
    }
    let mut csv = std::fs::File::create(out.join("samples.csv")).map_err(|e| e.to_string())?;
    writeln!(csv, "op,index,wall_ns").map_err(|e| e.to_string())?;
    for (op, i, ns) in samples {
        writeln!(csv, "{op},{i},{ns}").map_err(|e| e.to_string())?;
    }
    println!("ENS6_RM_ROOTS refusals={note:?}");
    probe(&watchdog, &out, "after")
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--run-roots") {
        return roots(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("--run-churn") {
        return churn(&args[1..]);
    }
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
