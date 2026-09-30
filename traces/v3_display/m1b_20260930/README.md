# m1b — M1's grade signals on hardware: a connected 1080p connector, 120/120 flips at 60 Hz

**STATUS: MEASURED, 2026-09-30 — M1 core grade met; two flip-event anomalies open (below).**
Exact binary: kf3 `8075c011` (`run_m1b_rev.txt`), vast 53505783, RTX 3060 (GA106), host and guest NVIDIA
580.159.04, nested KVM, `display=on`. `KF_DEVICE=kf3 bash scripts/bench/display/lane.sh m1b`.

The lane's verdict lines (`m1b.log`, hook artefacts in `hook/`):
- `DISPLAY_DRM_MODPROBE rc=0`, `DISPLAY_DRM_NODES by-path card0 renderD128`;
- `KFDISP_CONNECTOR id=126 type=DVI-D-1 status=connected modes=7 preferred=1920x1080@60`, 4 CRTCs,
  a primary + overlay + cursor plane per head (`hook/list.log`);
- `modetest -M nvidia-drm -c`: `DVI-D-1 connected`, the authored EDID's 7 modes, 1920x1080 60.00
  preferred, the EDID blob itself (`hook/modetest_c.log`);
- `nvidia-smi -q`: `Display Attached : Yes`, `Display Active : Disabled` (`hook/smi_display.log`);
- `DISPLAY_GPU_PROGRESS_ERRORS=0` — the 2026-09-29 wall (`0xc67d:0 … 4040`) and m1a's are gone;
- `KFDISP_SETCRTC_OK ms=13.1`, **`KFDISP_FLIPS=120/120 flip_hz=59.99`** — page flips complete at
  the host vblank timer's rate (head 3 armed at 2200x1125 / 148.5 MHz, period 16 666 µs, `run_m1b_qemu.log.gz`);
- guest dmesg: `[drm] Initialized nvidia-drm … on minor 0`, `Console: switching to colour frame buffer
  device 240x67` (1920x1080 fbcon), `fb0: nvidia-drmdrmfb frame buffer device`.

Open, both flip EVENTS (not flip completion):
1. At the first fbdev modeset (31.05 s) a kernel WARNING in `__nv_drm_handle_flip_event`
   (`nvidia-drm-crtc.h:335`, `WARN_ON(nv_flip == NULL)`): a FLIP_OCCURRED arrived for a CRTC with no
   queued flip. nvidia-drm queues events only for planes that were active before the commit
   (`__will_generate_flip_event`, "Hardware generates flip event for only those planes which were
   active previously"); the engine raised AWAKEN for the newly enabled window.
2. At 67.9 s, after the probe exited (fbdev restore): `Flip event timeout on head 0`.

Harness: `PATTERN_MATCH=absent` — the hook's `screendump … kf0` made QEMU abort
(`Unexpected error in object_property_find_err()`, `run_m1b_qemu.log.gz` tail): kf0 has no graphic
console yet (M2), and QEMU's device-name console lookup hits the dummy console. Hence the empty
post-workload guest dmesg and `DISPLAY_LANE_EXIT rc=1`.

`SHA256SUMS` = local hashes; all 17 files matched their remote counterparts when copied. No executable
was copied back.
