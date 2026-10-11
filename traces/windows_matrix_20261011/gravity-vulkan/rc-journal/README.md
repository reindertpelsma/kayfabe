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

## Measured query and next discriminator

At19:47:15UTC, after VFIO restored and before the frozen3252 campaign,
uid65534 GET_ERROR_COUNT refused with0x1b INSUFFICIENT_PERMISSIONS.
No raw record was returned. This does not establish an empty journal.
The query had no channels/GPU work and did not change the product or history.

Separate native lifetime diagnostic is queued under the serial GPU lock after
the production-default controls and before manual VM creation, never interrupting that VM.
Same fresh binary runs nop, update, update-hold. The final mode changes only
retaining all resources for2s AFTER an actual GPU tail, before any cleanup.
Hypothesis: earlier native update fault is caused by freeing resources before
the non-stalling deferred action completes. Falsifier: Xid timestamp falls
between HOLD_BEGIN and HOLD_END, while those resources remain owned. A fault
only after cleanup keeps the hypothesis alive; no fault alone does not prove
the deferred action succeeded. This is bench observation, never a product fix,
release timeout, forged completion or Windows pass claim.

All native channels stay USER/uid65534. A separate root read of the bounded
global diagnostic journal follows each case, since its unprivileged form was
refused; raw records stay private. This diagnostic read supplies no product
interface/constants and never raises channel or guest privilege. Native API
failure is not proof that GravityMark has the same initiating cause. Hardware
results pending; build and Python/shell syntax checks pass.

Exact595.91.07 public g_device_nvoc.c has GET_PTE_INFO flags0x100008
(TEST_ONLY), whereas GET_PDE_INFO has0x10008. Thus direct leaf interrogation
cannot be assumed available in a production driver. A present PDE only proves
that a page-table directory exists, not that its leaf is valid or that the TLB
contains the current mapping. No full PT/RM agreement claim follows.

The first read-only stage finished19:47:16UTC. The later attempt to add the
native hold test to that pending stage arrived after it had already executed;
the hold test did NOT run there. Its separate waiting process was canceled
before acquiring the lock, and the test now precedes manual VM creation via
run_before_manual.sh. Active control runners were not edited. Original manual
script remains frozen SHA256
beb0513009bc0af205c5d8da44559ad0fc9020feb67fc07b1ef74fff9b9c7ad7.
The owner session itself has no test automation or expiration; native outcomes
cannot be substituted for its independent product controls.
