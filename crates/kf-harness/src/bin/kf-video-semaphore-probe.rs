//! Native, authored NVENC semaphore oracle; no guest bytes and no codec work.
//! Public source: NVIDIA/open-gpu-doc classes/video/clc9b7.h, SEMAPHORE_A..D.
//! Release and trap are separate invocations so the latter changes one variable.
//! This deliberately uses an owned raw bench ring, not HostRing's production
//! submission API: its existing rejection of translated video work is unchanged.
use kf_abi::submit::{USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_host::{HostRm, MapBacking, MapNode, RingSpec, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

// Public clc9b7.h class selector, bench-only; product class sets stay generated.
const NVC9B7_VIDEO_ENCODER: u32 = 0xc9b7;
const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const USERD: u64 = 0x2000;
const SEM: u64 = 0x3000;
const FENCE: u64 = 0x3010;
const PB: u64 = 0x4000;
const PAYLOAD: u32 = 0x61fe_0001;

fn main() {
    if let Err(e) = run() {
        eprintln!("VIDEO_SEMAPHORE_PROBE=FAIL {e}");
        std::process::exit(1);
    }
    println!("VIDEO_SEMAPHORE_PROBE=PASS");
}

fn method(out: &mut Vec<u32>, sub: u32, at: u32, args: &[u32]) -> Result<(), String> {
    out.push(method_header_inc(sub, at, args.len() as u32).ok_or("method header")?);
    out.extend_from_slice(args);
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || !matches!(args[0].as_str(), "release" | "trap") {
        return Err("usage: kf-video-semaphore-probe release|trap NVENC_INDEX".into());
    }
    let index: u32 = args[1].parse().map_err(|_| "invalid NVENC index")?;
    let engine = kf_abi::submit::engine_type_nvenc(index).ok_or("NVENC index outside0..3")?;
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    // This experiment implements only the named public C9B7 method contract.
    if !rm.supported_class_ids().contains(&NVC9B7_VIDEO_ENCODER) {
        return Err("unmeasured video class: public C9B7 oracle does not apply".into());
    }
    println!(
        "VIDEO_SEMAPHORE driver={} mode={} index={index} class={NVC9B7_VIDEO_ENCODER:#x}",
        rm.driver_version(),
        args[0]
    );
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let mem = rm
        .alloc_device_local(BYTES)
        .map_err(|e| format!("memory: {e:?}"))?;
    let va = rm
        .map(space, mem, MapBacking::Dedicated, 0, BYTES, None, false)
        .map_err(|e| format!("map: {e:?}"))?;
    if va.checked_add(BYTES).is_none_or(|end| end > 1 << 40) {
        return Err("owned allocation exceeds40-bit semaphore/GP address width".into());
    }
    let (node, cookie) = rm
        .arm_cpu_view(MapNode::Gpu, mem, 0, BYTES, ViewAccess::ReadWrite)
        .map_err(|e| format!("CPU view: {e:?}"))?;
    let cpu = VolatileRegion::map(
        Backing::DeviceFile { fd: node.as_fd() },
        BYTES,
        CachePolicy::Uncached,
        HostPageSize::query(),
    )
    .map_err(|e| format!("mmap: {e:?}"))?;
    let mut channel = None;
    let result = (|| {
        for off in (0..BYTES).step_by(4) {
            cpu.store_u32(HostOffset::new(off), 0)
                .map_err(|e| format!("zero: {e:?}"))?;
        }
        let chan = rm
            .birth_channel(
                space,
                engine,
                RingSpec {
                    gp_fifo_va: va + GP,
                    gp_fifo_entries: 32,
                    userd_memory: mem,
                    userd_offset: USERD,
                    err_notifier: 0,
                },
            )
            .map_err(|e| format!("birth: {e:?}"))?;
        channel = Some(chan);
        chan.assert_user().map_err(str::to_string)?;
        let object = rm
            .alloc_video_object(chan, NVC9B7_VIDEO_ENCODER, index)
            .map_err(|e| format!("video object: {e:?}"))?;
        println!("VIDEO_SEMAPHORE channel={chan:x?} object={object:#x} privilege=USER");
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        let mut words = Vec::new();
        method(&mut words, 4, 0, &[NVC9B7_VIDEO_ENCODER])?;
        // C9B7_SEMAPHORE_A/B/C: authored owned40-bit address and own payload.
        method(
            &mut words,
            4,
            0x240,
            &[((va + SEM) >> 32) as u32, (va + SEM) as u32, PAYLOAD],
        )?;
        method(&mut words, 4, 0x304, &[0])?; // STRUCTURE_SIZE_ONE, OPERATION_RELEASE.
        if args[0] == "trap" {
            method(&mut words, 4, 0x304, &[3 << 16])?; // OPERATION_TRAP, same owned address.
        }
        // Actual GPU completion tail; the CPU initializes but never completes it.
        method(
            &mut words,
            0,
            fifo::SEM_ADDR_LO,
            &[(va + FENCE) as u32, ((va + FENCE) >> 32) as u32, 1, 0],
        )?;
        method(
            &mut words,
            0,
            fifo::SEM_EXECUTE,
            &[fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN],
        )?;
        let gp = gp_entry(va + PB, 4 * words.len() as u64).ok_or("GP entry")?;
        for (i, w) in words.iter().enumerate() {
            cpu.store_u32(HostOffset::new(PB + 4 * i as u64), *w)
                .map_err(|e| format!("PB: {e:?}"))?;
        }
        cpu.store_u32(HostOffset::new(GP), gp as u32)
            .map_err(|e| format!("GPlo: {e:?}"))?;
        cpu.store_u32(HostOffset::new(GP + 4), (gp >> 32) as u32)
            .map_err(|e| format!("GPhi: {e:?}"))?;
        kf_linux_raw::release_fence();
        cpu.store_u32(HostOffset::new(USERD + USERD_GP_PUT), 1)
            .map_err(|e| format!("PUT: {e:?}"))?;
        kf_linux_raw::release_fence();
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        loop {
            let sem = cpu
                .load_u32(HostOffset::new(SEM))
                .map_err(|e| format!("sem: {e:?}"))?;
            let fence = cpu
                .load_u32(HostOffset::new(FENCE))
                .map_err(|e| format!("fence: {e:?}"))?;
            if fence == 1 {
                println!(
                    "VIDEO_SEMAPHORE sem={sem:#x} fence={fence} elapsed_us={}",
                    start.elapsed().as_micros()
                );
                if sem != PAYLOAD {
                    return Err(
                        "GPU tail completed without requested video semaphore payload".into(),
                    );
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                return Err(format!(
                    "bench deadline sem={sem:#x} fence={fence}; no completion invented"
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    })();
    // Quiesce/free the owned channel before revoking its memory, even on a failed run.
    let freed = channel
        .map(|c| rm.free_channel(c))
        .transpose()
        .map_err(|e| format!("free channel: {e:?}"));
    drop(cpu);
    let view = rm.release_cpu_view(kf_host::CpuViewRelease {
        h_memory: mem,
        p_linear_address: cookie,
    });
    let unmap = rm.unmap(space, va, false);
    let memory = rm.free(mem);
    rm.free_vaspace(space);
    println!(
        "VIDEO_SEMAPHORE_CLEANUP channel={freed:?} view={view:?} unmap={unmap:?} memory={memory:?}"
    );
    result
        .and(freed.map(|_| ()))
        .and(view.map_err(|e| format!("view release: {e:?}")))
        .and(unmap.map_err(|e| format!("unmap: {e:?}")))
        .and(memory.map_err(|e| format!("free memory: {e:?}")))
}
