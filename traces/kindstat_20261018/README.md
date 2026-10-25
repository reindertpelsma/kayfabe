# Optional PTE-kind mapping census

STATUS: LIVE, 2026-10-25. CODE READ: `kf-mem/src/kindstat.rs` observes accepted
ordinary GPU mapping rows after final map verdicts. `host_pte_kind`, permissions,
mapping order, batching, invalidation and acknowledgements remain unchanged.
The previous PF1 interrupt observer is retained separately on this branch.

Ordinary builds omit `diag-kindstat`, compiling out its storage and hooks.
The normal host build flow selects it with `KF3_BUILD_DIAG_KINDSTAT=1`;
runtime collection additionally requires the exact value `KF3_DIAG_KINDSTAT=1`.
The runtime-off diagnostic build performs no counter updates, clock reads or
formatting. No measurement option is required for boot.

CODE READ: the per-device histogram uses fixed arrays on the heap, indexed by
raw guest kind byte, accepted host kind byte, vidmem/sysmem and guest leaf size.
It counts bytes and equivalent 4 KiB pages, split into 4K/64K/2M/unknown/other
guest leaf sizes. Raw unknown kinds are retained. Accepted host kinds are the
mapping requests the host accepted, not a readback of hardware PTE bytes.
The two fixed atomic arrays total 10 MiB, allocated only when opted in.

The apply hook excludes CPU views, held/refused rows, and newly placed pieces
rolled back by a failed linked run. Batch success and per-row fallback use the
same accounting path. It counts mapping activity: aliases and remaps count again;
it does not count unique resident VRAM or DRAM traffic. Kept mappings that
need no new placement do not increment the histogram. A workload with no mappings
in a selected interval therefore has no interval exposure denominator.

CODE READ: existing status lines append `KINDSTAT unix_ns=...` and JSON only when
enabled. Each row includes `raw`, `host`, `ram`, guest `page`, cumulative `bytes`
and `pages4k`, plus rates since the previous report. Snapshot subtraction supplies
explicit interval counts. Top-level totals include mapped, compressible and
stripped bytes. Compressible classification comes from `kf-chip`'s source-derived
uncompressed-kind table. Divide stripped bytes by all mapped bytes for exposure,
and by compressible bytes for the fraction of compressible mappings stripped.

`KINDSTAT SUMMARY` is emitted at most once on device shutdown or normal QEMU
process exit. Both paths share the same once guard; the exit notifier is registered
only when opted in. Rust prints the summary without the C status buffer limit.
This is a shutdown-request snapshot, not a claim that all producer threads were quiescent.
Snapshots are independently atomic and may straddle concurrent mapping activity.
No counter or formatting work runs on the vCPU path.

MEASURED, run `pf1-kindstat-tests-20261025`: `expected-red.txt` records two failing
source-hook tests before implementation, in tests-only commit `b6a563076`.
Runtime contracts were included before the optional module existed. Validation
and mutation receipts are in `validation.txt` and `mutation-results.json`.
Every listed mutant has at least two killing tests. Hardware results stay private.

MEASURED, run `pf1-kindstat-exit-tests-20261025`: `exit-expected-red.txt` records
both normal-exit wiring tests failing in tests-only commit `95120c5fc`.
`exit-mutation-results.json` records three wiring mutants, each killed by both
added tests. The final full feature-on/off suites and seam checks are recorded in
`validation.txt`. These source wiring checks do not substitute for a real exit log.
