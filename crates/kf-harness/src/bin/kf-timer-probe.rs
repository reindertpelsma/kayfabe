//! Native, unprivileged timer mapping evidence. No guest and no GPU workload.
fn main() -> Result<(), String> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    for line in status.lines().filter(|s| {
        [
            "Uid:",
            "Gid:",
            "Groups:",
            "CapInh:",
            "CapPrm:",
            "CapEff:",
            "CapBnd:",
            "CapAmb:",
            "NoNewPrivs:",
        ]
        .iter()
        .any(|p| s.starts_with(p))
    }) {
        println!("{line}");
    }
    let dev = kf_linux_raw::DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = kf_host::HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    println!(
        "host_driver={} arch={:?}",
        rm.driver_version(),
        rm.arch_info()
    );
    for round in 0..3 {
        let timer = rm.open_timer().map_err(|e| format!("timer: {e:?}"))?;
        let a = timer.read_ns().map_err(|e| format!("read: {e:?}"))?;
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = timer.read_ns().map_err(|e| format!("read: {e:?}"))?;
        let usermode = rm
            .gpu_time_ns()
            .map_err(|e| format!("usermode read: {e:?}"))?;
        println!(
            "round={round} base={:#x} layout={:?} first={a} second={b} delta={} usermode={usermode} delta_to_usermode={}",
            timer.bar0_base(),
            timer.layout(),
            b.saturating_sub(a),
            usermode.abs_diff(b)
        );
        if b <= a || usermode.abs_diff(b) > 10_000_000 {
            return Err("timer stalled or differs from usermode by >10ms".into());
        }
        drop(timer);
    }
    println!("PASS native read-only timer map/read/drop x3");
    Ok(())
}
