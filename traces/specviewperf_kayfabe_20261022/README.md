MEASURED: Phase 1 CATIA 03_ThreeLoftJetsDarkMirror completed at 06:19:42
CEST. Counter checkpoint: pointer_large_entries=30, maximum descriptor 1025068
bytes, zero budget refusals/poisoned/guest153/4101/hostXid. The large case was
exercised and survived in this n=1 run; full CATIA composite is pending.

MEASURED: Phase 1 Blender completed with official composite 35.95 (n=1),
versus baseline 35.85 (n=1): +0.28% in these samples. VFIO mean 61.11 (n=2).
CATIA is next; Phase 1 has zero observed recovery cycles/TDR records/Xids.
This is a workload checkpoint, not full-suite or performance-regression proof.

MEASURED: baseline Blender completed all seven subtests, official vendor composite
35.85 (n=1), versus VFIO 61.61/60.61 (n=2; mean 61.11), baseline/native-mean
0.5866. CATIA began next; no guest TDR or host Xid observed at this checkpoint.
See baseline-scores.csv and baseline-subtests.csv. Remaining measurements pending.

# RESUMED 2026-10-22, after owner-authorized host CUDA repair

MEASURED: new baseline run 3453 launched at 05:44:27 CEST, cuInit=0, no reload
required by K1. SPEC started at 05:46:54 CEST through the working documented
owner command channel. QGA did not answer. All nine workloads, FullHD, one
iteration, original flags and no vsync override; results pending.
CODE READ: owner explicitly authorized one NVIDIA module reload and recheck if
cuInit returns 999 again. New pre-arm gate tests: EXPECTED RED 12 errors;
final Python suites 53 passed. Four new mutants killed by 2/4/3/3 tests.
MEASURED: the pinned Phase 1 binary remains the previously green built revision.
The earlier refusal below is historical; it does not describe this resumed run.

# K1 SPECviewperf 15 — stopped at baseline device realization

STATUS: STOP / PARTIAL, 2026-10-22. No new SPEC workload ran.

MEASURED: baseline 89fc8a222 was launched once at 05:21:39 CEST through the
production launcher on its own CoW. QEMU exited before boot with:

```text
kf3: realize refused: GPU walker bring-up on 0000:01:00.0: cuInit refused: 999 (CUDA_ERROR_UNKNOWN)
```

CODE READ: the lane requires STOP after a safety refusal. No retry, CUDA repair,
replacement binary, flag change or second arm was attempted. The descriptor
arm was built successfully but not launched. This is a host CUDA initialization
failure observed during the baseline launch; its underlying cause is unverified.
NVIDIA being bound and nvidia-smi succeeding do not establish CUDA initialization.

MEASURED: exact Phase 1 source 468a35c9d845afc1938683540acc251bdef90e5a passed
fresh GPU-free release suites: 1046 passed, zero failed, four existing ignores,
36 groups. build_kf3.sh installed kf3-bins/468a35c9 at 05:20:49 CEST. Baseline
used installed kf3-bins/89fc8a222. SHA-256 values are in binary-hashes.txt.
Full logs remain in the authorized host K1 directory: phase1-tests-20261022.log,
phase1-build-20261022.log and campaign-20261022/boundary-kayfabe-3451/qemu.log.

MEASURED: production flags were read from run 3440, KF3_PROF excluded and
KF3_DIAG_PROGRESS=1000 set. Generated command.json was asserted equal to that
flag map. Both arms had fresh CoW disks and copied UEFI variables prepared;
owner backing size and mtime were unchanged after cleanup. The unchanged VF1
CLI runner was prepared for FullHD, all nine workloads, one iteration and no
vsync override; it was never invoked because the guest did not boot.

MEASURED: comparison.csv retains all nine native workload composites from both
VF1 iterations. Baseline/fix scores and ratios are blank, not zero; every new
workload is NOT_RUN. No failing SPEC workload or descriptor-fix outcome can be
identified from a preboot refusal. The native scores are attributed VF1 evidence,
not new K1 results. No overall suite composite is calculated.

INFERRED hypothesis stated before the attempted run: Phase 1 prevents the
Pointer-descriptor budget failure. FALSIFIER: CATIA subtest 03 fails after large
Pointer entries are forwarded with zero budget refusals. MEASURED: not tested;
there are zero executed workload subtests and no large-entry observation.

MEASURED: new host Xids = 0 in the kernel journal from lock acquisition at
05:16:11 through cleanup. The log has zero budget/progress event records because
device initialization refused; these zeros are not an exercised pass. Guest
153/4101 events, viewperf kills, TDR cycles, pointer_large_entries, presented FPS
and frame-gap metrics are unavailable. No guest or performance claim follows.

MEASURED: the lane reused 26 runner/parser tests; the added arm guard contract
started with one missing-module loader error in EXPECTED RED 0d28fd5b4. Final
Python suites: 41 passed, zero failed. Four hand-applied mutations each fail two
independent tests; arm-mutations.json records them. No product Rust/C/CUDA change
was made. The configuration helper retains explicit zero production flags and
implements new-Xid, >3 cycles/15 minutes and restoration-deadline abort rules;
those pure tests do not establish end-to-end watcher operation in a booted VM.

MEASURED: both bounded 120-second lock acquisitions succeeded without breaking
a lock. At 05:24 CEST NVIDIA remained bound, nvidia-smi reported RTX 4070 /
595.91.07 / 45 MiB, GDM was active and Ubuntu Wayland was Active=yes. Own QEMU,
broker and deadline helper had exited. Own cargo target, both overlays and copied
variables were removed; the lock was released. See restoration.json/cleanup.json.
The installed per-revision binary and its supporting QEMU build remain available.

CODE READ recommendation: restore/verify host CUDA health independently, then
explicitly resume both original arms; keep the pinned revisions and settings.
No CUDA reset, driver reload or timer workaround was attempted after the refusal.
