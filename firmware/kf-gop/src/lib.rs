// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The safe half of kf3's boot-display GOP: every decision the driver makes, as pure functions over
//! bytes, testable on the host (`cargo test --lib --target x86_64-unknown-linux-gnu`).
//!
//! The firmware binaries (`src/main.rs`, the driver; `src/bin/kf-gop-test.rs`, the stand-in's checker)
//! hold the UEFI ABI and every raw pointer, in `*_unsafe.rs` files only (`firmware/README.md`). What
//! they do with a pointer is decided here first:
//! - [`probe`] — whether a PCI controller is ours (`Supported`): display class, a `KFGP` descriptor in
//!   its ROM image with the controller's own ids, and later whether the framebuffer fits the BAR the
//!   PCI bus driver assigned.
//! - [`blt`] — `EFI_GRAPHICS_OUTPUT_PROTOCOL.Blt`'s parameter rules and the copies themselves, over
//!   byte slices: the RAM shadow, the framebuffer and the caller's buffer.
//! - [`log`] — a fixed-size line formatter for the test build's debug port.
#![no_std]
#![forbid(unsafe_code)]

pub mod blt;
pub mod log;
pub mod probe;

/// Embedded in the driver so `kf-oprom` can tell a kf-gop build that decodes descriptor version 1
/// from any other PE (`kf_oprom::pe::check`). The driver keeps it alive with `black_box`.
pub const ABI_MARKER: &[u8] = kf_oprom::pe::ABI_MARKER;

/// UEFI `PixelBlueGreenRedReserved8BitPerColor`: the byte order of `KFGP` format 1 (XRGB8888).
pub const PIXEL_BGRX: u32 = 1;

/// The stand-in's full-screen test pattern, as BGRX bytes. The host script recomputes it to compare
/// the framebuffer it reads back (`scripts/display/gop_standin.sh`), so it must stay in step with
/// that script's `pattern()`.
#[must_use]
pub fn test_pattern(x: u32, y: u32) -> [u8; 4] {
    [
        (x.wrapping_mul(3).wrapping_add(y.wrapping_mul(5)) & 0xFF) as u8,
        ((x ^ y) & 0xFF) as u8,
        (x.wrapping_add(y.wrapping_mul(7)) & 0xFF) as u8,
        0,
    ]
}
