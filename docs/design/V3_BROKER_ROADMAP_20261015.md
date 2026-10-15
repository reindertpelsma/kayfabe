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

## C. Headless broker for streaming (planned)
A broker with no window: a backend that exposes a high-quality VNC-like stream over the network, no in-guest tools. Open design
questions: the broker has **no GPU stack by design**, but encoding wants one (NVENC or a CPU encoder), so the encoder is probably a
separate backend process receiving the same frame fds; the network side needs authentication and TLS before it listens on anything
but loopback/unix; cursor travels as a separate stream (the broker already owns it, item A); input comes back through the same
event path as item B.

## D. Clipboard (protocol exists, kayfabe side not wired)
**Measured:** the protocol has `EV_CLIPBOARD` (host to guest, chunked, 1 s window per direction), `CMD_CLIPBOARD` (guest to host),
a transfer cap, and `CMD_CAPS` with the client bit `CLIENT_CLIPBOARD`; the broker side (`nvkvm_broker.c`) implements them;
`crates/kf-broker/src/wire.rs` defines the constants and states they are "not used by this relay". **Open:** what the guest talks to.
Candidates: a spice-vdagent-compatible virtio-serial channel (interop with existing guest tools, check licence), or our own small guest
agent. Hostile-guest rules apply: size cap, text only first, bounded, no unvalidated forwarding.

## E. ACPI / power (partly there)
**Measured:** `EV_CLOSE` carries `CLOSE_POWERDOWN` (graceful) or force; `kf-broker/src/conn.rs` surfaces it to the VMM
("the user closed the window: force = stop the machine now, else an ACPI powerdown"). **Open:** which other ACPI actions the broker
should offer (the owner remembers options to send such things; to be inventoried in `broker/nvkvm_broker.c --help`), and the
VMM-neutral hook through which kayfabe turns them into the VMM's powerdown/reset/sleep (QEMU: QMP `system_powerdown`,
`system_reset`, `system_wakeup`; another VMM: its own).

## F. Audio, VMM-agnostic (new)
Nothing exists. Wanted: a guest audio device model in a neutral crate with a sink trait, played on the host through the broker or
sent over the remote stream (item C). **Open:** the guest-visible device (Intel HDA vs virtio-snd; the Windows driver story decides),
the broker channel (PCM in a memfd ring + clock, new protocol messages and capability bit), the host backend (PipeWire/Pulse), latency
and clock-drift handling. Needs its own design document before any code.

## Suggested order (owner decides)
A (running) -> B screendump + key/mouse -> E (small) -> D -> C -> F.
