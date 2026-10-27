# Mutation checks

MEASURED in ENS11 CPU validation 2026-10-27 at source `d5a24bf80fdc0ecca2d950e754b6963ed529e0da` (full regression), with earlier mutation stages at `7733a696a6b325abda40bf53440e46b5a1289c38`: every listed mutation produced at least two distinct failing tests; compile errors count zero. Raw-log SHA256, exact commands and failing test names are retained in `mutations.json`.

| Family | Mutation | Distinct failures | Evidence | Source |
|---|---|---:|---|---|
| ENS11 | `stage_sum_omits_gate_time` | 9 | runtime/model | `7733a696a` |
| ENS11 | `gpu_done_replaced_by_host_observation` | 4 | runtime/model | `7733a696a` |
| ENS11 | `wrong_vector_is_accepted` | 2 | runtime/model | `7733a696a` |
| ENS11 | `unreached_marker_is_accepted` | 4 | runtime/model | `7733a696a` |
| ENS11 | `cross_stream_marker_is_accepted` | 2 | runtime/model | `7733a696a` |
| ENS11 | `failed_irq_is_accepted` | 3 | runtime/model | `7733a696a` |
| ENS11 | `packet_metadata_is_unbounded` | 2 | runtime/model | `7733a696a` |
| ENS11 | `notification_source_is_lost` | 5 | runtime/model | `7733a696a` |
| ENS11 | `packet_slow_record_freezes_at_first_dump` | 2 | runtime/model | `7733a696a` |
| ENS11 | `split_slow_record_freezes_at_first_dump` | 2 | runtime/model | `7733a696a` |
| ENS11 | `journal_sequence_encoding_wraps` | 2 | runtime/model | `7733a696a` |
| ENS11 | `specific_blockers_hidden_by_parent_loop` | 3 | runtime/model | `7733a696a` |
| ENS11 | `native_tracker_publication_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_tracker_marker_observation_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_notification_journal_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_activity_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_request_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_queue_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_start_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_apply_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_ready_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_release_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_resume_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_split_cancel_is_dropped` | 2 | runtime/model | `7733a696a` |
| ENS11 | `split_critical_layout_is_reenabled` | 4 | runtime/model | `7733a696a` |
| ENS11 | `new_request_does_not_interrupt_layout` | 2 | runtime/model | `7733a696a` |
| ENS11 | `empty_namespace_cleanup_obeys_optional_budget` | 2 | runtime/model | `7733a696a` |
| ENS11 | `same_snapshot_starts_one_walk_per_request` | 4 | runtime/model | `7733a696a` |
| ENS11 | `late_split_joins_an_already_started_walk` | 2 | runtime/model | `7733a696a` |
| ENS11 | `deferred_layout_is_not_retained_after_apply` | 3 | runtime/model | `7733a696a` |
| ENS11 | `deferred_layout_always_selects_first_key` | 2 | runtime/model | `7733a696a` |
| ENS11 | `deferred_layout_lifetime_cleanup_is_lost` | 2 | runtime/model | `7733a696a` |
| ENS11 | `row_release_does_not_wake_registered_relays` | 2 | runtime/model | `7733a696a` |
| ENS11 | `row_handoff_runs_before_the_actual_unlock` | 2 | runtime/model | `7733a696a` |
| ENS11 | `resolved_named_pdb_stays_global` | 2 | runtime/model | `7733a696a` |
| ENS11 | `all_pdb_borrows_a_named_scope` | 2 | runtime/model | `7733a696a` |
| ENS11 | `split_enqueue_does_not_interrupt_layout` | 2 | runtime/model | `7733a696a` |
| ENS11 | `idle_layout_never_clears_the_interrupt` | 2 | runtime/model | `7733a696a` |
| ENS11 | `native_optional_retry_is_on_split_path_again` | 2 | source-composition | `7733a696a` |
| ENS11 | `latency_alone_does_not_start_the_probe` | 2 | source-composition | `7733a696a` |
| ENS11 | `refresh_readiness_and_retry_refusals_are_ignored` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_applied_milestone` | 4 | runtime/model | `7733a696a` |
| ENS10 scope replay | `ack_untaken_statements` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `retain_applied_holds` | 4 | runtime/model | `7733a696a` |
| ENS10 scope replay | `blind_ready_registration` | 6 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_translated_ready_rearm` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_before_apply_ready_wake` | 2 | source-composition | `7733a696a` |
| ENS10 scope replay | `late_statement_release` | 3 | runtime/model | `7733a696a` |
| ENS10 scope replay | `vm_wide_again` | 16 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_own_hold` | 27 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_global_hold` | 15 | runtime/model | `7733a696a` |
| ENS10 scope replay | `lose_running_identity` | 9 | runtime/model | `7733a696a` |
| ENS10 scope replay | `ignore_new_arm` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `early_statement_clear` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_source_waiter` | 3 | runtime/model | `7733a696a` |
| ENS10 scope replay | `forget_scope_decision` | 3 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_scope_size_cap` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `drop_cpu_window_fallback` | 3 | runtime/model | `7733a696a` |
| ENS10 scope replay | `bypass_publication_gate` | 2 | source-composition | `7733a696a` |
| ENS10 scope replay | `bypass_migration_gate` | 2 | source-composition | `7733a696a` |
| ENS10 scope replay | `restart_hold_clock` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `restart_writeback_clock` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `omit_pending_targets` | 2 | runtime/model | `7733a696a` |
| ENS10 scope replay | `omit_active_targets` | 3 | runtime/model | `7733a696a` |
| ENS10 scope replay | `collapse_root_aliases` | 2 | runtime/model | `7733a696a` |
| Wait-owner correction | `wait_inherits_unrelated_owner` | 2 | runtime/model | `70eff1096` |
| Wait-owner correction | `wait_drops_matching_owner` | 2 | runtime/model | `70eff1096` |
