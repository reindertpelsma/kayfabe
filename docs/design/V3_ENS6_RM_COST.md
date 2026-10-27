# ENS6: host RM cost, refusal fallback and the layout time slice

**STATUS: LIVE ENS6 implementation, 2026-10-27; GPU-free validation, host benchmark and Windows boot pending
(see *Measured on the host* for what ran).** Sparse twin stays default OFF. The 8192-block per-space and
32768-object per-session bounds are unchanged. Labels: MEASURED (a named log or run), CODE READ (source at the
named revision), INFERRED (reasoning, with its falsifier).

Parent: `docs/design/V3_SPARSE_TWIN.md`, `docs/design/V3_ENS5_SPARSE_ADMISSION.md`. Base revision
`6d95870c7` (ENS5). Evidence: Windows boot 7330 (twin ON, production perf config) and the same binary's run
7350, host `/data/claude-ens5-20261026/lane/windows-20261022/boundary-kayfabe-{7330,7350}/qemu.log`.

## What 7330 and 7350 show (MEASURED 2026-10-27, rev `6d95870c7`, heartbeat series)

1. **The slow applies were reservation-verb churn, not slow single RM calls.** Between heartbeat 510 and 543 of
   7330 (about 66 s) `made` grew by 241 584 and `freed` by 241 583, `refused` by 120 142 and `coarse_maps` by
   120 229. Per event: 2 reserves, 2 releases, 1 RM refusal, 1 coarse map. That signature is a *reserve refused by
   RM right after a release*: either the merged 4 MiB reservation of `recycle` (release a, release b, reserve
   both) or the second piece of a split in `replace`; each time the pair is restored and the plain map is
   counted coarse. It starts exactly when the busiest space first reaches 8192 blocks (7330 heartbeat 510, 7350
   heartbeat 620) and then runs once per plain map into a not-yet-refined 2 MiB area, for as long as the guest
   keeps mapping. A reservation verb costs about 17 us on this host (first slow apply: 271 154 717 ns /
   15 798 calls; the apply held 12 051 reserve/release verbs of them, `made` +8 037, `freed` +4 014, plus 3 747
   map/unmap/control verbs). The heaviest heartbeats carry about 59 000 such verbs (33 us per verb there, apply 1.94 s).
2. **The first two slow applies are boot.** 287.7 ms (7330) and 289.0 ms (7350): the same 15 798 RM calls,
   deterministic. The single 190 .. 191 ms `max_ns` in that snapshot is a *thread-lifetime gauge*
   (`diag::Snapshot::since` subtracts calls/ns/work only), so it is not proven to belong to that apply (rev `6d95870c7`). It is
   identical in both runs, which points at one fixed boot step on the VA thread; the host benchmark's `roots`
   arm measures the candidate (the space-creation reservations of up to 127 TiB).
3. **The boot-time `NoMemory` is a host RM refusal, not our budget.** `refused` equals `admit_hard_err`
   (1, 4, 5, 7, 9, 11, 12) through heartbeat 169 of 7330, and the one printed refusal is
   `sparse admit 0x19200000+0x110000: NoMemory`: a sparse tile at 421 MiB, inside `[1 MiB, 4.5 GiB)`, which host
   RM withholds (`GUEST_VA_RANGES` doc: a reservation there is refused with `NoMemory`). The tree asked RM for it
   because the VA is outside every root, and it came back as a hard `Err`, which the walker re-emitted for every
   distinct run.
4. **The first 153 of 7330 is not explained by RM latency.** It is about 8 s after the Enscape start (guest clock
   pairs: start 23:33:40.6, event 23:33:48.1), heartbeat ~1000. The last 100 heartbeats show the VA thread idle (`walks_per_beat=0`,
   `host_calls=0`), all eight vCPUs at 100 %, and no slow apply since heartbeat 543. What does coincide with
   the onset is one beat with 1 838 sparse declines (`overflow` 22 -> 1 858; 117 MiB of sparse intent left
   invalid, the closed fallback of a full budget) and four RM refusals returned as hard errors. INFERRED, not
   established: a guest that needs more than 8192 live sparse blocks (Enscape) gets invalid pages where it
   asked for sparse ones. Falsifier: ENS6 bounds the layout work and still shows 153 with `admit_decline`
   bursts at the onset and no layout slice spent; then the budget (not time) is the next lever.

## Rules (ENS6)

* **Per-verb RM timing** (`kf_host::diag::Verb`): `reserve` (alloc of `NV50_MEMORY_VIRTUAL`), `alloc_other`,
  `free`, `map`, `unmap`, `control`, `invalidate`, `other`; calls / total / max single call, session-wide with
  relaxed atomics, in the heartbeat as `rmverbs[verb=calls/total_us/max_us ...]`, per thread in
  `FIRST_SLOW_APPLY`. The ioctl is timed once; the same sample feeds the disjoint `Rm` / `Invalidate` totals.
* **A host RM `NoMemory` is a counted closed fallback, never a hard error.** `rm_declined` counts it, the
  refusal starts a **back-off** (2, 4, ... 64 retry ticks; ticks are `retry` calls, two per refresh) during which
  optional and demand layout work starts no RM verb (`backoff_skips`), and the refused sparse tile is remembered
  in a bounded **refused-window cache** (64 windows, expires after 256 ticks; `window_hits`). A decision taken
  on a refusal is asked again when its back-off ends (`Negative::due`). Other RM errors stay hard.
* **Hysteresis, no churn.** `normal()` returns before any recycling when nothing needs splitting. Plain-map
  refinement above the high-water mark (`limit - limit/32`) with no empty pair to recycle issues no RM verb
  (`speculative_skipped`). When recycling is needed it frees `spare + limit/256` slots in one batch (up to 256
  pairs) so the next admissions do not recycle again.
* **The layout time slice** (`sparse/slice.rs`). One refresh (`begin_refresh` .. `end_refresh`, the unit that
  holds the VM-wide walk gate and the placement lock) may *start* layout RM work until it has spent
  `SLICE_NS = 100 ms` or `SLICE_VERBS = 4096` reservation verbs; optional work (plain-map refinement,
  recycling, converting retired sparse blocks) stops at half. Work that started is finished or restored, so a
  refresh overshoots by at most one replacement and its rollback. The unstarted rest waits for the next
  refresh: a deferred sparse run keeps the tiles it already made (an already sparse VA never loses coverage),
  is answered `HeldByHost` for the unfinished part and is resumed by `retry`; retired sparse blocks are queued
  (`cleanup`) and converted within the optional slice. `layout_refresh_peak_{us,verbs}` in the heartbeat is the
  busiest refresh so far. Map/unmap/invalidate verbs of the guest's own diff are not sliced (they are the
  diff); they are visible in `rmverbs`.
* Bounds: refused windows 64, negative cache 256, cleanup queue <= blocks <= 8192, pairs per recycle 256,
  layout verbs per refresh 4096 plus one restore. `tsgmirror.rs` is untouched; the twin stays default OFF.

## Contract changes (tests amended, none deleted or weakened)

A host RM `NoMemory` during admission now answers `Ok(false)` and counts `rm_declined` instead of `Err`: the
tests that asserted `.is_err()` for an injected RM refusal (`restore`, `late_admission_refusal`,
`ens5_hard_error_cached`, `ens5_unaccepted_error_not_published`) assert the counted fallback and keep their
other assertions (no half-made geometry, no publication by an unrelated retry, still cached). The breaker
full-tree fixture fills the tree through `normal()` + `mark()` because `route_map(refine)` deliberately stops at
the high-water mark.

## Tests (EXPECTED RED commit `4d096870d`, green `4f401117b`)

kf-host `ens6_*` (12) and `diag::tests` (8 new); kf-qemu `tests/ens6_hooks.rs` (5). The sim proof of the gate
bound is `ens6_saturated_churn_sim_keeps_every_refresh_inside_the_slice` (4000 maps + 100 binds per refresh on
a saturated tree with a refusing RM, 20 refreshes, every refresh <= `SLICE_NS` + 8 verbs on the virtual
17 us/verb clock) and `ens6_deferred_sparse_work_resumes_each_refresh_until_honoured_and_never_unsparses`
(8000-tile admission over many refreshes, monotone coverage).

## Measured on the host (2026-10-27, rev `6d95870c7`)

See `traces/ens6_20261027/` (benchmark tables, mutation ledger) and the lane report.

## Next host boot

Same Windows hardened-capture configuration as 7330/7340 on the ENS6 head, twin ON. Read the heartbeat:
`layout_refresh_peak_us` <= ~110 000, `slow_apply` 0 after boot, `rm_declined` small and `backoff_skips` > 0
where RM refuses, `refused` no longer tracking `coarse_maps`, `rmverbs` table for what dominates. If the
first 153 still comes with the VA thread idle and the layout counters bounded, RM latency is exonerated and the
sparse admission burst (`admit_decline`, `unhonoured`) at the onset is the lead.
