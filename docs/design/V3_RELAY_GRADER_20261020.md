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

Branch head graded: `ab6193aff` (relay-simple, local head at the time; ahead of `origin/claude/relay-simple-20261020` 7d9a3948d by 6
commits) plus the grader. MEASURED on the dev box, release build, park infinite:

| group | new tests | result | red tests and why |
|---|---|---|---|
| G1 | 10 (+1 ignored OPEN) | **GREEN** (models; existing G1 files not re-run here) | -- |
| G2 | 2 | **GREEN** (model; kernel view 12 seeds) | -- |
| G3 | 2 default (+2 ignored deep/nightly) | **GREEN** (24 seeds x 0.3 s, 8..32 channels, 1..4 address spaces); deep / nightly NOT RUN | -- |
| G4 | 2 | **GREEN** | -- |
| G5 | 3 | **RED** | `g5_efficiency_model_no_hidden_spin`: (a) **act-hold spin**: a serve that lost the record to the ACT thread leaves demand, `unsourced()` is true, the worker spins with timeout 0 for the whole act hold although the act's release rings (`user_ring_wake`) -- a covered obligation treated as uncovered; (b) **phantom-demand ring** (2 workers): `enter()` sets the demand flag before the record `try_lock`; a peer look that lands in [`request()`, `retry.swap(false)`] sees demand + record busy and rings the token again; under an adversarial schedule this repeats for ever after the client's batch is done (25 cycles found for W=2; every quiet state CAN still settle, so INFERRED rare in practice, not measured on real threads). The real-thread G5 test is green (1 s quiet window held; parks 1725-3238 for ~1300 doorbells). |
| G6 | 3 | **GREEN** | baton (2 workers, 4 hot, 1 cold): cold resp p99 <= 2 us, max 0.11 ms with ~18 000 serves during a response (MEASURED under load); huge-delta fillers beside a cold client: max 3.34 ms |
| G7 | 1 (+ the existing TOCTOU unit test) | **GREEN** | GP_GET policy `Popped` not graded (const, section 4) |
| G8 | 4 | **GREEN** | negative controls red as required |
| G9 | 5 (+ wake_registry) | **RED** | `g9_watched_forwards_every_relayloop_method`: **CODE READ `crates/kf-chan/src/worker.rs` `impl RelayLoop for Watched`** forwards `waiting` and `peek` but not `unsourced`, so with `KF3_DIAG_PROGRESS` on the trait default (`false`) decides the block: the worker can park with the retry set non-empty (the hardware plan runs with the diagnostic on). |
| G10 | -- | mutation **NOT RUN** (time box); validity: section 7 | -- |
| G11 | 2 | **GREEN** (81 entries: 47 pinned, 5 hardware/process-gated, 29 open) | -- |

Grade on this head: **9 of 11 groups green; RED: G5 (model), G9 (Watched)**; G10a not run. Production code was not changed
(owner rule): both reds are reported, not fixed. Runtime: `grade_model` ~110 s, `grade_locks` + `grade_static` < 1 s,
`grade_stress` 1490 s **under a concurrent 4-revision validity build** (MEASURED; unloaded per-test timing in section 6a).


### 6a. Real-thread runtime and one unexplained red (MEASURED)

Per test, alone, before the default seed counts were cut (box load average 10-17 from other agents' jobs: wall times are upper
bounds): `g2_sleep_safety_kernel_view` 232 s (12 seeds), `g3_stress_default_seed_list` 195 s (24 seeds),
`g4_spin_reference_stress_equivalence` 200 s (8 x 2 runs), g5 5 s, g6 baton 4 s, g6 huge-delta 3 s, g7 14 s. Each scenario costs
~10-20 s of wall time (start, the guests' final drain, the quiet window, stop), not its 0.3 s of traffic. Defaults are now
G2 4 seeds (`RS_GRADE_G2`), G3 4 seeds (`RS_GRADE_SEEDS`), G4 2 seeds (`RS_GRADE_G4`); the full counts run with the env knobs or
in `g3_stress_full_seed_list`. Reduced default `grade_stress`: 17.4 min wall under that load (MEASURED once); NOT the "fast
default" the owner asked for -- the harness's per-scenario drain/stop cost has to come down first (INFERRED: the fixed 5 s
waits and 1 ms polls in `run_scenario`), or `grade_stress` moves behind `#[ignore]` with the model groups as the default.
In that reduced run `g5_efficiency_stress_idle_blocks_for_one_second` was RED once (the failure text was not captured); 3
immediate re-runs alone were green (parks 1565-3836 for 1181-1475 doorbells). Unexplained: either a quiet-window wake under heavy
load or the parks ceiling; open, re-run with `--nocapture` to classify.

## 7. Validity: the suite against known-bad revisions (`scripts/relay_validity.sh`, MEASURED)

`scripts/relay_validity.sh 0d325edab cfdcaf73b 82830cd5e edde750cd` (19.6 min). The grader files overlaid, lineage detected,
`grade_model` + `grade_locks` + `grade_static` run; `grade_stress` NOT APPLICABLE on these lineages (listed, not a pass).
Every red below is a test that compiled and ran (the "COMPILE ERRORS" banner in that run was a false match on cargo's
`error: test failed` line, fixed in the script).

| revision | lineage | red / ran | red groups (tests) |
|---|---|---|---|
| `0d325edab` fairness refactor, no fix | budget | 16 / 25 | G1 (f1, f3, f5, act, popped, exhaustive 1/2/3), G2, G4, G5, G6, G8 (lineage protocol), G9 (heuristics: `may_retry`, `CONTENTION_RETRIES`, `Peek::Backstop`, spin; blocking), G11 |
| `cfdcaf73b` LOOK_EVERY fix | look_every | 14 / 25 | as above minus `g1_f3` and `g1_f5` (LOOK_EVERY does fix the busy-worker look) |
| `82830cd5e` fairness review | look_every | 14 / 25 | same 14 |
| `edde750cd` claude/relay-fairness-2 head | look_every | 14 / 25 | same 14 (its `Handoff` module is not exercised by the adapter; the convoy/budget path still is the lineage's) |
| `ab6193aff` + grader (relay-simple) | simple | 2 / 33 default | G5 model, G9 Watched (section 6) |

Expected-red criteria met: 0d325edab red on G1 and G6 (G3 not applicable: no harness hooks on that lineage); cfdcaf73b /
82830cd5e red on the exhausted-try_lock cases in G1 (`g1_f1_convoy_past_budget_is_served`) and G8
(`g8_lineage_protocol_never_strands`). G11 is red on old revisions partly because cited tests do not exist there (not a
behavioural signal); G9 heuristics red there for the right reason (the budget / period / backstop lines).

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
