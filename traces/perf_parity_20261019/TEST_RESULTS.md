# P1 GPU-free validation and mandatory stop

> CODE READ — SUPERSEDED at 01:30 CEST: the coordinator overruled this stop, defining the gate as baseline diagnostic equality with zero additions. MEASURED: the complete base and candidate multisets match (89 errors, 364 warning occurrences, added=0); the restored/formatted diagnostic suite passed again, 322/0/0. See `RESUME.md` and `clippy_comparison.json`. Inherited lints are not repaired.

STATUS: PARTIAL, 2026-10-19. No hardware claim or production promotion.

MEASURED: Clippy exited 101 with 89 denied diagnostics (62 arithmetic-side-effects, 27 indexing/slicing). Every primary source text exists in production `0452613b5`; see `clippy_sites.tsv`. Ten sites are in unchanged portions of modified `chan.rs`; 79 are in files identical to production. No diagnostic intersected a P1 changed line, but the Clippy build failed: this is NOT a passed gate. No lint was suppressed or capped.

CODE READ: the common lane rule requires STOP and report when a safety check blocks a step. P1 stops at the failed check. Options: repair the inherited denied sites and rerun the exact checks before resuming (recommended), or obtain owner clarification of the changed-line Clippy requirement when Clippy cannot finish the crate. No policy relaxation is presumed.

MEASURED: an attempt to remove P1's target directory was rejected by automatic approval review: deletion commands of that form are not permitted. The target remains (418 MiB); no alternative deletion method was attempted, following the no-workaround rule. All P1 tool sessions have ended; P1 never took the GPU lock or stopped the owner's VM.

## Tests

MEASURED: pinned Rust 1.99.0, release, two cargo jobs, nice 19.

| Run | Passed | Failed | Ignored | Scope |
|---|---:|---:|---:|---|
| Before implementation | 833 | 0 | 3 | kf-chan + kf-qemu; all 15 pacer tests passed |
| Before, touched crate | 312 | 0 | 0 | kf-qemu within the preceding run |
| EXPECTED RED | 1 | 3 | 0 | wiring tests; tests-only commit b8bccde88 |
| Candidate default | 316 | 0 | 0 | kf-qemu; four wiring tests |
| Candidate diag-nsi | 322 | 0 | 0 | kf-qemu; six additional recorder tests |

MEASURED: runtime requirements were committed before implementation (b8bccde88, expanded by c3d5e634c). Green suites preceded mutations and final formatting-only changes to two call expressions. The recorder was restored after every mutant. No further cargo command was run after the Clippy stop. The new module, module declaration and tests passed rustfmt; only changed hunks of existing large files were formatted.

| Mutant | Failing tests | Kill witnesses |
|---|---:|---|
| default_on | 2 | `unset_and_non_one_values_leave_the_trace_unallocated_and_silent`, `mixed_event_streams_match_an_independent_tail_oracle_in_both_modes` |
| unbounded_tail | 2 | `the_trace_retains_a_bounded_tail_instead_of_filling_during_boot`, `mixed_event_streams_match_an_independent_tail_oracle_in_both_modes` |
| source_collapsed | 2 | `a_host_edge_keeps_source_time_vector_arming_and_decision`, `mixed_event_streams_match_an_independent_tail_oracle_in_both_modes` |
| snapshot_duplicated | 2 | `finishing_twice_does_not_duplicate_output_and_later_edges_do_not_grow_it`, `mixed_event_streams_match_an_independent_tail_oracle_in_both_modes` |

MEASURED: copied Python analyzers passed syntax parsing. PowerShell, BPF symbol resolution and guest execution remain unvalidated. The adapted harness is partial preparation.

## Not completed

MEASURED: zero P1 GPU runs; no GPU lease; no guest command sent; mirror DONE absent at the last read. No host binary build, 9/9 gates, 30/30 fast suite, Windows loop, Heaven comparison, host Xid census, broker content metrics, W1 result or performance gain. Production assembly/cost was not compared. CODE READ: module, storage and observation calls are compile-time excluded by default features; the diagnostic binary was not built.
