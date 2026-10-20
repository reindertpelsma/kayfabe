# H1: hitch timeline, Part A

STATUS: RESEARCH, updated 2026-10-20. Branch `codex/hitch-timeline-20261019`, based on
published candidate-5 `f8f9d1a5f`. Part B is gated on coordinator `go`.

[PREREG.md](PREREG.md) records S1–S9 falsifiers, Bedrock/Java, aging, the VRAM pair
and the expanded budget conflict. The requested four-hour window cannot fit
all required workloads plus a three-hour soak; no scope reduction is assumed.
No VM, GPU, guest diagnostic channel or GPU lock has been touched by this lane.

**OFFLINE IMPLEMENTED; IMPORT PENDING.** The coordinator assigned the single
runtime profiler/hooks/collectors to K1 and forbade a separate H1 ring.
[STOP.md](STOP.md) preserves the historical broker/clock discrepancy; that
ownership question is resolved. [K1_IMPORT.md](K1_IMPORT.md) records the current
format dependency. The published design and reader EXPECTED RED fixture do not
yet provide the complete JSONL contract or a real writer-produced fixture.
MEASURED: 115 passing tests; 25 mutants, each killed by at least two tests.
Tests-first EXPECTED RED stages precede each implementation stage. See
[tool schema/usage](../../scripts/bench/hitch/timeline/README.md),
[validation.txt](validation.txt) and the five mutation tables. No H1 runtime
recorder, collectors or binaries were built. Part B remains NOT GO, and the
coordinator will supply the `-prof` binary with the release.
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

## Java and S9 pre-TDR context

MEASURED by the coordinator: run 3017 (mirror OFF, iothread disk, Minecraft Java)
fell 57 → 45 → 31 → 2.9 → 0 fps before nvlddmkm 153 x3 at 16:04:30 CEST.
The reported disk await of 0.1–0.5 ms/no inflight excludes storage as the trigger
for that observed window. REFUSED/absence/unsettled counts matched production
3011. These are coordinator observations, not H1 guest/ETW measurements.

MEASURED: S9 reports ten pre-TDR one-second observation bins, separate guest
and broker series, first/last two-second count bounds and uptime. It requires
full exposure, active-game PID, a verified TDR, no loss and precise calibration.
Unknown inputs remain UNKNOWN; generic Event 153 does not create a TDR. S9
describes degradation and preserves each gap's S1–S8/UNKNOWN causal class.
The 31 S9 tests followed 26 EXPECTED RED cases (30 errors including subtests);
seven mutants were killed by 9/6/3/2/3/2/2 tests. Original code was restored.
CLI smoke passed on synthetic QPC-calibrated S9 data with a three-hour uptime
anchor and separate S1 cause. No owner stall or recorder soak was measured.

CODE READ: latest checked K1 revision
`d93d35a4133f6dc050305f14d0320a64e2522da6` has reader/runtime requirement tests,
but no `reader.py`, runtime JSONL writer or complete format document. H1's
importer remains conditional on that external contract. A read-only check of
the named saved sampler found 02:09–02:15 clock labels without run/monotonic
identity, so H1 did not align it to the coordinator's 16:04 run-3017 event.

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
