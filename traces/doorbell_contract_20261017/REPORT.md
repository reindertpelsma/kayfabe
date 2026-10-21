# Doorbell contract: GP_PUT poll removed, rules pinned (2026-10-17)

STATUS: ANSWERED, 2026-10-17 (GPU-free; no hardware run). Scratch lineage `codex/doorbell-contract-20261017`, based on `codex/lazy-observe-20261017` (5fb2a5405). Never merge to master.
Labels: MEASURED = a test run named here; CODE READ = source; INFERRED = reasoning.

## Contract as implemented (owner, 2026-10-17)
The last doorbell covers every GP_PUT write before it. Nothing else announces GP_PUT: no poll, no timer read. The relay loops until its cursor equals the GP_PUT it loaded; a ring during a pass forces another pass (repeated until a pass sees none); rings coalesce; a ring with no work is safe.

## What changed
- CODE READ: the removed branch was in `ChanPlane::user_ring_peek_one` (`userpump.rs`, `else if backstop && p.segment.is_none()`): on the 50 ms backstop it read guest `USERD_GP_PUT` and, if ahead of the cursor, rang the relay (`backstop_put_ahead`). The design doc (section 3.6 and the "Doorbell-less submission" bullet) had specified it as "information only". Corrected in the doc above the text it corrects.
- Removed that branch and the `backstop_put_ahead` counter. New pure `kf_chan::user_loop::look_needed(why, demand, waiting)`: a look needs demand or a recorded wait; the kind of look is never a reason.
- Model: the allowance for PUT-ahead is gone; it is now violation `PUT_POLL` and mutant `backstop_polls_put` re-creates the poll.
- Stop finding 1 fixed: `capacity_is_trusted_bounded_and_keeps_one_empty_slot` now follows `MAX_ENTRIES` (no change to the constant).

## Tests (MEASURED, `cargo test --release -j2`)
Full `-p kf-chan -p kf-trap -p kf-qemu --no-fail-fast`: **706 passed, 0 failed, 0 ignored.**

| Rule | Test | Killed by mutant (tests failing) |
|---|---|---|
| 4 coalesce | `kf-trap/tests/doorbell_contract.rs::doorbells_before_the_pass_coalesce_into_one_pass` (N=1,2,5,100,10000) | every ring wakes (2), ring on IDLE no wake (3), ring on RUNG becomes IDLE (2) |
| 3 ring during a pass | `..::a_doorbell_during_a_pass_forces_one_more_pass` | BUSY ring dropped (2), release forgets BUSY_RUNG (2), every-ring-wakes (2) |
| 3 repeated / fairness bound | `..::rings_during_every_pass_are_never_lost_even_across_the_fairness_bound` | BUSY ring dropped (2), release forgets (2), no fairness bound (1) |
| 5 spurious | `..::a_doorbell_with_no_work_is_safe_and_real_work_behind_it_is_still_relayed` | ring resurrects retired token (1), ring on IDLE no wake (3) |
| 1+2 last doorbell, no poll, real threads (40 seeds x 2000 rings) | `..::after_the_last_doorbell_the_cursor_reaches_the_last_put_with_no_poll` | ring on IDLE no wake (3), RUNG->IDLE (2) |
| no poll (model, all interleavings, timers + backstop on) | `kf-chan/tests/user_loop_model.rs::put_ahead_of_the_last_doorbell_is_owed_nothing_and_no_timer_reads_it`; liveness with timers never firing: existing `design_a_relay_is_live_without_any_timer` | n/a |
| no poll (mutant found) | `mutant_the_backstop_polls_gp_put_is_found` | the re-created poll is found (PUT_POLL) |
| ring during a turn / lost wakeup (model) | existing `mutant_a_ring_during_a_turn_is_lost_is_found`, `mutant_lost_wakeup_is_found`, `doorbell_storm_coalesces` | unchanged, pass |
| look decision | `kf-chan::user_loop::tests::no_look_touches_a_relay_with_no_demand_and_no_wait` | `look_needed` made to look on every backstop (1 test; the only behavioural difference without the removed branch is taking the lock) |
| structural | `kf-qemu/tests/doorbell_no_put_poll.rs::the_look_function_never_reads_gp_put` | GP_PUT read re-added in the look (1), look no longer decides through `look_needed` (1) |

Honest limits: the last two rows are caught by ONE test each. The structural pin reads source text (the look needs a live plane and cannot run GPU-free). The real-token tests cover the atomic word and a one-worker model of its use, not `Pump::turn` itself (the existing 38-test loop model is the transliteration).

## Which waits still depend on the timer (INFERRED from the model configuration, not a new run)
The model proves liveness with timers that never fire for the doorbell cases (`design_a_relay_is_live_without_any_timer`, `doorbell_storm_coalesces`, all contract scenarios). The existing tests that need the backstop are: `design_a_lost_interrupt_is_covered_by_the_backstop_and_counted`, `design_get_visibility_lag_after_the_interrupt_needs_the_backstop`, `design_pt_admission_contention_is_spin_then_the_counted_backstop`, and `design_never_depends_on_the_spin` (the spin is only latency). The doc §3.6 also cites `design_carried_gate_needs_the_backstop_and_counts_it_separately`, which is NOT a test name in the current file (doc/code mismatch, not fixed here). So the backstop still carries: `Wait::Popped` (a control-witness gate with no interrupt), `Wait::Contended` (admission nobody announces), a lost/never-armed marker interrupt, and a GP_GET that becomes visible after its interrupt. These are waits on GPU/lock progress, not on GP_PUT. Owner decision pending: keep as documented counted safety net, or design events for them.

## Other reads of guest USERD_GP_PUT (CODE READ, classified)
| Site | Class |
|---|---|
| `userpump.rs:1488` (PutCache read in the turn) | doorbell turn |
| `userpump.rs:840 trace_stall`, `1269 trace_refusal`, `3083 trace_idle` | trace-only (`p.trace` is None unless `KF3_USER_RING_TRACE`; `trace_idle` returns early without it) |
| `userpump.rs:2562, 2646` | measurement only, at a ring switch (`KF3_` trace) |
| `kf-qemu/src/chan.rs:7710` `g.last_put = userd.load(GP_PUT)` | Translated path serve (not USER ring); runs per serve, not on a timer (not reviewed further) |
| `kf-qemu/src/chan.rs:7991 probe_tick` | diagnostic probe tick (reads PUT and GET to explain a stall); a timer-driven read, diagnostic only, not carrying relay work. Flag: `other` in the sense of the prompt; not changed |
| `kf-chan` `host.rs`, `user_storage.rs` | private/host USERD accesses and layout, not guest-PUT polls |

`chan.rs:7991` is the one timer-driven GP_PUT read left; it is Translated-path diagnostics. Not changed (outside scope); owner to decide.

## Not done
No hardware, no split-arm measurement, no N ladder, no OpenGL arm. `Wait::Popped`/`Contended` backstop untouched. rustfmt not applied to `userpump.rs`/`user_loop_model.rs` (large pre-existing diffs).
