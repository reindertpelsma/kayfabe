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
| app:3dmark | PASS | FILE | app | 3DMark score 7297 (graphics 7387), graphics tests 48.37, 42.18 FPS, finished 2026-11-12T17:59:35Z on NVIDIA GeForce RTX 4070 driver 32.0.15.8088, all 10 workload status codes 0; CLI refused (Professional/Developer key needed), so the run went through UI Automation |
| app:edge | PASS | UIA | app | WebGL drew 230 frames in 4 s on ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 (0x00002786) Direct3D11 vs_5_0 ps_5_0, D3D11) (title read through UIA) |
| app:superposition | LAUNCH_ONLY | NONE | unqualified | UIA tree collected; verified run controls and result schema pending |
| stop | PASS | NONE | harness | own QEMU stopped; other own helpers cleaned by ExitStack |
