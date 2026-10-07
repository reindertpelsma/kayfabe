//! Emit a synthetic VBIOS image to a file.
//!
//! ```text
//! cargo run -p kayfabe-abi --example synth_vbios -- <out-path> [pci-device-id]
//! ```
//!
//! The device id defaults to `0x2504` (GA106) and selects a row from
//! [`kayfabe_abi::vbios::VBIOS_PROFILES`]. An unknown id is refused — there is no
//! nearest-neighbour fallback, because serving one card's ROM for another is the
//! host/guest disagreement the generated-not-dumped design exists to prevent.
//!
//! # Why a file, and what it is a stand-in for
//!
//! This exists so the **experimental** QEMU device shim (which serves the PROM
//! window in C) can be handed an image without embedding a blob in C or growing
//! a Rust call on its MMIO read path. That is a scaffold, not the destination:
//! in production the device should ask the core for the image at realize, built
//! from the *same* profile that answers its registers — which is the entire
//! point of generating rather than dumping. A file on disk reintroduces, weakly,
//! the possibility of serving an image that disagrees with the device, so it
//! should not outlive the experiment.

use kayfabe_abi::vbios::{VbiosWire, build, profile_for_device_id};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (out, id) = match args.as_slice() {
        [_, out] => (out.clone(), 0x2504u16),
        [_, out, id] => {
            let t = id.trim_start_matches("0x");
            let Ok(v) = u16::from_str_radix(t, 16) else {
                eprintln!("synth_vbios: `{id}` is not a hex PCI device id");
                return std::process::ExitCode::FAILURE;
            };
            (out.clone(), v)
        }
        _ => {
            eprintln!("usage: synth_vbios <out-path> [pci-device-id-hex]");
            return std::process::ExitCode::FAILURE;
        }
    };

    let profile = match profile_for_device_id(id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("synth_vbios: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let image = match build(profile, VbiosWire::Tu102Bit) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("synth_vbios: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out, &image) {
        eprintln!("synth_vbios: write {out}: {e}");
        return std::process::ExitCode::FAILURE;
    }
    println!(
        "wrote {out}: {} bytes, profile {} ({:#06x}:{:#06x})",
        image.len(),
        profile.name,
        profile.pci_vendor_id,
        profile.pci_device_id,
    );
    std::process::ExitCode::SUCCESS
}
