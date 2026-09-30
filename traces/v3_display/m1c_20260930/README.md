# m1c — M1 with the flip-event rule and KF3_DISPLAY_TRACE: no WARN, one teardown timeout

**STATUS: MEASURED, 2026-09-30 — M1 grade met again; m1b's WARN is gone; the one remaining
"Flip event timeout" is explained (nvidia-drm's plane teardown, see below).**
Exact binary: kf3 `d6482923` (`run_m1c_rev.txt`), vast 53505783, RTX 3060 (GA106), host and guest NVIDIA
580.159.04, nested KVM, `display=on`.
`bash dlane.sh m1c KF3_DISPLAY_TRACE=1` → crate tests (`kf-disp kf-rm kf-qemu kf-cuda`) 650 passed / 0
failed, `build_kf3.sh`, then `KF_DEVICE=kf3 bash scripts/bench/display/lane.sh m1c`.

The lane's verdict lines (`m1c.log`, hook artefacts in `hook/`):
- `DISPLAY_DRM_MODPROBE rc=0`, `DISPLAY_DRM_NODES by-path card0 renderD128`;
- `DVI-D-1 connected`, 7 modes, `1920x1080@60` preferred; 4 CRTCs, each with a primary, overlay and
  cursor plane (`hook/list.log`, `hook/modetest_c.log`, `hook/modetest_p.log`);
- `DISPLAY_GPU_PROGRESS_ERRORS=0` and `DISPLAY_GPU_PROGRESS_ERRORS_AFTER=0`;
- **`KFDISP_FLIPS=120/120 flip_hz=59.98`**, `KFDISP_SETCRTC_OK ms=7.9`;
- fbcon at 1920x1080 (`Console: switching to colour frame buffer device 240x67`);
- **no kernel WARNING** (m1b had one in `__nv_drm_handle_flip_event` at the first fbdev modeset):
  the engine raises a window's flip AWAKEN only if the window scanned a surface on an active head
  before the update (`26fa5997`);
- the guest powered down cleanly (sync + sysrq s/u before poweroff; the fat-guest image needed no fsck).

Engine counters at poweroff (`run_m1c_qemu.log.gz`, last status line): `methods=40858 updates=146
notifies=136 releases=0 vblanks=2822 irqs=134 exceptions=0 refused=0`; head 3 ACTIVE at 2200x1125,
period 16 666 µs.

**The one error, explained.** `[69.124367] Flip event timeout on head 0` (`run_m1c_dmesg_after.log`).
The `KF3_DISPLAY_TRACE` lines at the probe's exit show two core-interlocked updates of window 6:
the first DISABLES it (no notifier programmed), the second re-enables it with fbcon's surface. The
first is the kernel removing the probe's framebuffer when the probe's DRM file closes
(`drm_fb_release` → `atomic_remove_fb`, a BLOCKING commit with the CRTC still active). nvidia-drm
counts one flip event for that plane (old CRTC active, old fb non-NULL), but the KAPI programs a
disabled layer with no completion notifier (`nvkms-kapi.c:2956-2970`, `nvkms-evo3.c:3901-3904`), so no
event can come and the commit waits its 3 s. Real GPUs log the same on framebuffer removal at DRM
file close (NVIDIA/open-gpu-kernel-modules#1361). The m1b README's attribution of this timeout to the
fbdev restore is superseded by this reading. Fix in the harness, not the engine (`80a2c324`): the
probe restores the CRTC it found before it exits.

Also open (not errors of the display path): `nvidia-smi -q` reports `Display Active : Disabled`
because `SYSTEM_GET_ACTIVE` answered 0 for every head (fixed in the M2 work: it now reports the
connector on the SOR the head's armed state drives); `DISPLAY_LANE_EXIT rc=1` was the lane reading
`grep -c`'s exit status (1 = zero matches) instead of boot_capture's (fixed with the M2 work);
`PATTERN_MATCH=not-run (… console=no)` — no graphic console before M2.

`SHA256SUMS` = local hashes; all 17 files matched their remote counterparts when copied. No executable
was copied back.
