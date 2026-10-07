# m3g — Vulkan presents through weston on the virtual monitor; the console lacks overlay planes

**STATUS: MEASURED, 2026-09-30 — M3 in progress.**
Exact binary: kf3 `0f29e375` (`run_m3g_rev.txt`), vast 53505783, RTX 3060, 580.159.04,
`bash dlane.sh m3g DISPLAY_WESTON=1 KF3_DISPLAY_TRACE=1` (tests 664 / 0), `DISPLAY_HOST_XID=0`,
0 GPU-progress errors, the probe stage pixel-exact again.

- **`vkcube-wayland --c 400` exits 0** on "NVIDIA GeForce RTX 3060" — Vulkan rendering on the real
  GPU through kayfabe, presented through weston (nvidia-drm KMS -> the emulated display engine).
- `weston-simple-egl` runs its 10 s.
- The trace (`run_m3g_qemu.log.gz`): 343 updates latch windows 6 AND 7 together (`group [7, 8]`,
  channels 7/8), plus window-immediate channel 40 — weston puts the client surfaces on the
  OVERLAY window 7. The console composes only the lowest window, so `console_weston_during_vkcube.png`
  shows weston's desktop-shell without the cube. Next: compose every enabled window of the head.

`SHA256SUMS` = local hashes of the 21 copied files (all matched remote); the PNG was made locally
from the copied screendump. No executable was copied back.
