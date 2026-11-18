| step | verdict | evidence | class | detail |
|---|---|---|---|---|
| prerequisite | PASS | NONE | harness | swtpm >= 0.10 (libtpms state compatibility still required) |
| unpack | PASS | NONE | harness | SHA256 of image and state; derived seed verified |
| overlay | PASS | NONE | harness | throwaway clone of image.base.qcow2; base not booted |
| clock | PASS | NONE | harness | RTC base 2026-11-12T17:49:48Z (fresh overlay) |
| boot | PASS | NONE | harness | ci mode, restrict=on, disposable overlay |
| ssh | PASS | NONE | harness | loopback guest transport ready |
| power | OK | FILE | harness | OK values={"sleep":{"ac":0,"dc":0},"hibernate_after":{"ac":0,"dc":0},"unattended_sleep":{"ac":0,"dc":0},"hybrid_sleep":{"ac":0,"dc":0},"monitor":{"ac":0,"dc":0},"disk":{"ac":0,"dc":0},"wake_signin":{"ac":0,"dc":0},"hibernate_enabled":0,"fast_startup":0,"lock_screen":1,"inactivity":0,"desktop_active":"0","desktop_secure":"0","desktop_timeout":"0","policy_active":"0","policy_secure":"0","policy_timeout":"0","hiberfile_exists":false,"restart_required":false,"screen_active":false,"screen_secure":false,"screen_timeout":0} |
| gpu | PASS | NONE | harness | post-fix seed UUID exact match |
| verify | PASS | NONE | harness | identity, clock, TPM and seed GPU checked |
| app:superposition | HARNESS | NONE | harness | Task timeout; no verdict |
| app:3dmark | FAIL | NONE | app | No RUN link under the Time Spy card |
| app:solidworks | PASS | FILE | app | API exported STEP with entity data (not a GPU/render benchmark) |
| app:resolve | PASS | FILE | app | Resolve created project kfa5b08266 (UIA main window 'DaVinci Resolve - kfa5b08266'); its log after the start shows CUDA compute, NVDEC/NVENC initialised and OpenGL on NVIDIA Corporation NVIDIA GeForce RTX 3090/PCIe/SSE2; no render or benchmark was run |
| app:edrawings | PASS | UIA | app | eDrawings opened the sample part (UIA TabItem 'kfsample.sldprt'); licence dialog accepted: True (BM_CLICK on the UIA window handle) |
| app:edge | PASS | UIA | app | WebGL drew 133 frames in 4 s on ANGLE (NVIDIA, NVIDIA GeForce RTX 3090 (0x00002204) Direct3D11 vs_5_0 ps_5_0, D3D11) (title read through UIA) |
| stop | PASS | NONE | harness | own QEMU stopped; other own helpers cleaned by ExitStack |
