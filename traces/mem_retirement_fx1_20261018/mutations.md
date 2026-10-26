# FX1 mutation kills

MEASURED, 2026-10-26: each mutant compiled and exited 101 under the retirement test corpus; `mutations.json` lists every killing test. Sources were restored after each campaign.

| Mutant | Killing tests | Two independent killers |
|---|---:|---|
| `early_batch` | 11 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `sim::retirement::an_empty_retry_refresh_finishes_pending_batch_retirement` |
| `early_micro` | 8 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `batch::tests::a_micro_retired_during_invalidate_waits_for_the_next_completion` |
| `free_on_invalidate_error` | 4 | `sim::retirement::refused_invalidate_keeps_batch_pins_for_the_retry`; `sim::retirement::repeated_refused_invalidates_never_allow_hostile_page_reuse` |
| `snapshot_after_invalidate` | 2 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `batch::tests::a_micro_retired_during_invalidate_waits_for_the_next_completion` |
| `forget_failed_retirements` | 4 | `sim::retirement::refused_invalidate_keeps_batch_pins_for_the_retry`; `sim::retirement::refused_invalidates_do_not_accumulate_duplicate_micro_pins` |
| `skip_pending_apply` | 2 | `sim::retirement::an_empty_retry_refresh_finishes_pending_batch_retirement`; `sim::retirement::an_empty_retry_refresh_finishes_pending_micro_retirement` |
| `drain_without_invalidate` | 5 | `sim::retirement::a_failed_unmap_and_refused_invalidate_leave_the_booked_batch_pinned_at_retire`; `sim::retirement::a_failed_unmap_that_acted_still_requires_invalidate_before_drain_frees_backing` |
| `drain_only_queued` | 2 | `sim::retirement::a_failed_unmap_and_refused_invalidate_leave_the_booked_batch_pinned_at_retire`; `sim::retirement::a_failed_unmap_that_acted_still_requires_invalidate_before_drain_frees_backing` |
| `never_free_batch` | 9 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `sim::retirement::an_empty_retry_refresh_finishes_pending_batch_retirement` |
| `never_unpin_micro` | 7 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `batch::tests::a_micro_retired_during_invalidate_waits_for_the_next_completion` |
| `unpin_before_completion` | 3 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `batch::tests::a_micro_retired_during_invalidate_waits_for_the_next_completion` |
| `duplicate_micro_pins` | 3 | `sim::retirement::per_run_unmap_does_not_free_the_micro_before_stale_translation_is_gone`; `sim::retirement::refused_invalidates_do_not_accumulate_duplicate_micro_pins` |
| `steer_refuses_temporary_pins` | 2 | `batch::tests::a_batch_retired_during_invalidate_keeps_its_pin_until_the_next_completion`; `batch::tests::a_micro_retired_during_invalidate_waits_for_the_next_completion` |
| `model_forgets_reuse` | 3 | `sim::retirement::hostile_discard_cannot_reuse_a_page_reachable_by_a_stale_translation`; `sim::retirement::stale_translation_model_detects_early_unpin_after_a_refused_micro_invalidate` |
| `model_forgets_early_free` | 2 | `sim::retirement::stale_translation_model_detects_early_unpin_after_a_refused_micro_invalidate`; `sim::retirement::stale_translation_model_detects_early_unpin_and_reuse` |
