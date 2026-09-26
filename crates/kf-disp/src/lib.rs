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
//! - [`edid`] — the virtual monitor's EDID (authored, never captured);
//! - [`pushbuf`] — the bounded NVDisplay DMA pushbuffer decoder.

pub mod edid;
pub mod pushbuf;
