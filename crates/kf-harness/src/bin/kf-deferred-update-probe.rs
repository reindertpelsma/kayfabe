//! Native authored deferred channel-update probe, public ctrl5080 V2 path.
//! No guest bytes. Same-config update tests trigger permissions/completion only;
//! this is not proof of USERD migration or a production passthrough fix.
use kf_abi::submit::{USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_host::{HostRm, MapBacking, MapNode, RingSpec, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const USERD: u64 = 0x2000;
const FENCE: u64 = 0x3010;
const PB: u64 = 0x4000;

fn main() {
    if let Err(e) = run() {
        eprintln!("DEFERRED_UPDATE_PROBE=FAIL {e}");
        std::process::exit(1);
    }
    println!("DEFERRED_UPDATE_PROBE=PASS");
}

fn method(out: &mut Vec<u32>, sub: u32, at: u32, args: &[u32]) -> Result<(), String> {
    out.push(method_header_inc(sub, at, args.len() as u32).ok_or("method header")?);
    out.extend_from_slice(args);
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 || !matches!(args[0].as_str(), "nop" | "update") {
        return Err("usage: kf-deferred-update-probe nop|update".into());
    }
    let trigger = args[0] == "update";
    let engine = kf_abi::submit::ENGINE_TYPE_GRAPHICS;
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    println!(
        "DEFERRED_UPDATE driver={} mode={}",
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
            .alloc_deferred_api(chan)
            .map_err(|e| format!("5080 alloc: {e:?}"))?;
        // Ask the public channel control which software classID names our object.
        let mut query = [0u8; 16];
        query[..4].copy_from_slice(&object.to_le_bytes());
        rm.raw_control(
            chan.chan,
            kf_host::channel::NV906F_CTRL_GET_CLASS_ENGINEID,
            &mut query,
        )
        .map_err(|e| format!("class query: {e:?}"))?;
        let word = |i| u32::from_le_bytes(query[i..i + 4].try_into().expect("word"));
        if word(8) != kf_abi::generated::classes::NV50_DEFERRED_API_CLASS {
            return Err("class query mismatched object".into());
        }
        let class_id = word(4) & 0xffff;
        let api = rm.mint();
        rm.probe_deferred_channel_update(
            chan,
            object,
            api,
            RingSpec {
                gp_fifo_va: va + GP,
                gp_fifo_entries: 32,
                userd_memory: mem,
                userd_offset: USERD,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("V2 registration: {e:?}"))?;
        println!(
            "DEFERRED_UPDATE registered channel={chan:x?} object={object:#x} api={api:#x} sw_class_id={class_id:#x} privilege=USER"
        );
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        let mut words = Vec::new();
        method(&mut words, 5, 0, &[class_id])?;
        // Both invocations register the same owned configuration. Only the
        // trigger differs: 5080 NO_OPERATION vs DEFERRED_API_V2 (deferred_api.c).
        method(
            &mut words,
            5,
            if trigger { 0x200 } else { 0x100 },
            &[if trigger { api } else { 0 }],
        )?;
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
            let fence = cpu
                .load_u32(HostOffset::new(FENCE))
                .map_err(|e| format!("fence: {e:?}"))?;
            if fence == 1 {
                println!(
                    "DEFERRED_UPDATE GPU_TAIL fence={fence} trigger={trigger} elapsed_us={}",
                    start.elapsed().as_micros()
                );
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                return Err(format!(
                    "bench deadline fence={fence}; no completion invented"
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
        "DEFERRED_UPDATE_CLEANUP channel={freed:?} view={view:?} unmap={unmap:?} memory={memory:?}"
    );
    result
        .and(freed.map(|_| ()))
        .and(view.map_err(|e| format!("view release: {e:?}")))
        .and(unmap.map_err(|e| format!("unmap: {e:?}")))
        .and(memory.map_err(|e| format!("free memory: {e:?}")))
}
