# m2c — M2 grade: a pixel-exact 1920x1080 screendump of the guest's scanout

**STATUS: MEASURED, 2026-09-30 — M2 grade MET.**
Exact binary: kf3 `945da292` (`run_m2c_rev.txt`), vast 53505783, RTX 3060 (GA106), host and guest NVIDIA
580.159.04, nested KVM, `display=on`, no emulated VGA (`-vga none`), VNC on 127.0.0.1.
`bash dlane.sh m2c` → crate tests (`kf-disp kf-rm kf-qemu kf-cuda`) 658 passed / 0 failed,
`build_kf3.sh`, `KF_DEVICE=kf3 bash scripts/bench/display/lane.sh m2c`.

The grade (`m2c.log`):
- **`DISPLAY_PATTERN_MATCH=yes (pixel-exact 1920x1080)`** — QEMU's `screendump … kf0`, taken on the
  HOST while the guest's stock nvidia-drm scanned out the probe's pattern A, is byte-identical to the
  reference PPM the same source emits on the host. Re-verified off the box: the copied screendump's
  sha256 (`screendump_m2c.ppm.sha256`, `8d80f3f1…4938431`) equals a reference built locally from
  `kfdisp_probe.c`; `screendump_m2c.png` is the same pixels, lossless.
- `KFDISP_FLIPS=120/120 flip_hz=60.01` — every flip's completion now waits for the GPU copy of its
  new surface, and the probe still flips at the head's refresh;
- `DISPLAY_FLIP_EVENT_TIMEOUTS=0 DRM_WARNS=0` (the probe restores its CRTC: `KFDISP_RESTORED=ok fb=128`);
- `nvidia-smi -q`: `Display Attached : Yes`, **`Display Active : Enabled`**;
- `DISPLAY_GPU_PROGRESS_ERRORS=0`, `DISPLAY_LANE_EXIT rc=0`; clean poweroff.

The copies (last status line, `run_m2c_qemu.log.gz`): `scanouts=332 scanout_refused=0
scanout_avg_us=3879 scanout_max_us=7327` — 8 MiB per copy (`cuMemcpy2DAsync` store → page-locked
frame on the display plane's own stream), queue-to-observed-completion ≈ 3.9 ms.
`kf3: display console registered (head 0 of kf0)`.

`SHA256SUMS` = local hashes; all 17 copied files matched their remote counterparts. The PNG and the
`.sha256` were produced locally from the copied screendump. No executable was copied back.
