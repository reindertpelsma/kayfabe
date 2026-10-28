# Display and the broker

A guest can run headless (compute and offscreen rendering need no display), or kayfabe can give it a virtual monitor. This page covers the second case and the host program that puts the picture on your desktop.

## What the display feature is

- **Opt-in.** The device property `display=on` makes the guest's own NVIDIA display driver drive a virtual NVIDIA display. The default is off. <!-- src: qemu/hw/misc/kf3/kf3.c, docs/STATUS_DETAIL.md section 0 -->
- The guest's driver programs the display; kayfabe decodes those display channels itself and scans the result out. Display channels have no GPU work behind them: they are emulated channels. <!-- src: ARCHITECTURE.md (kf-disp row, channel kinds) -->
- **What has been run:** a pixel-exact 1920 x 1080 scanout at 60 Hz; a Linux Mint Cinnamon desktop on Wayland with `vkcube` in a window; weston; Xorg with NVIDIA's X driver [measured 2026-09-30, Linux guest 580.159.04]. Linux display planes (overlay, YUV) and Windows 11 desktops [measured, see the README]. <!-- src: docs/STATUS_DETAIL.md section 0, README.md -->
- **The boot display** (`gop=on`): a small UEFI driver that kayfabe carries in the card's expansion ROM, so the firmware and boot screens, the boot menu and the early console appear on the virtual monitor. It is compiled from source during the Rust build. For Secure Boot guests it has to be signed, and its path is given by `gop-efi=`. <!-- src: firmware/README.md, qemu/hw/misc/kf3/kf3.c, scripts/bench/windows/README.md -->
- **Per-guest-driver layouts:** the display wire layouts were derived for Linux guest driver 580.159.04 only (as of 2026-09-30), and any other guest version is refused. Check before relying on another version. <!-- src: docs/design/V3_DRIVER_MATRIX.md (2026-09-30 check) -->
- `x11-dispsw` (a display class that X11 desktops need) is experimental and off by default. <!-- src: qemu/hw/misc/kf3/kf3.c -->
- Colour: only an opt-in, bounded SDR subset has been qualified; HDR and arbitrary colour pipelines are not claimed. <!-- src: docs/design/V3_DISPLAY.md status 2026-10-06 -->

## Ways to see the screen

The build in [Building from source](building.md) disables the GTK and SDL front ends. What is left:

1. **VNC.** QEMU registers a graphic console for each virtual head. The harnesses use `-vnc`: the Windows harness exposes it on a unix socket (`-vnc unix:<run>/vnc.sock`), and `KF_VNC=1` in the interactive launcher adds `-vnc 127.0.0.1:7` for view-only use. `win_vm.sh shot` takes a PNG with `screendump`. <!-- src: scripts/bench/windows/win_vm.sh, scripts/bench/display/interactive.sh, scripts/bench/build_kf3.sh -->
2. **The display broker**, a separate program that opens a window on your desktop and owns its input. This is the path the repository's interactive launchers use.

## The broker

The broker is a small program that owns the window, the input and the clipboard, so the VMM does not have to. QEMU hands it finished frames over a unix socket and holds no connection to your display server at all. It was imported from the author's `nvkvm-pv` repository and is built from `broker/`. <!-- src: broker/README.md, broker/IMPORTED.md -->

**Why a broker, and not QEMU's own window?** Kayfabe's QEMU is built without the GTK and SDL front ends on purpose (see above), so the broker is the way to get a desktop window; VNC stays as the second path. The reasons the repository records:

- **No lock-in to one hypervisor.** The owner's direction of 2026-10-01 is that frames leave through a hypervisor-agnostic broker, not QEMU's SDL or GTK UI, and hypervisor independence is a goal ([Goals and plans](goals.md)). In the broker relay only about 230 lines in the QEMU device are QEMU-specific; the connection logic, the frame ring and the input policy are VMM-neutral Rust. <!-- src: docs/OWNER_RULINGS.md section C item 6, section AO; docs/design/V3_DISPLAY.md section 8.0 -->
- **The VMM stays away from your display server.** QEMU holds no connection to it at all, which matters most when QEMU is confined (a container, `systemd-nspawn`, `bubblewrap`), the extra layer the [Security model](security.md) recommends. <!-- src: broker/README.md; docs/howto/security.md -->
- **One display path to maintain.** Pointer grab, absolute or relative input, buttons, the wheel and re-sync are decided in one place (`kf-broker`); only a thin shim names the VMM, so a second hypervisor adds a shim rather than a second display path. <!-- src: docs/OWNER_RULINGS.md section V (owner, 2026-10-08) -->
- **The broker owns the cursor and the pacing.** It draws the guest's cursor, including true inverting cursors, and kayfabe composes none; the design makes the broker the frame clock (not built yet), with kayfabe's clock as the fallback after 50 ms of silence. The display design notes record problems with QEMU's own front ends: SDL and GTK read a guest cursor with different conventions, GTK warps the host pointer under a grab, and GTK's window resizing re-modes the guest. <!-- src: docs/OWNER_RULINGS.md sections AJ, AK, AL; docs/design/V3_DISPLAY.md cursor-format and ui_info notes -->
- **Frames are copied on the GPU into a frame kayfabe owns.** For a compositor on the same NVIDIA GPU the frame never crosses to the CPU, and the guest cannot change it after the copy; other compositors get a copy in host RAM, with shared memory as the last fallback. <!-- src: docs/OWNER_RULINGS.md section L -->

The imported `broker/README.md` still says to use `-display sdl` for an unconfined QEMU. That advice is nvkvm-pv's and does not apply here: this QEMU has no SDL.

Note that `broker/README.md` shows QEMU options of the `nvkvm-pv` project (`-display nvkvm-broker,socket=...`). **Kayfabe does not use those.** In kayfabe the socket is a property of the device: `display-broker=<absolute socket path>`.

### Build and run

```sh
cd broker && make          # prints which backends it kept (Wayland, X11)
make check                 # build + selftest
nvkvm-display-broker --socket /run/user/<uid>/nvkvm/display.sock --backend auto --persist --title "kayfabe guest"
```
<!-- src: broker/README.md, scripts/bench/display/interactive.sh -->

Run it in your desktop session as your normal user. Then give QEMU the same socket and your uid:

```
-device kf3-gpu,...,display=on,gop=on,display-broker=/run/user/<uid>/nvkvm/display.sock,display-broker-uid=<uid>
```
<!-- src: scripts/bench/display/interactive.sh -->

`display-broker-uid` names one more uid accepted as the broker; uid 0 and QEMU's own effective uid are always accepted, and the owner of the socket's directory is not. <!-- src: qemu/hw/misc/kf3/kf3.c -->

Test the broker alone before involving QEMU: `nvkvm-broker-testclient /run/nvkvm/display.sock --present 640x480` prints the handshake and every input event and sends one buffer. <!-- src: broker/README.md -->

Broker options (from the program's own usage text): `--socket PATH`, `--socket-mode OCTAL` (default 0600), `--socket-group NAME`, `--backend auto|wayland|x11|test`, `--size WxH` (default 1920x1080), `--title TEXT`, `--allow-user NAME`, `--allow-group NAME`, `--drop-user WHO`, `--fullscreen`, `--scale aspect|stretch|none`, `--persist` (keep the window when the VMM disconnects), `--clipboard off|guest-to-host|consent`, `--clipboard-trigger LIST`, `--refresh auto|none|<Hz>`, `--resolution auto|none|WxH`, `--present-mode`, `--verbose`. Run it with `--help` for the full list. <!-- src: broker/nvkvm_broker.c (usage text), scripts/bench/display/interactive.sh -->

By default root and the invoking user may connect. The socket is mode 0600. <!-- src: broker/README.md, broker/nvkvm_broker.c -->

### Grab, focus and hotkeys

- `CTRL+ALT+G` toggles grab: all keyboard is captured and the pointer is locked. `CTRL+ALT+F` toggles fullscreen. Both chords are consumed and never reach the guest.
- Grab ends automatically on focus loss. This is a security property: a grab that survives focus loss would be a keylogger.
- If a session cannot observe focus loss, grab is not offered. The broker logs one line at start saying whether grab is total, partial (a compositor protocol is missing, so some compositor shortcuts still fire) or not offered.
- Hovering gives an absolute pointer; grab gives a relative one. On GNOME the first grab asks once whether the broker may inhibit shortcuts.

<!-- src: broker/README.md, scripts/bench/display/interactive.sh banner -->

### Clipboard

Off by default. `guest-to-host` lets the guest write your clipboard (it can never read it); `consent` adds host-to-guest only after an explicit paste key. Text only, UTF-8, at most 7168 bytes, rate limited. It also needs QEMU's `-chardev qemu-vdagent` and `spice-vdagent` in the guest. <!-- src: broker/README.md, broker/nvkvm_broker.c (usage text) -->

### Containers

Bind-mount the socket's directory, not the socket file (a bind-mounted socket inode breaks if the broker restarts). Do not bind the X11 or Wayland sockets or `/dev/dri/renderD*` for the display-only profile. With rootless podman, `--userns=keep-id` is the clean answer because the broker checks the peer's uid with `SO_PEERCRED`. <!-- src: broker/README.md -->

### The guest's cursor

The broker draws the guest's cursor (hover: the host pointer shows the image; grab: an overlay surface), including XOR cursors such as the Windows text I-beam where it can read the frame; where it cannot, it approximates and logs which. Kayfabe composes no cursor itself when the broker advertises this. <!-- src: broker/README.md -->

### Refresh rate

`display-max-fps` caps the emulated monitor's vblank (24 to 75 Hz; default 75). The broker's `--refresh auto` tells the guest the host's presentation rate (a 144 Hz host is told 72 Hz), and the guest paces its virtual vblank to it. The kayfabe side falls back to its own clock after 50 ms without ticks. <!-- src: qemu/hw/misc/kf3/kf3.c, broker/README.md -->

## A Linux desktop on a broker window

`scripts/bench/display/interactive.sh` is the author's one-command launcher: a Linux guest shown in a broker window on the live desktop, with keyboard, tablet and mouse. It must run as root (QEMU opens `/dev/nvidia*`, the tap and the GPU lock) while the desktop session is logged in; it stops nothing of the session. <!-- src: scripts/bench/display/interactive.sh -->

```sh
scripts/bench/display/interactive.sh prep      # once: build the broker into /opt/nvkvm-broker; make the guest disk
scripts/bench/display/interactive.sh           # boot; the window appears; Ctrl-C stops it
scripts/bench/display/interactive.sh broker    # only (re)start the broker
scripts/bench/display/interactive.sh stop
```
<!-- src: scripts/bench/display/interactive.sh -->

What shows, in order: the firmware (TianoCore), the grub menu, the kernel console, then `nvidia-drm`'s console and the Cinnamon desktop on the stock NVIDIA X driver. Environment: `KF3_REV`, `KF_IMG`, `KF_RAM_MB` (8192), `KF_FB_MB` (8192), `KF_SMP` (6), `KF_EXTRA_QEMU`, `BROKER_ARGS`, `KF_BROKER_BACKEND`, `KF_VNC`. <!-- src: scripts/bench/display/interactive.sh -->

Its QEMU line uses `-vga none`, a `kf3-gpu` device with `display=on,gop=on,x11-dispsw=on` and the broker properties above, plus `virtio-keyboard-pci`, `virtio-tablet-pci,display=kf0,head=0` and `virtio-mouse-pci`. <!-- src: scripts/bench/display/interactive.sh -->

What its `prep` step puts in the guest (an example for your own image): the `cinnamon-core`, `lightdm` and X packages; the kernel arguments `console=tty1 console=ttyS0 nvidia-drm.modeset=1 nvidia-drm.fbdev=1`; `nvidia_drm` in `/etc/modules-load.d`; an autologin; and an `/etc/X11/xorg.conf` that selects `Driver "nvidia"` for the device at `BusID "PCI:0:2:0"`. Without that xorg.conf, Xorg picked the `modesetting` driver, whose glamor failed on `nvidia-drm`, and lightdm restarted X forever [measured 2026-10-08]. <!-- src: scripts/bench/display/interactive.sh (prep) -->

Its `prep` also builds the broker from a pinned revision of the `nvkvm-pv` repository rather than from `broker/` in this tree (`KF_NVPV_REV`, default `badf2d7`). If you build `broker/` yourself, pass its path with `BROKER_BIN`. <!-- src: scripts/bench/display/interactive.sh, broker/IMPORTED.md -->

## A Windows desktop on a broker window

`scripts/bench/windows/windows_broker.sh` shows the Windows 11 guest in a second broker window beside the Linux one (`desktop`, or `run N`, and `stop`, `status`, `broker`, `reboot`, `autologon`, `nvidia`). Its default directories (`/var/lib/kf-windows-20261005`, `/opt/nvkvm-broker`) belong to the author's host and are overridable (`WIN_DIR`, `BROKER_BIN`); read the header before use. The input is PS/2 plus a USB tablet. <!-- src: scripts/bench/windows/windows_broker.sh -->

For Windows, the recorded way to get a visible desktop is the one in [Windows guest](windows-guest.md): GSP forced on, the three settings, the signed boot-display driver and `display=on`.

## Not documented yet

- A broker package or a service unit.
- A recipe for a broker with `display-broker` under a sandboxed QEMU (container) end to end.
- Current supported display coverage beyond the named Linux guest driver. The branch `codex/sweep-integrate-20261018`, not merged, reports earlier AD102 broker runs with Linux 590.48.01 and 595.91.07 [reported in `traces/sweep_integrate_20261018/MATRIX.md` on that branch, historical revisions `9d6f210ba` and `6265b980f`]. Those historical runs do not establish current-master display compatibility.
- HDR.

Verified on: the Linux desktop at kayfabe `0e64a960` (host 595.91.07, RTX 4070, GNOME Wayland, 2026-10-08) and the Windows 11 desktop in the README's screenshot (2026-10-21). Not verified end to end: the broker build and the launchers were read from the repository, not run for this page.
