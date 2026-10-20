# Historical H1 stop: superseded by the single-profiler decision

STATUS: SUPERSEDED, 2026-10-20. CODE READ: the coordinator assigned runtime
recording/hooks/collectors to K1 and explicitly forbade an H1 ring. H1 continues
offline analysis; its importer awaits K1's concrete JSONL format. The historical
findings below remain relevant acceptance conditions, not a pending request to
approve a second recorder. See [K1_IMPORT.md](K1_IMPORT.md) and [PREREG.md](PREREG.md).

HISTORICAL STATUS: RESEARCH, 2026-10-19. CODE READ. At that time the runtime
contract remained unresolved. Coordinator additions 2–4 were implemented in the
offline analyzer/inventory tools; no runtime instrumentation or launcher
behavior was changed. H1's binding instruction is: "If something contradicts
the code or these instructions, STOP and write down options; never improvise
a design change." Implementation stopped on the following discrepancy.

## Per-present broker evidence

The lane says the broker's stats-line gives broker present timestamps to align
by sequence. Candidate-5 `f8f9d1a5f` does not emit that evidence:

- `broker/nb_tick.c:301`, `nb_tick_stats_format`, prints per-second deltas,
  mean/max latencies, new/repeat/merged counts and gap threshold counts. `t` is
  the stats-line timestamp in milliseconds, not a list of presentation times.
- `broker/nvkvm_broker.h:691` explicitly documents the per-second window.
- `broker/nb_session_wl.c:4707`, `pres_presented`, receives the compositor's
  nanosecond timestamp, discards presentation sequence words `sh` and `sl`,
  and passes the time into tick accounting. That sequence is also a compositor
  output sequence, not automatically the VMM's content/request sequence.
- `broker/nb_tick.c:384`, `nb_tick_content_present`, determines new content from
  commits since the preceding present. Multiple commits can merge into one
  present and a pump's feedback can report new content. A naive join of any
  feedback's sequence to a VMM frame would therefore misattribute a hitch.

INFERRED: aggregate windows cannot reconstruct every >80 ms content gap or its
correlated frame chain. Adding per-present records and a tested content/commit
join is required before the requested end-to-end recorder can be validated.

## Clock premise not established by the located harnesses

CODE READ: `scripts/bench/hitch/fr.py:2` maps guest FILETIME through a fixed
timezone offset. `plot_timeline.py` optionally matches aggregate flip rates.
The read-only P1 harness at revision `1fa726fee` uses matched doorbell periods,
a drift fit (`align.py`) and fence-observation/QPC calibration
(`iteration_hops.py`, `capture_broker.py`). These are not a demonstrated
invariant-TSC offset method. Supplying `+invtsc` in a launch command is not a
measurement of QPC-to-host offset or its error. The owner VM was not queried.

## Options for the owner/coordinator

1. **Recommended:** explicitly accept a default-off per-present broker recorder
   with a content/commit/request join and a measured, bounded-error QPC-to-host
   calibration. First add failing tests for merged/discarded/pump feedback and
   clock uncertainty; preserve feature-off production builds. Resume the rest
   of Part A only after that collection contract is settled.
2. Identify the existing per-present/sequence collector and invariant-TSC
   calibration artifact if they live outside the reviewed source. Read those
   artifacts and implement the originally intended contract if they resolve
   the discrepancy.
3. Accept aggregate-only reporting. **Not recommended:** it cannot meet H1's
   every-gap/first-stage requirement and must not be presented as Part A done.

## Preserved work and limits

MEASURED: the initial normalized-input analyzer stage passed 20 tests and three
mutations were killed by 2/2/3 tests. After coordinator additions 2–4, the suite
passes 84 tests with 18 measured mutants killed by at least two tests each.
It reports every qualifying supplied content gap,
retains event windows, exposes UNKNOWN and guards loss, clock uncertainty,
unlinked frames and same-thread S7 evidence. It requires already normalized,
correlated intervals and an explicit collection-coverage assertion; it does not
derive that correlation or convert raw ETL. S7 overlap is a candidate
classification, not proof that reclaim caused the entire gap.

NOT DONE: atomic recorder, runtime hooks, broker per-present collection, raw
dump/ETW adapters, scheduling/memory/taskstats collectors, guest collectors,
scripted walk, live guest backing audit, production/instrumented control
binaries, clippy comparison and hardware validation. No diagnosis of the
owner's hitches is established. The preceding stages are useful partial work,
not a ready-to-run Part B kit. PREREG.md includes S1–S8, Bedrock, aging and VRAM
experiments, S8-first priority, and the unresolved four-hour budget conflict.
