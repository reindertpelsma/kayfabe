//! Bare-host, offline BAR1 read probe. Run under the shared GPU lock, without a VM.
use kf_abi::submit::CpuViewCache;
use kf_host::{CpuViewRelease, HostRm, MapNode, ViewAccess};
use kf_linux_raw::{
    Backing, CachePolicy, DataReadKind, DevDir, HostOffset, HostPageSize, VolatileRegion,
};
use std::{hint::black_box, time::Instant};
const LENGTH: u64 = 64 << 10;
const REPS: usize = 10000;
fn pattern_word(off: u64) -> u64 {
    off ^ 0x1020_3040_5060_7080
}
fn expected(off: usize, n: usize) -> Vec<u8> {
    (off..off + n)
        .map(|at| pattern_word(at as u64 / 8 * 8).to_le_bytes()[at % 8])
        .collect()
}
fn measure(
    region: &VolatileRegion,
    bytes: usize,
    kind: Option<DataReadKind>,
    cache: CpuViewCache,
    off: usize,
) -> Result<(), String> {
    let want = expected(off, bytes);
    let mut out = vec![0; bytes];
    let mut times = Vec::with_capacity(REPS);
    // Exact comparison outside each timed access; every returned byte is checked.
    for _ in 0..REPS {
        let start = Instant::now();
        let result = match kind {
            None => region.copy_out(HostOffset::new(off as u64), black_box(&mut out)),
            Some(k) => region.bench_copy_data(HostOffset::new(off as u64), black_box(&mut out), k),
        };
        let ns = start.elapsed().as_nanos() as u64;
        if let Err(e) = result {
            println!(
                "READ_COST_REFUSED cache={cache:?} kind={kind:?} bytes={bytes} offset={off} why={e:?}"
            );
            return Ok(());
        }
        assert_eq!(out, want, "cache={cache:?} kind={kind:?} offset={off}");
        times.push(ns);
        black_box(&out);
    }
    times.sort_unstable();
    println!(
        "READ_COST cache={cache:?} kind={kind:?} bytes={bytes} offset={off} p50_ns={} p99_ns={} max_ns={} ns_per_byte={:.3}",
        times[REPS / 2],
        times[REPS * 99 / 100],
        times[REPS - 1],
        times[REPS / 2] as f64 / bytes as f64
    );
    Ok(())
}
fn main() -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("{e:?}"))?;
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let memory = rm
        .alloc_device_local(LENGTH)
        .map_err(|e| format!("{e:?}"))?;
    println!(
        "READ_COST_START driver={} iterations={REPS} bytes={LENGTH} pid={} uid={}",
        rm.driver_version(),
        std::process::id(),
        std::env::var("USER").unwrap_or_default()
    );
    for cache in [
        CpuViewCache::Legacy,
        CpuViewCache::Uncached,
        CpuViewCache::WriteCombining,
    ] {
        let (node, cookie) = match rm.arm_cpu_view_bench(
            MapNode::Gpu,
            memory,
            0,
            LENGTH,
            ViewAccess::ReadWrite,
            cache,
        ) {
            Ok(v) => v,
            Err(e) => {
                println!("READ_COST_MAP_REFUSED cache={cache:?} why={e:?}");
                continue;
            }
        };
        let result: Result<(), String> = (|| {
            let policy = if matches!(cache, CpuViewCache::WriteCombining) {
                CachePolicy::WriteCombining
            } else {
                CachePolicy::Uncached
            };
            let region = VolatileRegion::map(
                Backing::DeviceFile { fd: node.as_fd() },
                LENGTH,
                policy,
                HostPageSize::query(),
            )
            .map_err(|e| format!("{e:?}"))?;
            for off in (0..LENGTH).step_by(8) {
                region
                    .store_u64(HostOffset::new(off), pattern_word(off))
                    .map_err(|e| format!("{e:?}"))?;
            }
            kf_linux_raw::release_fence();
            println!(
                "READ_COST_MAP_LIVE cache={cache:?} flags={:#x}; requested cache hint, effective PAT requires observer",
                cache.flags()
            );
            for bytes in [64usize, 256, 416, 4096] {
                let offsets = [0usize, 16384, 32768, 49152];
                let want: Vec<u8> = offsets.iter().flat_map(|o| expected(*o, bytes)).collect();
                let mut out = vec![0; bytes * 4];
                let mut times = Vec::with_capacity(REPS);
                for _ in 0..REPS {
                    let start = Instant::now();
                    region
                        .bench_copy_four(
                            offsets.map(|o| HostOffset::new(o as u64)),
                            black_box(&mut out),
                        )
                        .map_err(|e| format!("{e:?}"))?;
                    times.push(start.elapsed().as_nanos() as u64);
                    assert_eq!(out, want);
                }
                times.sort_unstable();
                println!(
                    "READ_COST_FOUR cache={cache:?} bytes_per_stream={bytes} total_bytes={} p50_batch_ns={} p99_batch_ns={} max_batch_ns={} ns_per_byte={:.3}",
                    bytes * 4,
                    times[REPS / 2],
                    times[REPS * 99 / 100],
                    times[REPS - 1],
                    times[REPS / 2] as f64 / (bytes * 4) as f64
                );

                for kind in [
                    None,
                    Some(DataReadKind::Scalar8),
                    Some(DataReadKind::Simd16),
                    Some(DataReadKind::Simd32),
                    Some(DataReadKind::Inflight16),
                    Some(DataReadKind::Stream16),
                ] {
                    measure(&region, bytes, kind, cache, 0)?;
                    if !matches!(kind, Some(DataReadKind::Stream16)) {
                        measure(&region, bytes, kind, cache, 3)?;
                    }
                }
            }
            Ok(())
        })();
        rm.release_cpu_view(CpuViewRelease {
            h_memory: memory,
            p_linear_address: cookie,
        })
        .map_err(|e| format!("release: {e:?}"))?;
        drop(node);
        result?;
    }
    rm.free(memory).map_err(|e| format!("free: {e:?}"))?;
    println!("READ_COST_EXIT=0");
    Ok(())
}
