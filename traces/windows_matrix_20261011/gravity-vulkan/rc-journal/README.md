# Bounded host RC journal reader

STATUS: RESEARCH, 2026-10-11. Bench only; no production caller or GPU work.

Hypothesis: the host RC journal preserves additional fault details for the
measured GravityMark Xid32. Falsifier: a matching record has no such details.
A refused query, absent/overwritten record or changed count is inconclusive.
Hardware execution pending: same-GPU VFIO detached before the build completed.

Public source: NVIDIA/open-gpu-kernel-modules tag595.91.07, commit
9f087a6d4e86d85acc0ce1d354d6276fe5047b29, src/common/sdk/nvidia/inc/
ctrl/ctrl2080/ctrl2080rc.h. Header SHA256:
e726cebbec725742205231d7a85a88366731459782b61ca35d5722db3ab82e12.
All four transitive project headers (nvtypes.h, cpuopsys.h, ctrl2080base.h,
ctrlxxxx.h) also fetched from this exact tag; gcc dependency output verified.
layout.c compiles the public sizeof/offsetof contract, rather than capturing
runtime bytes. layout-595.91.07.txt is compiler output, not a per-die table.
The exact driver must match the selected tag; unmeasured tags are refused.

kf-rc-journal-probe accepts that layout and a new private output directory.
It authors only GET_ERROR_COUNT/GET_ERROR_V2 on its own subdevice, reads at
most32 records of at most8192bytes, preserves refusal, validates lengths and
rechecks count. No history clear, watchdog change, channel or memory creation.
Run unprivileged. Raw records may contain diagnostic addresses and stay on
trusted/private storage. A stable count is not proof that a ring never changed.

Validation: rustfmt and cargo build -p kf-harness --bin kf-rc-journal-probe
PASS. No hardware pass claimed. No driver ABI or interface comes from private
analysis. This bench-only public layout does not alter product constant tables.
