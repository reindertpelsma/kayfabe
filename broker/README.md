# nvkvm display broker

A small privileged-side program that owns the window, the input and the
clipboard, so the VMM does not have to. QEMU hands it finished frames over a
unix socket and holds no connection to your display server at all.

STATUS: BETA CANDIDATE, updated 2026-10-20. No supported external-user path.
CODE READ: kayfabe's virtual display uses this broker even for a plain desktop
QEMU. Run it as the desktop user; it owns the compositor connection, input and
clipboard. QEMU connects through the kf3 **device properties**, not a special
QEMU display backend. A broker does not sandbox an otherwise unconfined QEMU.
For install/first boot see [INSTALL](../docs/INSTALL.md) and
[RUNNING_KAYFABE](../docs/RUNNING_KAYFABE.md).

CODE READ: build from the repository root with `make -C broker -j2`.
`broker/nvkvm-display-broker --version` prints the canonical VERSION and exits
before opening a compositor/socket; install matching QEMU and launcher parts.
Wayland needs libwayland-dev and wayland-protocols; X11 needs libxcb1-dev,
libxcb-dri3-dev and libxcb-present-dev. libxcb-render0-dev supplies scaling and
cursor support, and libxcb-xinput-dev supplies relative input. The build report
states which backends are present. The broker does not create a CUDA context.

CODE READ: GNOME/mutter Wayland with wp_presentation is the source-measured
pacing path; actual capability is announced, not inferred from the desktop name.
X11 remains a fallback and does not advertise CAP_TICK. Without presentation
feedback, the CLOCK_ONLY/frame-callback path and VMM fallback retain their
existing behavior. No tick source, timer constant or cursor behavior is changed
by this install work. `--stats-line` is measurement only and off by default.

Design rationale, threat model and the hardware verification log:
[display design](../docs/design/V3_DISPLAY.md).
Wire protocol: [`docs/reference/broker-protocol.md`](../docs/reference/broker-protocol.md).

## Grab, focus and hotkeys

Owned by the broker, never by the VMM.

- **`CTRL+ALT+G`** toggles grab. Under grab all keyboard is captured and the
  pointer is locked; ungrabbed, absolute coordinates are sent only while the
  pointer is over the window.
- **`CTRL+ALT+F`** toggles fullscreen.
- Both chords, and their key-ups, are **consumed** — they never reach the guest.
- Input flows **only while the window is active**.

**Grab exits automatically on focus loss. This is a security property, not a
convenience:** a grab that survives focus loss is a keylogger — you alt-tab to
your password manager and the guest keeps receiving the keystrokes. Focus-out
is `FocusOut` on X11 and `wl_keyboard.leave` on Wayland. On the transition the
broker synthesises a release for every key the client believes is down, so no
modifier is left latched.

**If a session cannot observe focus loss, grab is not offered at all.**
`CTRL+ALT+G` refuses and says why.

**Capability is detected and announced at startup**, in one plain line, and
again in `HELLO`. On Wayland `keyboard-shortcuts-inhibit`, `pointer-constraints`
and `relative-pointer` are optional protocols a compositor may not advertise; a
partial grab is fine **if announced**, and a partial grab claimed as total is
not. The startup log says exactly one of:

```
GRAB IS TOTAL: under CTRL+ALT+G every key reaches the guest and the pointer is locked to the window.
GRAB IS PARTIAL: compositor is missing: keyboard-shortcuts-inhibit (Super and other compositor shortcuts WILL still fire under grab); ...
GRAB IS NOT OFFERED: this session cannot report focus loss, ...
```

Nothing any X client can do intercepts input the kernel handles below X (SysRq,
VT switching on some setups). That is in the caveat string rather than claimed
away.

---


## Running it

### Build

```bash
cd broker && make          # prints which backends it kept
make check                     # build + selftest.sh
bash headless_present_test.sh  # real import + present, headless, no monitor
```

`selftest.sh` covers everything except the one thing that matters most — the
actual dma-buf import and present — because that needs a real compositor.
`headless_present_test.sh` covers it without one attached: weston on its
headless backend, and a bounded frame stream whose **last** frame must be
presented rather than left unflushed.

It also reaches a tier that is otherwise untestable on NVIDIA. Mutter
advertises XR24 + modifier `0x0` (LINEAR) and **NVIDIA then refuses to import
it**, so on an NVIDIA-only host every frame falls back to shm or to the
block-linear native tier and the linear path never runs. llvmpipe and AMD both
import it happily:

```bash
bash headless_present_test.sh                    # llvmpipe — needs no GPU, runs in CI
NVKVM_TEST_SOFTWARE=0 bash headless_present_test.sh   # real driver, still headless
NVKVM_TEST_NODE=/dev/dri/renderD129 ...          # pick the allocator explicitly
```

The allocator node defaults to a **non-NVIDIA** one when the box has several, since a
linear buffer allocated on NVIDIA has no consumer on an NVIDIA-only host. Skips
with status 77 when weston or a render node is missing.

Dependencies — note what is **not** there: no libdrm, no EGL, no GL, no GBM.

```
Wayland backend    libwayland-dev wayland-protocols
X11 backend        libxcb1-dev libxcb-dri3-dev libxcb-present-dev
X11 rel. motion    libxcb-xinput-dev     (optional; without it, no relative
                                          motion under grab, and it says so)
```

### Run

```bash
# in your desktop session, as your normal user
mkdir -p "$XDG_RUNTIME_DIR/kayfabe"
broker/nvkvm-display-broker --socket "$XDG_RUNTIME_DIR/kayfabe/broker.sock" --backend auto

# then, in another terminal as the same user (normal VM args omitted)
qemu-system-x86_64 ... -device kf3-gpu,display=on,display-broker="$XDG_RUNTIME_DIR/kayfabe/broker.sock",display-broker-uid="$(id -u)"
```

Before involving QEMU, prove the broker alone works:

```bash
broker/nvkvm-broker-testclient "$XDG_RUNTIME_DIR/kayfabe/broker.sock" --present 640x480
```

That prints the handshake and every input event, and sends one buffer. It
separates a broker problem from a QEMU problem, which is otherwise two
variables at once. Its `--bad-*` options are the ways a hostile VMM can lie;
each one must be rejected.

Options: `--backend auto|wayland|x11|test`, `--size WxH`, `--title`,
`--allow-user`, `--allow-group`, `--drop-user`, `--verbose`.
`--allow-group` compares the primary gid reported by `SO_PEERCRED`; Linux does
not expose the peer's supplementary groups through that interface.
By default root and the invoking user may connect.

Clipboard modes are `off` (default), `guest-to-host`, and `consent`. `consent`
adds host-to-guest transfer only after an explicit paste chord. Text is UTF-8
and capped at 7 KiB (7168 bytes) in either direction. Automatic `full` sync is
not implemented and is rejected rather than behaving exactly like `consent`.
Both display backends implement clipboard transfer in both directions: Wayland
through `wl_data_device`, X11 through `CLIPBOARD` selection ownership and a
single-property transfer (an `INCR` stream is refused rather than truncated,
since it only appears well above the 7 KiB cap), and `x11_open()` advertises
`NB_SESSION_CLIP_G2H | NB_SESSION_CLIP_H2G` for the same reason Wayland does —
the boundary the broker draws must not depend on which display server the host
happens to run. This paragraph used to say X11 rejected every non-off mode; it
had not been true since the X11 clipboard landed, and a doc that under-states
what a security boundary permits is worse than one that says nothing. The
explicit test backend has a simulated clipboard for regression tests.

### The guest's cursor in every input mode (`CAP_CURSOR_STREAM`)

A VMM that sees `CAP_CURSOR_STREAM` (HELLO bit 12) in this program's HELLO stops
drawing the guest's cursor into the picture in every input mode and only reports
it: the image (`CMD_CURSOR` `SET`), its position (`MOVE`, the image's top-left
on the guest frame), and whether there is one (`HIDE`/`SHOW`; hidden also means
the guest's cursor plane is off and the guest draws a software cursor into the
frame itself). This program draws it:

- **hover** (absolute pointer): the host pointer shows the image; `MOVE` is
  ignored, the host pointer is the guest's pointer.
- **grab** (locked, relative pointer): this program has no pointer of its own, so
  it draws the image at the last `MOVE`, scaled with the frame, as an overlay
  surface over the content that never touches the frame's buffer (Wayland: a
  `wl_subsurface`).
- **XOR cursors** (the Windows text I-beam inverts what is under it, which no
  image can express): the VMM sends the inversion mask with `CURSOR_F_XOR` (alpha
  `0xff` where it inverts).  Where this program can READ the frame it presents it
  composes the TRUE inversion itself, `below ^ colour` for the mask's pixels only,
  and shows it as a small overlay patch committed with the frame (a synchronised
  subsurface); in hover the host pointer is hidden while it is up and comes back
  the moment it is not.  Readable means a memfd carrying `F_SEAL_SHRINK` (the wl_shm
  tier) or a LINEAR dma-buf whose exporter is system memory (`udmabuf`, the
  system dma-heap), mapped read-only and read ONLY for the cursor's rows, bracketed
  by `DMA_BUF_IOCTL_SYNC` (the one exception to "the broker never touches frame
  memory", bounded as `nb_frame.c` and the design doc say).  Anything else (a
  tiled or device-memory dma-buf, a failed mapping) falls back to opaque black
  with a one-pixel opaque white outline, and the log says which, once per change:
  `cursor: XOR composed by the broker (...)` / `cursor: XOR approximated ...
  (frame not readable: <reason>)`.  An all-clear mask draws nothing.
  `docs/design/V3_BROKER_XOR_CURSOR_20261017.md`.

Backends that cannot overlay do not advertise the bit, and the VMM keeps
composing the cursor into the frame as it always did (the Wayland and test
backends advertise it; the X11 backend does not, see the log line at start).
`MOVE` and `F_XOR` without the capability are framing violations.

### The display tick and lock-step frame requests (`CAP_TICK`)

A VMM that sees `CAP_TICK` (HELLO bit 13) may reply `CMD_CAPS` with
`CLIENT_TICK` (bit 1; `height` = the most Hz its virtual monitor carries, 0 =
75). From then on this program sends `EV_TICK` once per (decimated) display
refresh while the window is shown, whether or not anything is wanted, so the
VMM's emulated vblank can follow the host's real refresh (it falls back to its
own clock after 50 ms of silence, and takes the ticks again when they resume).
A tick carrying `WANT_FRAME` opens one request, numbered by its `tick_seq`; the
VMM answers it with exactly one `COMMIT` whose `seq` is that number, or one
`CMD_NO_CHANGE`; the next request opens only after the answer **and** the
frame's presentation (or after 100 ms). Nothing is sent to a VMM that did not
opt in: it gets today's exchange. The decisions are pure and table-tested
(`nb_tick.c`, `test/test_tick.c`, 28 mutants in `test/mutate_tick.py`); the
real-socket tests are `test/test_tick.py` (floods, hostile replies, a peer that
never reads) and, against a real compositor, `make check-tick-probe`.
`docs/design/V3_DISPLAY_PACING_20261017.md` section 11 is the design.

* The host refresh is divided so the tick never runs faster than the VMM's cap:
  144 Hz host -> every 2nd feedback, and (`--refresh auto`) the mode the guest is
  told is 72 Hz. A feedback closer than half a host period to the last accepted
  one, a duplicate timestamp, or a backwards clock ticks nothing.
* Wayland: `wp_presentation` feedback is the tick; the pump is a 1x1-damage
  re-commit of the buffer already attached (an empty commit gets a frame callback
  at once and a `discarded` feedback, never a timestamp: MEASURED on mutter and
  wlroots), driven by the `presented` it produced, restarted by a 250 ms watchdog
  after the window was hidden. Without `wp_presentation` the frame callback stands
  in (`CLOCK_ONLY`). X11 does not advertise the bit.
* The test backend's clock is virtual (stdin: `t <present_ns> <refresh_ns>`,
  `F <n> <refresh_ns> <step_ns>`, `P`, `N <ns>`, `S`); `NB_TEST_NO_TICK=1` plays a
  broker that predates the bit.

### Putting the socket into a container

The broker socket is the **only** display-related thing the VMM needs. Bind-mount
the directory (not the socket file — a bind-mounted socket inode breaks if the
broker restarts and recreates it):

```bash
# podman / docker
--volume /run/nvkvm:/run/nvkvm

# systemd-nspawn
--bind=/run/nvkvm

# bubblewrap
--bind /run/nvkvm /run/nvkvm
```

and **do not** bind `/tmp/.X11-unix`, `$XDG_RUNTIME_DIR/wayland-*`, or
`/dev/dri/renderD*` for the display-only profile.

The capture/NVENC profile additionally needs `/dev/dri/renderD*` and the NVIDIA
userspace stack inside the container — but still no display server.


### `SO_PEERCRED` and rootless podman, precisely

`SO_PEERCRED` reports the peer's uid **as seen in the broker's user
namespace**. Measured, with the broker running as uid 1000:

| rootless invocation | broker sees | result |
|---|---|---|
| default (container root) | `uid 1000` | **works** — container root maps to the host user |
| `--user 1000:100` | *nothing* | **fails earlier than you expect** |
| `--userns=keep-id` | `uid 1000` | **works** |

The middle row is the trap, and it does not fail where §3 rule 7 would suggest:
container uid 1000 maps into the **subuid** range (100000+), and the socket is
`0600` owned by the broker's uid, so `connect()` returns **`EPERM` before
`SO_PEERCRED` is ever consulted**. Adding the mapped uid to `--allow-user` is
therefore *not enough* on its own — the socket's mode has to let that uid open
it too. **`--userns=keep-id` is the clean answer** and needs no `--allow-user`
at all.

### Fullscreen is where the frame reaches a hardware plane

Neither stack lets a client *request* a hardware plane; you can only qualify
for one, and the compositor decides:

- **Wayland**: a fullscreen, opaque, unoccluded surface whose format and
  modifier suit the plane is promoted to **direct scanout**. The broker sets an
  opaque region on every commit for exactly this reason.
- **X11**: a compositing WM **unredirects** a fullscreen window and the frame
  goes straight to the CRTC.

`CTRL+ALT+F` asks for the precondition. Whether it is granted is the
compositor's call, and it will not tell you.

---


## What it deliberately does not do

- **No DRM lease, no KMS, no mode-setting.** An earlier design leased a
  connector; findings 2 and 4 killed it. A lease hands the VM a *whole monitor*
  because neither X11 nor Wayland can yield a plane for a *window*.
- **No copy fallback.** If a compositor has no `zwp_linux_dmabuf_v1`, or an X
  server has no DRI3, the broker refuses to start and says so. A copy path
  would need a GL context in the privileged process, which is the thing being
  removed.
- **No allocation on behalf of a client**, and no size, count or index ever
  taken from the wire.
- **No cursor plane.** The guest head has none (`drm_simple_display_pipe_init`
  creates only the primary) and the supported guest composites its own cursor
  into the scanout buffer. ~~The broker has no cursor path at all~~ -- since
  2026-10-03 it **can** show a guest cursor that a VMM sends as `CMD_CURSOR`
  (advertised by `CAP_CURSOR`): an ARGB image of at most 256x256, copied out of
  a memfd, scaled with the frame, shown as the host pointer while hovering and
  hidden under grab. nvkvm-pv's own VMM sends none yet, so with it nothing
  changes; the path exists for a VMM whose guest does have a cursor plane.
- **No reverse channel beyond the commands in the protocol header.** Every
  field the privileged side reads is listed in
  [`docs/reference/broker-protocol.md`](../docs/reference/broker-protocol.md),
  and that list is meant to stay short. (This line used to say "the three
  commands"; there are seven now -- `ATTACH`, `COMMIT`, `WINDOW`, `CLIPBOARD`,
  `CAPS`, `QUERY_FORMAT`, `CURSOR` -- each fixed-size, and only `ATTACH` and
  `CURSOR`'s `SET` carry an fd.)
