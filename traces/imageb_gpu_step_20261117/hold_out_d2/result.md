Run 2026-11-17, image B gpu-step diagnostic boot hold_out_d2 (Vast box, kf3), harness result table:
| step | verdict | evidence | class | detail |
|---|---|---|---|---|
| prerequisite | PASS | NONE | harness | swtpm >= 0.10 (libtpms state compatibility still required) |
| platform | PASS | NONE | harness | kf3 |
| unpack | PASS | NONE | harness | SHA256 of image and state; derived seed verified |
| overlay | PASS | NONE | harness | throwaway clone of image.base.qcow2; base not booted |
| clock | PASS | NONE | harness | RTC base 2026-11-15T22:54:02Z (fresh overlay) |
| fb-mb | PASS | NONE | harness | DIAGNOSTIC ARM: guest-visible FB 8192 MiB (image default differs) |
| boot | PASS | NONE | harness | ci mode, restrict=on, disposable overlay |
| ssh | PASS | NONE | harness | loopback guest transport ready |
| power | OK | FILE | harness | OK values={"sleep":{"ac":0,"dc":0},"hibernate_after":{"ac":0,"dc":0},"unattended_sleep":{"ac":0,"dc":0},"hybrid_sleep":{"ac":0,"dc":0},"monitor":{"ac":0,"dc":0},"disk":{"ac":0,"dc":0},"wake_signin":{"ac":0,"dc":0},"hibernate_enabled":0,"fast_startup":0,"lock_screen":1,"inactivity":0,"desktop_active":"0","desktop_secure":"0","desktop_timeout":"0","policy_active":"0","policy_secure":"0","policy_timeout":"0","hiberfile_exists":false,"restart_required":false,"screen_active":false,"screen_secure":false,"screen_timeout":0} |
| gpu | PASS | NONE | harness | post-fix seed UUID exact match after 239s (expected=1 observed=1 host=n/a), problem code 0, started before restart=False |
| verify | PASS | NONE | harness | identity, clock, TPM and seed GPU checked |
| app:gpudiag | PASS | FILE | app | the applications' GPU sniffers (premiere, aftereffects, mediaencoder) report an NVIDIA compute device |
| stop | PASS | NONE | harness | own QEMU stopped; other own helpers cleaned by ExitStack |
