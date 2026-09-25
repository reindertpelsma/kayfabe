//! ★★★ **v3 gate 1 — a REAL copy engine, on our own ring, completing through the host EVENT.**
//!
//! No QEMU, no guest, no CPU copy anywhere. Every copy ends in NVIDIA's own completion tail — a host
//! fence release **with `RELEASE_WFI`**, then the chosen trigger — and a wake counts only if the
//! fence holds OUR payload when re-read AT that wake (the non-stall notifiers are GPU-wide and carry
//! no identity, `intr.c:1195-1205`). Arms, one trigger each, so the edge is attributed, not assumed:
//! - `quiet`   — nothing submitted: the background wake rate on this fd (another tenant, stale).
//! - `none`    — fence, no trigger: work completes, and no edge of ours should arrive.
//! - `ce_intr` — only the CE `LAUNCH_DMA` non-blocking interrupt (the CE(n) notifiers).
//! - `nsi`     — only the host `NON_STALL_INTERRUPT` (`FIFO_EVENT_MTHD`), ×20: the gating arm.
//!
//! ⊘ The deferred-map/one-invalidate property is NOT asserted here: these maps land in a fresh
//! VA space with nothing stale to invalidate, so they would pass with the defer bit ignored. Gate 2's
//! remap (a warm TLB, then deferred unmap+map, then one invalidate) is the arm that can fail.

use kf_harness::{CeRig, Ledger, Trigger};
use kf_host::{HostRm, event::notifier_ce};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset};

const DATA_BYTES: u64 = 0x1_0000;
const COPY_LEN: u32 = 4096;
const NSI_REPS: u32 = 20;

fn main() {
    let mut l = Ledger::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE1_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn run(l: &mut Ledger) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes).map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));

    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let data = rm.alloc_device_local(DATA_BYTES).map_err(|e| format!("data obj: {e:?}"))?;
    let data_va = rm.map(space, data, kf_host::MapBacking::Dedicated, 0, DATA_BYTES, None, true).map_err(|e| format!("map data: {e:?}"))?;
    rm.invalidate_tlb(space).map_err(|e| format!("invalidate: {e:?}"))?;
    let (_dn, data_cpu) = rm.map_cpu(data, DATA_BYTES, CachePolicy::Uncached).map_err(|e| format!("cpu data: {e:?}"))?;
    let at = HostOffset::new;

    let mut rig = CeRig::new(&rm, space)?;
    let chan = rig.channel();
    l.measure("channel", format!("tsg={:#x} chan={:#x} token={:#x}", chan.tsg, chan.chan, chan.token));
    let mut armed = Vec::new();
    for n in 0..10u32 {
        armed.push(format!("CE{n}:{}", rig.arm(&rm, notifier_ce(n)).is_ok()));
    }
    l.measure("ce_notifiers_armed", armed.join(" "));

    // Source pattern once; each arm copies into its own zeroed 4 KiB destination.
    for i in 0..(COPY_LEN / 4) {
        data_cpu.store_u32(at(u64::from(i) * 4), 0x5A00_0000 | i).map_err(|e| format!("{e:?}"))?;
    }
    let mut dst_slot = 0u64;
    let mut fresh_dst = |cpu: &kf_linux_raw::VolatileRegion| -> Result<u64, String> {
        dst_slot += 1;
        let off = (dst_slot % 15 + 1) * u64::from(COPY_LEN);
        for i in 0..(COPY_LEN / 4) {
            cpu.store_u32(at(off + u64::from(i) * 4), 0).map_err(|e| format!("{e:?}"))?;
        }
        Ok(off)
    };
    let bad_words = |cpu: &kf_linux_raw::VolatileRegion, off: u64| -> Result<u32, String> {
        let mut bad = 0;
        for i in 0..(COPY_LEN / 4) {
            if cpu.load_u32(at(off + u64::from(i) * 4)).map_err(|e| format!("{e:?}"))? != 0x5A00_0000 | i {
                bad += 1;
            }
        }
        Ok(bad)
    };

    let quiet = rig.quiet_wakes(300)?;
    l.measure("quiet", format!("wakes={quiet} in 300 ms with nothing submitted"));

    let off = fresh_dst(&data_cpu)?;
    let s = rig.submit(&rm, data_va, data_va + off, COPY_LEN, Trigger::None, 300)?;
    l.check("none_arm_work_completes", s.ce_released && s.fence_landed && bad_words(&data_cpu, off)? == 0, format!("{s:?}"));
    l.measure("none_arm_wakes", format!("wakes={} (an edge with no trigger of ours = someone else's)", s.wakes));

    let off = fresh_dst(&data_cpu)?;
    let s = rig.submit(&rm, data_va, data_va + off, COPY_LEN, Trigger::CeInterrupt, 500)?;
    l.check("ce_intr_arm_work_completes", s.ce_released && s.fence_landed && bad_words(&data_cpu, off)? == 0, format!("{s:?}"));
    l.measure("ce_intr_arm_edge", format!("seen_at_wake={} wakes={} after_us={}", s.seen_at_wake, s.wakes, s.event_us));

    let (mut seen, mut early, mut bad, mut lat) = (0u32, 0u32, 0u32, Vec::new());
    for _ in 0..NSI_REPS {
        let off = fresh_dst(&data_cpu)?;
        let s = rig.submit(&rm, data_va, data_va + off, COPY_LEN, Trigger::HostNsi, 2000)?;
        if s.seen_at_wake {
            seen += 1;
            lat.push(s.event_us);
        }
        early += s.early_wakes;
        bad += bad_words(&data_cpu, off)?;
    }
    lat.sort_unstable();
    let pct = |p: usize| lat.get((lat.len().saturating_sub(1)) * p / 100).copied().unwrap_or(0);
    l.check("bytes_copied_by_engine", bad == 0, format!("{NSI_REPS} copies x {} words, {bad} wrong", COPY_LEN / 4));
    l.check(
        "nsi_completion_seen_at_wake",
        seen == NSI_REPS,
        format!("{seen}/{NSI_REPS} fences read AT a wake; early_wakes={early}; us p50={} p90={} max={}", pct(50), pct(90), pct(100)),
    );
    Ok(())
}
