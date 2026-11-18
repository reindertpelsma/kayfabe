| step | verdict | evidence | class | detail |
|---|---|---|---|---|
| prerequisite | PASS | NONE | harness | swtpm >= 0.10 (libtpms state compatibility still required) |
| platform | PASS | NONE | harness | kf3 |
| unpack | PASS | NONE | harness | SHA256 of image and state; derived seed verified |
| overlay | PASS | NONE | harness | throwaway clone of image.base.qcow2; base not booted |
| clock | PASS | NONE | harness | RTC base 2026-11-18T00:46:56Z (fresh overlay) |
| boot | PASS | NONE | harness | ci mode, restrict=on, disposable overlay |
| ssh | PASS | NONE | harness | loopback guest transport ready |
| power | OK | FILE | harness | OK values={"sleep":{"ac":0,"dc":0},"hibernate_after":{"ac":0,"dc":0},"unattended_sleep":{"ac":0,"dc":0},"hybrid_sleep":{"ac":0,"dc":0},"monitor":{"ac":0,"dc":0},"disk":{"ac":0,"dc":0},"wake_signin":{"ac":0,"dc":0},"hibernate_enabled":0,"fast_startup":0,"lock_screen":1,"inactivity":0,"desktop_active":"0","desktop_secure":"0","desktop_timeout":"0","policy_active":"0","policy_secure":"0","policy_timeout":"0","hiberfile_exists":false,"restart_required":false,"screen_active":false,"screen_secure":false,"screen_timeout":0} |
| stop | PASS | NONE | harness | own QEMU stopped; other own helpers cleaned by ExitStack |
| harness | HARNESS | NONE | harness | Command '['ssh', '-i', '/workspace/pc1/winvm/win11_base/runs/abl_noguestdriver/id_kf', '-p', '<ssh_port>', '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null', '-o', 'LogLevel=ERROR', '-o', 'ConnectTimeout=20', '-o', 'BatchMode=yes', 'kfimage@127.0.0.1', 'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'C:\\kfhw\\ps_run.ps1']' timed out after 120 seconds |
