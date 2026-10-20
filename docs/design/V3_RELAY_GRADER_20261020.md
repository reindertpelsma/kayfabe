**STATUS: LIVE (GPU-free test suite; NOTHING here ran on hardware), 2026-10-20.**
Labels: **MEASURED** = a test run named here, on the dev box; **CODE READ** = file:line; **INFERRED** = reasoning.

# V3 — the USER-ring relay grader: eleven test groups that only a good concurrency design passes

Owner request (2026-10-20): the normal `cargo test` suite IS the grader -- every group is ordinary test functions with a group
prefix, and all green = pass. `scripts/relay_validity.sh` is the one-off proof that the suite sees the known bugs (it runs the
suite against known-bad revisions). No scoreboard.

## 1. The contract graded (owner architecture; `THE_ARCHITECTURE_v3.md`)

The worker is ONE loop; each pass re-evaluates all level state (no callbacks). The ONLY blocking point is `epoll_wait`; a worker
blocks only when every outstanding obligation has a registered, non-missable wake source (excess wakes allowed; eventfds are hints,
never ground truth; arm before evaluate); an obligation without a source makes the loop spin (timeout 0). The vCPU's doorbell
eventfd is in every worker's epoll set; 1, 2 or 3 workers. No heuristics: no retry budgets, no every-Nth-pass looks, no timers as
logic (the 50 ms park net may exist, the grader runs with it INFINITE). Per-pass work is bounded. Doorbell contract: no look polls
`GP_PUT` on an idle relay; `GP_GET` guards ring slots only.

## 2. How to run

```
cargo test --release -p kf-chan --test grade_model --test grade_locks --test grade_static --test grade_stress   # default set
cargo test --release -p kf-chan g5_                                                                              # one group
cargo test --release -p kf-chan --test grade_stress -- --ignored g3_stress_full_seed_list                        # deep (~8 min)
RS_FROM=0 RS_TO=5000 cargo test --release -p kf-chan --test grade_stress -- --ignored g3_stress_nightly_range    # nightly
python3 -I scripts/relay_grade/mutants.py [name ...]                                                             # G10a
scripts/relay_validity.sh <rev> ...                                                                              # G10b, one-off
```
Default runtime (MEASURED, this 4-core box, release, after the build): grade_model ~110 s, grade_locks < 1 s, grade_static < 1 s,
grade_stress see section 6. Build of the four targets from a warm dependency cache: ~2 min.

## 3. Files

| file | what |
|---|---|
| `crates/kf-chan/tests/grade_adapter.rs` | the LINEAGE ADAPTER (not a target; `#[path]`-included): every decision the grader asks of an implementation, answered with the implementation's own pure functions; `--cfg kf_grade_lineage="simple"|"look_every"|"budget"` (default `simple`) |
| `crates/kf-chan/tests/grade_model.rs` | G1 / G2 / G4 / G5 / G6 at model level: exhaustive explicit-state explorer (own) of 1..3 worker loops, a cold relay, 0..2 self-re-ringing hot relays, a lock holder (act / silent record / `pt` / in-turn admission), the hardware (marker -> REFERENCE -> interrupt; pops without interrupt); fair-cycle (Tarjan SCC + weak fairness) starvation, strand, unsafe-sleep, hidden-spin, phantom-ring checks; longest serviceable path <= K |
| `crates/kf-chan/tests/grade_locks.rs` | G8: interleaving explorer over the REAL `kf_trap::WakeWord` and `TokenWord` atomics, the lost-registration protocol, and the lineage's lost-lock protocol; negative controls |
| `crates/kf-chan/tests/grade_static.rs` | G9 audits and G11 catalogue check |
| `crates/kf-chan/tests/grade_stress.rs` | G2 / G3 / G4 / G5 / G6 / G7 on real threads: GENERATED from `relay_stress.rs` (@`ab6193aff`) by `scripts/relay_grade/mkstress.py` + `scripts/relay_grade/gradetests.rs`; regenerate after the harness changes |
| `scripts/relay_grade/{heuristics,blocking,unsafe}_allowlist.txt` | G9 allowlists, one reason per line |
| `scripts/relay_grade/catalogue.toml`, `docs/design/V3_FAILURE_CATALOGUE.md` | G11 data (81 entries) |
| `scripts/relay_grade/mutants.py` | G10a mutation check (>= 2 distinct killing tests per mutant) |
| `scripts/relay_validity.sh` | G10b one-off: overlay the grader on a revision, detect its lineage, list the red tests |

**Production code changed: NONE.** No hook was added; the harness uses the `chaos-hooks` feature, `worker::Inflight` and
`worker::set_park_net` that the relay-simple lineage already has.

## 4. The groups (test names; `cargo test -p kf-chan <prefix>` selects)

| group | grader tests (new) | existing tests counted in the group (unchanged files) |
|---|---|---|
| G1 exhaustive models | `g1_models_exhaustive_{one,two,three}_worker`, `g1_f1_convoy_past_budget_is_served`, `g1_f3_busy_worker_looks`, `g1_f5_busy_record_look_keeps_the_event`, `g1_popped_wait_is_spun_not_parked`, `g1_act_thread_release_rings`, `g1_models_checker_self_test_detects_a_lost_wake`; `#[ignore]` `g1_open_irq_before_reference_hazard` (OPEN: owner decision) | `user_loop_model_v3_pointer.rs` (breaker: 25, incl. break1/1b/2/2b + P13 names), `relay_fairness.rs` (30), `user_loop_model_v2.rs` (39), `user_loop_model.rs` (46) |
| G2 sleep safety | `g2_sleep_safety_model_never_blocks_with_an_unsourced_obligation`, `g2_sleep_safety_kernel_view` (real threads: every worker `S` in `ep_poll` per `/proc/self/task/*/{stat,wchan}` AND an empty zero-timeout poll of the workers' own fds AND no hardware interrupt queued, the same obligation over 3 ms) | relay_stress I6 |
| G3 stress | `g3_stress_default_seed_list` (24 x 0.3 s), `g3_stress_harness_glue_still_matches_production`; `#[ignore]` `g3_stress_full_seed_list` (300 x 0.5 s + 5 x 60 s), `g3_stress_nightly_range` | relay_stress smoke / baton |
| G4 spin reference | `g4_spin_reference_model_blocking_equals_spinning`, `g4_spin_reference_stress_equivalence` (an always-ready eventfd in every worker's epoll, ignored by `on_other`: the same loop never blocks) | -- |
| G5 efficiency | `g5_efficiency_model_no_hidden_spin` (hidden spin, quiet states that never settle, phantom-demand cycles), `g5_efficiency_model_idle_workers_block`, `g5_efficiency_stress_idle_blocks_for_one_second` (+ parks <= 4 x doorbells + 16) | relay_stress I7 |
| G6 fairness | `g6_fairness_model_baton_bound_in_worker_steps`, `g6_fairness_stress_baton` (p99 / max printed), `g6_fairness_stress_huge_delta_beside_cold` | relay_fairness P11-P13 |
| G7 ring fill / GP_GET | `g7_ring_fill_and_backpressure` (32768 guest ring, 512 private ring, stalls, SYNC_WAIT, 1..3 workers) | `user_turn_unit::copy_before_get_a_slot_overwritten_after_get_passed_it_is_never_forwarded` (TOCTOU) |
| G8 lock protocols | `g8_lock_protocol_real_wake_and_token_words_lose_no_doorbell`, `g8_lock_protocol_negative_controls_are_red`, `g8_lost_registration_is_red_for_the_naive_protocol`, `g8_lineage_protocol_never_strands` | -- |
| G9 structural | `g9_watched_forwards_every_relayloop_method`, `g9_no_banned_heuristics`, `g9_blocking_calls_are_audited`, `g9_unsafe_only_in_allowed_files`, `g9_adapter_pins_match_the_glue` | `wake_registry.rs` (20) |
| G10 test quality | `scripts/relay_grade/mutants.py` (11 mutants), `scripts/relay_validity.sh` | `scripts/fair_mutation_check.py`, `scripts/relay_stress_mutants.sh` |
| G11 catalogue | `g11_catalogue_every_entry_is_pinned_or_gated`, `g11_catalogue_ids_unique_and_expected_stated` | -- |

G7 GP_GET policy: `user_turn::GUEST_GET` is a `const` (`ReadIndex` on this lineage); the `Popped` policy is graded only by
rebuilding with the const flipped (no runtime switch exists; adding one would be a production change).

## 5. Knobs

| knob | default | meaning |
|---|---|---|
| `--cfg kf_grade_lineage=` | `simple` | which adapter arm (set by `relay_validity.sh` from the source) |
| `RS_GRADE_SEEDS`, `RS_GRADE_SECS` | 24, 0.3 | G3 default list |
| `RS_LONG_SECS` | 60 | G3 deep: the 5 long seeds |
| `RS_FROM`, `RS_TO`, `RS_SECS` | 0, 5000, 0.3 | G3 nightly |
| `RS_BOUND_MS`, `RS_STARVE_SERVES` | 250, 20000 | the harness's I2 bound (strict in the grader: any SOFT finding fails) |
| `RS_PARK_NET` | 0 (infinite) | 1 restores production's 50 ms net (never in the grade) |
| `GRADE_MUTANT_TESTS` | all four targets | narrows `mutants.py` |
| model K | `60*W + 20*hot + 40` steps | bounded wait of the cold relay, all workers' steps counted |

## 6. Results on the relay-simple head (`ab6193aff` + this branch; MEASURED)

@@RESULTS@@

## 7. Validity: the suite against known-bad revisions (`scripts/relay_validity.sh`, MEASURED)

@@VALIDITY@@

## 8. A new implementation plugs in by

1. a `cfg(kf_grade_lineage = "<name>")` arm in `grade_adapter.rs` answering `look`, `policy`/`after_pass`, `loop_shape`,
   `contended_leaves_demand`, `contended_wait`, `unsourced`, `PARK_KNOB`, `BUDGET`, `set_infinite_park` and `PINS` (the glue
   lines the mirror transliterates: `g9_adapter_pins_match_the_glue` fails when they drift), and the detection line in
   `relay_validity.sh`;
2. for the real-thread groups: `worker::Inflight`, the `chaos-hooks` feature, `worker::set_park_net` and the relay's turn API
   used by `tests/lifecycle_support` (or a regenerated `grade_stress.rs` from that lineage's harness);
3. nothing else: every other test is lineage-neutral.

## 9. Hardware-only

The model and the stress harness are GPU-free stand-ins. The hardware endpoint of the same contract (not run here): the Linux baton
reproducer (`scripts/bench/baton`, branch `claude/baton-pointer-20261020`) with `KF3_DIAG_PROGRESS=1000`: no `PROGRESS_STRAND`
line, no client gap >= 1 s, `USER_RING_LOOP SUMMARY backstop_found_work=0 timeouts_with_work=0`. NOTE (G9 finding below): with
`KF3_DIAG_PROGRESS` on, the `Watched` wrapper drops `unsourced()`, so the hardware run of the diagnostic does not run the loop the
grader grades until that is fixed.

## 10. Limits (stated)

* The model is an abstraction (INFERRED fidelity): one atomic step per loop phase; the old loop's pre-park spin capped at 4 looks
  (with an infinite park a bounded spin cannot fix a strand); 3-worker configurations run without hot relays and without record
  holders (> 6 M states); hold lengths are unbounded (adversary) but the number of holds is 1-2.
* `grade_stress.rs` compiles only on the simple lineage (it needs the harness hooks); on the others it is NOT APPLICABLE, listed,
  never counted as a pass.
* loom 0.7.2 is in the local cargo cache but not a workspace dependency; the explorers are own code (no `Cargo.lock` change).
* The G9 audits are text audits (conservative patterns, allowlist with reasons); a blocking call hidden behind a helper in another
  module is not seen.
