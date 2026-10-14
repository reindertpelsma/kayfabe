# Chromium / Electron GPU load on the Linux guest (kf3 c287af4f) — 2026-10-14

STATUS: RESEARCH, 2026-10-14 (in progress; results are added below the falsifiers)

## Question

The owner's Windows 11 guest is stable with Edge but three TDR cycles followed the start of the Minecraft Launcher
(Chromium/Electron class app): guest LiveKernelEvent 141 + nvlddmkm 14/153, three unplanned GSP unloads on the kayfabe
side, the first right after a refused RmControl 0x906f0102 (RESET_CHANNEL). Does the same class of GPU use reset the
GPU on the Linux guest (open module 580.159.04, which can be built from source and debugged)?

## Hypotheses and falsifiers (written before any graded run)

- **H1** "Chromium / Electron GPU use on the Linux guest triggers no unplanned GSP unload within >= 10 minutes of mixed
  load per configuration."  FALSIFIED by one unplanned unload (`GSP phase Running -> Suspending` in the kf3 log with
  no S3/PM request from the guest), one guest Xid 119 / GSP timeout / host `NVRM: Xid` delta, or a hang of the guest GPU
  (page fps counters stop, guest ssh `nvidia-smi` hangs). Supporting-only evidence: zero of those in every configuration.
- **H2** (only if H1 is falsified) "the failing feature is X" — gets a one-variable A/B (X on vs X off, everything else
  identical), falsified if the reset also happens with X off or does not happen with X on.

Measured and inferred are kept apart below: "measured" = a counter or a log line in this directory; "inferred" = my reading.

## Method

- `scripts/bench/display/chromium_lane.sh <rev> <tag> <guest-script>` boots a throw-away overlay of the desktop image
  (Ubuntu 24.04, Cinnamon on Xorg, NVIDIA 580.159.04 open module, GSP on) on kf3 c287af4f (same harness as
  `desktop_smoke.sh` / `desktop_s3.sh`), beside the owner's Win11 VM, under `flock /tmp/claude-linux-guest.lock`.
- Guest software: Google Chrome 155.0.8059.79 (deb) and Electron 44.7.2 (zip + a hello-world BrowserWindow app).
- Load: `scripts/bench/display/chromium/cdp_load.py` drives the browser over CDP: one tab (`mix.html`) with a WebGL2
  scene (3000 instanced lit cubes, 4x MSAA FBO, blur post pass, per-frame texImage2D from a canvas, readPixels sync),
  a canvas-2D scene (gradients, shadowBlur, filter blur), 220 CSS-animated layers + backdrop-filter, a scrolling
  3000-card DOM and a looping 1080p60 video (H.264, then VP9 at 20 %, AV1 at 30 %); fullscreen video at 40 %;
  24 extra tabs/windows at 50 %; `kill -9` of the GPU process at 70 %; a WebGPU tab at 80 %.
- Grades per run: guest dmesg (Xid, NVRM, Flip event timeout, nvAssert, GSP timeout), host kf3 log
  (`GSP phase Running -> Suspending`, USER_RING_REFUSED, USER_RING_SCAN, `display: STALL`, `REFUSED fn76`, `BLACK:`),
  host `NVRM: Xid` delta from `journalctl -k`.
- Baseline of a desktop boot without any browser (measured, setup run): guest dmesg has 24 NVRM lines / 12 nvAssert
  lines at boot (NV_ERR_NOT_SUPPORTED 0x56 from golden-image / gpushareddata paths), kf3 has about 30-39 `REFUSED fn76`
  lines (GSP_RM_CONTROL, status 0x56), 0 unplanned unloads, 0 stalls.
