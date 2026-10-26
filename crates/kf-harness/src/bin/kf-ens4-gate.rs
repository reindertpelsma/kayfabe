//! Explicit GPU-free model or Linux-guest CE latency/churn arm. No default GPU access.
use kf_mem::latency_gate::{Bound, model};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
const BASELINE: usize = 10_000;
const PACKETS: usize = 20_000;
const RUNS: u64 = 4174;
const PAGE: u64 = 4096;
const STRIDE: u64 = 2 << 20;
const BASE: u64 = 0x40_0000_0000;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let native_mode = args.first().is_some_and(|a| a == "--gpu");
    let prefix = if native_mode {
        "ENS4_CLIENT"
    } else {
        "ENS4_GATE"
    };
    let result = match args.first().map(String::as_str) {
        Some("--sim") => simulation(&args[1..]),
        Some("--gpu") => native(&args[1..]),
        _ => Err("use --sim [--apply-us N] or --gpu --csv FILE (Linux guest only)".into()),
    };
    let rc = match result {
        Ok(pass) => {
            println!("{prefix}_VERDICT={}", if pass { "PASS" } else { "FAIL" });
            u8::from(!pass)
        }
        Err(e) => {
            eprintln!("{prefix}_INVALID={e}");
            2
        }
    };
    println!("{prefix}_DONE rc={rc}");
    std::process::exit(i32::from(rc));
}
fn simulation(args: &[String]) -> Result<bool, String> {
    let us = match args {
        [] => 77_000,
        [flag, value] if flag == "--apply-us" => {
            value.parse::<u64>().map_err(|e| format!("{e:?}"))?
        }
        _ => return Err("--sim accepts only --apply-us N".into()),
    };
    if us > 10_000_000 {
        return Err("simulation apply exceeds 10 seconds".into());
    }
    let b = Bound::calibrate(&vec![100_000; BASELINE]).map_err(str::to_owned)?;
    let t = model(1_000_000, 0, us.saturating_mul(1000), 100_000);
    let latency = t.completed.saturating_sub(t.announced);
    println!(
        "ENS4_SIM synthetic=true runs={RUNS} apply_us={us} trace={t:?} \
        p999_ns={} k=4 bound_ns={} worst_ns={latency}",
        b.p999_ns, b.limit_ns
    );
    Ok(b.passes(Some(latency)))
}
fn native(args: &[String]) -> Result<bool, String> {
    let [flag, path] = args else {
        return Err("--gpu requires --csv FILE".into());
    };
    if flag != "--csv" {
        return Err("--gpu requires --csv FILE".into());
    }
    // A fresh arm never overwrites an earlier boot's raw latency population.
    let mut csv = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("new sample file: {e:?}"))?;
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("/dev: {e:?}"))?;
    let rm = kf_host::HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| format!("{e:?}"))?;
    let space = rm.alloc_vaspace_bare().map_err(|e| format!("{e:?}"))?;
    let data = rm.alloc_device_local(8192).map_err(|e| format!("{e:?}"))?;
    let va = rm
        .map(
            space,
            data,
            kf_host::MapBacking::SharedSlice,
            0,
            8192,
            None,
            false,
        )
        .map_err(|e| format!("{e:?}"))?;
    let mut rig = kf_harness::CeRig::new(&rm, space)?;
    println!(
        "ENS4_CHANNEL client={:#x} channel={:#x}",
        rm.client_handle(),
        rig.channel().chan
    );
    let mut sample = || -> Result<Option<u64>, String> {
        let start = Instant::now();
        let got = rig.submit(&rm, va, va + 4096, 16, kf_harness::Trigger::HostNsi, 10_000)?;
        let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        Ok((got.ce_released && got.fence_landed).then_some(ns))
    };
    for _ in 0..1024 {
        if sample()?.is_none() {
            return Err("warmup missing engine fence".into());
        }
    }
    let mut rows: Vec<(&str, Option<u64>)> = Vec::with_capacity(BASELINE + PACKETS);
    let mut clean = Vec::with_capacity(BASELINE);
    for _ in 0..BASELINE {
        let ns = sample()?.ok_or("calibration missing engine fence")?;
        clean.push(ns);
        rows.push(("calibration", Some(ns)));
    }
    let bound = Bound::calibrate(&clean).map_err(str::to_owned)?;
    println!(
        "ENS4_CALIB n={} p999_ns={} k=4 bound_ns={} frozen=true",
        clean.len(),
        bound.p999_ns,
        bound.limit_ns
    );
    let stop = AtomicBool::new(false);
    let active = AtomicBool::new(false);
    let rounds = AtomicU64::new(0);
    let (pass, churn_result) = std::thread::scope(|scope| {
        let worker = scope.spawn(|| churn(&rm, &stop, &active, &rounds));
        // Do not measure an empty interval before churn actually starts.
        while !active.load(Ordering::Acquire) && !worker.is_finished() {
            std::thread::yield_now();
        }
        let mut pass = true;
        for _ in 0..PACKETS {
            if worker.is_finished() {
                break;
            }
            match sample() {
                Ok(ns) => {
                    rows.push(("churn", ns));
                    if !bound.passes(ns) {
                        pass = false;
                        break;
                    }
                }
                Err(e) => {
                    stop.store(true, Ordering::Release);
                    return (Err(e), worker.join());
                }
            }
        }
        stop.store(true, Ordering::Release);
        (Ok(pass), worker.join())
    });
    let pass = pass?;
    let churn_result = churn_result.map_err(|_| "churn thread panicked")?;
    writeln!(csv, "phase,index,latency_ns,engine_receipt").map_err(|e| format!("{e:?}"))?;
    for (i, (phase, ns)) in rows.iter().enumerate() {
        writeln!(
            csv,
            "{phase},{i},{},{}",
            ns.map_or(String::new(), |v| v.to_string()),
            ns.is_some()
        )
        .map_err(|e| format!("{e:?}"))?;
    }
    churn_result?;
    let tested: Vec<_> = rows.iter().filter(|r| r.0 == "churn").collect();
    if tested.is_empty() || rounds.load(Ordering::Acquire) == 0 {
        return Err("no complete churn round/overlapping packet; arm invalid".into());
    }
    let worst = tested.iter().filter_map(|r| r.1).max();
    let first = tested.iter().position(|r| !bound.passes(r.1));
    println!(
        "ENS4_LINUX packets={} rounds={} maps_per_round={RUNS} worst_ns={worst:?} \
        first_fail={first:?} receipt=owned_CE_and_WFI_fence \
        route_and_large_apply_coverage=REQUIRE_CONTROLLER_VALIDATION csv={path}",
        tested.len(),
        rounds.load(Ordering::Acquire)
    );
    // The controller must reject absent USER_RING/large-apply coverage, even on local PASS.
    Ok(pass)
}
fn churn(
    rm: &kf_host::HostRm,
    stop: &AtomicBool,
    active: &AtomicBool,
    rounds: &AtomicU64,
) -> Result<(), String> {
    let mut space = rm.alloc_vaspace_bare().map_err(|e| format!("{e:?}"))?;
    println!(
        "ENS4_CHURN client={:#x} vaspace={:#x}",
        rm.client_handle(),
        space.space
    );
    // Keep the churn VA space live in the VMM; an unbound space is not a valid gate test.
    let anchor = rm.alloc_device_local(65536).map_err(|e| format!("{e:?}"))?;
    let anchor_va = rm
        .map(
            space,
            anchor,
            kf_host::MapBacking::Dedicated,
            0,
            65536,
            None,
            false,
        )
        .map_err(|e| format!("{e:?}"))?;
    let idle = rm
        .birth_channel(
            space,
            kf_abi::submit::ENGINE_TYPE_COPY0,
            kf_host::RingSpec {
                gp_fifo_va: anchor_va + 0x1000,
                gp_fifo_entries: 64,
                userd_memory: anchor,
                userd_offset: 0x3000,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    rm.alloc_ce_object(idle, kf_abi::submit::ENGINE_TYPE_COPY0)
        .map_err(|e| format!("{e:?}"))?;
    rm.schedule(idle).map_err(|e| format!("{e:?}"))?;
    let object = rm
        .alloc_device_local(32 << 20)
        .map_err(|e| format!("{e:?}"))?;
    let span = RUNS * STRIDE;
    for _ in 0..32 {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let reservation = rm
            .reserve_va_flags(
                space.space,
                BASE,
                span,
                kf_host::channel::guest_reserve_flags(true),
            )
            .map_err(|e| format!("{e:?}"))?;
        space.guest[0] = kf_host::channel::GuestVaRange {
            handle: reservation,
            lo: BASE,
            hi: BASE + span,
        };
        active.store(true, Ordering::Release);
        for i in 0..RUNS {
            rm.map(
                space,
                object,
                kf_host::MapBacking::SharedSlice,
                (RUNS - 1 - i) * PAGE,
                PAGE,
                Some(BASE + i * STRIDE),
                true,
            )
            .map_err(|e| format!("{e:?}"))?;
        }
        rm.invalidate_tlb(space).map_err(|e| format!("{e:?}"))?;
        for i in 0..RUNS {
            rm.unmap_row(space, BASE + i * STRIDE, PAGE, true)
                .map_err(|e| format!("{e:?}"))?;
        }
        rm.invalidate_tlb(space).map_err(|e| format!("{e:?}"))?;
        rm.free(reservation).map_err(|e| format!("{e:?}"))?;
        space.guest[0] = kf_host::channel::GuestVaRange::default();
        rounds.fetch_add(1, Ordering::Release);
    }
    active.store(false, Ordering::Release);
    rm.free_channel(idle).map_err(|e| format!("{e:?}"))?;
    rm.unmap_row(space, anchor_va, 65536, false)
        .map_err(|e| format!("{e:?}"))?;
    rm.free(anchor).map_err(|e| format!("{e:?}"))?;
    rm.free(object).map_err(|e| format!("{e:?}"))?;
    rm.free_vaspace(space);
    Ok(())
}
