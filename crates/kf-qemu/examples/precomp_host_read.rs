//! Before Windows media/setup, prove or refuse this host's unprivileged PRECOMP permission.
//! No guest, CUDA context, submitted GPU work, arbitrary-offset argument or word dump.
use kf_arch::ids::GpuId;
use kf_linux_raw::{DevDir, capability};

fn run() -> Result<(), String> {
    let gpu = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0".into())
        .parse::<u32>()
        .map_err(|e| format!("GPU index: {e}"))?;
    capability::clear_effective_for_thread_life(capability::CAP_SYS_ADMIN)
        .map_err(|e| format!("unprivileged posture refused: {e:?}"))?;
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = kf_host::HostRm::open(&dev, GpuId(gpu), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let die = kf_disp::capsderive::Die {
        architecture,
        implementation,
    };
    let row = kf_chip::display::display_for(architecture, implementation)
        .ok_or("selected die has no public display row")?;
    let table = kf_disp::class::for_version(rm.driver_version())
        .ok_or("host driver has no compiled display class vocabulary")?;
    let regs = kf_disp::regs::Regs::for_ip(rm.driver_version(), row.ip_version)
        .ok_or("host driver has no compiled display register vocabulary")?;
    let _snapshot = kf_disp::hostcaps::query(table, &regs, row, die, |addresses| {
        rm.read_precomp_capabilities(addresses)
    })?;
    println!(
        "PRECOMP_HOST_READ=OK pci={} arch={architecture:#x} impl={implementation:#x} host={} class={:#x} windows={} named_words={} CAP_SYS_ADMIN=clear",
        rm.card().bdf(),
        rm.driver_version(),
        row.classes.caps,
        row.windows,
        row.windows * 6
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("PRECOMP_HOST_READ=REFUSED derived fallback; {error}");
        std::process::exit(2);
    }
}

fn reply_text(_addresses: &[u32], _values: &[u32]) -> Result<String,String> {
    Ok(String::new())
}
