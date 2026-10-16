**STATUS: LIVE, 2026-10-16 13:30 UTC.** Hardware verification of the second candidate is running.

MEASURED: rental56246613 is GB205 (arch0x1b0/impl5), independently queried as UID65534/CapEff0. Host and thin guest are595.91.07. Native Linux concurrency passes4s. Native gates pass9/9, gate10PASS,12/12 USER births. The privileged read-only caps oracle has127 nonzero words and0x00c=0x102; this is a Linux595 oracle, never a shipped page or a Windows580 fact.

| GB205 Linux595 test | Actual source | KF3 environment | Loglevel | Budget | Measured result |
|---|---|---|---:|---:|---|
| Native concurrency |99ce8207c|none|n/a|180s|PASS4s|
| Before, noisy |99ce8207c|{}|6|180s|TIMEOUT180s|
| Before, quiet |99ce8207c|{}|3|180s|PASS18s; raw client7140ms|
| PTIMER selector candidate |277e5d895|{}|6|180s|TIMEOUT180s; heartbeat still offset twice|
| Fn72 offset candidate |304ca9adb|{}|6|180s|RUNNING; no pass claimed|

The same binary and initrd were used for the noisy/quiet ablation. Process receipts assert /proc/exe, argv, actual inherited KF3 environment, host/guest versions and loglevel. The noisy serial file is5,290,164bytes with41,645 heartbeat-related lines and20,823 formula lines; only a screened excerpt, size and SHA return to git. Quiet serial is5128bytes: suppressed messages do not prove a repaired clock. The console-storm falsifier did not fire on GB205. No GB206 result is inferred from this die.

The first candidate was hardware-FALSIFIED: the validated native PTIMER page, like NV_VIRTUAL_FUNCTION_TIME, already contains a system offset. Its CPU tests proved selector behavior but rested on a wrong clock-domain assumption. DraftPR23 remains unvalidated until the second hardware candidate passes.

Public595.91.07 source (ogkm9f087a6d4e86d85acc0ce1d354d6276fe5047b29) provides the missing derivation: kernel_gsp.c _kgspPrepareSystemInfo sends sysTimerOffsetNs in fn72; _kgspIsHeartbeatTimedOut separately divides that value by1e6 and adds it to the heartbeat. Existing compiled GSPSYSTEMINFO layouts already describe the field. Second test-first commitdeff13df8 gives4pass/5fail before; fixe0b4c28e6 gives290 kf-qemu tests pass. Hardware source304ca9adb subtracts the guest offset from the native system-clock milliseconds with wrapping arithmetic. No per-die constant, captured page, changed budget or logging workaround.

Second-candidate falsifier, declared before run: normal loglevel6 still reports heartbeat timeout or concurrency exceeds180s. Full raw30 starts only if the targeted run passes and heartbeat warnings are absent. Native gates are rerun and the per-revision QEMU, raw client and thin595 guest rebuilt first. Full Linux/CUDA/graphics/broker and all Windows acceptance remain UNRUN until their own receipts exist.

Lease: created12:40:48UTC; timer15:35:50UTC, hard cap15:40:48UTC. Rate0.2416666667USD/hour, max0.73USD; campaign conservative ceiling5.09USD plus transfers. Only own56246613 is active.
