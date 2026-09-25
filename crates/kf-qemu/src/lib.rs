//! ★★★★★ **v3 `kf-qemu` — the Rust half of the `kf3-gpu` QEMU device.**
//!
//! A small C QOM device (`qemu/hw/misc/kf3/kf3.c`) presents the PCI function and routes BAR0 writes
//! and guest-RAM registration here; everything else is [`device::Device`]: the host RM session, the
//! store, the family, the derived BAR0 (boot registers, VBIOS, GSP registers), the plane and its
//! ONE register drainer. BAR0 reads never exit — they read the shadow the C maps as a ROM device.
//!
//! The `unsafe` surface is two files: [`raw_unsafe`] (memory QEMU owns) and [`ffi_unsafe`].

pub mod cardbudget;
pub mod chan;
pub mod device;
pub mod ffi_unsafe;
pub mod hostfacts;
pub mod mem;
pub mod prof;
pub mod raw_unsafe;
pub mod rmfacts;
