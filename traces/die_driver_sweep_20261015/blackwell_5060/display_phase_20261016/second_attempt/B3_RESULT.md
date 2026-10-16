# GB206595 fixed-clock concurrency revalidation

**STATUS: LIVE, 2026-10-16 19:47UTC. Full Linux595raw30 PASS; startup heartbeat gap remains.**

MEASURED own56286918/GB206/PCI2d04, stock host595.91.07 and exact guest595.91.07.
Private sourcef8485f6dbe2cdec62c3b1fa69133b5f266801dcc, QEMU SHA256
 e9744a23ab38ff45afbf788d9e426d71913d1f1e6954fa1ea0da1ea1920b25a6.
Actual KF3 environment{}, normal loglevel6, frozen180second budget.
Concurrency passes31seconds total/client14180ms; complete serial heartbeat
formula warning count0. Native gates9/9+gate10 and native CUDA4 pass first.
Earlier wrong-clock GB206 run timed out with15954warnings. Different rental
and source means revalidation, not a same-box controlled speedup. The earlier
same-binary normal/quiet ablation was on GB205, documented separately. No
logging suppression or grader change.

H-B3-GB206 falsifier: still exceeds180seconds or sustained nonzero-heartbeat
formula mismatch. MEASURED falsifier did not fire in this arm. MEASURED full suite passes30/30 at180seconds,19:31:43..19:43:08UTC.
Thirty actual process receipts confirm private source/SHA, host/guest595,
normal loglevel6 and KF3{}.146/146channel births areUSER;90CUDA thread posture
records clear CAP_SYS_ADMIN for thread lifetime. Complete scoreboard and126
screened text artifacts are in linux595/ (one oversized QEMU log excerpt only).

MEASURED remaining gap:10warnings, all raw heartbeat0, only bus-info-sweep,
guest14.107586..14.188070seconds. heartbeatWithOffsetMs is initially up to82ms
ahead of currentTimeMs, yielding unsigned diff near2^32 until time catches up.
No nonzero-raw-heartbeat formula warning occurs. These are first-sample/startup
warnings; the readiness cause remains INFERRED and is not fixed by this result.
PR23 stays draft; do not describe this as a warning-free full suite. The B3
clock time-box stops here; next work is the prompt's display comparison.

Five host Xid31 records map by exact PID93242to defer-liveness (four) and
94176to missing-page-fault (one). Both frozen raw arms deliberately fault GPU
channels and pass; the PID mapping is in gb206phase_heartbeat_census.json.
This does not waive zero-new-Xid checks for apps/broker. Native CUDA4 passes;
Linux595guest CUDA/gfx/broker remainUNRUN. Windows on/off and tiling UNRUN.

Concurrency QEMU log is1.8MiB; only a screened excerpt and size/SHA receipt
return. Complete serial and native CUDA scoreboard are retained. An initial
collector correctly refused the oversized log before saving the concurrency
proof; the preceding7ca161749 commit saved only native caps equality despite
its broader subject. This follow-up supplies the actual concurrency evidence.
