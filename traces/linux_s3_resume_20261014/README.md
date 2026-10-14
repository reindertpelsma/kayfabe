# Linux/ogkm S3 suspend/resume under kf3 (2026-10-14)

**STATUS: EVIDENCE, 2026-10-14.** Host RTX 4070 (AD104), kf3 rev c287af4f, Linux desktop guest (Ubuntu 24.04, kernel 6.8.0-142,
NVIDIA open kernel module 580.159.04), `scripts/bench/display/desktop_s3.sh c287af4f a2` (suspend with `systemctl suspend`,
wake with the QEMU monitor's `system_wakeup`, `pm_print_times` on, console not suspended so the serial log carries the resume).

## Measured
- The guest suspends cleanly: `nvidia 0000:00:02.0: pci_pm_suspend returned 0 after 327387 usecs`; the GSP sees
  `UNLOADING_GUEST_DRIVER bInPMTransition=true newLevel=3`, phase `Running -> Suspending -> Halted`.
- QEMU's wakeup is a system reset: kf3 logs `device reset #2 ... GSP Halted -> Cold ... COMPLETE` before the vCPUs resume.
- The guest kernel resumes (all vCPUs in the idle loop afterwards) and enters `nvidia ...: calling pci_pm_resume`, which never returns:
  `kgspBootstrap_TU102: Failed to boot GSP` / `gpuPowerManagementResume: GSP boot failed at resume (bootMode 0x1): 0x55`,
  then Xid 119 (GSP Timeout, GSP_RM_ALLOC seq 1719). ssh never returns, the display stays black.
- The Windows 11 guest (run 2990, pc machine) shows the same host-side signature after its S3 sleep: `bInPMTransition=true newLevel=3`,
  Halted, then Cold after the wakeup, no BAR0 write traffic, all vCPUs busy.

⊘ *ANSWERED 2026-10-14: the inference below was right as far as it went (the FSM resume, E14/E15) and
three more walls followed it — the FBSR save's fn 4, the BAR1 resize word, DISABLE_USERMODE_CHANNELS;
see `docs/design/V3_S3_RESUME.md` §4-§6 and `fixed/` for the passing runs at kf3 `559db30c`.*

## Inferred (not measured)
- The cause is the unbuilt GSP resume: `crates/kf-gsp/Cargo.toml` says "reboot/resume S6-S8 not" built and `boot.rs` notes the
  CORE_RESUME mailbox pair arrives while the phase is `Halted`, which the E6 publish guard (`ProtectedRegionUp | Booted`) refuses.
- Falsifier for that: a resume-mode boot (bootMode 0x1) served by the FSM must let the same run resume and pass `nvidia-smi`
  and glxgears after the wake.
