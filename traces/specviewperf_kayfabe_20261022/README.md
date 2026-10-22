# K1 SPECviewperf 15 campaign, 2026-10-22

STATUS: IN PROGRESS. This checkpoint records the plan before hardware runs.

CODE READ: requested baseline is the installed 89fc8a222 binary; Phase 1 is
468a35c9d845afc1938683540acc251bdef90e5a. The evidence branch starts at
origin/master e64b8e5f6. No production code change is planned.

MEASURED: bounded 120-second flock acquisition succeeded at 05:04:50 CEST.
Under the lock, the GPU was bound to NVIDIA, nvidia-smi reported RTX 4070 /
595.91.07 / 0 MiB used, GDM was active, and no QEMU was listed. The published
VF1 report records both full native iterations and independent restoration.
G1's published report records 1046 passing GPU-free tests, four existing ignores,
and thirteen mutants each killed by at least two tests. K1 is rebuilding that
exact source in its own host directory, with fresh GPU-free tests first.

INFERRED hypothesis, stated before running either arm: mode-scoped descriptor
admission prevents CATIA's observed budget failures. FALSIFIER: CATIA subtest
03 fails after oversized Pointer entries are forwarded, with zero budget
refusals. A Scan/Window budget refusal instead falsifies the assumption that
Phase 1 covers the encountered descriptors. Zero oversized Pointer entries
means that the target case was not exercised; it is not a successful fix proof.

CODE READ: each arm uses a separate CoW of the same owner disk and copied UEFI
variables, production run-3440 flags excluding KF3_PROF, with
KF3_DIAG_PROGRESS=1000. The unchanged VF1 supported CLI runner requests all nine
workloads in the same order, FullHD, one iteration. No vsync override is applied.
The runner has a known null ExitCode defect; workload completion and vendor
composites will be reported independently, without assuming a zero CLI exit.

CODE READ: stop on a new host Xid, more than three TDR cycles in any fifteen
minutes, or the restoration deadline. Do not extend the deadline or restart a
failed arm. Restore NVIDIA/GDM by 06:45 CEST. Both native iterations took about
84 minutes; the remaining deadline may prevent complete coverage of both arms.
Incomplete/failed results will not be turned into workload composites.

MEASURED: the six unchanged VF1 runner/export/parser and test files imported
into this evidence branch pass 26 GPU-free Python tests. No new harness or
product behavior is claimed by that import. Native reference values and n=2
spread are retained in vfio-composites.csv; no overall suite score is inferred.
