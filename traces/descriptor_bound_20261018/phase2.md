# G1 Phase 2 — bounded worker classification

**STATUS: LIVE, 2026-10-22. GPU-free validated; hardware pending.**

CODE READ: follows the descriptor decision and the corrected inline boundary in
`V3_USER_DESCRIPTOR_PHASE2_20261022.md`. Pointer/NOP admission remains Phase 1.
Scan/Window classification uses snapshots <=917504 bytes, one per Pump turn,
<=10 per descriptor attempt, with cross-part parser state and END stopping later
parts. It publishes the original word once after re-admitting every source
receipt. A trigger above staging refuses as `USER_RING_STAGED_SPAN_BUDGET` before
any host software action or native publication. No new inline stage or timer.

MEASURED (G1 validation, 2026-10-22): tests-only RED `469251ded` compiled and
failed 17 stream tests on Phase 1. The resumed production Relay unit suite
passes 66/66; real `userread` receipt tests pass 13/13. The two actual Plane/Relay
fairness schedules require a second relay to progress within K=1 worker passes
with continuously advancing PUT, one bounded part per turn and at most nine
parts per visited relay/pass. Streamed source/private GET wrap tests reach
marker observation and guest GET write-back with simulated native observations.

MEASURED (G1 characterization, 2026-10-22): six unchanged wrap tests pass on
master `e9c6d39e5` and accepted Phase 1 `468a35c9d`. They enumerate every range on
small rings, ring-end copies, the maximum admitted ring, reserved-slot full,
equal/empty after a lap, native GET wrap and hostile u32 values. These cases
found no defect, so their characterization commit is green. Equality never
invents an N-entry lap: the current production contract reserves one slot.

MEASURED (G1 mutation campaign, 2026-10-22): all fourteen hand source mutations
compiled and failed in >=2 independent tests each; each was restored before the
next. Full logs remain outside git; this is the small evidence table.

| Source mutation | Killing tests | Two independent killers |
|---|---:|---|
| part_yield_removed | 19 | `stream_1026596_window_uses_two_turns_and_commits_only_on_intact_publication`; `stream_carries_packet_from_previous_descriptor_and_leaves_open_tail_for_next` |
| packet_state_forgotten | 3 | `stream_random_packet_forms_masks_triggers_and_end_match_whole_descriptor_scan`; `stream_software_header_at_part_end_is_a_trigger_not_a_cross_gp_packet_error` |
| end_does_not_stop_parts | 2 | `stream_end_in_first_part_never_reads_trigger_shaped_later_part`; `stream_end_in_second_part_never_reads_a_third_part_after_more_doorbells` |
| trigger_forwarded | 3 | `stream_random_packet_forms_masks_triggers_and_end_match_whole_descriptor_scan`; `stream_software_header_at_part_end_is_a_trigger_not_a_cross_gp_packet_error` |
| only_last_receipt_checked | 2 | `mem::userread::tests::stream_receipts_first_replaced_part_cannot_reserve_credit`; `mem::userread::tests::stream_receipts_middle_revoked_part_cannot_reserve_credit` |
| scan_committed_per_part | 11 | `stream_1026596_window_uses_two_turns_and_commits_only_on_intact_publication`; `stream_carries_packet_from_previous_descriptor_and_leaves_open_tail_for_next` |
| stream_survives_retry | 2 | `stream_publication_retry_keeps_scan_uncommitted_and_owes_a_fresh_attempt`; `stream_read_retry_drops_prior_parts_and_restarts_from_first_part` |
| receipt_topology_not_checked | 2 | `mem::userread::tests::stream_receipts_generation_change_after_last_copy_forbids_publication`; `mem::userread::tests::stream_receipts_topology_revocation_between_parts_forbids_publication` |
| receipt_owners_not_checked | 4 | `mem::userread::tests::stream_receipts_mixed_birth_generations_forbid_publication`; `mem::userread::tests::stream_receipts_mixed_ram_owners_are_never_a_joint_authority` |
| receipt_count_unbounded | 2 | `mem::userread::tests::stream_receipts_empty_or_eleventh_part_refuses_before_callback`; `mem::userread::tests::stream_receipts_twenty_part_attempt_is_not_an_unbounded_admission_walk` |
| original_word_changed | 10 | `stream_1026596_window_uses_two_turns_and_commits_only_on_intact_publication`; `stream_carries_packet_from_previous_descriptor_and_leaves_open_tail_for_next` |
| staged_budget_counter_dropped | 2 | `stream_software_header_at_part_end_is_a_trigger_not_a_cross_gp_packet_error`; `stream_trigger_in_second_part_refuses_staged_span_without_publication_or_action` |
| one_copy_exceeds_snapshot_bound | 21 | `stream_1026596_window_uses_two_turns_and_commits_only_on_intact_publication`; `stream_carries_packet_from_previous_descriptor_and_leaves_open_tail_for_next` |
| guest_cursor_does_not_wrap | 3 | `wrap_characterization_every_small_ring_range_copies_modulo_in_order`; `wrap_characterization_full_means_one_reserved_slot_even_when_put_is_below_get` |

MEASURED (G1 final checks, 2026-10-22): `cargo test -p kf-chan -p kf-qemu
--locked --release -j2 --no-fail-fast` passed 1087 tests, failed zero, with four
existing ignores across 36 test/doc-test groups. All original small-entry
corpus/model/property/stress checks remain green. After the typed-chunk lint
cleanup at commit `5baaa55eb`, the entire kf-qemu package rerun passed 404/404
(zero failed or ignored); kf-chan source was unchanged.

MEASURED (G1 clippy comparison, 2026-10-22): identical baseline/final
`cargo clippy -p kf-chan -p kf-qemu --all-targets --locked --release -j2
--message-format=json` commands emit 379 warnings each and zero errors.
The repository diagnostic identity function counts 263 sites before and after,
zero added. Both emission counts and site identities were checked, because an
identical new source line can otherwise look like an existing site. No lint
suppression or diagnostic baseline was added. The qemu guest-facing deny set
and the new chunk module's deny set remain active. Rustfmt check passed.

MEASURED (G1 documentation checks, 2026-10-22): `python3 scripts/ci/debt.py claims`
reports `new=0` with the original repository baseline, after adding original
record attribution to inherited G1/T1 notes. Product decisions and fixture data
were not changed by those documentation edits. Phase 1 branch `468a35c9d` and
master `e9c6d39e5` remain unchanged. All builds used the repository cargo flock,
nice 19, release, two jobs and the lane-owned target. One full-suite lock wait
expired at 600 seconds; the bounded retry acquired the lock. No holder was
interrupted and no GPU/VM was used.

INFERRED: only hardware can confirm whole catia-07 and isolated
`03_ThreeLoftJetsDarkMirror` three times on the run-3440 configuration, real
native/marker completion and interrupt ordering, `pointer_large_entries > 0`,
`pointer_entry_max_bytes >= 947340`, application survival with no new guest
153/4101 or host Xid. G1 hardware n=0; no performance/TDR-repair claim. The reset
without a Budget refusal remains a separate investigation.

CODE READ: production-log regression recipe, for each kept baseline and matching
candidate Heaven/Minecraft/Steel Nomad log:

```sh
rg -n 'USER_RING_GP_FORMAT Budget' "$kept_log"
rg -n 'USER_RING_(GP_FORMAT Budget|SCAN_BUDGET|STAGED_SPAN_BUDGET)' "$candidate_log"
rg -n 'USER_RING_LIFECYCLE|poisoned=' "$candidate_log"
```

INFERRED: no budget matches are required (rg exits 1 for no matches); a zero large
Pointer count means catia did not exercise [T1’s measured case](../../docs/design/V3_TDR_SPECVIEWPERF_20261022.md).
These hardware/log checks remain pending. Draft PR #79; Phase 1 PR #76 stays unchanged.
