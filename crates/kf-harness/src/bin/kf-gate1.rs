//! ★★★ **v3 gate 1 — a REAL copy engine, on our own ring, completing through the host EVENT.**
//!
//! No QEMU, no guest, no CPU copy anywhere. Asserts: the host session opens on the family the
//! host reports; a CE channel is born, bound and scheduled; one copy moves real bytes; the
//! semaphore is released BY THE ENGINE; and MEASURES whether the non-stall interrupt reaches our
//! `NV2080_NOTIFIERS_CE(0)` event fd (the unmeasured §37 question).

use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_GP_PUT, gp_entry};
use kf_chip::Family;
use kf_harness::{Ledger, ce_copy_push};
use kf_host::{HostRm, RingSpec, event::notifier_ce};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset, PollTimeout, Poller, ReadyTokens, release_fence};
use std::time::{Duration, Instant};

const RING_BYTES: u64 = 0x1_0000;
const PB_OFF: u64 = 0x0;
const GPFIFO_OFF: u64 = 0x1000;
const GPFIFO_ENTRIES: u32 = 64;
const SEM_OFF: u64 = 0x2000;
const USERD_OFF: u64 = 0x3000;
const DATA_BYTES: u64 = 0x1_0000;
const COPY_LEN: u32 = 4096;
const PAYLOAD: u32 = 0xC0FF_EE01;

fn main() {
    let mut l = Ledger::default();
    let ok = run(&mut l);
    if let Err(e) = ok {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE1_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn run(l: &mut Ledger) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let pick = |a: u32, i: u32| Family::from_arch(a, i).ok().map(Family::host_classes);
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &pick).map_err(|e| e.to_string())?;
    l.check("session", true, format!("driver {}", rm.driver_version()));

    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let ring = rm.alloc_device_local(RING_BYTES).map_err(|e| format!("ring obj: {e:?}"))?;
    let data = rm.alloc_device_local(DATA_BYTES).map_err(|e| format!("data obj: {e:?}"))?;
    let ring_va = rm.map(space, ring, 0, RING_BYTES, None, true).map_err(|e| format!("map ring: {e:?}"))?;
    let data_va = rm.map(space, data, 0, DATA_BYTES, None, true).map_err(|e| format!("map data: {e:?}"))?;
    rm.invalidate_tlb(space).map_err(|e| format!("invalidate: {e:?}"))?;
    l.check("batched_map", true, format!("ring@{ring_va:#x} data@{data_va:#x}, 2 deferred maps + 1 invalidate"));

    let (_rn, ring_cpu) = rm.map_cpu(ring, RING_BYTES, CachePolicy::Uncached).map_err(|e| format!("cpu ring: {e:?}"))?;
    let (_dn, data_cpu) = rm.map_cpu(data, DATA_BYTES, CachePolicy::Uncached).map_err(|e| format!("cpu data: {e:?}"))?;
    let at = HostOffset::new;
    for i in 0..(COPY_LEN / 4) {
        data_cpu.store_u32(at(u64::from(i) * 4), 0x5A00_0000 | i).map_err(|e| format!("{e:?}"))?;
        data_cpu.store_u32(at(0x8000 + u64::from(i) * 4), 0).map_err(|e| format!("{e:?}"))?;
    }
    ring_cpu.store_u32(at(SEM_OFF), 0).map_err(|e| format!("{e:?}"))?;

    let ev = rm.open_event_fd().map_err(|e| format!("event fd: {e:?}"))?;
    let _evo = rm.alloc_os_event(rm.subdevice(), notifier_ce(0), &ev).map_err(|e| format!("os event: {e:?}"))?;
    let armed = rm.set_notification(notifier_ce(0), kf_abi::eventnotify::ACTION_REPEAT);
    l.measure("event_armed", format!("{armed:?}"));

    let chan = rm
        .birth_channel(space, ENGINE_TYPE_COPY0, RingSpec {
            gp_fifo_va: ring_va + GPFIFO_OFF,
            gp_fifo_entries: GPFIFO_ENTRIES,
            userd_memory: ring,
            userd_offset: USERD_OFF,
            err_notifier: 0,
        })
        .map_err(|e| format!("birth: {e:?}"))?;
    rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0).map_err(|e| format!("ce object: {e:?}"))?;
    rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
    l.check("channel", true, format!("tsg={:#x} chan={:#x} token={:#x}", chan.tsg, chan.chan, chan.token));

    let ce_class = rm.ce_class_id();
    let words = ce_copy_push(ce_class, data_va, data_va + 0x8000, COPY_LEN, ring_va + SEM_OFF, PAYLOAD, true)
        .ok_or("push encode")?;
    for (i, w) in words.iter().enumerate() {
        ring_cpu.store_u32(at(PB_OFF + 4 * i as u64), *w).map_err(|e| format!("{e:?}"))?;
    }
    let entry = gp_entry(ring_va + PB_OFF, 4 * words.len() as u64).ok_or("gp entry")?;
    ring_cpu.store_u32(at(GPFIFO_OFF), entry as u32).map_err(|e| format!("{e:?}"))?;
    ring_cpu.store_u32(at(GPFIFO_OFF + 4), (entry >> 32) as u32).map_err(|e| format!("{e:?}"))?;
    release_fence();
    ring_cpu.store_u32(at(USERD_OFF + USERD_GP_PUT), 1).map_err(|e| format!("{e:?}"))?;
    release_fence();

    let poller = Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
    poller.watch(ev.as_fd(), 1).map_err(|e| format!("watch: {e:?}"))?;
    let t0 = Instant::now();
    rm.doorbell(chan.token).map_err(|e| format!("doorbell: {e:?}"))?;

    let mut ready = ReadyTokens::new();
    let n = poller.wait(&mut ready, PollTimeout::Millis(2000)).map_err(|e| format!("wait: {e:?}"))?;
    let event_us = t0.elapsed().as_micros();
    let drained = ev.drain().map_err(|e| format!("drain: {e:?}"))?;
    l.measure("event_fd", format!("ready={n} after_us={event_us} records={drained:?}"));

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut sem = ring_cpu.load_u32(at(SEM_OFF)).map_err(|e| format!("{e:?}"))?;
    while sem != PAYLOAD && Instant::now() < deadline {
        std::thread::sleep(Duration::from_micros(200));
        sem = ring_cpu.load_u32(at(SEM_OFF)).map_err(|e| format!("{e:?}"))?;
    }
    l.check("semaphore_released_by_engine", sem == PAYLOAD, format!("sem={sem:#x} want={PAYLOAD:#x}"));
    let mut bad = 0u32;
    for i in 0..(COPY_LEN / 4) {
        let v = data_cpu.load_u32(at(0x8000 + u64::from(i) * 4)).map_err(|e| format!("{e:?}"))?;
        if v != 0x5A00_0000 | i {
            bad += 1;
        }
    }
    l.check("bytes_copied_by_engine", bad == 0, format!("{} words, {bad} wrong", COPY_LEN / 4));
    l.check("completion_edge_is_an_event", n > 0 && !drained.is_empty(), format!("ready={n} records={}", drained.len()));
    Ok(())
}
