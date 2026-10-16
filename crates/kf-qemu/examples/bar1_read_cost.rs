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
fn verify_freshness(region: &VolatileRegion, cache: CpuViewCache) -> Result<(), String> {
    for turn in 0..128u64 {
        let mut want = Vec::new();
        for off in (0..64u64).step_by(8) {
            let value = pattern_word(off) ^ turn.wrapping_mul(0x0101_0101_0101_0101);
            region
                .store_u64(HostOffset::new(off), value)
                .map_err(|e| format!("{e:?}"))?;
            want.extend(value.to_le_bytes());
        }
        region.bench_data_fence();
        for kind in [
            DataReadKind::Scalar8,
            DataReadKind::Simd16,
            DataReadKind::Simd32,
            DataReadKind::Inflight16,
            DataReadKind::Stream16,
        ] {
            let mut out = [0u8; 64];
            match region.bench_copy_data(HostOffset::new(0), &mut out, kind) {
                Ok(()) => assert_eq!(
                    out.as_slice(),
                    want,
                    "freshness cache={cache:?} kind={kind:?} turn={turn}"
                ),
                Err(_)
                    if matches!(kind, DataReadKind::Stream16)
                        && !matches!(cache, CpuViewCache::WriteCombining) =>
                {
                    ()
                }
                Err(e) => return Err(format!("freshness {kind:?}: {e:?}")),
            }
        }
    }
    println!(
        "READ_COST_FRESHNESS cache={cache:?} turns=128 writer=CPU result=PASS GPU_writer=NOT_TESTED"
    );
    Ok(())
}
/// Existing USER copy-engine ring, with one private allocation mapped in its own
/// VA space. Only its real GPU fence authorizes reading the changed destination.
fn verify_gpu_freshness(
    rm: &HostRm,
    memory: u32,
    region: &VolatileRegion,
    cache: CpuViewCache,
) -> Result<(), String> {
    use kf_abi::submit::{SET_OBJECT, ce, method_header_inc};
    let space = rm
        .alloc_vaspace()
        .map_err(|e| format!("GPU space: {e:?}"))?;
    let va = match rm.map(
        space,
        memory,
        kf_host::MapBacking::Dedicated,
        0,
        LENGTH,
        None,
        false,
    ) {
        Ok(va) => va,
        Err(e) => {
            rm.free_vaspace(space);
            return Err(format!("GPU map: {e:?}"));
        }
    };
    let mut ring = match kf_chan::host::HostRing::new(rm, space) {
        Ok(ring) => ring,
        Err(e) => {
            let _ = rm.unmap(space, va, false);
            rm.free_vaspace(space);
            return Err(e);
        }
    };
    let result: Result<(), String> = (|| {
        ring.channel().assert_user().map_err(str::to_owned)?;
        for bytes in [64usize, 256, 416, 4096] {
            for turn in 0..32u64 {
                let n = bytes + 8;
                let source = 32768u64;
                assert!(source + n as u64 <= LENGTH && n as u64 <= source);
                let mut want = Vec::with_capacity(n);
                for at in (0..n).step_by(8) {
                    let value =
                        pattern_word(at as u64) ^ (turn + 1).wrapping_mul(0x0303_0303_0303_0303);
                    region
                        .store_u64(HostOffset::new(source + at as u64), value)
                        .map_err(|e| format!("{e:?}"))?;
                    want.extend(value.to_le_bytes());
                }
                region.bench_data_fence();
                // ctrl2080fb.h (595.84): aperture 0 is VIDEO_MEMORY. This
                // correctness control invalidates cached GPU source data after
                // CPU stores; these RM calls are outside all timed read cases.
                rm.fb_flush()
                    .map_err(|e| format!("GPU source sysmembar: {e:?}"))?;
                rm.flush_gpu_cache(0, false, true)
                    .map_err(|e| format!("GPU source invalidate: {e:?}"))?;
                let src = va.checked_add(source).ok_or("GPU source wraps")?;
                let h = |method, count| method_header_inc(0, method, count).ok_or("GPU header");
                let words = [
                    h(SET_OBJECT, 1)?,
                    rm.ce_class_id(),
                    h(ce::OFFSET_IN_UPPER, 4)?,
                    (src >> 32) as u32,
                    src as u32,
                    (va >> 32) as u32,
                    va as u32,
                    h(ce::LINE_LENGTH_IN, 2)?,
                    n as u32,
                    1,
                    h(ce::LAUNCH_DMA, 1)?,
                    ce::LAUNCH_TRANSFER_NON_PIPELINED
                        | ce::LAUNCH_FLUSH_ENABLE
                        | ce::LAUNCH_SRC_PITCH
                        | ce::LAUNCH_DST_PITCH
                        | ce::LAUNCH_SRC_VIRTUAL
                        | ce::LAUNCH_DST_VIRTUAL,
                ];
                ring.push(&words)?.map_err(|_| "GPU copy ring busy")?;
                let seq = ring.fence(rm)?.map_err(|_| "GPU fence ring busy")?;
                let deadline = Instant::now() + std::time::Duration::from_secs(2);
                while ring.completed()? != seq {
                    if Instant::now() >= deadline {
                        return Err("GPU freshness fence timeout".into());
                    }
                    std::thread::sleep(std::time::Duration::from_micros(10));
                }
                for kind in [
                    None,
                    Some(DataReadKind::Scalar8),
                    Some(DataReadKind::Simd16),
                    Some(DataReadKind::Simd32),
                    Some(DataReadKind::Inflight16),
                    Some(DataReadKind::Stream16),
                ] {
                    if matches!(kind, Some(DataReadKind::Stream16))
                        && !matches!(cache, CpuViewCache::WriteCombining)
                    {
                        continue;
                    }
                    for off in [0usize, 3] {
                        if off != 0 && matches!(kind, Some(DataReadKind::Stream16)) {
                            continue;
                        }
                        let mut out = vec![0; bytes];
                        match kind {
                            None => region.copy_out(HostOffset::new(off as u64), &mut out),
                            Some(kind) => {
                                region.bench_copy_data(HostOffset::new(off as u64), &mut out, kind)
                            }
                        }
                        .map_err(|e| format!("GPU freshness {kind:?}: {e:?}"))?;
                        if out != want[off..off + bytes] {
                            return Err(format!(
                                "GPU freshness mismatch cache={cache:?} kind={kind:?} bytes={bytes} offset={off} turn={turn}"
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    rm.free_channel(ring.channel())
        .map_err(|e| format!("GPU channel cleanup: {e:?}"))?;
    if ring.release(rm).is_some_and(|s| s.contains("REFUSED")) {
        return Err("GPU ring cleanup refused".into());
    }
    rm.unmap(space, va, false)
        .map_err(|e| format!("GPU unmap: {e:?}"))?;
    rm.free_vaspace(space);
    result?;
    println!(
        "READ_COST_GPU_FRESHNESS cache={cache:?} turns_per_size=32 sizes=64,256,416,4096 result=PASS fence=GPU_RELEASE_WFI"
    );
    Ok(())
}
fn main() -> Result<(), String> {
    let gpu_freshness = std::env::args().any(|a| a == "--gpu-freshness");
    let dev = DevDir::open(c"/dev").map_err(|e| format!("{e:?}"))?;
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let memory = rm
        .alloc_device_local(LENGTH)
        .map_err(|e| format!("{e:?}"))?;
    println!(
        "READ_COST_START driver={} iterations={REPS} bytes={LENGTH} pid={} uid={} gpu_freshness={gpu_freshness}",
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
            region.bench_data_fence();
            verify_freshness(&region, cache)?;
            if gpu_freshness {
                verify_gpu_freshness(&rm, memory, &region, cache)?;
            }
            for off in (0..LENGTH).step_by(8) {
                region
                    .store_u64(HostOffset::new(off), pattern_word(off))
                    .map_err(|e| format!("{e:?}"))?;
            }
            region.bench_data_fence();
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
