# V3 — bounded phase profiling of the VA apply path

**STATUS: LIVE, 2026-11-16 (code on branch `claude/va-profile-20261116`; GPU-free tests and mutants only; NOT
run on hardware).** Observation only: no acknowledgement, commit, capacity limit, ordering or refusal changes.
Its first use is the next FG-off Wukong run on the pc at 4096 MiB (`/root/night/PC_WUKONG_20261115.md`). Why it
exists: the design-only analysis of 2026-11-16 (`/root/night/CODEX_VA_LATENCY_20261116.md`, "smallest justified
follow-up") found that the existing counters cannot say where the 1-3.4 s of one guest-visible VA apply go
(preparation, stitching, host RM map or unmap, ledger, inbox delay). This document says what is recorded, where,
what each printed field means, and how to read it.

## What is recorded, and where

| quantity | where it is taken | cost |
|---|---|---|
| worker enqueue time of a split ticket | `kf-qemu` `Inbox::request_split` / `request_gated_split` / `request_user_split`: one `Instant::now()` stored in the existing queue item, under the lock the call already takes | 1 clock read |
| manager acceptance; `q_inbox` = acceptance - enqueue | `VaManager::on_split_queued` / `on_split_space_queued` (the VA thread; the old `on_split` / `on_split_space` pass no enqueue time) | 1 clock read (existing) |
| per window: submit wall, collect/decode wall (`Walker::poll` that returned the report), hidden capacity re-walks (`Walker::capacity_retries`, the walker's `CapacityStats::retries`, moved between submit and collect) | `VaManager::pump`, `VaManager::on_walk_ready` | 2 clock reads per window |
| per space apply: wall, and its partition `prep` (planning: kept/overlap pairs, linked failures, host rows), `unmapph` (unmap step), `mapph` (usermode/SKED verbs, batched and single maps), `takedown` (new pieces of a failed MAP run), `inval` (the one host invalidate) | `kf_mem::apply::apply_entry` records into `kf_host::phase` (a thread-local accumulator, no lock, no atomic shared with another thread, no allocation) | 5 clock reads per apply |
| host verbs: `rm_map` (row, SKED row, or stitched batch whole), `rm_unmap` (whole, row, range, through a reservation), `rm_free`, `rm_resv` (count, sum, max) | `kf_mem::batch::BatchedVas`, around each `SpaceVerbs` call | 2 clock reads per verb |
| inside a stitched batch map: `stitch` (the `MAP_FIXED` mmaps), `desc` (OS descriptor: RM pins the pages), `reap` (hand the view to the reaper) — subsets of that call's `rm_map` | `kf_host::channel::HostRm::map_scattered_through` (it already timed them; it logged only the first 32) | none new |
| `ledger`: the ledger's work after an unmap call landed (own-map cut, book, emptied objects), the frees it makes excluded | `BatchedVas::unmap_span` | 2 clock reads per landed unmap call |
| per ticket at its verdict: windows ridden, summed submit/walk/collect, summed apply of its own spaces and of every space of its windows, re-walks, request latency (enqueue, else acceptance, to verdict) | `Want::Split` carries a `Copy` `TicketProf` across its windows; `VaManager::note_ticket_prof` | none per window beyond the above |
| result publication and the worker's take (`pub`, `resume`, `e2e`) | `Inbox::finish_split_timed` stamps the result in the existing result map; `Inbox::split_result` (the worker) records into relaxed atomics after the lock is released | 2 clock reads per ticket |
| slot-pool geometry at a ceiling | `kf_cuda::capacity::Capacity::slot_geometry`, pushed as a `walk capacity ceiling geometry:` event after the existing ceiling event (first 16 ceilings of a walker) | only at a ceiling |

What the VA thread keeps: fixed counters and two 32-bucket log2 histograms in `VaTiming::prof`
(`kf_mem::vaprof::VaProfCounters`), a ring of the newest 64 slow records (`SlowRing`, boxed once at construction,
never grows), and a map ticket -> enqueue time in `Device::va_loop` bounded at 4096 entries (past the bound a ticket
is simply not profiled). No lock is added and none is taken that a vCPU takes; nothing runs in a trap.

Overhead (MEASURED 2026-11-16, a micro-benchmark on the dev container, not on the pc; not committed): `Instant::now()`
36-56 ns, one thread-local phase record 20-36 ns, a timed empty closure 74-88 ns. INFERRED from the 2026-11-15/16 PC
records (a1:22605: 27 batch calls and 10368 single RAM maps in an 842 ms apply): about 10^4 timed verbs, about 1 ms,
about 0.1 % of that apply. The per-apply and per-window additions are under 1 us.

## The printed lines

**1. The status line's `vat[...]` block** (the drainer's heartbeat, at most every 2 s, printed when it changed).
The old fields come first, unchanged in name, order, unit and meaning (`vat_old_fields`; tested byte for byte
against the 2026-11-13 format string): `invals arrive->clear_avg_us max_us walks walk_avg_us gpu_avg_us plan_avg_us
apply_avg_us leaves host_calls apply_max_ms split_max_ms slow_applies slow_splits`. The extraction
`vat\[(.*?)\]` then `(\w+)=(\d+)` keeps working; every new field is `name=integer` and no new name repeats an old one.

New fields, appended (window since the previous status line unless named `*_max_*`, or listed as boot-wide):

| fields | meaning |
|---|---|
| `q_inbox_n q_inbox_avg_us q_inbox_max_us` | split tickets with a known enqueue time: enqueue -> manager acceptance (`Q_inbox`) |
| `submit_n submit_avg_us submit_max_us` | walk submits (the VA thread's wall) |
| `collect_n collect_avg_us collect_max_us cap_retries` | `Walker::poll` calls that returned a report (decode, validation, conversion; hidden re-walks INCLUDED in the walk wall, not here) and the hidden re-walks counted |
| `req_n req_avg_us req_p50_us req_p95_us req_p99_us req_max_ms` | split tickets that reached a verdict after an apply walk: enqueue (else acceptance) -> verdict. Percentiles are the exclusive upper bound of a log2 bucket of microseconds: `req_p95_us=2097152` means 95 % were under 2.1 s |
| `aps_n aps_avg_us aps_p50_us aps_p95_us aps_p99_us aps_max_ms` | per-space applies (`aps` = apply per space; the old `apply_avg_us` divides by walks, not spaces) |
| `<phase>_n <phase>_us <phase>_max_us` for `prep unmapph mapph takedown inval rm_map rm_unmap rm_free rm_resv ledger stitch desc reap` | count, summed us and boot-wide max us of that phase over the window's applies |
| `slow_reqs slow_aps windows_max slow_printed slow_suppressed ring_overwritten` | boot-wide: tickets and applies over 250 ms, the most windows one ticket rode, slow records printed live / held back by the print budget / overwritten in the ring |
| `pub_n pub_avg_us pub_max_us` | enqueue -> result published by the VA thread |
| `resume_n resume_avg_us resume_max_us` | result published -> the worker took it (its resume) |
| `e2e_n e2e_avg_us e2e_p50_us e2e_p95_us e2e_p99_us e2e_max_ms` | enqueue -> the worker took the result: the full worker-visible latency |

The partition `prep + unmapph + mapph + takedown + inval` is the apply's wall less the bookkeeping between
steps; the host-verb phases are nested inside it (`rm_unmap` and `ledger` inside `unmapph`, `rm_map` inside
`mapph`), and `stitch + desc + reap` inside `rm_map`. So `rm_map - stitch - desc - reap` is the RM map proper.
Phases of statement handling between applies are discarded (the accumulator is taken before each apply).

**2. `kf3: VA_SLOW n=<seq> kind=apply ...`** — one space's apply over 250 ms: `t_ms` (since the VA manager
started), `ticket` (the first split of the walk naming the space, 0 for an invalidate or a root walk), `walk`
(the `walk#` of `SLOW-APPLY` and `KF3_MAPLOG`), `key`, `pdb`, `slot`, `window` (this walk's ordinal for that request),
`apply_us maps unmaps`, and every phase as `<phase>_n <phase>_us <phase>_max_us`.

**3. `kf3: VA_SLOW n=<seq> kind=ticket ...`** — one split over 250 ms enqueue -> verdict: `ticket walk key pdb
windows req_us q_inbox_us mgr_us submit_us walk_us collect_us apply_own_us apply_all_us cap_retries`.
`apply_all_us - apply_own_us` is the head-of-line time other spaces' applies in the same windows cost it.

Both kinds go into the ring and are printed at once while the print budget allows: 16 lines back to back, one
more every 4 s, at most 256 per boot. So a relaunch minutes after an early storm prints its own records (the old
`SLOW-APPLY` cap of 20 per boot hid them). The old `SLOW-APPLY` / `SPLIT-LATE` lines are unchanged.

**4. At teardown** (end of `Device::va_loop`): the held records not printed live, each suffixed `(at teardown)`, then
`kf3: VA_SLOW SUMMARY recorded held overwritten printed_live suppressed_live dumped_now slow_reqs slow_aps req_max_ms
aps_max_ms windows_max`. Lost if QEMU is killed; the live lines and the status line are not.

**5. `kf3: walk capacity ceiling geometry: slot S region OFF+CAP required R pool P free F in N region(s),
largest [OFF+CAP ...]; adjacent free: head H tail T (in-place growth would fit|would not fit); neighbours left
sI@OFF+CAP right ...; slots held K`** — after a `walk capacity ceiling: slot pool exhausted` line. "would fit"
means the slot's own region plus the free run directly after it holds the required size.
**Correction 2026-11-18:** adjacent-tail growth IS implemented now (`Capacity::slot_needed` falls back to it when no
free block holds a new region; `docs/design/V3_REPLAY_TESTS.md`, "The fix"), so a ceiling line followed by "would
fit" should no longer occur; if one appears in a log the fix is wrong, or the pool changed between the two lines
(a GPU-free replay says it cannot for a single walker). Not yet confirmed on hardware. Old text: "an adjacent-tail
growth would avoid the copy (not implemented; `CODEX_VA_LATENCY_20261116.md`, "Fix slot fragmentation")".

## How to read it (`scripts/bench/va_profile_table.py`)

`python3 -I scripts/bench/va_profile_table.py qemu.log [--all] [--json] [--top N]` prints: the busy intervals with
the new columns; the phase totals over the log as shares of the summed apply wall; the slow tickets and applies,
slowest first, with `rm_map_only_us` and `other_us` derived; the summary, the last `VA_WORST` and any ceiling
geometry. Only these fields leave the log (no GPU UUID). `--self-test` runs its own check (CI).

Questions it answers, each with its falsifier stated before the run is read:

- H5 of the 2026-11-16 analysis ("most remaining map cost is stitch/mmap, not host RM mapping") is falsified if, in
  the slow applies, `rm_map_only_us` exceeds `stitch_us + desc_us + reap_us`.
- "Inbox delay is negligible" is falsified if `q_inbox_max_us` or a slow ticket's `q_inbox_us` is a material share
  (say over 10 %) of its `req_us`.
- "Other spaces' applies (head of line) dominate a slow ticket" is falsified if `apply_all_us - apply_own_us` is
  small against `apply_own_us` for the slow tickets.
- "Result delivery and worker resume are negligible" is falsified if `resume_max_us` or `e2e_max_ms - req_max_ms` is
  a material share of `e2e_max_ms`.
- The fragmentation reading (H3) is refined by the geometry line: "would fit" said an adjacent-tail growth would
  have avoided that ceiling (since 2026-11-18 such a growth is taken instead of refused); "would not fit" says it
  would not.

## Tests and mutants

GPU-free: `kf_host::phase::tests::phase_*`, `kf_mem::vaprof::tests::*` (buckets, percentiles, ring bounds,
print budget, both `vat` formats under the analysis regexes), `kf_mem::vasmgr::tests::profiling_*` (inbox time,
ticket records, apply phases, ring bound in the manager, per-window counts, and two equivalence tests),
`kf_mem::sim::adversarial::profiling_*` (host verbs and ledger timed once each against the simulated host RM),
`kf_qemu::mem::tests::profiling_*` (enqueue stamp, publication/resume/e2e, field names),
`kf_cuda::capacity::tests::ceiling_geometry_*`.

Behaviour: `profiling_does_not_change_acks_commits_or_host_ops` runs a fixed sequence (a three-window split, an
invalidate, a remap with a refused map, `PDB_ALL`, a re-issued invalidate) and compares everything the manager
decided (host ops in order, each walk's verdict codes, split outcomes, reconciled spaces, decision counters) with a
golden captured by running the same sequence on `origin/next` `2831e3c9c`, before any profiling code existed, and
again with every apply and ticket recorded (threshold 0) and with enqueue times; `profiling_does_not_change_a_space_split`
does the same for the space-keyed split.

`scripts/ci/va_profile_mutants.py` (CI, Rust job): 21 one-line mutants (inbox time taken at acceptance, request
latency without the inbox, apply/phases/collect/re-walks/windows not counted, ring overflow, overwrite not counted,
histogram off by one, percentile at the lower bound, print budget without cap or refill, an old field renamed or
changed in meaning, ledger or span unmap not timed, enqueue stamped at the take, worker take not counted, geometry
tail misplaced or fit ignoring the tail), each killed by at least two named tests (MEASURED 2026-11-16: 21 of 21).

## Not done here (NOT RUN)

No hardware run, no latency fix. The first measurement is the Wukong agent's FG-off 4096 MiB boots on the pc from
this branch; until then every statement about where the time goes stays as the 2026-11-16 analysis left it.
