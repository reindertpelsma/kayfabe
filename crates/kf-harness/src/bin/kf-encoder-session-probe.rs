//! Measure the authored NVENC session acquire/release contract on the host.
//! This bench deliberately bypasses the layout gate; no guest bytes are used.
use kf_abi::gssreplay::{GSS_ENC_SESSION_ACQUIRE, GSS_ENC_SESSION_RELEASE};
use kf_host::HostRm;
use kf_linux_raw::DevDir;

fn main() {
    if let Err(e) = run() {
        eprintln!("ENC_SESSION_PROBE=FAIL {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    println!("ENC_SESSION_DRIVER={}", rm.driver_version());
    for cycle in 0..2 {
        let mut acquire = [0u8; 4];
        let st = rm
            .raw_control_opaque(rm.subdevice(), GSS_ENC_SESSION_ACQUIRE, &mut acquire)
            .map_err(|e| format!("acquire ioctl: {e:?}"))?;
        println!("ENC_SESSION cycle={cycle} acquire_status={st:#x} reply={acquire:02x?}");
        if st != 0 {
            return Err(format!("acquire refused {st:#x}"));
        }
        // Balance every successful acquire, even if its reply breaks the hypothesis.
        let mut release = [0u8; 4];
        let st = rm
            .raw_control_opaque(rm.subdevice(), GSS_ENC_SESSION_RELEASE, &mut release)
            .map_err(|e| format!("release ioctl: {e:?}"))?;
        println!("ENC_SESSION cycle={cycle} release_status={st:#x} reply={release:02x?}");
        if st != 0 || acquire != [0; 4] || release != [0; 4] {
            return Err("release or four-zero-byte reply contract failed".into());
        }
    }
    println!("ENC_SESSION_PROBE=PASS");
    Ok(())
}
