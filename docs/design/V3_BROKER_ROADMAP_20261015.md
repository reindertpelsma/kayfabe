# V3_BROKER_ROADMAP_20261015 — what the broker becomes, and what kayfabe has to wire for it

**STATUS: DESIGN-ONLY, 2026-10-15 (owner direction; nothing below except item A is started).** The broker program now lives
in this repository (`broker/`, imported from nvkvm-pv `badf2d7`, see `broker/IMPORTED.md`) and is the version to change.
Rule for every item: **VMM-agnostic** (kayfabe's logic sits in `kf-disp` / `kf-broker` / neutral crates; `kf-qemu` only adapts).
**Measured** = read from the protocol header (`broker/common/nvkvm_broker_proto.h`) or `crates/kf-broker` at this commit.
**Inferred / open** = marked as such.

## A. Cursor owned by the broker (IN PROGRESS, agent on branch `claude/cursor-hover-20261015`, host `/data/claude-cursor2/wt`)
Kayfabe reports the guest cursor (image, hot spot, position, visible; plane off = hidden, the guest then draws its own); the
broker draws it in every mode (hover: host pointer, position ignored; grab: the broker draws at kayfabe's position, Wayland
subsurface, X11 best effort). XOR cursors: kayfabe converts to an "inverting mask" image, the broker approximates (black fill, white
outline). Kayfabe stops composing cursors into frames when the broker advertises `CAP_CURSOR_STREAM`; the old composing path stays
as the fallback for older brokers until the owner decides to delete it. The XOR/non-XOR hover/hide bug lives in that fallback.

## B. Broker control CLI (queued, after A is verified live)
A small tool talking to the running broker over a local unix socket (same uid, mode 0600; whoever connects sees the guest screen).
- `screendump`: the broker hands back a **read-only fd** of the current frame (+ format, modifier, and the cursor state); the CLI
  has its own library access (EGL/Vulkan/gbm), reads the buffer back, composes the cursor and writes PNG/JPEG. No GPU stack in the
  broker, no VMM involvement. Details to get right: hold the slot's `RELEASE` until the CLI closes its copy (bounded, with a timeout),
  a read-only handle, and **open: importing the NVIDIA VRAM export from a standalone tool is untested** (mutter does it for the window).
  This replaces QEMU's `screendump` (QEMU's console stays, small, isolated in kf-qemu, until this exists).
- `key` / `mouse`: inject keystrokes and pointer events through the broker's normal event path (so the input policy of
  `OWNER_RULINGS` §V applies unchanged: grab, focus, shortcuts).
- Later: set broker options at runtime (e.g. cursor style). A capability change mid-connection needs an event to the VMM; do it
  only after A is verified.

## C. Headless broker for streaming (planned; owner scope decision 2026-10-15: LOCAL NETWORK ONLY)
A broker with no window: a backend that exposes a high-quality VNC-like stream on the **local network**, no in-guest tools. **Out of
scope, by owner decision: making it reachable from the internet.** Whoever wants that puts a mature gateway in front (Apache
Guacamole or similar); kayfabe does not wire one, and does not build TLS termination, user management or NAT traversal.
What that scope still needs: it listens only where it is told to (default unix socket / loopback; a LAN address needs an explicit
option), and a simple shared secret so that the LAN is not an open console (the transport security of anything beyond the LAN is the
gateway's job). Open design questions: (1) **which protocol**: a gateway like Guacamole already ingests VNC (RFB), RDP and SSH, so
speaking RFB (with its better encodings; H.264 extensions exist in some servers and clients, not verified here) would work with such
gateways and standard viewers without us wiring anything; a custom protocol would not. Inferred, not decided. (2) the broker has **no GPU
stack by design**, but encoding wants one (NVENC or a CPU encoder), so the encoder is probably a separate backend process receiving the
same frame fds. (3) the cursor travels as a separate stream (the broker already owns it, item A; RFB has a cursor pseudo-encoding);
input comes back through the same event path as item B.

## D. Clipboard (protocol exists, kayfabe side not wired; owner decision 2026-10-15: the HYPERVISOR'S OWN GUEST TOOLS, interface abstract)
**Measured:** the protocol has `EV_CLIPBOARD` (host to guest, chunked, 1 s window per direction), `CMD_CLIPBOARD` (guest to host),
a transfer cap, and `CMD_CAPS` with the client bit `CLIENT_CLIPBOARD`; the broker side (`nvkvm_broker.c`) implements them;
`crates/kf-broker/src/wire.rs` defines the constants and states they are "not used by this relay".
**Decision:** kayfabe writes no guest agent and no guest-side protocol. The clipboard travels through whatever the hypervisor
already ships for its guests; under QEMU that is the SPICE agent (`spice-vdagent` on Linux, SPICE guest tools on Windows, installed
by the user in the image). Kayfabe defines only an **abstract, VMM-neutral clipboard port** (a trait in a neutral crate: host text in,
guest text out, size-capped, bounded, text first) that the broker relay drives; each VMM adapter implements it with that VMM's
mechanism. **Inferred, not verified here:** under QEMU the adapter would use QEMU's own agent chardev (`qemu-vdagent` on a
virtio-serial port, QEMU >= 6.1) and register as the clipboard peer of QEMU's clipboard API; with `-display none` there is no other
peer, so nothing else competes. To check before building: that this works with `-display none`, that the guest tools run in the Win11
image (QGA's MSI did not, see the diagnostics notes; vdagent is separate software), and that the guest's clipboard bytes are
size-capped before they reach the broker (hostile guest).

## E. ACPI / power (partly there)
**Measured:** `EV_CLOSE` carries `CLOSE_POWERDOWN` (graceful) or force; `kf-broker/src/conn.rs` surfaces it to the VMM
("the user closed the window: force = stop the machine now, else an ACPI powerdown"). **Open:** which other ACPI actions the broker
should offer (the owner remembers options to send such things; to be inventoried in `broker/nvkvm_broker.c --help`), and the
VMM-neutral hook through which kayfabe turns them into the VMM's powerdown/reset/sleep (QEMU: QMP `system_powerdown`,
`system_reset`, `system_wakeup`; another VMM: its own).

## F. Audio (new; owner decision 2026-10-15: same model as the clipboard, HYPERVISOR-AGNOSTIC interface)
Nothing exists yet. **Decision:** kayfabe defines an abstract, VMM-neutral **audio sink interface** (a trait in a neutral crate:
PCM format and clock in, optionally capture later, bounded buffers) and the broker carries it (new protocol channel and capability
bit; played on the host, or sent into the remote stream of item C). **The guest-visible audio device is the hypervisor's own:**
under QEMU one of its audio devices, under Cloud Hypervisor whatever audio device that hypervisor offers; each VMM adapter connects
its device to the neutral interface. Kayfabe writes no guest driver and no guest agent.
**Open (inferred, none of it verified here):** (1) under QEMU, the device is probably Intel HDA (Windows has an inbox HDA driver) or
virtio-snd (needs a Windows driver from the virtio-win project); QEMU's audio backends are compiled in, so the adapter is probably a
small audio backend in our QEMU tree rather than a separate process, to be checked in the QEMU source. (2) whether Cloud Hypervisor
offers any audio device at all (not checked), and if not, that adapter does not exist yet. (3) the broker channel: PCM in a shared
ring plus a clock, host playback through PipeWire/PulseAudio, latency and clock-drift handling (resampling or drop/insert). (4) a
hostile guest cannot stall or flood the host: bounded ring, drop on overrun, rate limits. Needs its own design document before code.

## Suggested order (owner decides)
A (running) -> B screendump + key/mouse -> E (small) -> D -> C -> F.
