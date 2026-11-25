# Display and the broker

**Assumes:** a Linux x86-64 host with KVM, an NVIDIA GPU from Turing onward driven by the open NVIDIA kernel module, and QEMU 10.2.4 with the kayfabe device. The broker also needs a Wayland or X11 desktop session on the host.

A guest can run headless (compute and offscreen rendering need no display), or kayfabe can give it a virtual monitor. This page covers the second case and the host program that puts the picture on your desktop. A headless VM needs no broker and none of this page.

## What the display feature is

- **Opt-in.** The device property `display=on` makes the guest's own NVIDIA display driver drive a virtual NVIDIA display. The default is off. <!-- src: qemu/hw/misc/kf3/kf3.c, docs/STATUS_DETAIL.md section 0 -->
- **What has been run:** a 1920 x 1080 picture at 60 Hz, pixel-exact; a Linux Mint Cinnamon desktop on Wayland with `vkcube` in a window; weston; Xorg with NVIDIA's X driver (Linux guest driver 580.159.04); Linux display planes (overlay, YUV); and Windows 11 desktops. <!-- src: docs/STATUS_DETAIL.md section 0, README.md -->
- **The boot display** (`gop=on`): a small UEFI driver that kayfabe carries in the card's expansion ROM, so the firmware screens, the boot menu and the early console appear on the virtual monitor. It is compiled from source during the Rust build. A Secure Boot guest needs it signed, and its path is given by `gop-efi=`. Set `gop=on` together with `display=on`. <!-- src: firmware/README.md, qemu/hw/misc/kf3/kf3.c, scripts/bench/windows/README.md -->
- **Guest driver versions:** the display layouts were derived for Linux guest driver 580.159.04, and other versions of the guest driver are refused for display. Earlier runs on an AD102 card with Linux drivers 590.48.01 and 595.91.07 were reported; they do not establish that the current source works with them. <!-- src: docs/design/V3_DRIVER_MATRIX.md (2026-09-30 check) -->
- `x11-dispsw` (a display class that X11 desktops need) is experimental and off by default. <!-- src: qemu/hw/misc/kf3/kf3.c -->
- Colour: only an opt-in, bounded SDR subset has been qualified. HDR and arbitrary colour pipelines are not supported. <!-- src: docs/design/V3_DISPLAY.md status 2026-10-06 -->

## Ways to see the screen

The kayfabe QEMU is built without the GTK and SDL front ends, so QEMU opens no window of its own. Two ways remain:

1. **The display broker**, a small separate program that opens a window on your desktop and owns its input and clipboard. QEMU hands it finished frames over a unix socket and keeps no connection to your display server. This is the way to get a desktop window.
2. **VNC.** QEMU registers a graphic console for each virtual head; add `-vnc 127.0.0.1:7` (view only) or `-vnc unix:<path>`. `screendump` on the QEMU monitor takes a PNG. <!-- src: scripts/bench/windows/win_vm.sh, scripts/bench/display/interactive.sh, scripts/bench/build_kf3.sh -->

## Quickstart: the broker in two commands

You need a desktop session (Wayland or X11) and the same user for the broker and for QEMU. Build the broker once (Debian and Ubuntu packages: `libwayland-dev wayland-protocols` for Wayland; `libxcb1-dev libxcb-dri3-dev libxcb-present-dev` for X11, optionally `libxcb-render0-dev` and `libxcb-xinput-dev`; the build prints which backends it kept):

```sh
cd broker && make
```

`make` leaves the programs in `broker/`; `make install` puts the broker itself in `/usr/local/bin` (the `PREFIX` variable changes that). Then, in a terminal of your desktop session:

```sh
nvkvm-display-broker --socket /run/user/$(id -u)/kayfabe-gpu0.sock
qemu-system-x86_64 ... -vga none \
    -device kf3-gpu,id=kf0,...,display=on,gop=on,display-broker=/run/user/$(id -u)/kayfabe-gpu0.sock
```

The broker opens a window when QEMU connects. The binary is called `nvkvm-display-broker` today. Use a socket path inside `/run/user/<uid>/`: the broker creates a missing socket directory (mode 0700, one level), and refuses an existing one that belongs to a user it does not trust or that other users can write to. The broker serves one VM and exits when it disconnects; add `--persist` to keep the window. Press `Ctrl+Alt+G` in the window to grab the keyboard and pointer, `Ctrl+Alt+F` for fullscreen.

- **Same user.** If the broker and QEMU run as different users, name the broker's user on the device with `display-broker-uid=<uid>` (root and QEMU's own user are always accepted).
- **Started from outside the desktop session** (a service, `sudo`, ssh): the broker finds your desktop through the session's environment (`WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` for Wayland, `DISPLAY` for X11), and a terminal of the session has them. Outside the session you need no environment: `--wayland-display NAME` (or an absolute path), `--runtime-dir DIR`, `--x11-display :N`, `--xauthority FILE` and `--allow-uid UID` replace `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DISPLAY`, `XAUTHORITY` and `SUDO_UID`; the variables stay as the fallback when a flag is not given. `--allow-uid` pairs with `display-broker-uid=` on the QEMU side when the two users differ. <!-- src: broker/README.md, broker/nb_launch.c, broker/nvkvm_broker.c -->
- **Test the broker alone** before involving QEMU: `nvkvm-broker-testclient <socket> --present 640x480` prints the handshake and every input event and sends one buffer. It is built by the same `make`.

**Not verified end to end:** the start-up flags were tested without a display server, against stand-in sockets; they have not been run in a real Wayland or X11 session.

The `broker/README.md` file in the repository comes from an earlier project and shows QEMU options (`-display nvkvm-broker,socket=...`) that kayfabe does not use. In kayfabe the socket is the device property `display-broker=<absolute socket path>`.

### Broker options

Every option, with its default, is in the [Command-line reference](cli-reference.md#the-display-broker); `nvkvm-display-broker --help` prints the full text. By default root and the invoking user may connect, and the socket is mode 0600. <!-- src: broker/nvkvm_broker.c (usage text), broker/README.md -->

### Grab, focus and hotkeys

- `CTRL+ALT+G` toggles grab: all keyboard is captured and the pointer is locked. `CTRL+ALT+F` toggles fullscreen. Both chords are consumed and never reach the guest.
- Grab ends automatically on focus loss. This is a security property: a grab that survives focus loss would be a keylogger.
- If a session cannot observe focus loss, grab is not offered. The broker logs one line at start saying whether grab is total, partial (a compositor protocol is missing, so some compositor shortcuts still fire) or not offered.
- Hovering gives an absolute pointer; grab gives a relative one. On GNOME the first grab asks once whether the broker may inhibit shortcuts.

<!-- src: broker/README.md, scripts/bench/display/interactive.sh banner -->

### Clipboard

Off by default. `guest-to-host` lets the guest write your clipboard (it can never read it); `consent` adds host-to-guest only after an explicit paste key. Text only, UTF-8, at most 7168 bytes, rate limited. It also needs QEMU's `-chardev qemu-vdagent` and `spice-vdagent` in the guest. <!-- src: broker/README.md, broker/nvkvm_broker.c (usage text) -->

### Containers

Bind-mount the socket's directory, not the socket file (a bind-mounted socket inode breaks if the broker restarts). Do not bind the X11 or Wayland sockets or `/dev/dri/renderD*` for the display-only profile. With rootless podman, `--userns=keep-id` is the clean answer because the broker checks the peer's user id with `SO_PEERCRED`. A complete recipe for a sandboxed QEMU with a broker is not documented yet. <!-- src: broker/README.md -->

### The guest's cursor

The broker draws the guest's cursor (hover: the host pointer shows the image; grab: an overlay surface), including XOR cursors such as the Windows text I-beam where it can read the frame; where it cannot, it approximates and logs which. <!-- src: broker/README.md -->

### Refresh rate

`display-max-fps` caps the emulated monitor's refresh rate (24 to 75 Hz; default a cap of 75). The broker's `--refresh auto` tells the guest the host's presentation rate (a 144 Hz host is told 72 Hz), and the guest paces its virtual refresh to it. <!-- src: qemu/hw/misc/kf3/kf3.c, broker/README.md -->

## Windows

For Windows, the recorded way to a visible desktop is the one in [Run a Windows guest](windows-guest.md): the guest settings, the signed boot display driver and `display=on`, with the broker window or VNC.

## Optional helper: the project's own launchers

`scripts/bench/display/interactive.sh` is the author's one-command launcher for a Linux guest shown in a broker window, and `scripts/bench/windows/windows_broker.sh` does the same for the Windows test guest. They are written for the author's test host, run as root, and are not needed for the steps above. The Linux launcher's `prep` step builds the broker from a pinned revision of the earlier project's repository instead of from `broker/`; if you build `broker/` yourself, pass its path with `BROKER_BIN`. Its guest preparation is a useful example for your own image: the kernel arguments `console=tty1 console=ttyS0 nvidia-drm.modeset=1 nvidia-drm.fbdev=1`, `nvidia_drm` in `/etc/modules-load.d`, and an `/etc/X11/xorg.conf` that selects `Driver "nvidia"` for the device at `BusID "PCI:0:2:0"`. Without that `xorg.conf`, Xorg picked the `modesetting` driver, whose glamor failed on `nvidia-drm`, and the display manager restarted X forever. <!-- src: scripts/bench/display/interactive.sh, scripts/bench/windows/windows_broker.sh, broker/IMPORTED.md -->

## Not documented yet

- A broker package or a service unit.
- Display coverage beyond the Linux guest driver named above.
- HDR.
