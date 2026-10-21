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
