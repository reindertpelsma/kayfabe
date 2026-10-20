# HW3 review of M and B′ (2026-10-21)

STATUS: RESEARCH; review separate from those branches' authors, by the HW3 lane.
No other session or agent was contacted; source branches remain read-only.

Before validation: H3 — B′'s green subsets do not imply its independent grader
is green. FALSIFIER: the unchanged exact B′ snapshot passes the full selected
grader with all cases explored and none softened/ignored.
H4 — M's real handoff answers novel schedules. FALSIFIER: any lost attempt is
left without an immediate retry or holder ring at quiescence, or the test never
actually loses a lock. Holdouts use new seeds, 1–3 workers, forced-loss positive
controls and racing releases. Source remains M's unchanged `Handoff`.

CODE READ, M (`89fc8a222`):

- `user_loop_model` now calls copied `legacy_look_needed`/`legacy_idle`, not the
  corresponding production decisions. It remains a historical-model check;
  its passing count cannot substantiate the current loop. The replacement
  fairness, v2, Pointer-v3 and actual handoff tests provide separate coverage.
- `Handoff::register/take` use a blocking Mutex and a Vec. This does not meet the
  latest worker-only-epoll blocking rule by construction. `PtGuard::drop` can
  acquire that registry lock after unlocking. HW3's slot handoff therefore uses
  per-cell atomics, with no shared registry mutex. This review does not repair M.
- The current worker samples the wake word before scanning and dispatches
  ready fds before its private-word evaluation. No test-mode branch was found
  in that loop or Handoff. The optional infinite-park flag tests a production
  configuration, rather than silently substituting test decisions.

CODE READ, B′ (`c452ca7cb`):

- The snapshot's `grade_adapter` describes M's lineage as the old budget/period
  implementation; M actually deleted those mechanisms. Do not use that adapter
  to claim a cross-lineage comparison against M without updating it.
- Known weaker checks, explicitly recorded in B′'s design §5.4: I7 hard failure
  becomes soft unless `RS_STRICT=1`; I6 excludes wake-in-flight/kernel-ready
  states; quiescence requires no rung/owned token or demand and counts ended
  wakes instead of parks; classified seed 1 is ignored; ReadIndex `i1` accepts
  the 6 M-state exploration bound; lifecycle/lazy tests are restated by policy.
- The `lazy_d6` ReadIndex replacement checks `!dead` and `written_get == cursor`
  but does not assert all expected entries were consumed in that branch. Those
  equalities alone allow an alive, motionless relay. Keep an independent
  completion-count assertion in any coverage claim for this replacement.
- `worker::set_park_net` is an unconditional public test knob with a process-wide
  atomic; normal production has no call. Chaos hooks compile only under a
  feature, enabled by the crate's self dev-dependency. That makes test and normal
  builds differ structurally; it is useful instrumentation, not proof of
  identical scheduling. No hidden test-aware success substitution was found.
- The branch records remaining phantom cycles with an act holder and grader
  exploration-budget failures. Do not promote B′ based only on smoke results.

Limits: source review plus GPU-free validation; neither confirms Windows hangs,
B′ hardware backstop bucket classification, host ioctl latency, or GPU ordering.
Validation results and exact commands are recorded in README.md.

H7 (before B′ holdouts): exact archived B′ remains live under novel contention
seeds, strict soft failures enabled, infinite park, 1/2/3 workers. FALSIFIER:
any seed fails the harness's ordering/progress/sleep invariants or strict soft
checks. Seeds 16127–16132, six per worker count, 0.3 s generation per seed;
the drain and invariant checks extend each run. Archive contents are unchanged
from c452ca7cb. Builds use an isolated target subtree and the shared cargo lock.

Named B′ changes requiring care when comparing counts (CODE READ):

| Test or harness | Change and limit |
| --- | --- |
| `relay_stress::run_scenario` I7; copied `grade_stress` harness | HARD→SOFT; strict runs requested here. Quiet-window eligibility now excludes rung/owned/demanded work and counts wakes rather than parks. |
| `relay_stress` I6 monitor; `i6_classified_seeds_hold_the_sleep_invariant` | Kernel-ready and wake-in-flight exclusions; evidence is a conditional sleep-safety check. |
| `relay_stress::i6_classified_slow_seed_1` | Moved to ignored; long completion unexplained. |
| `user_lifecycle_model::i1_pointer_mode_forwards_the_guests_entries_bit_exact_and_scans_none_of_them` | Under ReadIndex only, max-state failure becomes successful bounded exploration. Default Popped keeps the old bound failure. |
| `user_lifecycle_model` `m1_no_pointer_markers...`, `l1_latched_markers_off...`, `lazy_every_other_scenario...` | Expected deadlock restated by GET policy. Valid policy distinction, but comparison must identify the policy. |
| `user_lazy_turn::b_a_turn_after_a_wait_always_observes`, `b_a_wait_forces_a_look_on_every_turn_until_it_is_over` | ReadIndex cases use a smaller private ring to force a credit wait. |
| `user_lazy_turn::b_d6_a_guest_ring_that_looks_full_is_written_back_by_the_very_next_turn` | ReadIndex replacement asserts cursor 63 and GET equals cursor, rather than the pop-look sequence. |
| `user_lifecycle_props::lazy_d6_a_turn_that_skipped_its_look_and_saw_a_full_guest_ring_is_followed_by_a_look` | ReadIndex branch checks alive and GET=cursor; missing explicit total-consumed assertion noted above. |
| `user_turn_unit::the_guest_ring_is_full_when_gp_put_leaves_one_slot_before_the_gp_get_the_guest_sees` | Initial GET assertion now depends on policy; later behavior still checked. |
| `grade_model::exhaustive` and `g5_efficiency_model_no_hidden_spin` | Three-worker hot cases reduced; three-worker record-holder combinations skipped to avoid exploration cap. This is a coverage exclusion even if all selected tests pass. |

M changes that limit comparison (CODE READ): legacy v1 helpers replace calls to
current `look_needed`/`idle`; old budget/period source pins are replaced by
current-loop and handoff pins; v2 contention and unannounced-obligation models
are restated for holder handoff. These do not substitute test decisions inside
production. The old v1 green count is expressly historical, not current-loop
coverage. HW3 removes no upstream test, ignore, or assertion.
