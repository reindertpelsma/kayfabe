// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! QEMU's `isa-debugcon` on I/O port 0x402: one `out` per byte, one VM exit per byte.
//!
//! ⚠ TEST BUILDS ONLY. The driver includes this module only with the `debugcon` feature
//! (`src/main.rs`); the release ROM contains no port I/O. The test application always includes it,
//! because reporting to the host is its whole job.

/// The port QEMU's `-debugcon` listens on with `-global isa-debugcon.iobase=0x402` (OVMF's own
/// debug port).
pub const DEBUGCON: u16 = 0x402;

/// Write `bytes` to the debug port.
pub fn write(bytes: &[u8]) {
    for &b in bytes {
        // SAFETY: an `out` to an I/O port reads and writes no memory. In a QEMU guest with
        // `-debugcon` the port is a write-only character sink; without one, a write to an
        // unclaimed ISA port is discarded. Firmware runs at CPL 0, so `out` cannot fault.
        unsafe {
            core::arch::asm!("out dx, al", in("dx") DEBUGCON, in("al") b, options(nomem, nostack, preserves_flags));
        }
    }
}
