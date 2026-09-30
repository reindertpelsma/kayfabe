# m3h — vkcube on the virtual monitor, in the host screendump: the console composes the head

**STATUS: MEASURED, 2026-09-30 — M3 in progress (see `V3_DISPLAY.md` STATUS for the grade state).**
Exact binary: kf3 `b4da2089` (`run_m3h_rev.txt`), vast 53505783, RTX 3060 (GA106), host + guest
580.159.04, `bash dlane.sh m3h DISPLAY_DESKTOP=1 DISPLAY_WESTON=1 KF3_DISPLAY_TRACE=1`
(tests 666 / 0). `DISPLAY_HOST_XID=0`, 0 GPU-progress errors, lane rc=0.

- The console now shows what the head composes — every enabled window, back to front, through
  the hand-written `kf_compose` PTX kernel (bring-up self-test PASSED: a block-linear window at an
  offset word for word against `bl_offset`, and a premultiplied-alpha blend).
- The probe stage is still **pixel-exact** (`PATTERN_MATCH=yes`) through the compose path.
- ★ **`console_weston_vkcube.png`**: weston's desktop-shell on the virtual monitor with
  **`vkcube-wayland` (Vulkan on the RTX 3060) on the overlay window**, as the host screendumped it;
  `vkcube-wayland --c 400` exited 0.
- ★ **`console_x_desktop.png` == `xserver_root.png` in all 2 073 600 pixels**: the NVIDIA X driver's
  block-linear desktop, composed by our kernel, equals the X server's own screenshot.
- X11: glxinfo `NVIDIA GeForce RTX 3060/PCIe/SSE2`, direct; glxgears ~54 FPS. Still open: Cinnamon
  segfaults in `libnvidia-glcore` (fallback dialog), and X11 Vulkan presentation fails at
  `vkCreateSwapchainKHR` — both need the GF100_DISP_SW object, refused because its software methods
  trap on the HOST GPU (m3c: 186 host Xid 32 when offered).

`SHA256SUMS` = local hashes of the 38 copied files (all matched remote); the PNGs were made locally
from copied screendumps / `import` output. No executable was copied back.
