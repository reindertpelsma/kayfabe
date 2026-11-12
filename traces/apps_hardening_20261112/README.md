STATUS: LIVE, 2026-11-12. Native runs of the apps-hardening probes (no VM, no guest).

Box: the trusted project PC, RTX 4070 (sm_89), host driver 595.91.07, driver API 13.2, run under `flock /root/prov/pc_gpu.lock`.
Source revision: the head of PR claude/apps-hardening-20261112 (`scripts/apps/src/{cuda_drv_probe.c,egl_offscreen.c,renderer_check.py}`).

- `cdp_1..3.log`: `cuda_drv_probe`, three passes, 7/7 checks, rc=0, wall 0.24 s, 0.23 s, 0.21 s (JIT of the sm_75 PTX on an sm_89 die).
- `egl_pos.log`: `egl_offscreen 200 5000`: NVIDIA renderer, exact pixels, rc=0.
- `egl_neg1.log`: `LIBGL_ALWAYS_SOFTWARE=1 __EGL_VENDOR_LIBRARY_FILENAMES=/nonexistent.json`: no EGL device, `CHECK ... FAIL`.
- `egl_neg2.log`: `__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json`: the AMD iGPU through Mesa radeonsi, `CHECK egl_renderer FAIL`.
Measured only on this one die; not run in a kayfabe guest.
