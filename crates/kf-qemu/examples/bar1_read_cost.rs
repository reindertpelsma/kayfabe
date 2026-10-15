//! Bare-host read-cost probe. Run only under the shared GPU lock, without a VM.
//! Initial version establishes the actual copy_out and aligned-u64 baselines.
use kf_host::{CpuViewRelease, HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::{hint::black_box, time::Instant};
fn main() -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("{e:?}"))?;
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let memory = rm
        .alloc_device_local(64 << 10)
        .map_err(|e| format!("{e:?}"))?;
    let (node, cookie) = rm
        .arm_cpu_view(MapNode::Gpu, memory, 0, 64 << 10, ViewAccess::ReadWrite)
        .map_err(|e| format!("{e:?}"))?;
    let result = (|| {
        let region = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            64 << 10,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("{e:?}"))?;
        for off in (0..(64 << 10)).step_by(8) {
            region
                .store_u64(HostOffset::new(off), off ^ 0x1020_3040_5060_7080)
                .map_err(|e| format!("{e:?}"))?;
        }
        kf_linux_raw::release_fence();
        println!(
            "READ_COST_START driver={} mapping=requested-uncached actual-driver-selected iterations=10000",
            rm.driver_version()
        );
        for bytes in [64usize, 256, 416, 4096] {
            let mut out = vec![0; bytes];
            let mut times = Vec::new();
            for _ in 0..10000 {
                let start = Instant::now();
                region
                    .copy_out(HostOffset::new(0), black_box(&mut out))
                    .map_err(|e| format!("{e:?}"))?;
                times.push(start.elapsed().as_nanos() as u64);
                black_box(&out);
            }
            times.sort_unstable();
            println!(
                "READ_COST method=current-copy bytes={bytes} p50_ns={} p99_ns={} max_ns={} ns_per_byte={:.3}",
                times[5000],
                times[9900],
                times[9999],
                times[5000] as f64 / bytes as f64
            );
            times.clear();
            for _ in 0..10000 {
                let start = Instant::now();
                for off in (0..bytes).step_by(8) {
                    let v = region
                        .load_u64(HostOffset::new(off as u64))
                        .map_err(|e| format!("{e:?}"))?;
                    out[off..off + 8].copy_from_slice(&v.to_le_bytes());
                }
                times.push(start.elapsed().as_nanos() as u64);
                black_box(&out);
            }
            times.sort_unstable();
            println!(
                "READ_COST method=aligned-u64 bytes={bytes} p50_ns={} p99_ns={} max_ns={} ns_per_byte={:.3}",
                times[5000],
                times[9900],
                times[9999],
                times[5000] as f64 / bytes as f64
            );
        }
        println!(
            "READ_COST_PENDING wider-16 wider-32 independent-inflight WC-MOVNTDQA; no winner claimed"
        );
        Ok(())
    })();
    rm.release_cpu_view(CpuViewRelease {
        h_memory: memory,
        p_linear_address: cookie,
    })
    .map_err(|e| format!("release: {e:?}"))?;
    rm.free(memory).map_err(|e| format!("free: {e:?}"))?;
    result
}
