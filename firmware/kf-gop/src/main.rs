// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! `kf-gop.efi` — kf3's boot-display GOP, a UEFI boot-service driver (`docs/design/V3_DISPLAY.md`
//! §4.11).
//!
//! It ships inside kf3's PCI expansion ROM. OVMF's PCI bus driver loads it from the ROM, and tries
//! its driver binding on the device first (the bus-specific driver override). It binds only to the
//! controller whose ROM carries a `KFGP` descriptor with that controller's own ids, enables memory
//! decode and nothing else, and publishes one GOP mode whose framebuffer is the descriptor's range of
//! the descriptor's BAR. kf3 has already prepared that range (VRAM behind BAR1) and scans it out;
//! the firmware programs no hardware. `Blt` is software, with a RAM shadow for reads.
//!
//! Layout: every decision is in the safe library (`src/lib.rs`: `probe`, `blt`); this binary holds
//! the UEFI ABI (`efi_unsafe.rs`) and the entry points (`driver_unsafe.rs`). The debug port module is
//! compiled in only with the `debugcon` feature, for test builds.
#![no_std]
#![no_main]

mod driver_unsafe;
mod efi_unsafe;
#[cfg(feature = "debugcon")]
mod port_unsafe;

/// A panic cannot unwind out of firmware. Nothing in the driver is expected to panic — every slice
/// access in the library is checked and refused — so reaching this is a bug, and stopping here is
/// the only answer that does not draw through a bad pointer.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
