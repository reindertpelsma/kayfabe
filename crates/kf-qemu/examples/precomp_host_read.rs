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

fn reply_text(_addresses: &[u32], _values: &[u32]) -> Result<String, String> {
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::reply_text;
    #[test]
    fn first_reply_retains_its_actual_address_and_value() {
        let s = reply_text(&[0x640788], &[0x12345678]).unwrap();
        assert!(s.contains("address=0x00640788 value=0x12345678"));
        assert_eq!(s.lines().count(), 1);
    }
    #[test]
    fn last_bounded_reply_is_recorded_without_truncation() {
        let a: Vec<_> = (0..192).map(|i| 0x640780 + i * 4).collect();
        let v: Vec<_> = (0..192).map(|i| 0x12340000 + i).collect();
        let s = reply_text(&a, &v).unwrap();
        assert_eq!(s.lines().count(), 192);
        assert!(s.lines().last().unwrap().contains("value=0x123400bf"));
    }
    #[test]
    fn short_reply_cannot_be_labelled_complete() {
        assert!(reply_text(&[4, 8], &[1]).is_err());
    }
    #[test]
    fn long_reply_cannot_be_labelled_complete() {
        assert!(reply_text(&[4], &[1, 2]).is_err());
    }
    #[test]
    fn an_empty_plan_does_not_emit_a_fake_success() {
        assert!(reply_text(&[], &[]).is_err());
    }
    #[test]
    fn an_oversized_plan_cannot_emit_unbounded_evidence() {
        assert!(reply_text(&vec![4; 193], &vec![1; 193]).is_err());
    }
}
