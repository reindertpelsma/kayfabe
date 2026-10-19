# H1: hitch timeline, Part A

STATUS: RESEARCH, 2026-10-19. Branch `codex/hitch-timeline-20261019`, based on
published candidate-5 `f8f9d1a5f`. Part B is gated on coordinator `go`.

[PREREG.md](PREREG.md) records falsifiers and the three-hour hardware plan.
No VM, GPU, guest diagnostic channel or GPU lock has been touched by this lane.

## Existing counter evidence

MEASURED, read-only scan of existing run logs, 2026-10-19; these are lifetime
maxima across status snapshots, not latency distributions or dated hitch events:

| Run | Snapshots | Invalidate max (us) | Act max (us) | PRAMIN max (us) | Mirror max (us) | Zero-window count with nonzero invalidate max |
|---|---:|---:|---:|---:|---:|---:|
| 3014 | 1089 | 362665 | 31290 | 12546 | 1806 | 982 |
| 3015 | 2609 | 109493 | 875734 | 7379 | 2401 | 2563 |

CODE READ: `kf-mem/src/vasmgr.rs`, `VaTiming::since`, subtracts cumulative counts
and sums, but retains lifetime `inval_ns_max` and `leaves_last`. `device.rs`
formats that mixed snapshot as `vat[...]`. Thus `invals=0 max_us>0` is expected.
`complete_invalidate` times VA-manager acceptance to trigger completion (including
deferred target settlement); acceptance timestamps are created in `invalidate`.
It excludes earlier vCPU-to-drainer/manager delay. `walk_ns` is submit-to-collected
wall time; `gpu_us` comes from the walker; plan/apply are separate sums.
INFERRED: these maxima justify event-level measurement, but identify no culprit.

## Coordinator memory evidence

MEASURED by the coordinator at 14:20 (not independently resampled here): 30 GiB
RAM, 21 GiB used, 14 GiB shmem, 3.1 GiB free, 9.5 GiB available, no swap; THP
enabled/defrag=madvise. Eleven-day totals: allocstall_normal=383369,
pgscan_direct approximately 2.46e9 pages, compact_stall=146428,
thp_fault_fallback=4966; memory PSI some=2694 s, full=2689 s; avg10/60=0.
INFERRED: averages over eleven days are about 0.403 allocstalls/s,
2588 direct-scan pages/s and 0.154 compaction stalls/s; PSI full accounts for
about 0.283% of wall time. These totals include previous builds and VM sizes,
cannot locate a hitch, and say nothing about which thread stalled.
