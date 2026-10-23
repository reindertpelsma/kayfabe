//! Q4 guest-only channel-birth input probe. No scheduling and no GPU work submission.
use kf_host::{HostRm, MapBacking, RingSpec};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset};
fn run() -> Result<(), String> {
    if std::env::var("Q4_GUEST_PROBE").as_deref() != Ok("1") {
        return Err("GUEST_GUARD_REQUIRED".into());
    }
    let vendor = std::fs::read_to_string("/sys/devices/virtual/dmi/id/sys_vendor")
        .map_err(|e| e.to_string())?;
    if vendor.trim() != "QEMU" {
        return Err("REFUSE_NON_QEMU_HOST".into());
    }
    let dev = DevDir::open(c"/dev").map_err(|e| format!("{e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace {e:?}"))?;
    let memory = rm
        .alloc_device_local(65536)
        .map_err(|e| format!("memory {e:?}"))?;
    let va = rm
        .map(space, memory, MapBacking::Dedicated, 0, 65536, None, false)
        .map_err(|e| format!("map {e:?}"))?;
    let (_node, cpu) = rm
        .map_cpu(memory, 65536, CachePolicy::Uncached)
        .map_err(|e| format!("cpu {e:?}"))?;
    for at in (0..65536).step_by(4) {
        cpu.store_u32(HostOffset::new(at), 0)
            .map_err(|e| format!("{e:?}"))?;
    }
    println!(
        "Q4_PROBE_BIRTH_ATTEMPT client={} admin_control={} scheduled=false submitted=false",
        std::env::var("Q4_PROBE_CLIENT").unwrap(),
        std::env::var("KF3_NEGCTL_SKIP_CAP_BRACKET").unwrap_or_default()
    );
    match rm.birth_channel(
        space,
        kf_abi::submit::ENGINE_TYPE_COPY0,
        RingSpec {
            gp_fifo_va: va,
            gp_fifo_entries: 32,
            userd_memory: memory,
            userd_offset: 16384,
            err_notifier: 0,
        },
    ) {
        Ok(channel) => {
            println!("Q4_PROBE_BIRTH_USER handle={:#x}", channel.chan);
            rm.free_channel(channel)
                .map_err(|e| format!("free {e:?}"))?;
        }
        Err(e) => {
            println!("Q4_PROBE_BIRTH_REFUSED {e:?}");
        }
    }
    println!("Q4_PROBE_INPUT_COMPLETE scheduled=false submitted=false");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Q4_PROBE_ERROR {e}");
        std::process::exit(2);
    }
}
