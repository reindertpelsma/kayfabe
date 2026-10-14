# S3 suspend/resume, fixed (2026-10-14)

**STATUS: EVIDENCE, 2026-10-14.** Trusted host (RTX 4070, host driver 595.91.07), kf3 `559db30c`,
Linux desktop guest (Ubuntu 24.04, 6.8.0-142, stock open module 580.159.04), harness
`scripts/bench/display/desktop_s3.sh` (suspend: `systemctl suspend`; wake: QEMU monitor `system_wakeup`).

- `harness-b7.log` — `desktop_s3.sh 559db30c b7`, one cycle: PASS.
- `harness-b8-3cycles.log` — `S3_CYCLES=3 desktop_s3.sh 559db30c b8`: 3/3 PASS in one boot
  (ssh, `nvidia-smi`, glxgears ~300 frames/5 s, 0 Xid after each wake).
- `kf3-b8-excerpt.txt` — kf3's phase, reset, resume, PM-protocol and refusal lines of b8 (the refusals
  listed are the boot's own; none after the first fn 47).
- `guest-serial-b8-excerpt.txt` — the guest's NVRM and PM lines (0 WARNING).

The runs between the failure and the fix (b1-b6, kf3 44e9b092 / 91afc335 / 89362cbe) and what each one
measured are in `docs/design/V3_S3_RESUME.md` §4b, §4c.
