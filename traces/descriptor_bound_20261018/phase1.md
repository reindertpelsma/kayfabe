# G1 Phase 1: mode-scoped descriptor admission

**STATUS: GPU-FREE VALIDATED, 2026-10-22. Hardware pending.**

CODE READ: follows `V3_USER_DESCRIPTOR_DECISION_20261022.md` Phase 1. Pointer and
NOP words use hardware-format admission, remain bit-exact and copy no PB bytes.
Scan/Window above 917504 bytes refuse as `USER_RING_SCAN_BUDGET` before a PB read
or gate. Snapshot/staging bounds remain unchanged; no native descriptor is split.

MEASURED (G1 validation record 2026-10-22): test-only RED `149bc39e0` compiled and produced 227 passes / 15 failures;
RED `eb9e513e0` separately proves the 917508-byte boundary (0 passes / 1 failure).
The production Relay tests passed 38/38 after the fix. The real Storage tests
cover all twelve T1 words, maximum format length, malformed encodings and real
credit fields. The 1026596-byte relay test reaches marker observation and guest
GP_GET write-back; these are simulated native observations, not GPU execution.

MEASURED (G1 validation record 2026-10-22): all 13 source mutations below have at least two independent killing
tests. Every mutation was restored. Full logs stay outside git; this table is
the small evidence record.

| Mutation | Killing tests | Two independent killers |
|---|---:|---|
| pointer_admission_keeps_snapshot_budget | 12 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `a_large_captured_descriptor_is_not_reloaded_after_guest_mutation` |
| storage_keeps_snapshot_budget | 3 | `user_storage::tests::large_user_descriptors_cross_snapshot_bound_without_changing_real_credit_fields`; `user_storage::tests::write_user_gp_stores_each_run3440_descriptor` |
| pointer_format_check_bypassed | 2 | `an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published`; `pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit` |
| storage_format_check_removed | 3 | `user_storage::tests::bad_user_descriptor_cannot_modify_private_ring_or_userd`; `user_storage::tests::shadow_writer_preserves_opaque_bits_but_cannot_publish_or_forge_progress` |
| scan_budget_removed | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| scan_budget_after_gate | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| large_pointer_attempts_pb_read | 9 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `a_large_captured_descriptor_is_not_reloaded_after_guest_mutation` |
| pointer_large_counter_dropped | 4 | `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum`; `pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer` |
| pointer_max_counter_dropped | 5 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum` |
| vm_pointer_maximum_is_summed | 2 | `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum`; `pointer_maximum_is_not_summed_on_repeated_delta_flushes` |
| scan_refusal_counter_dropped | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| scan_budget_event_dropped | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| large_counter_gates_publication | 9 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `a_large_captured_descriptor_is_not_reloaded_after_guest_mutation` |

MEASURED (G1 validation record 2026-10-22): identical baseline/final clippy commands emit 379 warnings each,
261 distinct sites each, zero added sites and zero errors. The kf-qemu non-test
deny set remains active. No warning suppression was added.

MEASURED (G1 validation record 2026-10-22): the small-entry action transcripts agree before/after in default and
always-scan modes. The original long randomized I/O golden remains unchanged
and passes: its legacy counter serialization omits only the three newly added
observation fields, which have independent counter tests. The bit-flip coverage
is now exact: 4 direct bases x 3 unsupported bits + 2 NOP bases x 9 illegal high
bits = 30 invalid / 354 valid encodings, each still checked word-by-word. The
fixture ID decoder no longer confuses a legal large length with a torn word.

MEASURED (G1 validation record 2026-10-22): the first full run exposed those oracle maintenance issues; the two
corrected targets then passed 63 model tests (3 existing ignores) and 38 property
tests. A fresh full `kf-chan` + `kf-qemu` run then exited 0: 1046 passed,
0 failed, 4 existing ignores across 36 test binaries/doc-test groups.

CODE READ: H-A and its falsifier are in the decision before these runs. MEASURED:
all twelve words are forwarded unchanged in the production relay fixture, the
maximum-length/32768-entry tests read no PB, and no Pointer read violation is
recorded. CODE READ: no new host allocation is sized by Pointer length. Native
execution and the TDR-causality hypotheses remain unverified.

INFERRED — hardware still owes the whole catia-07 viewset and isolated subtest
03_ThreeLoftJetsDarkMirror three times on the run-3440 configuration. Require
`pointer_large_entries > 0`, `pointer_entry_max_bytes >= 947340`, real marker/native
completion, no budget refusal, no guest event 153/4101 and no new host Xid. The
reset without a Budget refusal remains a separate investigation. Hardware n=0
in G1; no performance or application-survival claim.

CODE READ — regression recipe for each kept Heaven/Minecraft/Steel Nomad log
and each matching candidate log:

```sh
rg -n 'USER_RING_GP_FORMAT Budget' "$kept_log"
rg -n 'USER_RING_(GP_FORMAT Budget|SCAN_BUDGET|STAGED_SPAN_BUDGET)' "$candidate_log"
rg -n 'USER_RING_LIFECYCLE|poisoned=' "$candidate_log"
```

INFERRED: no budget matches are required (rg exits 1 for no matches); zero large
Pointer count means the catia case was not exercised. G1 did not query production
logs or run a GPU workload/trusted-host VM. CODE READ: the GPU-free Rust suite
includes local KVM integration fixtures. Local builds used the repository cargo lock, two jobs,
release, nice 19 and the lane-owned target. One initial 600-second lock wait
expired; the next acquired it without disturbing the holder. Phase 2 begins only after this green Phase 1 handoff is pushed, on a separate branch/PR.
