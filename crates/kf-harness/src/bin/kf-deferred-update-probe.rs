//! Native authored deferred channel-update probe, public ctrl5080 V2 path.
//! No guest bytes. Same-config update tests trigger permissions/completion only;
//! this is not proof of USERD migration or a production passthrough fix.
use kf_abi::submit::{fifo, gp_entry, method_header_inc, USERD_GP_PUT};
use kf_host::{HostRm, MapBacking, MapNode, RingSpec, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const USERD: u64 = 0x2000;
const FENCE: u64 = 0x3010;
const PB: u64 = 0x4000;
// Bench-owned disjoint spans in the same live, mapped 64 KiB object.
const NEW_GP: u64 = 0x5000;
const NEW_USERD: u64 = 0x6000;
const NEW_PB: u64 = 0x7000;
const NEW_FENCE: u64 = 0x8010;

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
    if args.len() != 1
        || !matches!(
            args[0].as_str(),
            "nop" | "update" | "update-hold" | "migrate" | "migrate-nop"
        )
    {
        return Err(
            "usage: kf-deferred-update-probe nop|update|update-hold|migrate|migrate-nop".into(),
        );
    }
    let trigger = !matches!(args[0].as_str(), "nop" | "migrate-nop");
    let migration = matches!(args[0].as_str(), "migrate" | "migrate-nop");
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
        let caller = std::fs::read_to_string("/proc/self/status")
            .map_err(|e| format!("registration caller status: {e}"))?;
        println!(
            "DEFERRED_UPDATE_REGISTRATION_CALLER {}",
            caller
                .lines()
                .filter(|line| line.starts_with("Uid:") || line.starts_with("CapEff:"))
                .collect::<Vec<_>>()
                .join("; ")
        );
        rm.probe_deferred_channel_update(
            chan,
            object,
            api,
            RingSpec {
                gp_fifo_va: va + if migration { NEW_GP } else { GP },
                gp_fifo_entries: if migration { 64 } else { 32 },
                userd_memory: mem,
                userd_offset: if migration { NEW_USERD } else { USERD },
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("V2 registration: {e:?}"))?;
        if migration {
            // Only the replacement ring references this independent GPU-written
            // marker. Two harmless identical entries accommodate initial GET0
            // or continuity GET1 without pretending either is established.
            let mut replacement = Vec::new();
            method(
                &mut replacement,
                0,
                fifo::SEM_ADDR_LO,
                &[
                    (va + NEW_FENCE) as u32,
                    ((va + NEW_FENCE) >> 32) as u32,
                    2,
                    0,
                ],
            )?;
            method(
                &mut replacement,
                0,
                fifo::SEM_EXECUTE,
                &[fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN],
            )?;
            let replacement_gp =
                gp_entry(va + NEW_PB, 4 * replacement.len() as u64).ok_or("replacement GP")?;
            for (i, word) in replacement.iter().enumerate() {
                cpu.store_u32(HostOffset::new(NEW_PB + 4 * i as u64), *word)
                    .map_err(|e| format!("replacement PB: {e:?}"))?;
            }
            for slot in 0..2 {
                cpu.store_u32(HostOffset::new(NEW_GP + slot * 8), replacement_gp as u32)
                    .map_err(|e| format!("replacement GPlo: {e:?}"))?;
                cpu.store_u32(
                    HostOffset::new(NEW_GP + slot * 8 + 4),
                    (replacement_gp >> 32) as u32,
                )
                .map_err(|e| format!("replacement GPhi: {e:?}"))?;
            }
            cpu.store_u32(HostOffset::new(NEW_USERD + USERD_GP_PUT), 2)
                .map_err(|e| format!("replacement PUT: {e:?}"))?;
            println!("DEFERRED_MIGRATION_PREPARED memory={mem:#x} mapped_va={va:#x} bytes={BYTES} old_gp={GP:#x}/32 old_userd={USERD:#x} new_gp={NEW_GP:#x}/64 new_userd={NEW_USERD:#x} replacement_put=2; registration OK; distinct marker only in replacement PB; no CPU completion");
        }
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
        if migration {
            // Matched, fixed bench stimulus in BOTH cases: registration-only
            // cannot masquerade as trigger execution plus an extra doorbell.
            for _ in 0..128 {
                rm.doorbell(chan.token)
                    .map_err(|e| format!("replacement doorbell: {e:?}"))?;
                std::thread::sleep(Duration::from_millis(1));
            }
            println!("DEFERRED_MIGRATION_MATCHED_STIMULUS post_tail_doorbells=128 interval_ms=1 trigger={trigger}");
        }
        if migration && trigger {
            loop {
                let marker = cpu
                    .load_u32(HostOffset::new(NEW_FENCE))
                    .map_err(|e| format!("replacement marker: {e:?}"))?;
                let get = cpu
                    .load_u32(HostOffset::new(NEW_USERD + kf_abi::submit::USERD_GP_GET))
                    .map_err(|e| format!("replacement GET: {e:?}"))?;
                if marker == 2 && get == 2 {
                    println!("DEFERRED_MIGRATION_NATIVE_EFFECT marker={marker} new_get={get} new_put=2 post_tail_doorbells=128 elapsed_us={}", start.elapsed().as_micros());
                    break;
                }
                if start.elapsed() > Duration::from_secs(10) {
                    return Err(format!("migration bench deadline marker={marker} new_get={get}; no inner status or completion invented"));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if args[0] == "update-hold" || migration {
            // DIAGNOSTIC ONLY: retain every owned resource after the real GPU
            // tail to separate an asynchronous action fault from early cleanup.
            // A tail is not a deferred-action verdict. No product state is
            // released by this observation interval and no completion is forged.
            let unix_us = || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_micros())
            };
            println!("DEFERRED_UPDATE_HOLD_BEGIN unix_us={}", unix_us());
            std::thread::sleep(Duration::from_secs(2));
            println!("DEFERRED_UPDATE_HOLD_END unix_us={}", unix_us());
        }
        if migration && !trigger {
            let marker = cpu
                .load_u32(HostOffset::new(NEW_FENCE))
                .map_err(|e| format!("NOP replacement marker: {e:?}"))?;
            let get = cpu
                .load_u32(HostOffset::new(NEW_USERD + kf_abi::submit::USERD_GP_GET))
                .map_err(|e| format!("NOP replacement GET: {e:?}"))?;
            if marker != 0 || get != 0 {
                return Err(format!(
                    "registration-only control unexpectedly moved: marker={marker} new_get={get}"
                ));
            }
            println!("DEFERRED_MIGRATION_NOP_CONTROL replacement_marker=0 replacement_get=0");
        }
        Ok(())
    })();
    // Quiesce/free the owned channel before revoking its memory, even on a failed run.
    let freed = channel
        .map(|c| rm.free_channel(c))
        .transpose()
        .map_err(|e| format!("free channel: {e:?}"));
    if let Err(why) = &freed {
        eprintln!("DEFERRED_UPDATE_CLEANUP retained native memory/mapping/view after uncertain channel free; closing own RM session before releasing CPU view: {why}");
        let why = why.clone();
        drop(rm);
        drop(cpu);
        return Err(why);
    }
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
