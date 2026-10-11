//! ★★★ **v3 `kf-disp` — the display plane: a virtual NVDisplay the stock guest driver drives.**
//!
//! `docs/design/V3_DISPLAY.md` is the design. The guest's own `nvidia.ko` (KernelDisplay),
//! `nvidia-modeset.ko` (NVKMS) and `nvidia-drm.ko` bring up a real KMS device against a display
//! engine that exists only here: kayfabe answers the physical-RM display controls, executes the
//! display channels (core, window, window-immediate, cursor) as **emulated channels** — there is no
//! guest GPU work behind them — keeps each head's state, drives vblank from a host timer, and asks
//! for each latched surface to be copied out of the store by kayfabe's own GPU work, which is what
//! leaves the VM (a QEMU console: VNC, SPICE, GTK, `screendump`).
//!
//! Nothing here touches the host's display engine, and nothing a guest writes is forwarded.
//!
//! This crate holds the parts that are pure logic and GPU-free:
//! - [`class`] — the display classes' methods, fields and caps registers, derived from ogkm;
//! - [`regs`] — the display engine's BAR0 registers, derived from ogkm's `dev_disp.h` per family;
//! - [`caps`] — the capabilities page our virtual engine presents, authored from the caps class;
//! - [`edid`] — the virtual monitor's EDID (authored, never captured);
//! - [`layout`] — the wire layouts, derived from ogkm by compiling its headers;
//! - [`model`] — the physical-RM side: the controls' answers and the channel registry;
//! - [`ports`] — the lock-free state the vCPU, the control link and the worker share;
//! - [`pushbuf`] — the bounded NVDisplay DMA pushbuffer decoder;
//! - [`engine`] — the emulated front end: channels, interlocked updates, arming, completions;
//! - [`inst`] — context-DMA resolution through the guest's display instance memory;
//! - [`pace`] — `display-max-fps`: the capped vblank tick, the meter, the non-flip copy rule.

pub mod caps;
pub mod class;
pub mod color;
pub mod dispcaps;
pub mod edid;
pub mod engine;
pub mod inst;
pub mod layout;
pub mod model;
pub mod pace;
pub mod ports;
pub mod pushbuf;
pub mod regs;
pub mod scanout;
pub mod vramslot;
