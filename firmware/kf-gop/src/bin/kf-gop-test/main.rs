// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! `kf-gop-test.efi` — the local stand-in's checker, a UEFI application (never shipped).
//!
//! Booted as `\EFI\BOOT\BOOTX64.EFI` from a FAT drive in a VM whose stand-in display device carries
//! a kf-gop ROM (`scripts/display/gop_standin.sh`). It finds the GOP that kf-gop published, checks
//! it against the descriptor in the device's own ROM image and against the BAR the PCI bus driver
//! assigned, exercises `QueryMode`, `SetMode` and every `Blt` operation (including the refusals),
//! then draws `kf_gop::test_pattern` over the whole screen and reports on I/O port 0x402:
//! `KFGOP-TEST RESULT PASS|FAIL …`, the framebuffer's address, and `KFGOP-TEST READY`. It then waits,
//! so the host can read the framebuffer back through QEMU's monitor and compare it byte for byte.
#![no_std]
#![no_main]

// ★ The test application reports on x86 I/O port 0x402, and the stand-in that reads it is x86 only
// (`port_unsafe.rs`; `docs/OWNER_RULINGS.md` §K). Elsewhere it is a named build error; CI's aarch64
// job builds the driver alone (`--bin kf-gop`).
#[cfg(not(target_arch = "x86_64"))]
compile_error!(
    "kf-gop-test reports on x86 I/O port 0x402 and is built for x86_64-unknown-uefi only; \
     build the driver alone with `--bin kf-gop` for this target"
);

mod check_unsafe;
#[path = "../../efi_unsafe.rs"]
mod efi_unsafe;
#[path = "../../port_unsafe.rs"]
mod port_unsafe;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    port_unsafe::write(b"KFGOP-TEST PANIC\n");
    loop {
        core::hint::spin_loop();
    }
}
