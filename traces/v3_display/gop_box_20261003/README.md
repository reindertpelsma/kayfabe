# GOP boot display on hardware — B0, B9r, B1, B2 (2026-10-03)

Box: vast 54032077, RTX 3060 (GA106, 10de:2504), host driver 580.159.04 (open), guest Ubuntu noble
6.8.0-142 with 580.159.04. kf3 binary `kf3-bins/8c86c94a` (branch `v3-display-test` = `v3-gop` at
`0a7f5cac` + `v3-b0a` at `71395167`). OVMF from Ubuntu 22.04's `ovmf` package. Script: the box's
`/root/gop_tests.sh` (build, then the three lane runs); its output is `gop_tests.log`. The first attempt
(`gop_tests.try1.log`) never booted: `build_kf3.sh` did not copy the new `kf3_gop.h` (fixed in `0a7f5cac`).
Text logs and PNG renders of the console screendumps only.

| test | command (from `scripts/bench/display/lane.sh`) | result |
|---|---|---|
| B0 | `KF_FIRMWARE=ovmf` (no ROM) | PASS: `boot_vga=1`; the M1/M2 probe pixel-exact at 1920x1080; 120/120 flips at 60.03 Hz; host Xid 0 |
| B9r | `KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on,rombar=0` | PASS: realize refused by name, `kf3: gop=on refused: rombar=0 asks for no ROM BAR`; no `boot display ON` line |
| B1 | `KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on` | PASS: `boot display ON — option ROM 9728 bytes (10de:2504, KFGP BAR1 +0x0, 1920x1080 pitch 7680, G = 0x7f0000)`; `option ROM registered: 9728 bytes in a 16384-byte ROM BAR`; the console shows, in order, the zeroed framebuffer, the TianoCore logo (16 s), OVMF's boot manager and the Linux EFI stub (20–25 s), kernel and systemd text (30–40 s), the login prompt (50–60 s), the probe's pattern through nvidia-drm (75 s) and the fbcon console after it (90 s) — `b1_contact_sheet.png`, `b1_t*.png` |
| B2 | same boot, after nvidia.ko | PASS: `GET_GSP_STATIC_INFO: the guest preserves a firmware console of 0x7f0000 bytes … region 0 = [0, 0x7f0000) reserved, the heap starts at 0x7f0000, 3 regions`; `boot display seed [0x0, +0x7f0000) retired at the first change: the console is ONE run, VA 0 -> store 0`; `the guest armed its first head at +53507 ms — the boot layer is retired after 598 boot frame(s)`; guest `fbcon: nvidia-drmdrmfb (fb0) is primary device`; probe pixel-exact, 120/120 flips at 59.10 Hz; host Xid 0 |

Seen, not graded: for the first ~6 s the QEMU console shows its own *"Guest has not initialized the
display (yet)"* placeholder before the boot layer's first frame (cosmetic).
| B3 | `KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on DISPLAY_DESKTOP=1 DISPLAY_CINNAMON_WAYLAND=1` (no xorg.conf: the lane's default since `v3-b0a`) | Xorg: PASS for device and driver selection with simpledrm holding `card0` (nvidia-drm is `card1`): `boot_vga=1`, `PCI:*(0@0:2:0)`, the NVIDIA X driver from the stock OutputClass file, GLX on the RTX 3060, direct. X11 desktop: FAIL on the known `GF100_DISP_SW` gap, now named by the X driver itself (`(EE) NVIDIA(0): Failed to allocate display software resources`), so Cinnamon X11 segfaults in `libnvidia-glcore`, X11 vkcube aborts, and one `Flip event timeout on head 0` follows at guest 80.48 s. Cinnamon Wayland: PASS — session up, vkcube-wayland running (`b3_cinnamon_wayland_vkcube.png`). Host Xid 0 |

