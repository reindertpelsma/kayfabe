# K1 SPECviewperf 15: baseline versus descriptor Phase 1

STATUS: PARTIAL / RESTORED, 2026-10-22. Baseline reproduced the target failure;
Phase 1 passed CATIA and Creo. Both arms stopped as required; no full-suite claim.

MEASURED: baseline run 3453 used installed 89fc8a222. Phase 1 run 3454 used
468a35c9d845afc1938683540acc251bdef90e5a, freshly tested and built with
build_kf3.sh. See phase1-tests.json, build.txt and binary-hashes.txt. No product
Rust/C/CUDA change or merge was made by this lane.

MEASURED: vendor workload composites. VFIO n=2; baseline/fix n=1 when Completed.
CODE READ: blank scores/ratios mean unavailable; failed/incomplete output is not a composite.

| Workload | VFIO 1 / 2 | Baseline | Phase 1 | Phase 1 / VFIO mean | Status baseline / fix |
|---|---:|---:|---:|---:|---|
| blender-01 | 61.61 / 60.61 | 35.85 | 35.95 | 0.588283 | Completed / Completed |
| catia-07 | 63.1 / 63.13 | — | 50.83 | 0.805355 | Failed / Completed |
| creo-04 | 104.94 / 103.97 | — | 70.42 | 0.674166 | Incomplete / Completed |
| energy-04 | 54.57 / 54.67 | — | 46.46 | 0.850604 | NotRun / Completed |
| enscape-01 | 37.26 / 37.39 | — | — | — | NotRun / Incomplete |
| maya-07 | 162.14 / 160.37 | — | — | — | NotRun / NotRun |
| medical-04 | 50.54 / 50.58 | — | — | — | NotRun / NotRun |
| solidworks-08 | 29.51 / 29.19 | — | — | — | NotRun / NotRun |
| unreal_engine-01 | 63.58 / 63.7 | — | — | — | NotRun / NotRun |

MEASURED: each qualified kayfabe workload has n=1, with no repeatability spread
established. Native n=2 ranges/mean span 0.05–1.64%. Blender changed 35.85 to
35.95 (+0.28% in these samples); this is not a regression-equivalence proof.
The CATIA/Creo baseline failures supply no qualified composite or speedup ratio.
No partial subtest score is promoted into a workload composite.

MEASURED: baseline started SPEC at 05:46:54 CEST. CATIA 03 and 06 failed;
Creo 01 failed, and a subsequent Windows reboot interrupted Creo 02. There
were three fatal GP_FORMAT Budget refusals and poisoned maximum 3. The cycle
budget monitor reached four function-47 RM unload entries within fifteen minutes
at 06:03:01; the benchmark was found stopped by the reboot when the abort command
ran. VM shutdown completed at 06:04:51. Seven total unload entries in the final
log include later recovery and clean shutdown; they are not seven distinct TDRs.
See baseline-budget-stop.json, baseline-budget-excerpts.txt and baseline-metrics.json.

MEASURED: saved baseline samples contain six event-153 records, zero observed
4101 and zero ApplicationError1000. Six is a LOWER BOUND: the final since-boot
query followed a new Windows boot and excluded older events. This limitation is
preserved, not replaced with a false zero. The CoW and raw logs remain available
for further diagnostics. Failed subtests are not an exact viewperf kill census.

MEASURED: Phase 1 started SPEC at 06:06:47. CATIA subtest 03 completed at
06:19:42, full CATIA at 06:21:31, Creo at 06:29:17 and Energy at 06:32:39.
Four workloads / 32 subtests completed; Enscape supplied one numeric partial
subtest (33 rows total) before the intentional deadline stop at 06:34:20.
Enscape is Incomplete; Maya, Medical, SolidWorks and Unreal were not run.
The guest task/CLI/harness/viewperf stop targets are in fix-intentional-stop.json;
these intentional cutoff actions are not counted as application failures.

MEASURED: Phase 1 observed a large-Pointer counter maximum of 8005 and a
maximum descriptor length of 1043244 bytes. These are maxima of logged counters,
not a sum of potentially repeated records. The checkpoint after CATIA-03 completed
recorded 30 / 1025068 bytes. The case was exercised: size exceeded 917504 and
947340, CATIA 03 completed, and budget refusals, Scan-budget count, poisoned,
guest153/4101 and host Xids remained zero. Benchmark RM unloads and reboots were
zero; the single unload in the final full log is the clean shutdown.
Exact per-descriptor native marker chronology was not separately captured.

INFERRED hypothesis and FALSIFIER, recorded before hardware: Phase 1 prevents
the Pointer descriptor admission failure. Falsifier: CATIA 03 fails after large
Pointer forwarding with zero budget refusals. MEASURED: it did not fire in this
one fresh viewset run. INFERRED: this supports Phase 1 for the observed case,
not general Windows stability or the unrun workloads. Isolated CATIA repeats,
long-run stability and the broader regression/merge gates remain outstanding.

MEASURED: both arms used independent CoW disks and copied UEFI variables from
the owner's image. Backing size/mtime stayed unchanged. Generated flags were
asserted equal to run 3440 minus KF3_PROF, with KF3_DIAG_PROGRESS=1000. The same
unchanged VF1 FullHD CLI runner requested all nine workloads, one iteration,
with no vsync override. CPU, 12 GiB RAM and 8 GiB kayfabe store were retained.
CODE READ: VF1's reference uses physical 12282 MiB VRAM; its report identifies
capacity/display differences and the below-requirement RAM as confounds. These
are diagnostic references, not submission-compliant scores or pure overhead.
The known null CLI ExitCode limitation remains; workload statuses are separate.

MEASURED: new host Xids and PROGRESS_STRAND event records were zero for both
arms. The latter watcher does not prove all channel execution completed.
NOT MEASURED: broker new-content FPS / 1% / 0.1% gap metrics, exact process-kill
counts and capped-run present consistency. The production broker was retained.

MEASURED: Phase 1 GPU-free suites passed 1046/0 with four existing ignores.
Final Python suites passed 57/0. Separate EXPECTED RED commits held the tests:
0d28fd5b4 (one loader error), 78696f92a (12 errors), b64802c59 (four errors).
Nine hand-applied mutations were each killed by at least two tests (2–4);
all were restored. See validation-summary.json and the three mutation records.
The original unload observer matched an optional leftover-object summary. Tests
proved clean unloads were missed, so the marker was corrected to function-47
entry and the two existing unloads replayed before the budget was reached.
That is a measurement correction, not a runtime/timer change.

CODE READ: the owner's explicit resume authorized a pre-arm ctypes cuInit gate
and one NVIDIA stack reload/recheck only for 999. All resumed K1 gates and the
final gate returned zero, so K1 performed no module reload. Reload ordering and
failure restoration are unit-tested; a real reload was not exercised by K1.
The earlier pre-repair launch refusal is retained in baseline-refusal.json and
its historical restoration/cleanup records; those files do not describe these
successful resumed boots.

MEASURED: Phase 1 clean VM shutdown completed 06:34:39. At 06:35:42 cuInit=0,
GPU=nvidia, audio=snd_hda_intel, nvidia-smi successful, GDM active and Ubuntu
Wayland Active=yes. Own QEMU/brokers/agent/monitors/deadline helpers exited;
owned cargo target deleted, lock released and reacquirable at 06:36:52.
See final-restoration.json and cleanup-final.json. Both CoW images, raw exports,
full qemu.log/serial/broker logs and scripts remain only under the authorized
host lane directory /data/codex-K1/campaign-20261022/ and /data/codex-K1/.
No binary, VM image, credential or closed-binary analysis was committed.

INFERRED owner decision 1 / recommendation: retain Phase 1 as the tested candidate;
run the remaining five workloads and repeated CATIA/regression gates before
broader promotion. No Scan-budget refusal was observed in this limited run;
these results do not establish that Phase 2 is unnecessary for other workloads.
