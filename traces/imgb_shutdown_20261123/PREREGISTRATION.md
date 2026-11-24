STATUS: LIVE, 2026-11-23. Preregistration for the root cause of the image B guest stop of 2026-11-22 (`traces/adobe_vast_part2_20261122/README.md`, boot `g1` of box 61997456: the guest went down 45 minutes after boot, 16:59:00Z, and QEMU exited without an error line). Written on 2026-11-23, after the Phase 1 read-only inspection started, before any Vast run and before the harness test. Labels in the result: MEASURED (a file, log line or source line named), INFERRED.

# Observation to explain
Boot `g1` of box 61997456 on 2026-11-22: the sealed image B (`kfimg-adobefull2-20261122T015401Z`, offline, recorded clock) stopped about 45 minutes after boot. The guest overlay and the g1 harness log were not kept, so no guest event log of that boot exists. The trace README called the cause "INFERRED: the evaluation timer the harness' 55-minute boot-age guard is about".

# Hypotheses and falsifiers
- H1, our harness or scripts end the guest after about 45-55 minutes (a boot-age guard, a wall-clock limit, `-watchdog-action`, `-no-reboot`, a script that sends `Stop-Computer`).
  Falsifier: no code path in `propimage_check.py`, `propimage.py`, `hold_run.py`, `hold.sh` or `host_xid_watch.sh` stops the guest or QEMU while the harness holds the guest (`--hold-until-file`, 3 h) and the release file is absent.
- H2, a Windows setting in the image powers the guest off: an evaluation licence (`wlms.exe`), a scheduled task with a shutdown action, a Group Policy shutdown script, a sleep, hibernate or idle timeout, an inactivity policy.
  Falsifier: the image is not an evaluation edition and has no `wlms`; no task, GPO script or startup entry calls a shutdown; sleep, hibernate and idle timeouts are 0 or disabled; the image's own System log holds no 1074 from a process other than the ones our harness uses.
- H3, an Adobe licence, trial or clock check ends the session.
  Falsifier: an Adobe process cannot power Windows off without a System 1074 that names it; the 1074 records in the image name only our harness processes. Without the g1 log, H3 stays untested for g1 itself.
- H4, a QEMU-side end of the VM after a guest event: a Windows bugcheck reported through the Hyper-V crash MSRs (`hv-crash` is in the image's CPU flags) and QEMU's default panic action, or a guest ACPI power-off.
  Supported if: the harness argv has `hv-crash`, sets no `-action panic=...`, and QEMU's default panic action ends the process with status 0 and no log line without `-d guest_errors`; and the image's own System log shows an earlier boot that ended with Kernel-Power 41 plus EventLog 6008 and no 1074 (a crash or power loss, not an orderly shutdown).
  Falsifier: the argv sets `-action panic=none` or `pause`, or a bugcheck would leave a QEMU log line.

# Phase 2 (one Vast box), only if Phase 1 leaves the cause unresolved
Boot image B with the fixed harness (a QMP event recorder for SHUTDOWN, GUEST_PANICKED and RESET), drive the g1 workload (four applications at once), and keep the guest up for 60 minutes.
- H4 (bugcheck) is MEASURED if a GUEST_PANICKED event with Hyper-V crash parameters is recorded when the VM ends (arg1 = the bugcheck code).
- H2/H3 are MEASURED if instead a SHUTDOWN event with `reason=guest-shutdown` is recorded and the guest's System log, read by the harness every minute, shows a 1074 that names the process.
- A boot that stays up 60 minutes falsifies a fixed timer at 45 minutes and leaves the g1 cause unmeasured; one boot is a small sample.

# The fix rule
If the cause is in the harness: the smallest change plus a GPU-free test that fails on the current tree and passes after it. If it is in the image: no change to the sealed image; the finding and the exact layer or first-boot change go to the owner.
