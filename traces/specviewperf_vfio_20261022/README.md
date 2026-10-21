# VF1 SPECviewperf 15 physical-GPU reference — 2026-10-22

MEASURED: lane started 2026-10-21 23:19 UTC (01:19 CEST); five-hour stop is
06:19 CEST, before the 06:30 desktop restoration deadline. Host initial Xid count
26. Branch starts at origin/master e64b8e5f6. GPU PCI ID 10de:2786 (RTX 4070),
audio 10de:22bc; guest NVIDIA driver 580.88. No product change is planned.

CODE READ: the owner launcher links run 3440 to the specified persistent disk;
MEASURED: the two disk names resolve to the same inode. The VFIO disk must be a
new overlay, with copied UEFI variables, after the owner VM exits.

MEASURED: third snapshots preserved before shutdown: QEMU log, active broker log,
and guest System/Application events. The manifest lists their hashes and sizes.
Full logs stay on the trusted test host in the owner-designated evidence folder;
the lane folder holds 21 exported SPEC text files (run logs, XML and stdout).
The first export met a Windows sharing violation on an active workload log;
ordinary shared-read file access completed the export without changing the files.

INFERRED hypothesis before inventory: the current guest contains SPEC and its
requested result directory. Falsifier: either is absent. MEASURED: both present,
with nine installed workloads. 3DMark Demo is installed as well.

MEASURED: preserved run log reports blender, catia and creo failed; energy marked
completed; enscape still active. Remaining workloads had no output at snapshot.
This is later evidence than the lane handoff. No complete kayfabe suite score is
claimed; the partial XML is not a full workload result.

CODE READ: SPEC documents a supported CLI, selected-workload and FullHD switches,
and result CSV output in its [user guide](https://gwpg.spec.org/wp-content/uploads/2025/12/SPECviewperf15-User-Guide.pdf).
The installed CLI's help and installed workload inventory will govern execution.
SPEC documents at least 15 GB available RAM; this lane explicitly requires the
owner's 12 GiB VM configuration. Treat these as diagnostic reference runs, not
submission-compliant SPEC scores, and keep resolution/settings attached to rows.

## Final disposition — safety stop

MEASURED: **desktop restoration is PARTIAL**. NVIDIA remains bound to the GPU,
`snd_hda_intel` remains bound to audio, GDM is active, `nvidia-smi` reports RTX 4070
and host driver 595.91.07, and SSH works. At 01:27:56 CEST, loginctl shows a GDM
greeter on seat0 and no ubuntu desktop session. The original desktop session did
not survive the authorized display-manager stop. No authentication or persistent
login-policy change was attempted.

MEASURED: the owner VM cleanly exited at 01:22:56 CEST. A new 500 GiB virtual
copy-on-write overlay and copied UEFI variables were prepared under the lane
folder. The supervisor acquired the GPU lock, stopped the display manager, then
its open-device safety gate refused: existing PIDs still held NVIDIA/DRM nodes.
See `safety-refusal.txt`. No GPU unbind, VFIO bind, or native QEMU launch occurred.
The rule requiring STOP after a safety refusal was followed; there was no retry.

CODE READ: the supervisor's `restored: true` in `session-result.json` means its
restoration commands and driver checks succeeded. It does not prove a user desktop;
the separate loginctl check above is authoritative for that missing outcome.

MEASURED: Xids remained 26 (delta zero). The unused overlay was removed at
01:27:56 CEST; supervisor/flock PIDs had exited, no lane QEMU had been started,
and a fresh nonblocking flock succeeded after cleanup. No cargo build or lane
target directory was created. All lock acquisitions used bounded waits (120 s);
no lock was broken and no wait timeout occurred.

MEASURED: new safety tooling tests first failed because its module was absent
(EXPECTED RED commit 8136cd342). The standalone-QEMU-switch regression then ran
16 tests with one error (EXPECTED RED commit 4898f89d9); final 16/16 pass. Mutation
checks killed five mutants with 2, 2, 2, 2 and 7 failing tests respectively (see
`mutations.json`). Successful hardware detach/boot and a full desktop restoration
were not verified. No production Rust/C/CUDA code changed.

MEASURED: preserved kayfabe stdout contains 27 numeric subtest rows; they are in
`kayfabe-partial-fps.csv`. Numeric rows do not establish a passed workload, as the
run-level status below shows. Composite scores were not recovered or fabricated.

| Workload | Kayfabe snapshot3 status | VFIO runs / FPS / composite |
|---|---|---|
| blender-01 | Failed; tests 02–07: 27.36 / 51.20 / 60.06 / 27.83 / 36.28 / 60.06 FPS | 0 / unavailable / unavailable |
| catia-07 | Failed; four numeric subtests recovered | 0 / unavailable / unavailable |
| creo-04 | Failed; three numeric subtests recovered | 0 / unavailable / unavailable |
| energy-04 | Completed in run log; six numeric subtests recovered | 0 / unavailable / unavailable |
| enscape-01 | Still active; eight numeric subtests recovered | 0 / unavailable / unavailable |
| maya-07 | No output at snapshot | 0 / unavailable / unavailable |
| medical-04 | No output at snapshot | 0 / unavailable / unavailable |
| solidworks-08 | No output at snapshot | 0 / unavailable / unavailable |
| unreal_engine-01 | No output at snapshot | 0 / unavailable / unavailable |

INFERRED: no largest VFIO-versus-kayfabe gap or repeatability spread can be ranked:
there are zero native samples. The physical-GPU hypothesis is untested because
execution stopped before attachment, not falsified by a workload failure.

CODE READ (lane handoff): broker output refresh is 59.996 Hz. MEASURED: several
preserved guest rendering results exceed 60 FPS (up to 205.42); therefore broker
output pacing and benchmark rendering FPS must be kept distinct. No VFIO refresh
or cap was measured, no vsync override was applied, and no matched uncapped
comparison was performed.

CODE READ: the supported CLI is the appropriate automation entry point; use
`--list`, then the nine installed workloads at `--resolution FullHD`, in an
interactive-logon kfadmin scheduled task with files under the public profile.
Collect official CSV scores plus workload statuses, and preserve each iteration
separately. This is a recommendation, not a delivered/tested suite runner. Full
suite x2, selected repeats and Steel Nomad were NOT RUN; the delivered code is the
VFIO launch/restoration supervisor and its GPU-free tests only.
