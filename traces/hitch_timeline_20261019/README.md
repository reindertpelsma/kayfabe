# H1: hitch timeline, Part A

STATUS: RESEARCH, 2026-10-19. Branch `codex/hitch-timeline-20261019`, based on
published candidate-5 `f8f9d1a5f`. Part B is gated on coordinator `go`.

[PREREG.md](PREREG.md) records S1–S8 falsifiers, Bedrock, aging, the VRAM pair
and the expanded budget conflict. The requested four-hour window cannot fit
all required workloads plus a three-hour soak; no scope reduction is assumed.
No VM, GPU, guest diagnostic channel or GPU lock has been touched by this lane.

**PARTIAL.** The runtime recorder remains stopped under the lane's contradiction
rule; [STOP.md](STOP.md) records the missing broker/clock contract and options.
Coordinator additions 2–4 were implemented independently in the offline tools:
MEASURED: 84 passing tests; 18 mutants, each killed by at least two tests.
Tests-first EXPECTED RED stages precede each implementation stage. See
[tool schema/usage](../../scripts/bench/hitch/timeline/README.md),
[validation.txt](validation.txt) and the four mutation tables. No runtime
recorder, collectors or binaries have been built. Part B is not ready.
MEASURED: unchanged Rust baseline passed 1038 tests, 15 ignored, zero failures;
[validation.txt](validation.txt) records the command and cleanup limitation.
The lane-owned target directory remains (204 MiB): deletion was rejected by
the execution safety check and the lane forbids working around a refused step.

## Bedrock, S8 and reply waits

[BEDROCK_EVIDENCE.md](BEDROCK_EVIDENCE.md) distinguishes coordinator measurements,
H1 read-only snapshots, code semantics and unresolved provenance differences.
INFERRED priority is S8 (guest reply wait), then S1 (invalidate apply), then the
remaining suspects. No freeze has been attributed from real ETW in this lane.

MEASURED: offline tools report ten-minute uptime bins with actual exposure;
per-gap invalidate arrival/clear/apply intervals, CE2/CE3/GR0 completion rates
only with coverage, bracketing residency samples, and held/forwarded reply
phases plus longest waits and per-gap overlap. S8 requires verified guest
waiter identity and bounded clock error; otherwise the causal class stays
UNKNOWN. Pending/missing phases do not become zero durations or fake posts.
MEASURED: CLI smoke passed on synthetic data with a three-hour uptime anchor;
this is not a three-hour recorder/soak test.

MEASURED: [3014 long acts](long_acts_3014.tsv),
[3015 long acts](long_acts_3015.tsv): 12/9 printed acts >20 ms. JSON companions
preserve parse coverage and unknown identity/timing fields. The 3015 table
reproduces SET_TIMESLICE acts of 265635/875734 us; these are whole-closure wall
times, not scheduling quanta or isolated RM-call durations.

MEASURED: [held-reply snapshot](held_reply_counts_3015.json) contains 4055 holds
and 4055 n=1 posts: RmControl=1996, RmAlloc=1186, Free=872, UpdateBarPde=1.
CODE READ: the common "map reply" log text also covers deferred acts. These
untimestamped totals neither measure wait duration nor prove gameplay overlap.
The coordinator's Bedrock-running sample (2010/8192 MiB dedicated, 2396 MiB
committed, 268 MiB shared; host 9093/12282 MiB) weakens overcommit at that instant;
the stall peak remains unmeasured.

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
deferred target settlement); acceptance timestamps are created in `on_invalidate`.
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
