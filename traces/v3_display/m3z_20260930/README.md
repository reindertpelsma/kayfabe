# m3z — the whole display lane at the merge revision `c0a35924`

**STATUS: MEASURED, 2026-09-30 — the display lane result that accompanies the merge bar
(`../merge_c0a35924/`).** kf3 `c0a35924` (`run_m3z_rev.txt`), vast 53505783, RTX 3060 (GA106),
host + guest 580.159.04, `bash dlane.sh m3z DISPLAY_DESKTOP=1 DISPLAY_CINNAMON_WAYLAND=1
DISPLAY_WESTON=1` (crate tests 666 / 0), `DISPLAY_LANE_EXIT rc=0`, `DISPLAY_HOST_XID=0`,
0 GPU-progress errors in every stage, 0 DRM WARNs.

- M1: card0 + renderD128, `DVI-D-1 connected` 1920x1080@60, `modetest` connected, `nvidia-smi`
  `Display Attached : Yes` / `Display Active : Enabled`.
- M2: `PATTERN_MATCH=yes (pixel-exact 1920x1080)` — re-verified off the box against a locally built
  reference (`console_probe_pattern_a.png`); 120/120 flips at 59.48 Hz; `FLIP_EVENT_TIMEOUTS=0`.
- M3, Mint's desktop: Cinnamon's Wayland session up, **vkcube (Vulkan, RTX 3060) in a Cinnamon
  window** (`console_cinnamon_wayland_vkcube.png`). weston: desktop-shell + vkcube-wayland on the
  overlay (`console_weston_vkcube.png`), weston-simple-egl running.
- M3, X11: Xorg + NVIDIA X driver, glxinfo `NVIDIA GeForce RTX 3060/PCIe/SSE2` direct, glxgears
  56 FPS; `console_x_desktop.png` == `xserver_root.png` (0 of 2 073 600 pixels differ). Open:
  Cinnamon-on-X11 segfaults in `libnvidia-glcore` and X11 Vulkan presentation fails (both need the
  refused GF100_DISP_SW object — `V3_DISPLAY.md` STATUS).
- Notes on this run's counters: `CINNAMON_WAYLAND_CRASHES 1` is the SAME X11 `cinnamon[2788]` segfault
  from the X stage (the guest's dmesg is cumulative, `hook/cw_errors.log`); the Wayland session did
  not crash. `DISPLAY_FLIP_EVENT_TIMEOUTS_AFTER=1` is nvidia-drm's fbdev giving the head to the X
  driver (one "Flip event timeout" at X start — the teardown behaviour m1c explains).

Engine counters at poweroff: `updates=1293 notifies=1265 vblanks=13219 irqs=5241 exceptions=0
refused=0 scanouts=2008 scanout_refused=0 scanout_avg_us=3876 scanout_max_us=6778`;
`compose kernel self-test PASSED`.

`SHA256SUMS` = local hashes of the 42 copied files (all matched remote); PNGs made locally from
copied screendumps / `import` output. No executable was copied back.
