# Translated slot contention — 2026-10-21

STATUS: RESEARCH; GPU-free HW3 lane, based on M (`89fc8a222`).

CODE READ: the worker's Translated `serve` returns on a lost slot `try_lock`
without a durable re-ring. The drainer's group selection, token query and
schedule update block on the same slot. Act retirement and worker pumping can
hold that slot across host calls. These observations do not prove the remaining
Windows freezes have this cause.

Hypothesis H1 (before tests): a lost slot attempt can strand queued work when no
new external hint arrives. FALSIFIER: forced contention followed by holder
release always gives a durable ring and bounded worker-pass progress without a
new doorbell, engine edge or timeout.

Hypothesis H2 (before tests): a busy slot blocks drainer metadata and schedule
processing. FALSIFIER: those operations finish or defer while the holder remains
held and cannot delay an unrelated drainer request.

Preparation: extract the existing lock/try-lock and drainer access behavior to a
generic production cell so fake payload tests execute the adapter's real locking
code. This is a behavior-preserving preparation, before EXPECTED RED tests;
no fix is included. Token/group identity is immutable after birth (CODE READ).

MEASURED preparation baseline: `cargo test -j2 --release -p kf-chan -p kf-qemu
--lib`: 200 + 366 passed, zero failed. EXPECTED RED commit `79f5f01de`:
`cargo test -j2 --release -p kf-qemu --lib translated_slot::tests`:
1 passed, 12 failed, 366 existing tests filtered. Failures cover the final
lost doorbell, ordinary/probe holder release, coalesced losers, 1–3-worker
fake-GPU stress, and blocking token/group/schedule accesses.

Draft-fix review stopped a mixed inline/deferred scheduling approach: GSP held
replies do not serialize subsequent statement effects (`kf-gsp/src/boot.rs`).
Options recorded before proceeding: (1) queue every Translated schedule on the
existing serial act thread; (2) invent another sequencing protocol. Chosen (1),
the already-prescribed act-thread pattern. Group replies cover all members.
Cells are captured by allocation identity and rejected if retired/replaced.

The two busy-schedule tests are restated for that stronger rule: source pins
forbid inline edits, a fake act thread uses the real cell and real Deferred,
an unrelated drainer item completes while the pump remains held, and success
is unresolved until the act finishes. The original busy-drainer property is
retained. The fake stress drainer now queues every schedule edit. No existing
upstream test is removed or loosened; these two new tests changed shape during
review. Production has no test-aware decision or new flag.

H5, before mutation: every repaired behavior has nonvacuous tests. FALSIFIER:
a compiling production mutant survives, has fewer than two failed tests, or
fails only to compile. The mutation runner restores source in `finally` and
records failed test names; structural adapter pins are distinguished from
execution of the generic production cell.

H6, before holdouts: the cell does not strand new scheduling seeds with
1–3 workers. FALSIFIER: lost/duplicate/reordered entry, absent holder re-ring,
no completion within the test bound, or a control that never exercises loss.
Test bounds limit a failed test; no product timer or polling is added.

CODE READ remaining RM work, intentionally listed without claiming a measured
Windows stall: `chan.rs` StoreViews::view_for arms/releases CPU views while the
worker holds a Translated slot; `userstore.rs` Native::arm/release do the same
for USER snapshots. UC copies and other map/record locks from the blocking
audit also remain. No hardware measurement in this lane establishes their
latency, so the requested act-thread view conversion is not claimed.

MEASURED fix-stage regression: 26 passed, zero failed, one ignored holdout
(includes one existing Translated test). Full crate, holdout and clippy results
are pending at this checkpoint. All ten mutants compile and have >= 2 killers.

| Mutant | Failed tests | Coverage |
| --- | ---: | --- |
| `loser_drops_registration` | 10 | Production cell execution |
| `holder_omits_ring` | 9 | Production cell execution |
| `ring_before_unlock` | 2 | Production cell execution |
| `late_registration_no_recheck` | 4 | Production cell execution |
| `release_never_advances_epoch` | 4 | Production cell execution |
| `holder_checks_before_epoch_bump` | 2 | Structural adapter/protocol pins |
| `metadata_takes_pump_lock` | 2 | Production cell execution |
| `worker_bypasses_handoff` | 3 | Structural adapter/protocol pins |
| `schedule_blocks_on_drainer` | 2 | Structural adapter/protocol pins |
| `diagnostic_blocks_on_drainer` | 2 | Structural adapter/protocol pins |

The full failed-test names and elapsed times are in `mutations.jsonl`. Run the
runner with `CARGO_TARGET_DIR` set to the lane target and `HW3_CARGO_LOCK` set to
the shared dev build lock. It restores production source after every mutant.
