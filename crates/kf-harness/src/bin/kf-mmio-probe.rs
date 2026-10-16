//! MEASUREMENT ONLY (perf lifecycle campaign S2, 2026-10-16, scratch, never for merge).
//!
//! Reads a VRAM object of this process through the SAME path the USER-ring relay uses for its store
//! views: `HostRm::arm_cpu_view(MapNode::Gpu, .., ReadOnly)` -> `VolatileRegion::map_read_only(.., CachePolicy::Uncached, ..)`
//! -> `VolatileRegion::copy_out` (a `copy_nonoverlapping`). Per-call latency, ns/byte and MB/s for
//! several read sizes, one or more threads (each with its OWN view, as each kayfabe worker has its own
//! cache). Pin with `taskset`.
//!
//! usage: `kf-mmio-probe [--mib N] [--threads T] [--secs S] [--sizes 8,64,256,1024,4096] [--mode rand|seq]`

use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_string())
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn pat_lines() {
    // The BAR1 physical window of the GPU, and the PAT type the kernel recorded for ranges inside it.
    let res = std::fs::read_to_string("/sys/bus/pci/devices/0000:01:00.0/resource").unwrap_or_default();
    let bar1 = res.lines().nth(1).unwrap_or("").to_string();
    println!("PROBE bar1_resource_line={bar1}");
    let pat = std::fs::read_to_string("/sys/kernel/debug/x86/pat_memtype_list").unwrap_or_default();
    let mut shown = 0;
    let start = bar1
        .split_whitespace()
        .next()
        .and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    for l in pat.lines() {
        // "PAT: [mem 0x0000000c00000000-0x0000000c00200000] uncached-minus"
        if let Some(a) = l.split("0x").nth(1).and_then(|x| u64::from_str_radix(&x[..16.min(x.len())], 16).ok())
            && start != 0
            && a >= start
            && a < start + (1 << 40)
            && shown < 12
        {
            println!("PROBE pat {l}");
            shown += 1;
        }
    }
    if pat.is_empty() {
        println!("PROBE pat unreadable (debugfs)");
    }
}

fn percentile(v: &[u32], p: f64) -> u32 {
    if v.is_empty() {
        return 0;
    }
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i.min(v.len() - 1)]
}

fn main() -> Result<(), String> {
    let mib: u64 = arg("--mib", "64").parse().map_err(|_| "mib")?;
    let threads: usize = arg("--threads", "1").parse().map_err(|_| "threads")?;
    let secs: f64 = arg("--secs", "2").parse().map_err(|_| "secs")?;
    let mode = arg("--mode", "rand");
    let read_kind: u8 = arg("--read", "0").parse().map_err(|_| "read")?;
    let sizes: Vec<usize> = arg("--sizes", "8,64,256,1024,4096")
        .split(',')
        .filter_map(|x| x.parse().ok())
        .collect();
    let len = mib << 20;
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    println!("PROBE driver={} mib={mib} threads={threads} mode={mode} secs={secs} read_kind={read_kind} (0 memcpy, 1 movntdqa, 2 avx2)", rm.driver_version());
    let h = rm.alloc_device_local(len).map_err(|e| format!("alloc: {e:?}"))?;
    let mut views = Vec::new();
    for _ in 0..threads {
        let (node, _cookie) = rm
            .arm_cpu_view(MapNode::Gpu, h, 0, len, ViewAccess::ReadOnly)
            .map_err(|e| format!("arm: {e:?}"))?;
        let cpu = VolatileRegion::map_read_only(
            Backing::DeviceFile { fd: node.as_fd() },
            len,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("map: {e:?}"))?;
        views.push((node, cpu));
    }
    pat_lines();
    for &size in &sizes {
        let barrier = std::sync::Barrier::new(threads);
        let results: Vec<(Vec<u32>, u64, f64)> = std::thread::scope(|sc| {
            let hs: Vec<_> = views
                .iter()
                .enumerate()
                .map(|(t, (_n, cpu))| {
                    let barrier = &barrier;
                    let mode = mode.as_str();
                    let read_kind = read_kind;
                    sc.spawn(move || {
                        let mut buf = vec![0u8; size];
                        let mut lat: Vec<u32> = Vec::with_capacity(1 << 20);
                        let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ ((t as u64 + 1) * 0x1234_5678_9ABC);
                        let slots = (len / size.max(8) as u64).max(1);
                        let mut seq = 0u64;
                        // warm-up: touch each page class once
                        for _ in 0..64 {
                            let o = (xorshift(&mut rng) % slots) * size.max(8) as u64;
                            let _ = cpu.copy_out_wide(HostOffset::new(o), &mut buf, read_kind);
                        }
                        barrier.wait();
                        let t0 = Instant::now();
                        let mut bytes = 0u64;
                        while t0.elapsed() < Duration::from_secs_f64(secs) {
                            for _ in 0..64 {
                                let slot = if mode == "seq" {
                                    seq = (seq + 1) % slots;
                                    seq
                                } else {
                                    xorshift(&mut rng) % slots
                                };
                                let o = slot * size.max(8) as u64;
                                let c0 = Instant::now();
                                let _ = cpu.copy_out_wide(HostOffset::new(o), &mut buf, read_kind);
                                let ns = c0.elapsed().as_nanos() as u64;
                                bytes += size as u64;
                                if lat.len() < (1 << 22) {
                                    lat.push(ns.min(u64::from(u32::MAX)) as u32);
                                }
                            }
                        }
                        let el = t0.elapsed().as_secs_f64();
                        (lat, bytes, el)
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut all: Vec<u32> = results.iter().flat_map(|r| r.0.iter().copied()).collect();
        all.sort_unstable();
        let total_bytes: u64 = results.iter().map(|r| r.1).sum();
        let el = results.iter().map(|r| r.2).fold(0.0, f64::max);
        let per_thread: Vec<String> = results
            .iter()
            .map(|r| format!("{:.2}", r.1 as f64 / r.2 / 1e6))
            .collect();
        let sum_ns: u64 = all.iter().map(|x| u64::from(*x)).sum();
        println!(
            "RESULT read={read_kind} mode={mode} threads={threads} size={size} calls={} mean_ns={:.0} p50_ns={} p90_ns={} p99_ns={} max_ns={} ns_per_byte_mean={:.2} agg_MBps={:.2} per_thread_MBps=[{}]",
            all.len(),
            sum_ns as f64 / all.len().max(1) as f64,
            percentile(&all, 0.5),
            percentile(&all, 0.9),
            percentile(&all, 0.99),
            all.last().copied().unwrap_or(0),
            sum_ns as f64 / (all.len().max(1) as f64 * size as f64),
            total_bytes as f64 / el / 1e6,
            per_thread.join(",")
        );
    }
    Ok(())
}
