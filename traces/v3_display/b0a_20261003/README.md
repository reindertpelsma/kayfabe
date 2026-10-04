# B0a — does the guest's Xorg need the BusID pin? (2026-10-03)

Box: vast 54032077, RTX 3060 (GA106), host driver 580.159.04 (open), guest Ubuntu noble 6.8.0-142 with
580.159.04 from the .run, SeaBIOS, kf3 with `display=on`, **no option ROM**. Lane:
`DISPLAY_DESKTOP=1 DISPLAY_XORG_PIN=0 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh <tag>`.
Text logs only; the lane's helper binary and the PPM screendumps were left on the box.

| run | kf3 rev | guest config | result |
|---|---|---|---|
| 1 (`b0a_run1/`) | `597d28c7` | no xorg.conf; guest lacks the NVIDIA OutputClass file | kf3 is `boot_vga=1`; Xorg marks it primary (`PCI:*(0@0:2:0) 10de:2504`); lightdm crash-loops (27 restarts); the captured Xorg.0.log is a cut-off attempt |
| 2 (`b0a_run2/`) | `fb394211` | same | the complete failed attempt (`Xorg.0.log.old`): no `xorg.conf.d` file names the NVIDIA driver, so Xorg autoconfigures nouveau (missing), then **modesetting** on `/dev/dri/card0`; glamor initialises on the RTX 3060; then `Fatal server error: failed to create screen resources` |
| 3 (`b0a_run3/`, `runlogs/`) | `656e2bb9` | no xorg.conf; `/usr/share/X11/xorg.conf.d/nvidia-drm-outputclass.conf` installed, as the .run and Ubuntu's `xserver-xorg-video-nvidia-580` both do | **PASS for B0a:** `boot_vga=1`; Xorg primary = kf3; the NVIDIA X driver loads (38 `NVIDIA(0)` lines, `Setting mode "DFP-0:nvidia-auto-select"`); the session answers; GLX renderer `NVIDIA GeForce RTX 3060/PCIe/SSE2`, direct; glxgears ~84 FPS. Cinnamon's X11 session still segfaults in `libnvidia-glcore` (fallback dialog, `b0a_run3/desk_1_1280.png`) and X11 vkcube aborts (rc 134): the known `GF100_DISP_SW` gap, not the pin |

**Conclusions.**
- The BusID pin is not needed: on today's SeaBIOS bench, kf3 is already the boot VGA device and Xorg's
  primary. The lane now runs without an xorg.conf by default (`DISPLAY_XORG_PIN=0`).
- The bench guest was not stock: it lacked the OutputClass file because the driver was installed before
  Xorg existed. `provision_guest_display.sh` now re-runs the installer when that file is missing.
- Open, not a B0a question: the generic modesetting X driver fails on kf3 (`failed to create screen
  resources`). A bare-metal comparison on the same GPU decides whether that is kayfabe's.
