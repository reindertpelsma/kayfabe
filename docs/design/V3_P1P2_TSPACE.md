# P1 + P2: no window in any space an unprivileged guest channel uses; Translated work in its own space

**STATUS: RESEARCH, 2026-10-05.** Integration branch
`codex/p1p2-integration-2026-10-05` combines master `906a76a4` with this branch's
complete `31b64802` tip. `KF3_TSPACE` remains **off by default**. No hardware has
run this integration. P0 (`v3-sec-nonpriv`, through `76dba5bd`) and scratch-window
bounds are now included from master; the historical P0-missing notices and
`--p0-pending` instructions below do not apply to this integration. Its hardware
log gate must require the real USER birth evidence. The window-reach and
Translated/physical-CE probes below still need implementation and hardware runs.

**Probe preparation, 2026-10-05 (`e25e1bf9`):** the separate research branch
`codex/tspace-probes-2026-10-05` adds `kf-window-probe`, a bounded raw USER-channel
read with native error-notifier and independent neighbor-liveness checks. It
builds and passes GPU-free tests only. The native instrumentation self-test,
host coordinator, full guest framebuffer/RAM/page-table fixtures and actual A/B
hardware evidence are still required; the historical NOT WRITTEN notices below
continue to apply to the complete acceptance probes. No physical-mode operand
is submitted: an audited fixture proving ownership of its actual host physical
target is a prerequisite. `scripts/p1p2/WINDOW_PROBE.md` records that boundary.

The merge preserves the display/broker and scratch-window counters alongside
the new carve-out counters, and keeps the existing unsafe allowances unchanged
while adding the zero-unsafe `kf-chan` perimeter. A display-SW test fixture now
initializes the new optional twin-state field. The unsafe-count CI pipeline
accepts grep's no-match result as zero while preserving real read/find failures;
four tests exercise zero, empty, nested, and broken-input cases. This correction
does not grant `kf-chan` permission to contain unsafe Rust.

The same integration also takes Windows source `c1d4e415`, with the signed-GOP
realize argument combined with the broker/max-fps surface at `6e52aebd`, then
the real read-only timer mapping at `e71e4a8b` as **KF3 ABI 20**. The mapping's
new FFI entry point and output-write block move the reviewed QEMU unsafe count
59 to 61; no Translated-plane unsafe power is added. Windows-specific
experiments remain opt-in. Neither this merge nor the earlier Windows boot
comparisons establish T-space correctness on hardware. See
`traces/windows_p1p2_integration_20261005/README.md` for exact local test scopes.

Historical branch implementation status follows; its hardware limits remain.

**STATUS (2026-10-04): LIVE — DESIGN, being implemented on `v3-p1p2` behind `KF3_TSPACE` (default
OFF). No box has run any increment.** The implementation record is the table directly below; the
design text after it is the 2026-10-04 revision (security review + feasibility review folded, §14).
★ **Later on 2026-10-04: two implementation reviews of incs A–D folded (row R below, §14).** The
default path is byte-for-byte today's again: inc A's refusals are COUNT-ONLY there
(`KF3_INCA_REFUSE=1` or `KF3_TSPACE=1` makes them refuse).
Audit findings: `docs/audits/2026-10-03-v3-stage1.md` S1-20, S1-21, S1-23, S1-43. Owner rulings:
`docs/OWNER_RULINGS.md` §Q (the address model and the privilege rule) and §R (the memory-safety
perimeter). It supersedes `THE_TRANSLATED_PLANE.md` §12's "every host VAS we create" (the
supersession is recorded in §12's own text).

## Implementation record (kept current; newest last)

| inc | state on `v3-p1p2` | what landed | box step owed |
|---|---|---|---|
| A | landed, CI | §3.2 refusals by name (`kf_chan::translated::REFUSED_METHODS`); `SubDeviceMask` refused; GP control entries: `SET_PB_SEGMENT_EXTENDED_BASE` decoded into the ring, `NOP` kept, every other opcode refused (`RingRefusal::ControlEntry`); the count-only census (`kf_chan::census`, `KF3_TCENSUS=1`, one `TCENSUS` line per Translated channel at free); USERD/notifier bounded to the usable heap before any host call (S1-43; ⊘ count-only on the default path since row R: `inca[… heap_out=…]`); `PlacedRows` cut exactly at both edges (`kf_qemu::mem::cut_rows`); count-only carve-out counters (`carve_gpu=` / `carve_cpu=` on the status line) | CENSUS (§8) |
| A2 | built; ON under `KF3_TSPACE=1`, OFF on the default path | the GPU-target refusal exists (`ApplyCfg::carve_refuse`, tested). ⊘ Corrected by review (row R): with `KF3_TSPACE=1` it refuses in every twin a guest non-kernel channel may run in (`kf_qemu::tspace::carve_cfg`; a guest-KERNEL space is counted, `carve_kernel=`); the default path counts only | flip the default after A's box run shows `carve_gpu=0` on each measured family |
| B | landed, CI | `kf_host::HostRm::alloc_vaspace_bare` and `map_window_perm`; `kf_qemu::tspace::TSpace` built on the VA thread as the first prewarm step with `KF3_TSPACE=1` (ring region reserved first, store window `[0, carve)`, RAM window GPU-uncached, both bottom-up, both ends refused past the ring region), logged as `tspace space=… fb=…+… ram=…+… rings=… build_us=…`; unused. ⚠ Built only with the flag: the default path stays byte-for-byte today's (the task's rollout rule), so T-TSPACE-BUILD runs with `KF3_TSPACE=1`. `TSpace` lives in its own module (`tspace.rs`) to keep out of `mem.rs` ahead of `v3-scratch-bound` | T-TSPACE-BUILD (§8) |
| C | landed, CI | `kf_chan::ttables` (the hand-written per-tier tables: CE methods per tier with inheritance, host methods, the `LAUNCH_DMA` / `SEMAPHORED` / `SEM_EXECUTE` field tables — each row held to the class header that states it, `hwref_check`); `kf_chan::tmode` (decode → unbound IR, `bind` against the placement rows at the call, launch splitting at row boundaries with the piece rules, chunking, the `Shadow`); `kf_chan::tspace_unsafe` (the perimeter, gate-3 table in §3.8; `kf-chan:0` added to the CI unsafe-containment list — zero `unsafe` blocks); `PlacedRow` carries the guest leaf's `MapPerm`; `RamMap::dma_to_file_range` (the §2.6 seam) routed through the walker's closure, `SlotWindow`, the T-mode resolver and sysmem USERD/notifiers (the PRAMIN closure waits for `v3-scratch-bound`); the shadow on today's path behind `KF3_TSHADOW=1` (one `TSHADOW` line per Translated channel at free). Deviation: host `NOP` payloads are consumed, not re-emitted — the engine reads UVM's inline data from the guest's own pushbuffer through the window, never from our ring | CENSUS (§8), with `KF3_TSHADOW=1` |
| D | landed, CI | `KF3_TSPACE=1` (default OFF; read once): Translated births go to the T-space — refused by name when it is not built (`tspace: not built (…)`, never a fallback) — on a T-space ring slot in `kf_chan::host::TSPACE_LAYOUT` (one object, one CPU view, three FIXED maps: pushbuffer and GPFIFO read-only, the fence read-write, USERD unmapped), bound at push (`tmode::push_bound`; a `Busy` piece is stashed UNBOUND); mirrors, prewarmed spares and recycled spares carry no window and no ring (`windows=none`, `reserved` empty) and are always recorded; the per-twin state word (`kf_qemu::twin::TwinState`) replaces the `kernel_vas` flip — passthrough births count `User(n)` and are refused in `Kernel`, Translated births refused in `User(n>0)`; the stale-bind counter (`TSPACE-RETIRE … stale_binds=stale/checked`, positive control `KF3_NEGCTL_STALE_BIND=1`); counters `tspace[built= twin_refused= tspace_refused= slots_leaked=]` on the status line. The box-log gate every T-mode box step runs is `scripts/p1p2/tspace_log_gate.py` (T-TSPACE-BUILD ordering and bounds, `windows=none` on every mirror and spare, the counters, the stale-bind counter and its positive control, unprivileged births present; `--windows` is the `KF3_TSPACE=0` known-positive arm), self-tested in CI by `scripts/ci/test_tspace_log_gate.py` with one planted defect per check. ⚠ Not done: the walker-context test (§7.12 second half — it needs a CUDA context, so it is a box step: a `windows=none` boot whose walks complete) | T-TSPACE-BUILD, T-WINDOW-USER, T-RING-TRANSLATED, T-PHYS-CE, REGRESSION A/B, PERF A/B (§8) |
| R | landed, CI (review fixes, 2026-10-04) | ★ Two implementation reviews of A–D folded, every finding with a test that can fail and its mutation (table below). **Default path:** inc A's refusals are COUNT-ONLY unless strict (`kf_qemu::tspace::inca_strict`: `KF3_INCA_REFUSE=1` or `KF3_TSPACE=1`) — `REFUSED_METHODS` writes forwarded as written, `SubDeviceMask` pushed raw, unnamed GP control entries skipped, the extended base recorded but not applied, the S1-43 heap bound counted (`heap_out=`), the pre-inc-A row removal kept (`rows_inexact=`), all summed on the status line as `inca[strict= counted= heap_out= rows_inexact=]`; opcode 4 is named only on Hopper/Blackwell. **T-mode:** the CPU store-view bound is the carve-out base, never a twin's window length (it is 0: every vidmem fetch failed); carve-out leaves refused in twins (A2 under the flag); the perimeter emits each launch piece's footprint registers and trigger and recomputes the footprint from them (`put_launch_piece`, `put_host_semaphore`, `put_host_sem_execute`; `WindowAddr` carries its validated bytes); a Hopper+ scrub keeps the guest's pattern; `SET_REMAP_COMPONENTS` unnamed bits refused; the stale-bind counter reads the rows' commit log (`kf_qemu::mem::RowsLog`) and counts only changes made while the fence was seen incomplete; an operand with no row yet WAITS for a pending walk (`Inbox::walk_busy`, rung by the VA thread) or an unfinished acquire, else is refused; the shadow binds post-invalidate items after the split; twin frees end on host success (`TwinState::user_freeing`/`user_released`); every ring map unmapped on release; the store window is a 2 MiB-page part and a 4 KiB tail (`fb_pages=` on the `tspace` line). **Instruments:** the twin paths are tested against a host that counts window maps, and their lines derived from the records; the box-log gate gained `--default`, `--census`, `--user`, `--p0-pending`, `--negctl NAME` and the BORN-TRANSLATED / retire / any-DEAD / P0-EVIDENCE checks; positive controls `KF3_NEGCTL_{SHADOW,HEAP,CARVE,TWIN,TSPACE_OVERSIZE,TWIN_WINDOW}`; ci.yml Gate A keeps `kf-chan` inheriting `unsafe_code = forbid` | box step 1 now gates the default path; every T-mode step as before |

★ **Mutations run on 2026-10-04** (each applied to the source, the named test run, the source restored and re-dated so cargo rebuilds it — `scratchpad` runner `mut.py`; an earlier pass whose restores left the files OLDER than the mutant artefacts was discarded and every batch re-run). One mutation survived its first test (`D-busy-loses-the-refused-chunk`: the test never refused a push inside the chunk loop); the test was strengthened to exercise both `Busy` arms and the mutation re-run red.

| mutation | result | test that turned red |
|---|---|---|
| `A1-refused-method-check-removed` | RED | refusals_by_name_every_refused_method, the_census_counts_what_the_channel_pushed |
| `A1-subdevice-mask-forwarded-raw` | RED | a_subdevice_mask_header_is_refused |
| `A1-extended-base-not-applied` | RED | extended_base_applies_to_later_entries |
| `A1-bad-control-entry-skipped` | RED | control_entries_other_than_nop_and_extended_base_are_refused |
| `A1-census-cap-ignored` | RED | a_guest_cannot_grow_the_census_past_its_cap |
| `A2-heap-accepts-reserved-regions` | RED | userd_and_notifier_bounded_to_the_usable_heap |
| `A2-heap-bound-uses-store-not-carve` | RED | userd_and_notifier_bounded_to_the_usable_heap |
| `A2-userd-bound-ignores-footprint` | RED | userd_and_notifier_bounded_to_the_usable_heap |
| `A2-cut-rows-misses-straddling-start` | RED | placed_rows_track_host_unmap_at_both_edges |
| `A2-cut-rows-drops-end-remnant` | RED | placed_rows_track_host_unmap_at_both_edges |
| `A2-carve-bound-back-to-store` | RED | carve_out_is_excluded |
| `A2-carve-refuse-flag-ignored` | RED | carve_out_is_excluded |
| `B-store-window-grows-down` | RED | tspace_builder_never_grows_down |
| `B-ring-region-check-removed` | RED | tspace_builder_never_grows_down |
| `B-store-window-whole-store` | RED | tspace_builder_never_grows_down |
| `B-ram-window-cached` | RED | tspace_builder_never_grows_down |
| `C-unclassified-ce-forwarded-as-written` | RED | every_emitted_pair_is_allowlisted_or_an_authored_address |
| `C-refused-ce-forwarded-as-written` | RED | every_emitted_pair_is_allowlisted_or_an_authored_address |
| `C-subdevice-mask-ignored` | RED | refusals_by_name |
| `C-launch-unnamed-bits-cleared-not-refused` | RED | no_guest_word_is_copied |
| `C-set-object-guest-word-copied` | RED | no_guest_word_is_copied |
| `C-no-chunking` | RED | chunking_keeps_every_piece_under_the_cap |
| `C-last-piece-keeps-pipelined` | RED | uvm_pte_write_inline_source_splits_at_a_page_seam |
| `C-every-piece-releases` | RED | uvm_pte_write_inline_source_splits_at_a_page_seam |
| `C-store-window-covers-carve-out` | RED | carve_out_is_excluded |
| `C-read-only-row-written` | RED | refusals_by_name |
| `C-no-piece-bound` | RED | refusals_by_name |
| `C-guest-va-forwarded-as-address` | RED | a_guest_address_value_is_never_emitted |
| `C-shadow-not-observed` | RED | the_shadow_observes_every_segment_and_changes_nothing |
| `C-resolve-rows-drops-permission` | RED | an_operand_resolves_through_our_rows_and_names_its_first_hole |
| `D-ring-fence-at-legacy-offset` | RED | the_tspace_ring_layout |
| `D-gpfifo-mapped-writable` | RED | the_tspace_ring_layout |
| `D-busy-loses-the-refused-chunk` | GREEN(SURVIVED) | — |
| `D-stale-negctl-ignored` | RED | the_stale_bind_counter_and_its_positive_control |
| `D-stale-retired-before-fence` | RED | the_stale_bind_counter_and_its_positive_control |
| `D-user-birth-in-kernel-space` | RED | kernel_is_sticky_and_a_recycled_space_is_reclassified, twin_state_cas |
| `D-kernel-birth-in-user-space` | RED | kernel_is_sticky_and_a_recycled_space_is_reclassified, twin_state_cas |
| `D-tmode-mirror-needs-a-window` | RED | a_tmode_mirror_is_recorded_without_any_window |
| `D-tmode-ring-falls-back-to-legacy` | RED | a_tmode_ring_hands_out_unbound_work_and_splits |
| `D-busy-loses-the-refused-chunk` | RED | a_busy_stash_is_unbound_and_rebinds_at_the_next_push |
| `D-busy-final-arm-loses-chunk` | RED | a_busy_stash_is_unbound_and_rebinds_at_the_next_push |
| `D-words-item-unbounded` | RED | a_long_run_of_address_free_methods_is_still_chunked |

★ **Review-fix mutations, 2026-10-04 (row R)** — same runner discipline (each applied, the named tests
run, the source restored and re-dated; local runs, scratchpad `p1p2fix/mut.py`, gate-script ones by
`pymut.py`). One gate survivor (`G4-negctl-shadow-any-line`: the test had no TSHADOW line whose
counters stood still) was fixed by strengthening the test and re-run red. The reviewers' three
extent mutations are `F3-M1`, `F3-M2` and `F3-M3`.

| mutation | result | test that turned red |
|---|---|---|
| `F1-countonly-method-refuses` | RED | refusals_by_name_every_refused_method |
| `F1-countonly-method-dropped` | RED | refusals_by_name_every_refused_method |
| `F1-countonly-sdm-dropped` | RED | a_subdevice_mask_header_is_refused |
| `F1-strict-sdm-pushed-raw` | RED | a_subdevice_mask_header_is_refused |
| `F1-countonly-control-refuses` | RED | control_entries_other_than_nop_and_extended_base_are_refused |
| `F1-extbase-applied-countonly` | RED | extended_base_applies_to_later_entries |
| `F1-opcode4-below-hopper` | RED | extended_base_applies_to_later_entries |
| `F1-tmode-ring-not-strict` | RED | control_entries_other_than_nop_and_extended_base_are_refused |
| `F1-heap-countonly-refuses` | RED | the_heap_bound_refuses_only_when_strict |
| `F1-legacy-cut-is-exact` | RED | placed_rows_track_host_unmap_at_both_edges |
| `F2-store-bound-is-window-length` | RED | a_tmode_twin_reads_vidmem_rows_through_the_store_bound |
| `F2-carve-tmode-not-refused` | RED | the_carve_bound_refuses_in_tmode_only |
| `F2-carve-negctl-refuses` | RED | the_carve_bound_refuses_in_tmode_only |
| `F2-carve-kernel-space-refused` | RED | carve_out_is_excluded |
| `F2-carve-twin-not-refused` | RED | carve_out_is_excluded |
| `F3-M1-remap-dst-elem-ignores-num-dst` | RED | a_hopper_scrub_keeps_the_guests_pattern, the_whole_footprint_stays_inside_its_row |
| `F3-M2-semaphored-16-counted-as-4` | RED | the_whole_footprint_stays_inside_its_row |
| `F3-M3-multiline-extent-ignores-pitch` | RED | the_whole_footprint_stays_inside_its_row |
| `F3-perimeter-fits-always` | RED | a_launch_piece_footprint_must_fit_its_validated_bytes, a_semaphore_footprint_must_fit_its_validated_bytes |
| `F3-perimeter-semaphored-size-4` | RED | a_semaphore_footprint_must_fit_its_validated_bytes |
| `F3-perimeter-multiline-ignores-pitch` | RED | a_launch_piece_footprint_must_fit_its_validated_bytes |
| `F3-scrub-zero-fill` | RED | a_hopper_scrub_keeps_the_guests_pattern |
| `F3-remap-unnamed-bit-accepted` | RED | no_guest_word_is_copied |
| `F3-wide-tier-not-wide` | RED | a_wide_sem_addr_hi_is_resolved_on_hopper_and_refused_before |
| `F3-checker-sees-4-bytes-only` | RED | the_property_checker_catches_a_forwarder |
| `F4-tmode-twin-maps-a-window` | RED | no_tmode_twin_path_maps_a_window |
| `F4-tmode-spare-maps-a-window` | RED | no_tmode_twin_path_maps_a_window |
| `F4-reuse-forgets-the-spares-window` | RED | no_tmode_twin_path_maps_a_window |
| `F4-windows-text-is-a-literal` | RED | no_tmode_twin_path_maps_a_window |
| `F4-heap-negctl-refuses` | RED | the_heap_bound_refuses_only_when_strict |
| `F4-oversize-negctl-ignored` | RED | tspace_builder_never_grows_down |
| `F4-shadow-binds-at-fetch` | RED | the_shadow_binds_after_the_segments_split |
| `F4-shadow-negctl-ignored` | RED | the_shadow_positive_control_moves_every_counter |
| `G4-counters-ignore-carve` | RED | test_a_refusal_a_leak_a_carve_leaf_or_a_heap_leaf_is_caught |
| `G4-counters-ignore-heap` | RED | test_a_refusal_a_leak_a_carve_leaf_or_a_heap_leaf_is_caught |
| `G4-dead-only-tspace-bind` | RED | test_any_dead_translated_channel_is_caught |
| `G4-stale-passes-with-no-retire` | RED | test_a_stale_bind_or_no_retire_is_caught |
| `G4-p0-scoped-without-flag` | RED | test_p0_evidence_is_required_or_the_pass_is_scoped |
| `G4-no-born-translated-check` | RED | test_a_run_with_no_translated_birth_is_caught |
| `G4-census-accepts-user-only` | RED | test_the_default_and_census_arms |
| `G4-negctl-shadow-any-line` | GREEN(SURVIVED) | — |
| `G4-negctl-shadow-any-line` | RED | test_every_positive_control_must_move_its_counter |
| `F5-stale-counts-every-change` | RED | the_stale_bind_counter_and_its_positive_control |
| `F5-incomplete-reading-not-advanced` | RED | the_stale_bind_counter_and_its_positive_control |
| `F5-epoch-not-recorded-at-bind` | RED | the_stale_bind_counter_and_its_positive_control |
| `F5-unresolved-refused-at-push` | RED | an_unresolved_operand_is_handed_back_unbound |
| `F5-wait-on-ignores-walk` | RED | an_unresolved_operand_is_handed_back_unbound |
| `F5-rows-log-never-unknown` | RED | the_rows_commit_log_answers_what_changed_since_a_bind |
| `F5-walk-busy-ignores-splits` | RED | a_walk_waiter_is_registered_once_and_taken |
| `F5-resolve-epoch-wrong` | RED | the_rows_commit_log_answers_what_changed_since_a_bind |
| `F6-gateA-zero-bar-exempt` | RED | the extracted ci.yml Gate A run on a `kf-chan` manifest without `[lints]` (the old gate passed it) |
| `F6-free-statement-ends-the-user` | RED | twin_state_cas, kernel_is_sticky_and_a_recycled_space_is_reclassified |
| `F6-kernel-birth-ignores-freeing` | RED | kernel_is_sticky_and_a_recycled_space_is_reclassified, twin_state_cas |
| `F6-unmap-every-short-circuits` | RED | a_refused_unmap_does_not_leave_the_other_maps_live |
| `F6-store-window-not-rounded` | RED | tspace_builder_never_grows_down |
| `F6-tail-placement-not-read-back` | RED | tspace_builder_never_grows_down |

★ **Box steps owed, in order (none run).** Rewritten 2026-10-04 (review: the plan could not be run as
written, and parts of it would have passed with no evidence). Every run records the source revision,
kf3 `euid`/`CapEff`, and — once P0 merges — bit 5 of every birth reply (P0's `kf-host: channel birth
… PRIVILEGED_CHANNEL=0` line). ⚠ **Every step ends by UNLOADING the guest driver** (`rmmod nvidia_uvm
nvidia_drm nvidia_modeset nvidia`, or a clean guest shutdown) **before the log is taken**:
`TSPACE-RETIRE`, `TCENSUS` and `TSHADOW` lines print when a Translated channel is FREED, and CeUtils'
channels live until the adapter is torn down. The gate is `scripts/p1p2/tspace_log_gate.py LOG …`
(self-tested in CI by `scripts/ci/test_tspace_log_gate.py`, one planted defect per check).
1. **Default-path regression** (`KF3_TSPACE` unset; inc A count-only): 30-arm suite + app matrix +
   `cup3`/`cup8` on GA106 at least, plus one Hopper or Blackwell run (the extended base). Gate:
   `--default` — `inca[strict=no counted=0 heap_out=0 rows_inexact=0]`, no T-space line; verdicts as
   master; record `carve_gpu=`/`carve_kernel=`/`carve_cpu=` per family (the A2 input). Then the owner
   decides whether inc A's refusals become the default.
2. **CENSUS** per measured family (TU116, GA106, AD106, GB203): `KF3_TSHADOW=1` on the same runs.
   Gate: `--census` — at least one `TCENSUS` line from an RM-internal channel (CeUtils), every `TSHADOW`
   line with `would_refuse=[]`, `resolve_miss=0`, `unknown_field=0`, `unclassified=0`,
   `max_pieces≤3`; commit every `TCENSUS`/`TSHADOW` line. Positive control, one run:
   `KF3_TSHADOW=1 KF3_NEGCTL_SHADOW=1`, gate `--negctl shadow`.
3. **T-TSPACE-BUILD** (`KF3_TSPACE=1`, one boot per family, driver load + unload). Gate (no `--user`;
   `--p0-pending` until P0 merges): the `tspace` line before the first `BORN Translated`, both windows
   below the ring region, `fb_pages=2M:…+4K:…` recorded, `BORN-TRANSLATED`, `TSPACE-RETIRE` with
   `checked>0` and `stale_binds=0/…`, no `DEAD:` on a Translated token, `COUNTERS` (incl.
   `carve_gpu=0`, `heap_out=0`). UNVERIFIED until this run: the 2 MiB-page pin and the 4 KiB tail
   placed FIXED; the three ring maps at big-page size; PBDMA fetching from the read-only maps.
   Positive controls, one dedicated run each (each is destructive or count-only as named):
   `KF3_NEGCTL_TSPACE_OVERSIZE=1` (`--negctl oversize`: the build refuses by name, every Translated
   birth is refused, the driver does not load), `KF3_NEGCTL_CARVE=1` (`--negctl carve`, count-only),
   `KF3_NEGCTL_HEAP=1` (`--negctl heap`, count-only), `KF3_NEGCTL_STALE_BIND=1` (`--negctl stale`),
   `KF3_NEGCTL_TWIN_WINDOW=1` (`--negctl window`: the S1-21 defect reintroduced; the plain gate's
   `WINDOWS=NONE` must FAIL on the same log).
4. **REGRESSION A/B + T-WINDOW-USER** (`KF3_TSPACE=0` vs `1`, at least one app as an unprivileged
   guest user). Gate: the `1` arm with `--user` (and `--p0-pending` until P0), the `0` arm with
   `--windows` (the known positive). `KF3_NEGCTL_TWIN=1` once (`--negctl twin`: every passthrough
   birth refused — destructive). ⊘ **The T-WINDOW-USER probe is NOT WRITTEN yet** (named here:
   `tests/p1p2/window_reach_probe`, a guest-user CUDA program issuing virtual CE reads at the old
   window bases and at a store page holding a guest page table; pass = MMU fault on that twin only
   with `KF3_TSPACE=1`, success with `0`). Blocked on writing it.
5. **T-RING-TRANSLATED** and **T-PHYS-CE**. ⊘ **The guest kernel module is NOT WRITTEN yet** (named
   here: `tests/p1p2/kmod_tring`; it also exercises inc A's counted methods — the `inca` counters'
   only box positive control — and the CPU-written sysmem semaphore acquired by a T channel, the §13
   cache-attribute check). T-PHYS-CE and every bit-5 criterion are **blocked until P0
   (`v3-sec-nonpriv`) merges**: this build logs no birth-reply bit 5, so the gate's P0-EVIDENCE check
   can only pass SCOPED.
6. **PERF A/B**, as §8. Then the owner decides inc A2's default (refusal on the default path) and inc E
   (default-on, delete the window code) on that evidence.

⚠ Deviation from §8's inc A row, recorded here: the PRAMIN-plan carve-out counter is **not** in
inc A. §10 keeps inc A out of the PRAMIN plan until `v3-scratch-bound` merges (it rewrites that
code); the GPU-target and CPU-view counters in `kf-mem` `apply.rs` are in.

⚠ Deviation, review fix (2026-10-04): the Translated route's sysmem USERD view now goes through the
vIOMMU seam (`dma_to_file_range` + `at_file_offset`), but has no unit test — a `RamMap` holds process
memory adopted only through the unsafe FFI. It equals the block lookup it replaced for the one
fd-backed guest RAM kf3 supports; T-TSPACE-BUILD exercises it (CeUtils/UVM sysmem USERD).

⚠ Deviation, S1-43: the channel plane bounds a guest FB USERD/notifier by the layout it declared at
realize. A preserved console region is part of the fn-72 layout, which the channel plane does not
see, so a USERD inside a console is accepted there (it is guest memory, not kayfabe's). The
`kf_chip::bar0::FbLayout::in_usable_heap` check itself refuses a console region and is tested so.

---

**Design text, revised 2026-10-04 after an adversarial security review and an adversarial
feasibility/regression review.** This text supersedes the 2026-10-03 P1+P2 design, its review, and
the 2026-10-04 security-review revision (whose journaled copy is cut off mid-sentence; §14 records
what each review changed).

- Read at kayfabe `origin/master` `789dee9f`; `crates/kf-*` are unchanged since `e4fb0190`. Owner
  rulings §Q and §R are on master (`docs/OWNER_RULINGS.md`).
- Ground truth: `research_clones/ogkm-580.159.04`. Closed GSP/firmware behaviour is marked
  **UNVERIFIED**.
- This document states properties and checks only. It does not describe misuse procedures.

## 0. The defect, restated so the fix is keyed to it

Every mirrored host VA space carries two read-write, non-privileged whole-object maps:
- the **store window**: all guest VRAM, including the guest kernel's GPU page tables and the firmware
  carve-out that holds kayfabe's declared BAR1/BAR2 roots (`kf-chip/src/bar0.rs`, `fb_layout`);
- the **guest-RAM window**: all guest RAM.

They are placed in three paths: `create_mirror`, `prewarm` and recycled spares
(`kf-qemu/src/mem.rs`). `map_window` adds only `GROWS_DOWN` and, like every kayfabe map,
`CACHE_SNOOP` (`kf-host/src/channel.rs`, `kf-host/src/lib.rs` `nvos46_map_flags`). Passthrough
twins, including guest user processes' channels, are born in the same space (`kf-qemu/src/chan.rs`).

Recorded in the 2026-10-03 GOP box run: guest-USER twins (`privilege=0`) were born in spaces logged
with both windows (`traces/v3_display/gop_box_20261003/run_b3_qemu.log:2747`, `:2803`). Only the
Translated rewriter uses window addresses (`chan.rs` `Windows`/`SlotWindow`, used in the Translated
pump); passthrough is unparsed. The windows are therefore surplus to passthrough. This is audit
S1-21, critical under §Q.

Two related gaps are closed by the same work:
- **S1-23.** The Translated rings sit in the guest kernel's mirror, and virtual operands and
  semaphores reach the engine unchanged (`kf-chan/src/translated.rs`).
- **The carve-out through the guest's own tables.** One `VaManager` bounds every vidmem/SKED leaf by
  `fb_length`, not `carve` (`kf-qemu/src/device.rs`; `kf-mem/src/apply.rs`). So a guest leaf into
  `[carve, fb_len)` is mapped, in twins as well. §Q says a passthrough space maps "no kayfabe memory".

## 1. The invariant, keyed on channel privilege

### 1.1 Where privilege comes from, and that it is known at the alloc

- The drainer decodes `ChannelPrivilege` from the alloc's `internalFlags` (`kf-rm/src/chanlink.rs`).
  It classifies the channel with `kernel_channel`: kernel iff `PRIVILEGE == KERNEL`, or the client is
  in the RM-internal handle range.
- Guest userspace cannot set either. Guest CPU-RM recomputes the level from the call's security
  context (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:220`, `:274-291`). A userspace
  ioctl is `USER` or `USER_ROOT`, never `KERNEL` (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`).
  The level reaches the GSP (kayfabe) in the RPC's `internalFlags`. Internal client handles start at
  `0xC1E00000`; user clients start at `0xC1D00000` (`ogkm-580: resserv.h:135`, `:138`).
- The route split already turns on this (`passthrough = !a.kernel_client`, `chan.rs`), at the alloc,
  before any work runs.

### 1.2 The invariant (a new `THE_CONSTRAINTS` section)

> A host VA space in which **any** channel the guest created non-kernel runs holds only rows derived
> from the guest's own page tables, placed FIXED at the guest's own VAs, at **user** privilege. None
> of those rows names the firmware carve-out. It holds no window and nothing kayfabe owns. The one
> space that holds the windows is kayfabe's per-VM Translated space (the **T-space**). Only
> kayfabe-authored work runs there, and every address that work dereferences is computed by kayfabe.

"At user privilege" is load-bearing. Host RM places its own privileged GR context buffers in the
host hole of any twin with a GR channel (`kf-host/src/channel.rs`, `GUEST_VA_RANGES`). Only the PRIV
PTE bit keeps a USER channel off them, so this half depends on P0 (§9).

### 1.3 What P1 asserts by itself, and what it inherits

- P1 by itself: no window and no ring in any twin; a twin's guest leaves bounded by `carve` (§4.3);
  the mirror record separated from the windows.
- Inherited from P0 (`v3-sec-nonpriv`): twin channels **and T channels** are born USER (bit 5
  clear), so `DENY_PHYSICAL_MODE_CE` (`kf-host/src/channel.rs`) and the PRIV PTE bit take effect
  (S1-20).

## 2. The T-space

### 2.1 What it maps, and nothing else

The T-space is one `FERMI_VASPACE_A` per kf3 device, built from objects kayfabe owns. It is
allocated with a new `alloc_vaspace_bare`, which makes none of the `GUEST_VA_RANGES` reservations,
because no guest row ever lands in it.

| region | contents | mapping | placement |
|---|---|---|---|
| ring region | `[RING_REGION_BASE, 2^40)` | reserved first (`reserve_va`); rings are mapped FIXED through the reservation | fixed |
| store window | guest VRAM `[0, carve)`, including a preserved console region; **excludes** `[carve, fb_len)` | RW, `CACHE_SNOOP` | **bottom-up** (`high = false`), base read back |
| guest-RAM window | the whole guest memfd OS-descriptor object (`guest_ram_object`) | RW, `CACHE_SNOOP`, GPU-uncached (§13) | bottom-up, first fit after the store window, base read back |

Nothing else is mapped. In particular there are no guest rows, no USERD, and no kernel CPU mapping on
any map (the `KERNEL_MAPPING` pin in `nvos46_map_flags` on `v3-dispsw-exp` covers T-space maps too).

### 2.2 Why every T-space address sits below 2^40, on every family

- The **legacy host semaphore** `SEMAPHOREA/B` is 40-bit on every family (`cl906f.h`, `clc46f.h`,
  `clc56f.h`). Stock CeUtils releases its PB-get index with it **on every family**
  (`channelAddHostSema`, `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:731-746`).
- `SEM_ADDR_HI` is 40-bit on Turing–Ada (`clc46f.h`, `clc56f.h`) and 57-bit from Hopper (`clc86f.h`,
  `clc96f.h`, `clca6f.h`).
- A GP entry's address is 40-bit on every family (`kf-chan/src/host.rs`, `RING_VA_LIMIT`).

So both windows and the ring region must lie below 2^40. **`GROWS_DOWN` is forbidden in the
T-space.** RM placed a `GROWS_DOWN` window at `0x1fffe00000000` (`run_b3_qemu.log:2747`). With that
placement every CeUtils PB-get release would be truncated, and `RmInitAdapter` would fail on every
family. Bottom-up first fit was recorded at `fb_base=0x120000000`, `ram_base=0x405200000`, 306 ms
total on GA106/580 (`THE_TRANSLATED_PLANE.md`).

The T-space build refuses by name if either window ends above `RING_REGION_BASE`. That bounds guest
RAM + FB to about 1 TiB − 4.5 GiB − 4 GiB. A per-operand check at `≥ 2^40` stays as a second bound
(§3.4).

### 2.3 Built once, at prewarm, never lazily

- **Order:** `alloc_vaspace_bare`, then reserve the ring region, map the store window, map the RAM
  window, check both ends, and record the result. This is the first thing `prewarm` does, on its
  first tick, which already pins the RAM object (as soon as QEMU registers guest RAM and long before
  the guest driver loads).
- **Cost:** paid once per VM, not per mirror. Store window ~1 ms (`THE_TRANSLATED_PLANE.md` §14). RAM
  window 65–86 ms on one box (`mem.rs`, w827) and 672–906 ms in the app-matrix run
  (`traces/v3_cdp/app_matrix_2830988f`).
- **Never lazily.** A Translated birth before the T-space exists is refused by name:
  `tspace: not built (<why>)`. There is no fallback to mirror windows, because a fallback would reopen
  S1-21. Building on the birth path would put the pin and map inside a held reply on the single act
  thread. The first-mirror cold cost is recorded at 7.6 s against a 6 s GSP RPC timeout (`mem.rs`
  `prewarm` doc, run pr1).
- **The T-space requires the RAM object.** CeUtils and UVM semaphores and pushbuffers are in sysmem
  (`THE_TRANSLATED_PLANE.md`). With no RAM window, the build refuses by name.
- **Gate:** the box log shows the `tspace …` line before the first `BORN Translated` line.

### 2.4 The rings

Each Translated ring is born in the T-space at a T-space ring slot. The ring keeps **one** 1 MiB
device-local object and **one** CPU view: a separate object per region would add host BAR1 views per
ring, and BAR1 exhaustion is the v3-appfix J regression (`kf-chan/src/host.rs`). The ring is
re-laid-out so that every region is whole 64 KiB granules. Each map is then big-page-congruent, so
`nvos46_page_size_flag` keeps big pages (`kf-abi/src/bringup.rs`) and no 4 KiB sub-mapping question
arises.

| range | offset | length | GPU mapping |
|---|---|---|---|
| pushbuffer | `0x0` | `0xD_0000` (832 KiB) | read-only |
| GPFIFO | `0xD_0000` | `0x1_0000` (512 entries use 4 KiB) | read-only |
| fence | `0xE_0000` | `0x1_0000` | read-write |
| USERD | `0xF_0000` | `0x1_0000` | **not mapped** |

- PBDMA reaches USERD through the instance block (`RingSpec{userd_memory, userd_offset}`), and
  kayfabe writes `GP_PUT` through its CPU view, so USERD needs no GPU mapping.
- §Q requires the read-only maps: "mapped read-only except its fence". The **mechanism** that keeps
  guest work off the ring is authorship (§3): no emitted address can land in the ring region,
  because every emitted address is a window address bounded by the window's length. The read-only
  maps are the second bound.
- The half-pushbuffer limit becomes 416 KiB (`push_inner`). §3.5's chunk cap keeps every piece far
  below it.
- **Ring slots become per-VM:** 4096 slots shared by all Translated channels for the VM's life. Today
  a slot is returned only on a fully successful release (`give_ring_slot`); in the T-space a refused
  release leaks the slot until the VM ends. This is counted, the count is gated (§8), and exhaustion
  is refused by name, as today.
- Read-only GPU maps of guest leaves are already placed by v3-roperm (`MapPerm`). PBDMA fetching
  through a read-only map is **UNVERIFIED**; T-TSPACE-BUILD checks it.

### 2.5 Isolation by type, and the client

- `TSpace` is a distinct type. `birth_twin_in` cannot accept it, it is never inserted into `Mirrors`,
  and only `TSpace::ring()` places a map in it.
- For now the T-space is built in C1. OWNER_RULINGS §N says kayfabe's own allocations live outside
  the client whose display-SW address check is client-wide (`V3_DISPLAY.md` on `v3-dispsw-exp`,
  true-bound item 1). Host RM writes a display-SW release only through a kernel CPU mapping, and no
  kayfabe map carries one (the `KERNEL_MAPPING` pin), so the T-space adds no write path. The move to
  client K happens with the §N client split. That split is already a precondition of x11-dispsw
  default-on, so P1+P2 does not wait on it. Duplicating the store and RAM objects into K is allowed
  by source (`ogkm-580: sharing.c:345-352`) and **UNVERIFIED**.

### 2.6 The vIOMMU seam (§Q fourth address kind)

- A single function, `dma_to_file_range(dma, len)`, is the only path from a device DMA address to a
  memfd range. Every resolver uses it: walker sysmem leaves (the `VaManager` `ram_offset` closure),
  `SlotWindow`'s sysmem arm, and sysmem USERD and notifiers.
- Today it is `RamMap::file_range`. Under a vIOMMU it does IOVA→GPA per run first, splitting a run
  where the mapping is discontiguous and refusing (never reading through) an unmapped run
  (`V3_VIOMMU.md` §3).
- The T-space RAM window is never unmapped on a guest vIOMMU invalidation. The sync point for T
  channels is therefore: every piece bound under the old translation has **completed** (its fence)
  before the guest's invalidation completes. This has the same shape as `V3_VIOMMU.md` §3 item 3.
- Until this is built, kf3 refuses at realize behind a vIOMMU. That is unchanged.

## 3. The rewriter as an address author

Today the rewriter (`kf-chan/src/translated.rs`) rewrites physical CE operands and forwards
everything else as written: host methods below `0x100`, the CE default arm, the `SubDeviceMask`
header raw (refused from inc A), `SET_OBJECT` with the guest's full word, MEMBAR/L2 `MEM_OP` A–C as
written, and the `LAUNCH_DMA` word minus two bits. Under T-mode, every word kayfabe emits is
authored from decoded, validated fields.

### 3.1 The property

- On a T channel, every address the engine dereferences is computed by kayfabe. It lies inside
  `[fb_base, fb_base+carve)` or `[ram_base, ram_base+ram_len)`.
- No guest-written address value is ever emitted.
- Every emitted `(subchannel, method)` is in the per-tier allowlist, or is an authored address
  register.
- No guest word is copied. Each emitted word is built from decoded fields that passed the tier's
  field table.

### 3.2 What is authored, by method class

- **Address registers are state, never output:** CE `OFFSET_IN/OUT_UPPER/LOWER`,
  `SET_SEMAPHORE_A/B`, `SET_MONITORED_FENCE_SIGNAL_ADDR_*`; host `SEMAPHOREA/B`, `SEM_ADDR_LO/HI`.
- **Footprint registers are state, re-emitted at each trigger:** `LINE_LENGTH_IN`, `LINE_COUNT`,
  `PITCH_IN/OUT`, `SET_REMAP_*`. The engine therefore always runs with exactly the values the bound
  was computed from. This replaces today's post-launch restore.
- **`SET_*_PHYS_MODE` are state only.** No T launch is physical. `PEER` is refused (as today). A
  non-zero FLA, PEER_ID or BASIC_KIND field is refused by name where the tier defines one.
- **`SET_OBJECT`** is authored as the host ring's own CE class with `ENGINE = 0`. `GP100_UVM_SW` is
  consumed (as today). Any other class is refused (as today).
- **`LAUNCH_DMA`** is authored from a per-tier field table. The table must carry every field stock
  sets:
  - `SRC_TYPE` and `DST_TYPE` are always VIRTUAL; reserved bits are zero.
  - Forwarded field values: `DISABLE_PLC` (UVM on C7B5, CeUtils on C8B5); `FLUSH_ENABLE/TYPE`;
    `DATA_TRANSFER_TYPE` NONE, PIPELINED or NON_PIPELINED (`CAB5` `PREFETCH = 3` is gated on the
    census); `SEMAPHORE_TYPE` 0, 1 or 2; the `SEMAPHORE_REDUCTION` fields;
    `SEMAPHORE_PAYLOAD_SIZE` (C8B5+); `INTERRUPT_TYPE` NONE or NON_BLOCKING (CeUtils sets
    NON_BLOCKING with a callback); `REMAP_ENABLE`; `MULTI_LINE_ENABLE`; the memory layouts.
  - Refused by name: `SEMAPHORE_TYPE == 3`; `INTERRUPT_TYPE == BLOCKING` (no stock emitter found);
    `VPRMODE ≠ 0` on C7B5; `COPY_TYPE ≠ DEFAULT` on C8B5+.
  - The Hopper+ fast scrub is converted to a virtual remap fill. ⊘ **Corrected 2026-10-04
    (review):** the fill keeps the guest's pattern — its `CONST_A`/`CONST_B`/`SET_REMAP_COMPONENTS`
    — and counts `LINE_LENGTH_IN` in pattern elements (UVM's `memset_8` sends an arbitrary 64-bit
    value through the scrubber, `uvm_hopper_ce.c:298-310`); "as today" was a zero fill that dropped
    the pattern. A scrub whose remap reads a source, or whose bytes are not whole elements, is
    refused by name. The scrubber's own semantics are closed firmware (UNVERIFIED). ⚠ The default
    path still zero-fills (pre-existing; §11 decision 10).
  - `SET_REMAP_COMPONENTS` is authored from its named fields; a word with any other bit is refused
    (`ttables::REMAP_NAMED`, held to `clc7b5.h` by a test).
  - A non-zero field the table does not name is counted by the shadow (§3.6), then refused per tier.
    It is **never silently cleared**, because a dropped field changes behaviour without a refusal.
- **`MEM_OP`:** `TLB_INVALIDATE` stays a split. MEMBAR and L2 operations are authored from the
  decoded operation and its type fields, never from the guest's A–C words. `ACCESS_COUNTER_CLR` is
  served (as today). Anything else is refused (as today).
- **Address-free host methods are authored with their argument:** NOP with inline payload, WFI,
  `NON_STALL_INTERRUPT`, `SET_REFERENCE`, `YIELD`, `FB_FLUSH`. Each is checked against the census
  per tier.
- **Hopper+ `SET_MEMORY_SCRUB_PARAMETERS` (0x6FC) is address-free.** Stock pushes it; refusing it
  would break Hopper and Blackwell.
- **Refused by name, on every tier** (landed in inc A, `kf_chan::translated::REFUSED_METHODS`):
  host `CLEAR_FAULTED` (0x84; UVM pushes it only while servicing non-replayable faults, and kayfabe
  delivers none); CE `SET_MONITORED_FENCE_TYPE` and `_SIGNAL_ADDR_*` (0x21C–0x224);
  `SET_RENDER_ENABLE_A/B/C` (0x254–0x25C); `PM_TRIGGER` (0x140) and `PM_TRIGGER_END`;
  `SET_SECURE_COPY_MODE` (0x500) and the Hopper confidential-computing address methods
  (0x514–0x53C; stock uses them only under confidential computing).

### 3.3 Triggers

Each of these is a trigger: a CE `LAUNCH_DMA` that moves data, releases a semaphore, or both;
**every** host `SEMAPHORED` operation (RELEASE, ACQUIRE, ACQ_GEQ, ACQ_AND, REDUCTION); **every** host
`SEM_EXECUTE` operation, including the acquire forms UVM uses (`ACQ_CIRC_GEQ` with
`ACQUIRE_SWITCH_TSG`) and `RELEASE_TIMESTAMP`.

An unknown operation is refused by name. At each trigger the rewriter resolves the operands (§3.4),
checks them, and emits the authored address registers, the footprint registers and the trigger word.

### 3.4 Translation, by operand kind

- **Physical operand:** today's arithmetic. A `LocalFb` operand is bounded to `[0, carve)`, not
  `fb_len`. A sysmem operand goes through `dma_to_file_range` and the RAM window.
- **Virtual operand or semaphore:** resolved through the placement rows of the guest VA space the
  channel was born in (`PlacedRows`, `resolve_placed_prefix`), **under one read guard per operand**.
  Adjacent rows that are contiguous in both VA and backing are merged. Each piece becomes
  `fb_base+off` or `ram_base+off`. A piece that resolves into `[carve, fb_len)` is refused.
- **Extents:** data uses `extents()`. A semaphore uses 4 bytes, or 8 with a 64-bit payload, or 16 for
  a four-word or timestamped release. A semaphore must lie inside one row, or it is refused.
- **Permissions:** `PlacedRows` gains a `MapPerm` field (the guest leaf's permission). A write,
  release or reduction through a read-only row is refused by name, and so is a reduction through an
  `atomic_disable` row.
- **Discontiguous operands:** a 1-D operand is split at the union of the source and destination row
  boundaries; a boundary inside a remap element is refused. The first piece keeps the guest's
  `DATA_TRANSFER_TYPE`; the **last** piece is `NON_PIPELINED` and alone carries the semaphore,
  interrupt and flush. A multi-line operand must be contiguous over its whole pitch footprint, or it
  is refused. A block-linear virtual operand is refused, because `extents` ignores layout. Stock UVM
  never sets `MULTI_LINE`; its virtual operands are inline data (≤ 8 KiB), semaphores, and one-page
  CPU staging.
- **Bounds:** at most 64 pieces per launch, refused by name past that; a host semaphore that resolves
  at or above 2^40 on a 40-bit form is refused (§2.2).
- **Row exactness is a safety property now** (landed in inc A, `kf_qemu::mem::cut_rows`). Before inc
  A, `unmap_range` removed only rows whose **start** lay in the range while host RM splits a
  straddling placement and keeps its outside part: a row straddling the start stayed recorded at full
  length (stale coverage, an A.9 concern), and a row starting inside but extending past the end was
  dropped whole (missing coverage, a false refusal that kills the channel). Fix: trim or split at both
  edges, exactly as host RM does, with a test for each edge.

### 3.5 Bind at submit, and chunking

- Today the ring rewrites a whole segment at fetch and holds the decoded words. Under T-mode the
  rewriter emits an unbound IR: `Raw(words)`, `CeLaunch{regs, launch}`, `HostSem{regs, op}` and
  `Invalidate{pdb}`.
- The pump binds each piece **when it pushes it** (inside `TranslatedChannel::pump`). That is after
  any earlier split's walk has completed. A piece the host ring refuses as `Busy` is stashed
  **unbound**.
- **Chunking:** the IR is cut into pieces at trigger boundaries, with a cap of 64 KiB per piece.
  T-mode re-emits ~10–14 authored words per trigger, so a launch-dense segment can expand ~3.5× over
  its 256 KiB input. An unchunked piece could exceed the half-pushbuffer limit, which is a
  channel-killing error. Ordering inside one channel is unchanged.
- **Guarantees:** an operand bound after the guest's last invalidate on its own channel sees the
  post-invalidate rows. That is legal stale-TLB behaviour within one channel.
- **Not guaranteed:** translation at execution time across channels. A piece pushed behind a host
  acquire is bound before another channel's walk could complete. Stock producers appear safe: UVM
  waits on its kernel-mapping PTE writes on the CPU, and CeUtils' `pbGpuVA` is fixed. This is
  **UNVERIFIED**. A stale-bind counter (§7.13) checks each bound piece when its fence retires. If it
  is ever non-zero, a host acquire becomes a bind barrier. Reach stays within guest memory either way.
- ⊘ **Corrected 2026-10-04 (review) — what the counter counts.** Re-resolving at retire counted any
  difference, including the benign case: the guest sees its own release inside the work before
  kayfabe's `RELEASE_WFI` fence is read, and UVM routinely unmaps and remaps then. Now every commit
  the walk makes to a mirror's rows is numbered and timed (`kf_qemu::mem::RowsLog`, bumped under the
  rows' write lock); each bound resolution records the epoch it was read at (under the same read
  guard); at retire the first later commit overlapping the operand is **stale** (gated) only if it
  landed before the last time the runner read the fence and found it INCOMPLETE — before the work
  completed, so (within the microseconds a `RELEASE_WFI` fence trails the guest's release) before the
  guest saw it complete — else **late** (benign or indeterminate, reported, never gated), and
  **indeterminate** past the bounded log. `TSPACE-RETIRE … stale_binds=s/c late= indeterminate=`.
- ⊘ **Corrected 2026-10-04 (review) — an operand with no row yet.** It no longer kills the channel:
  `push_bound` pushes what was bound before it and hands it back UNBOUND (`Pushed::Unresolved`); the
  runner WAITS while a walk is pending on the space (`Inbox::walk_busy`: a statement unsettled, a
  split queued or running, or the VA thread busy; the VA thread rings waiting channels every idle
  loop) or while a host acquire ahead of it is unfinished (our own fence wakes us), and binds it
  again; with nothing to wait on it is unmapped and refused by name. Each wait and refusal is
  counted (`unresolved[walk= acquire= refused=]`) — the measurement §11 decision 6 asked for.

### 3.6 Classification tables, the census, and the shadow

- The allowlist **cannot be generated from headers.** `clc9b5.h` defines only the class id;
  `clcab5.h` defines two methods (`LAUNCH_DMA` 0x300, `REQ_ATTR` 0x754); `clc86f.h` and `clc96f.h`
  list no `SEMAPHOREA-D`, `NOP` or `NON_STALL_INTERRUPT`. Yet CeUtils pushes `NV906F_SEMAPHOREA-D` on
  every family, and UVM inherits CE bodies by `parent_id` (C7B5 → C8B5 → C9B5 → CAB5).
- **The tables are per tier and per offset.** They are written by hand, classified by semantics, and
  each row names the class it inherits from. Each row has one of three dispositions: address (§3.4),
  address-free (authored, §3.2), or refused by name.
- **The census is a dedicated, count-only instrument** (landed in inc A, `kf_chan::census`,
  `KF3_TCENSUS=1`). It is not `KF3_COMPLETION_PROBE`, which keeps 4 and drops the rest. It counts per
  `(bound CE class, subchannel kind, method)`, records the distinct `LAUNCH_DMA` words, the operation
  values, the header forms and the GP-entry kinds, and dumps one `TCENSUS` line per channel at free.
  Every map is capped (the input is guest-controlled); a key past the cap is counted in `overflow`.
- **The shadow.** Before any behaviour change, the T-mode rewriter and resolver run **in shadow** on
  today's path behind `KF3_TSHADOW=1`. They compute what T-mode would emit and discard it, counting
  `would_refuse` by reason, resolution misses, pieces per launch (maximum), unknown fields, and
  unclassified pairs. Precedent: the live walk shadow (`compared=65 disagreements=0`).
  ⊘ **Corrected 2026-10-04 (review):** the shadow binds a segment's items AFTER its first
  invalidate only at the next observed segment — by then the default path has run that segment's
  splits — as T-mode binds them after the walk (`bound_after_split=`, `unbound_at_free=`); binding at
  fetch counted correct UVM streams as misses. Its positive control is `KF3_NEGCTL_SHADOW=1`: three
  synthetic writes per segment through the real decode and bind move `unclassified`,
  `unknown_field` and `resolve_miss`.
- **Hardware coverage is uneven.** The families run on hardware are TU116, GA106/GA104, AD106/AD104
  and GB203/GB205 (`docs/STATUS_DETAIL.md`). Hopper, GA100 and GB10x are source-only.
- **S1-21 is not gated on the census** (§8). Window removal depends only on the T-space and the
  resolver. The census gates refusal of *unclassified* methods (S1-23), per family.

### 3.7 Header forms and GP entries

- **`SubDeviceMask`** is refused by name (landed in inc A). A single-GPU guest has no use for it, and
  it can stop later methods from executing, which would separate an authored address write from its
  trigger.
- **Control GP entries** (landed in inc A). Hopper+ UVM writes `SET_PB_SEGMENT_EXTENDED_BASE` before
  each channel's first push. Before inc A, `gp_entry_decode` returned `None` for any zero-length entry
  and the ring skipped it, so later entries were read with address bits 56:40 taken as zero. The ring
  now decodes the extended base into its state and applies it to every later entry, keeps `NOP`
  control entries, and refuses `ILLEGAL`, `GP_CRC`, `PB_CRC` and unnamed opcodes by name.
- **`SYNC_WAIT`** stays refused (as today).

### 3.8 The §R perimeter

Code that produces an address hardware will dereference is perimeter material (§R(b)). Under P2 the
following move to `kf-chan/src/tspace_unsafe.rs`: the window bounds (`[0, carve)`, the RAM length, the
host-semaphore limit); an opaque `WindowAddr` type that only this file can construct; the only
functions that put an address word or a `LAUNCH_DMA` word into the output; the ring's own address
authoring (`fence_words`; the GP entry in `push_inner`).

Each export gets a row in the §R gate-3 table, with the test that shows each check. The perimeter
ratchet (§R(c)) is bumped once, deliberately, in the increment that adds the file.

★ **The gate-3 table (inc C, 2026-10-04).** `crates/kf-chan/src/tspace_unsafe.rs` exports, each
validating its own inputs (§R: no precondition is left to a caller):

| export | checks it performs | the test that shows each check (all `kf-chan`) |
|---|---|---|
| `TWindows::new(fb, ram, limit)` | each window non-empty, no overflow, ends at or below `limit`; `limit` ≤ 2^40 | `tspace_unsafe::tests::windows_are_bounded_at_construction` |
| `TWindows::fb(off, len)` / `ram(off, len)` | `len > 0`, no overflow, `off+len` ≤ the window's length (the store window's length IS `carve`); the `WindowAddr` carries `len` as its validated bytes | `a_window_address_is_only_ever_inside_its_window`; `tests/tmode.rs::carve_out_is_excluded` |
| `TWindows::contains` | a predicate (the property tests' oracle) | `a_window_address_is_only_ever_inside_its_window` |
| `WindowAddr` | constructible only inside this file, only by `fb`/`ram` | (by type: the field is private) |
| `put_launch_piece` (review fix 2026-10-04; replaces `put_ce_offset`/`put_ce_semaphore`/`put_launch`) | emits a piece's footprint registers, its addresses and `LAUNCH_DMA` itself; RECOMPUTES the footprint from the registers it emits — source `LINE_LENGTH_IN` × (remap `COMPONENT_SIZE` × `NUM_SRC`, else 1), destination × `NUM_DST`, `PITCH` × (`LINE_COUNT` − 1) when multi-line, the release 16/8/4 bytes from `SEMAPHORE_TYPE`/`PAYLOAD_SIZE` — and refuses unless each fits its operand's validated bytes; operands must match what the launch touches; block-linear sides, unnamed `SET_REMAP_COMPONENTS`/`REQ_ATTR` bits and overflowing extents refused; upper address bits masked to the tier's field; the launch word both types VIRTUAL, every unnamed bit zero | `tspace_unsafe::tests::a_launch_piece_footprint_must_fit_its_validated_bytes`; `tests/tmode.rs::the_whole_footprint_stays_inside_its_row`, `every_emitted_pair_is_allowlisted_or_an_authored_address` (whole footprints, six tiers), `no_guest_word_is_copied`; `ttables::hwref_check::the_launch_field_table_is_the_class_headers`, `the_remap_fields_are_the_class_header` |
| `put_host_semaphore` (review fix: takes the payload and `SEMAPHORED`) | below 2^40 and 4-byte aligned; the bytes the operation touches (16 for a non-4BYTE release, else 4) derived from the word it emits must fit the validated bytes — else refused with nothing emitted | `the_forty_bit_forms_refuse_what_they_would_truncate`, `a_semaphore_footprint_must_fit_its_validated_bytes` |
| `put_host_sem_execute` (review fix; replaces `put_host_sem_addr`) | below 2^40 (2^57 on a wide tier), aligned; 16 bytes for a timestamped release, 8 for a 64-bit payload, else 4, from the word it emits, must fit | `the_forty_bit_forms_refuse_what_they_would_truncate`, `a_semaphore_footprint_must_fit_its_validated_bytes` |
| `fence_words` | the fence VA below 2^40 and aligned | `the_forty_bit_forms_refuse_what_they_would_truncate` |
| `ring_gp_entry` | the range inside the pushbuffer; the VA below 2^40 | `the_ring_gp_entry_stays_in_the_pushbuffer` |

## 4. Guest spaces after the change

### 4.1 Mirrors and spares

- `create_mirror` and spares carry **no windows and no rings**.
- The mirror record is **separated from the windows.** A mirror is recorded whenever its space
  exists. Before this, only the window-success arms inserted, so a failed window map refused every
  birth in that space, passthrough included.
- `reserved()` and `vmm_ranges()` become empty for twins. Guest leaves at `[RING_REGION_BASE, 2^40)`
  are then accepted, which is more compatible than today.
- Spares become plain spaces. They are still prewarmed and recycled, because `alloc_vaspace` and the
  reservations cost RM calls, but they no longer pay the 65–906 ms window cost.

### 4.2 The per-twin state (one atomic word)

`mirror.live` and `kernel_vas` are separate atomics today, flipped on different threads. They are
replaced by one atomic per twin: `Unclassified | User(n) | Kernel`. Transitions are CAS, made on the
statement path in statement order:

- A passthrough birth moves `Unclassified` or `User(n)` to `User(n+1)`. It is **refused by name**
  (never waited on) while the state is `Kernel`.
- ⊘ **Corrected 2026-10-04 (review):** a passthrough FREE statement moves one user from live to
  *freeing*; the deferred host free ends it only if host RM freed the channel — a refused free
  leaves it counted for the life of the space, so the space can never become `Kernel` under a
  channel host RM may still run. A Translated birth refused only by freeing users is counted apart
  (`twin_freeing=`).
- A Translated birth moves `Unclassified` to `Kernel`. It is **refused by name** while the state is
  `User(n > 0)`.
- `Kernel` is sticky until retire. A recycled space is re-derived afresh, as `kernel_vas_for(key)`
  is today: `Kernel` for an RM-internal client, else `Unclassified`.
- The walker reads the word with one atomic load when it commits a privileged leaf: it places the
  leaf only in `Kernel`, and withholds it otherwise. No lock is added to the VA thread's path.
- T births still count in `mirror.live`, so a mirror under a live T channel is never recycled.
- Both refusals are counted. The expected count on stock drivers is 0: UVM's channels live in UVM's
  own VA space, not a user's.

### 4.3 The carve-out bound on the guest's own tables

- Today one `VaManager` bounds leaves by `fb_length` for every target: GPU mirrors, the BAR1/BAR2 CPU
  windows and, separately, the PRAMIN plan.
- The bound becomes per target: **GPU-mirror (twin) targets** are bounded by `carve` (§Q's rule for a
  passthrough space); **BAR1, BAR2 and PRAMIN CPU views** (the guest kernel's own views) keep
  `fb_len` and get a count-only counter of references into `[carve, fb_len)`.
- **Count-only first** (landed in inc A: `carve_gpu=` / `carve_cpu=`). RM's heap rejects client
  placements in a reserved region (`ogkm-580: heap.c:1067-1073`). But CeUtils in `VIRTUAL_MODE` maps
  an FB alias over `[heap.base, heap.base+heap.total)` (`ogkm-580: mem_utils_gm107.c:590-601`), and
  whether that range includes the carve-out is **UNVERIFIED**. A false refusal here fails
  `RmInitAdapter`. So the GPU-target bound lands **count-only** for one A/B run per measured family,
  then refuses (inc A2).
- ⊘ **Corrected 2026-10-04 (review, HIGH):** with `KF3_TSPACE=1` the refusal is already ON, in every
  GPU space a guest non-kernel channel may run in — a twin that withholds privileged leaves
  (`User(n)` or `Unclassified`); a guest-KERNEL space is counted apart (`carve_kernel=`) and mapped,
  since under T-mode no channel runs in one and CeUtils' `VIRTUAL_MODE` alias lives in one. The
  default path stays count-only. Positive control: `KF3_NEGCTL_CARVE=1` (every vidmem leaf counted,
  none refused).

### 4.4 Stated precondition: the walker never reads through a mirror window

Removing the windows is safe for the walker because it reads guest page tables and kayfabe's roots
through its **own** CUDA context's import of the store (`import_store`), never through a mirror
window. The rewriter is the only reader of a window address. The test in §7.12 guards this.

## 5. Producers, checked one by one

| producer | what it emits | under T-mode |
|---|---|---|
| `RmInitAdapter` CeUtils, Turing–Ada | `SET_OBJECT`; physical `LOCAL_FB` memset; CE release at `pbGpuVA+finishPayloadOffset`; `906F SEMAPHOREA-D` release at `pbGpuVA+semaOffset` | memset: arithmetic in `[0, carve)`. Both releases are resolved through the CeUtils space's rows, which already exist because the pump reads that pushbuffer through them. The host release is below 2^40 only because the windows are low (§2.2). |
| CeUtils fast scrub, Hopper+ | `SET_MEMORY_SCRUB_PARAMETERS`; scrub `LAUNCH_DMA` with `DISABLE_PLC` and optional `NON_BLOCKING` interrupt; `SEM_ADDR`/`SEM_EXECUTE` release | address-free; converted remap fill (as today) with authored fields; release resolved |
| CeUtils `VIRTUAL_MODE` (config-dependent) | virtual operands through the FB alias | resolved through rows into the store window; a piece in `[carve, fb_len)` is refused; the shadow counts this before default-on |
| UVM channel init | GP control `SET_PB_SEGMENT_EXTENDED_BASE` (Hopper+); `SET_OBJECT` on subchannel 0; `GP100_UVM_SW` | decoded (inc A); authored; consumed |
| UVM page-table writes | NOP inline payload plus CE copy: virtual source (pushbuffer), physical destination (PTE memory) | source resolved and split at 4 KiB seams of a sysmem pushbuffer (≤ 3 pieces for ≤ 8 KiB); destination is arithmetic |
| UVM TLB invalidate | `MEM_OP` A–D | a split (as today) |
| UVM migrations | physical vidmem/sysmem on both sides | arithmetic (as today) |
| UVM tracking semaphores | CE release with reduction `INC`, vidmem pool | resolved; a reduction through an `atomic_disable` row is refused |
| UVM cross-channel waits | host `SEM_EXECUTE ACQ_CIRC_GEQ` with `ACQUIRE_SWITCH_TSG` | resolved. Releaser and acquirer resolve the same backing to the same window address, even across VA spaces. |
| UVM user-semaphore release | CE release at the semaphore's **kernel** VA | resolved; lands on the same page the user's passthrough channel acquires |
| UVM CPU staging | one-page virtual CE copy | resolved |
| confidential computing; non-replayable fault servicing | secure-copy methods; `CLEAR_FAULTED` | refused by name (kayfabe supports neither) |

Completions are unchanged: kayfabe's fence tail, written with `RELEASE_WFI` at `ring_va + fence`,
plus the NSI. The guest's semaphores land in guest memory exactly as today.

## 6. Cost

- **Once per VM:** the T-space build (§2.3).
- **Saved per mirror:** two window maps — 0.3–0.9 s per new mirror on the app-matrix box (run at
  `2830988f`), and 67–90 ms per `create_mirror` on vh2, inside a held page-directory reply (w827).
  Also saved: host page-table VRAM for one RAM window per mirror.
- **Per T birth:** two more FIXED maps (three instead of one), about 1 ms each. UVM creates on the
  order of 10 CE channels per GPU registration.
- **Per trigger:** one row lookup per operand under a read guard (BTreeMap, `O(log rows)`) plus
  ~10–14 authored words. The NSI wake p50 is 127 µs (`host.rs`, w826).
- **The PERF A/B (§8)** records all of these. A regression in any of them is a gate, not a note.

## 7. Unit tests that can fail (each with the mutation that must flip it)

All tests are in `kf-chan` unless noted. ✔ = landed, with its mutation run red.

1. **`every_emitted_pair_is_allowlisted_or_an_authored_address`** — random method streams through
   T-mode with a mock resolver; decodes the **emitted word stream** and checks (a) every address pair
   decodes inside a window, (b) every `(subchannel, method)` is allowlisted or an authored address
   register. Mutations: an unclassified address-bearing method fails (b); restoring the raw
   `SubDeviceMask` push fails (b). Negative controls: the same stream through today's rewriter fails
   (a); `SET_MONITORED_FENCE_SIGNAL_ADDR_*` and `SET_RENDER_ENABLE_A` through today's rewriter fail
   (b).
2. **`a_guest_address_value_is_never_emitted`** — refusal case (no covering row ⇒
   `VirtualUnresolved`) and paired case (a covering row ⇒ the emitted words equal
   `window(resolve(va))`), so the property is not vacuous on a refusal.
3. **`no_guest_word_is_copied`** — reserved `LAUNCH_DMA` bits refused, never cleared; `DISABLE_PLC`
   and `FLUSH_TYPE` survive; `SET_OBJECT` authored with the host class and `ENGINE = 0`; MEMBAR and L2
   `MEM_OP` A–C authored.
4. **`ceutils_shape_rewrites_both_completions`** — data by window arithmetic, both semaphores become
   `window(resolve(va))`, the host one `< 2^40`; Hopper variant accepts `SET_MEMORY_SCRUB_PARAMETERS`.
5. **`uvm_pte_write_inline_source_splits_at_a_page_seam`** — two launches, bytes preserved, the first
   keeps the guest's transfer type, only the last carries semaphore/interrupt/flush and is
   `NON_PIPELINED`.
6. **`split_binds_after_the_walk`** — B binds to the new backing; binding at fetch (negative control)
   binds to the old; a `Busy` stash re-binds.
7. **`chunking_keeps_every_piece_under_the_cap`** — mutation: no chunking makes `push_inner` refuse.
8. **`refusals_by_name`** — ✔ (inc A part: `refusals_by_name_every_refused_method`,
   `a_subdevice_mask_header_is_refused`, `control_entries_other_than_nop_and_extended_base_are_refused`)
   plus, with T-mode: block-linear and discontiguous multi-line virtual operands; a 40-bit-form
   semaphore at `≥ 2^40`; a write or reduction through a read-only or `atomic_disable` row; 65 pieces;
   unknown `SEMAPHORED` and `SEM_EXECUTE` operations.
9. **`carve_out_is_excluded`** — ✔ `kf-mem` (`apply::tests::carve_out_is_excluded`: counted in
   count-only mode, refused in refusal mode, one page below maps; mutation: the bound back to
   `fb_len` fails it). The `kf-chan` half (a `LocalFb` operand at `carve`) lands with T-mode.
10. **Ring layout** — every region whole 64 KiB, non-overlapping, `< 2^40`; USERD in no GPU map;
    pushbuffer and GPFIFO read-only; big pages for all three maps; the old `0xF_8000` fence offset
    makes the flag select 4 KiB.
11. **`tspace_builder_never_grows_down`** (`kf-qemu`).
12. **Mirror and walker** (`kf-qemu`/`kf-cuda`) — a mirror whose window map is forced to fail is still
    recorded and a passthrough birth in it succeeds; `walker_reads_through_its_own_context`.
13. **Stale-bind counter** — positive control `KF3_NEGCTL_STALE_BIND` makes it non-zero.
14. **`placed_rows_track_host_unmap_at_both_edges`** (`kf-qemu`) — ✔ (mutations: the straddling-start
    row skipped, the end remnant dropped — each red).
15. **`extended_base_applies_to_later_entries`** — ✔ (mutation: the base not applied — red).
16. **`twin_state_cas`** — forced interleavings never leave a live `User(n>0)` space holding a
    privileged leaf; reset-on-recycle.
17. **`userd_and_notifier_bounded_to_the_usable_heap`** — ✔ `kf-chip` (`tests/usable_heap.rs`) and
    `kf-qemu` (`chan::heap_tests`): inside the carve-out, at a root page, inside a console region, at
    `fb_len−8` or at `2^40`: refused before any host call; the USERD footprint is at least
    `NV_RAMUSERD_CHAN_SIZE`.

★ **Added by the 2026-10-04 review fixes** (each ✔ with its mutation run red, table above):
`the_whole_footprint_stays_inside_its_row` and the footprint-aware checker (`check_full`: whole
footprints, the rows a virtual side resolved through, all six tiers), `a_hopper_scrub_keeps_the_guests_pattern`,
`a_wide_sem_addr_hi_is_resolved_on_hopper_and_refused_before`, the perimeter's
`a_launch_piece_footprint_must_fit_its_validated_bytes` / `a_semaphore_footprint_must_fit_its_validated_bytes`,
`the_shadow_binds_after_the_segments_split`, `the_shadow_positive_control_moves_every_counter`,
`an_unresolved_operand_is_handed_back_unbound`, the rewritten `the_stale_bind_counter_and_its_positive_control`,
`a_refused_unmap_does_not_leave_the_other_maps_live` (`kf-chan`); `no_tmode_twin_path_maps_a_window`
(replaces the predicate test of §7.12's first half), `a_tmode_twin_reads_vidmem_rows_through_the_store_bound`,
`the_heap_bound_refuses_only_when_strict`, `the_carve_bound_refuses_in_tmode_only`,
`the_rows_commit_log_answers_what_changed_since_a_bind`, `a_walk_waiter_is_registered_once_and_taken`,
the freeing arms of `twin_state_cas` (`kf-qemu`); the strict and count-only arms of every inc-A
refusal test; `the_remap_fields_are_the_class_header`.

## 8. Rollout and increments

Each increment is pushed to `v3-p1p2` with CI green. Before master, each needs the merge bar on a real
GPU at the exact commit, plus its own box test (§R). Box runs are **specified here, not run**.

| inc | what lands | why it lands green |
|---|---|---|
| **A** | on today's tree: the §3.2 refusals by name; `SubDeviceMask` refused; GP extended-base decode with other control opcodes refused; USERD and notifier bounded to the usable heap (S1-43); `PlacedRows` exact at both edges; the census table; count-only counters for GPU-target and CPU-view leaves in `[carve, fb_len)`. ⊘ **Corrected 2026-10-04 (review):** every one of these that changes behaviour is COUNT-ONLY on the default path (`inca[…]` on the status line) and refuses / cuts exactly only when strict (`KF3_INCA_REFUSE=1` or `KF3_TSPACE=1`) — the default path stays byte-for-byte today's until box step 1 | counters on the default path; the strict arm is unit-tested |
| **A2** | GPU-target leaf bound = `carve`, refused | after A's box run shows the counter at 0 on each measured family |
| **B** | T-space built at prewarm and unused; logs `tspace space=… fb=…+… ram=…+… rings=… build_us=…` | nothing uses it |
| **C** | the T-mode rewriter, resolver, IR, bind-at-submit and chunking (pure); the per-tier tables; `tspace_unsafe.rs`; the **shadow** wired on today's path (`KF3_TSHADOW`) | behaviour unchanged. **CENSUS** box runs (one per measured family) produce the census and the shadow counters. |
| **D** | `KF3_TSPACE=1`, default off, read once at realize: T births go to the T-space with the re-laid-out ring; mirrors and spares carry no windows and no rings; mirror record separated; the per-twin state word | flag off by default; the A/B runs show parity |
| **E** | default on for every family: **S1-21 closes**. Refusal of unclassified methods default-on for families with a committed census. The window, ring-slot and spare-window code is deleted. Docs folded. | after the A/B box evidence |
| **F** | the T-space and rings move to client K with the §N split | with §N |

**Box tests** (named, with pass criteria; not run here). Every run records the source revision, kf3
`euid` and `CapEff`, and bit 5 of each birth reply, T births included.

- **CENSUS (per family run on hardware: TU116, GA106, AD106, GB203; inc C).** The 30-arm suite, the
  app matrix and the CUDA ladder run with `KF3_TSHADOW=1`. Pass: the census is dumped and committed;
  `would_refuse = 0`, `resolve_miss = 0` and `unknown_field = 0` on stock drivers; pieces per launch
  ≤ 3; each counter is exercised once by an injected `KF3_NEGCTL_*` control. Inc A's part is
  available now: `KF3_TCENSUS=1` dumps the `TCENSUS` lines; `inca[… heap_out=…]`, `carve_gpu=` and
  `carve_cpu=` are on every status line.
- **T-WINDOW-USER (S1-21).** A guest **user** process places virtual CE reads at the old window bases
  and at a store page that holds a guest page table, plus a guest leaf aimed at a carve-out root page.
  Pass: with `KF3_TSPACE=0` the window reads succeed (the known positive); with `KF3_TSPACE=1` they
  MMU-fault on that twin only, the Translated gates stay green, and other channels are unaffected; the
  carve-out leaf is counted (A) or refused (A2); every user mirror logs `windows=none`, and a negative
  control that maps one must be caught.
- **T-PHYS-CE (S1-20).** A physical-mode CE operand and a privileged host method on a USER twin **and
  on a T channel**, on each family, in both privilege arms. Pass, shipping arm: an MMU/CE error on
  that channel only, an unchanged kayfabe-owned canary, and a GPU reset afterwards.
- **T-RING-TRANSLATED (S1-23; needs a guest kernel module).** Virtual CE destinations, CE releases and
  host releases aimed at T-space ring VAs, window bases and carve-out roots. Pass: each is refused by
  name with its counter moving, or lands only in guest memory the guest itself mapped; the ring
  canaries are unchanged. The same module checks that a CPU-written sysmem semaphore acquired by a T
  channel completes (the §13 cache-attribute check).
- **T-TSPACE-BUILD.** Pass: the `tspace` line precedes the first Translated birth; `fb` and `ram` both
  lie below `RING_REGION_BASE`, and `build_us` is recorded; a forced oversize RAM window is refused by
  name; the three ring maps succeed at big-page size; PBDMA runs from the read-only maps.
- **REGRESSION A/B (`KF3_TSPACE=0` vs `1`, per family).** The 30-arm suite; the app matrix with at
  least one app as an **unprivileged guest user**; the gfx set; `cup3`/`cup8`; the LLM lane. Pass:
  every arm passes in both modes, with `forwarded > 0` per token and Xid 0; every new refusal counter
  is 0 on stock drivers; ring slots in use return to baseline after each process; twin-state refusals
  are 0.
- **PERF A/B.** Recorded: `create_mirror` µs p50/max; `mirrors_reused`; `rpc_held` (raw client
  `--timer`); `--concurrency` wall time; host `nvidia-smi` memory with N CUDA processes; T-birth µs;
  bind ns and pieces per segment. Pass: no regression beyond noise; `create_mirror` and host memory
  are expected to improve.

## 9. Precondition on P0 (`v3-sec-nonpriv`)

- With `CAP_SYS_ADMIN` in the launcher's effective set, every host channel kayfabe births is ADMIN:
  twins **and T channels**. `DENY_PHYSICAL_MODE_CE` and the PRIV PTE bit then do not bound what it
  reaches (S1-20). The re-author never emits a physical launch, but the host boundary under it is
  P0's.
- **The bit-5 birth assert is a stated precondition of every P1 claim.** It refuses a birth unless the
  reply's `PRIVILEGED_CHANNEL` (5:5) is clear and `internalFlags.PRIVILEGE == USER`. It covers
  `birth_channel` for T rings too. T-WINDOW-USER and REGRESSION A/B record bit 5 and `CapEff`.
- **If P0 merges second:** P1 still removes the windows and bounds twin leaves, and holds §1.2 as
  worded. The A.9 claim then reads "holds for window and carve-out reach; the channel-privilege half
  waits on P0". P0's effective-set drop must also reach libcuda's C2/C3 threads before `cuInit`
  (S1-22).

## 10. Merge conflicts and merge order

- **`v3-scratch-bound`** rewrites `kf-qemu/src/mem.rs` (`WindowOps`, `MemCounters`, `MemPlane`
  construction) and `kf-mem/src/cpuwin.rs`. Land P1/P2's `mem.rs` work **after** it merges. Inc A keeps
  out of the PRAMIN plan and `WindowOps` (only a counter in `apply.rs`).
- **`v3-dispsw-exp`** adds display-SW twins across `chan.rs` and the `KERNEL_MAPPING` pin in
  `kf-host/src/lib.rs`. Inc D edits the Translated birth, `Windows`/`SlotWindow` and the pump. The
  overlap is moderate. The pin is a dependency (§2.5).
- **`v3-sec-nonpriv`** edits `kf-host/src/channel.rs` (births, bit 5), `lib.rs` and `device.rs`.
  `alloc_vaspace_bare` and `reserve_va` reuse are additive; the bit-5 assert covers T rings
  automatically.
- **`v3-broker`** touches `kf-host/src/lib.rs` and `device.rs`; overlap with the prewarm build is low.
- **`v3-sec-rawaddr`** is merged. **`v3-viommu`** is design-only on master; the seam in §2.6 is where
  its code slots in.
- `kf-chip/src/hwref` and the new `tspace_unsafe.rs` are clean. Rebase `v3-p1p2` onto master before
  each push.

## 11. Owner decisions (each with a recommendation)

1. **Interim P1 without the T-space?** Windows only in spaces classified `Kernel`, passthrough refused
   in a windowed space, today's rewriter unchanged. Its cost is the RAM-window map (65–906 ms) inside
   a held alloc reply on the single act thread, once per UVM VA space — per CUDA process in serial
   runs. **Recommendation: no.** Go straight to the T-space with shadow-first staging (§8).
2. **Unclassified methods on a family without a committed census:** count-only or fail-closed.
   **Recommendation:** fail-closed on every family once its census is committed; on the source-only
   families make the census part of each family's first hardware bring-up. Window removal (S1-21) is
   unconditional on every family either way.
3. **Carve-out bounds.** T-space translation and USERD/notifier: refuse now. GPU-target leaves:
   count-only for one A/B run per family, then refuse (A2). BAR1/BAR2/PRAMIN CPU views: count-only,
   decided later on that evidence. **Recommendation: as stated.**
4. **`PlacedRows` as the Translated plane's translation (§56 rule 2).** **Recommendation: confirm.**
   The alternative is asking host RM at decode time, which puts host calls on every trigger.
5. **Kernel-only spaces without host maps.** **Recommendation: keep the maps for now.** They back the
   pump's CPU reads.
6. **Cross-channel stale bind:** ship the counter count-only; if it is ever non-zero, a host acquire
   becomes a bind barrier.
7. **The guest-RAM window's reach in the T-space** (§Q/6c: allowed). All guest RAM, never VMM memory,
   narrowable only with the vIOMMU (§2.6). **Recommendation: keep.**
8. ★ (2026-10-04 review) **Inc A on the default path.** It is count-only now. After box step 1 shows
   `counted=0 heap_out=0 rows_inexact=0` on each measured family (and a Hopper or Blackwell run for
   the extended base), make strict the default? **Recommendation: yes** — the refusals name methods
   stock never emits; the heap bound protects kayfabe's roots (S1-43); exact rows are what the
   T-space resolver relies on.
9. ★ **Carve-out leaves in a guest-KERNEL space under T-mode** are counted, not refused (no channel
   runs there; CeUtils' alias lives there). **Recommendation: confirm**; revisit if `carve_kernel=`
   is ever non-zero on a stock driver.
10. ★ **The default path's Hopper+ scrub** zero-fills where the guest asked for a pattern (pre-existing,
    `translated.rs`); T-mode now keeps the pattern. Fixing the default path changes its words.
    **Recommendation:** fix it with inc A's flip (decision 8), after a Hopper/Blackwell box run.
11. ★ **The four destructive positive controls** (`KF3_NEGCTL_TWIN`, `_TSPACE_OVERSIZE`,
    `_TWIN_WINDOW`, `_STALE_BIND`) refuse or reintroduce the defect by design, so each needs a run of
    its own. **Recommendation: accept** — a counter that has only ever reported zero has not shown it
    can report one.
12. ★ **P0 scoping.** Until `v3-sec-nonpriv` merges, a T-space run's gate verdict reads "PASS, SCOPED
    (window and carve-out reach only)". **Recommendation:** merge P0 first, or accept scoped passes for
    inc D's A/B and re-run the A/B after P0.

## 12. Doc corrections (each folded above the text it corrects, dated)

- **`THE_TRANSLATED_PLANE.md` §12** (ruling `8ecad812`, 2026-09-22): supersession recorded **in §12's
  own text** (2026-10-04): windows live only in the per-VM T-space; UVM's mixed apertures are handled
  by resolving virtual operands through kayfabe's own placement rows, not by a window in every VAS.
  The 2026-09-22 ruling assumed the rewriter would forward virtual operands, so the windows had to sit
  beside the guest rows; under §Q the rewriter authors every address.
- **`THE_TRANSLATED_PLANE.md` §24.2**: "runs verbatim in a host VA space that mirrors the guest's
  kernel VA space" and "the windows are mapped with `GROWS_DOWN`" are superseded.
- New **`THE_TRANSLATED_PLANE.md` §28 "The T-space"**, and a new **`THE_CONSTRAINTS`** section holding
  §1.2 and its named tests.
- **`translated.rs` module doc**: corrected to "authored from the per-tier tables" with T-mode; the
  `SubDeviceMask` raw push is gone from inc A.
- **`V3_SECURITY_MODEL.md`** R1.1, R2.2, R2.3 and the barrier table: P1 closes the window reach, P2
  the ring reach, and the GPU-target `carve` bound the guest-page-table carve-out reach.
- **`mem.rs`** `create_mirror` comment: it understates that a failed window map refuses every birth.
  The comment is removed with the code.

## 13. UNVERIFIED (each with its check)

- **The RAM window's GPU cache attribute.** Today a virtual sysmem operand goes through a mirror row
  carrying the guest leaf's `volatile` (`MapPerm`). Through the window it gets the window's attribute.
  The window is mapped GPU-uncached (`NVOS46_FLAGS_GPU_CACHEABLE_NO`) so that a CPU-written sysmem
  semaphore is never served from L2. Check: the T-RING-TRANSLATED module's CPU-written semaphore
  acquire, and the PERF A/B.
- RAM-window build time at production RAM sizes. T-TSPACE-BUILD records `build_us`.
- RM placing both windows bottom-up below `RING_REGION_BASE` on every host driver and family. Seen
  only on GA106/580; the per-family A/B reads the bases back.
- PBDMA fetching from read-only pushbuffer and GPFIFO maps; big-page FIXED maps of the re-laid ring.
  Checked by T-TSPACE-BUILD.
- Hopper/Blackwell `SET_PB_SEGMENT_EXTENDED_BASE` being 0 today. Fixed regardless in inc A.
- Whether CeUtils `VIRTUAL_MODE` is ever on in our guest, and whether the FB alias range includes the
  carve-out. Checked by the CENSUS shadow and the inc-A counters.
- Whether GSP validates a `CLEAR_FAULTED` handle (closed firmware). Refused regardless.
- Cross-channel stale bind. Checked by the counter and its positive control.
- The dup of the store and RAM object into client K. Checked at inc F.
- That stock drivers never emit, per family: block-linear or multi-line virtual operands, monitored
  fences, render enable, `SubDeviceMask`, or a `LAUNCH_DMA` field outside the table. Checked by the
  CENSUS shadow.

## 14. Review trail

- **2026-10-03 review (folded):** census-gated per-family tables (§3.6); the headline test asserts
  emitted `(subchannel, method)` pairs, with negative controls and a non-vacuous paired case
  (§7.1–7.2); P0 stated as a precondition (§9); one ordered per-twin state (§4.2); carve-out bound
  (§2.1, §4.3); stale-bind counter (§3.5); no lazy build (§2.3); declared USERD size (§7.17).
- **2026-10-04 security review (all 17 journaled findings folded):** bottom-up windows and a reserved
  ring region; every word authored; the complete trigger set; `SET_MEMORY_SCRUB_PARAMETERS`,
  `SET_SECURE_COPY_MODE`, `PREFETCH` and `REQ_ATTR` classified, with a field-value census;
  `SubDeviceMask` refused; GP control entries; `PlacedRows` exactness; the §R perimeter; piece bounds;
  type and client isolation; the per-twin CAS state; ring layout by construction; the USERD bound in
  the usable heap; the carve-out claim scoped; the vIOMMU seam as one function; citations corrected;
  staging, answered in §11 decision 1.
- **Its unaccepted second pass, folded where it held:** the GPU-target `carve` bound, now count-first;
  the `SubDeviceMask` classifier route; ring page size (the re-layout replaces it); reset-on-recycle;
  the walker precondition; the `clcab5.h` method count.
- **2026-10-04, later — two implementation reviews of incs A–D (folded as row R; every finding fixed
  with a test that can fail and a mutation run red, none rejected):** HIGH — the T-mode CPU store
  bound taken from a twin's window length (0); the carve-out refusal OFF under the flag; inc A
  changing the default path (both reviewers). MEDIUM — the perimeter not holding the footprint and
  the checker seeing 4 bytes (both); the twin paths tested on a predicate and logged by literal; the
  P0 precondition ungated; the box plan not runnable (retire lines at free only, missing positive
  controls, `USER-BIRTHS` always required, unnamed probes); the stale-bind counter's false positives
  and the unresolved-operand death; the shadow binding past an invalidate; the Hopper+ scrub dropping
  the pattern. LOW — the release-twin result ignored and the free's statement order; the sysmem
  USERD seam; `HostRing::release` short-circuiting; the store window's page rounding; Gate A
  exempting `kf-chan`; opcode 4 accepted below Hopper; the property test on one tier; the docs
  stating more than holds (§28, §58, R2.3). The slow suite (`KAYFABE_SLOW=1`, nightly or on
  dispatch only) was dispatched on `v3-p1p2` at `96be1212`: GitHub Actions run `37171087282`, every
  job green, `slow` included. Push runs of the fix commits: `37168857856` (`0b59f6da`),
  `37169033136` (`b802fd2e`), `37169607054` (`d53befca`), `37170100865` (`49786008`), `37170515751`
  (`5d840021`), `37170855345` (`6e6694a2`), `37171075343` (`96be1212`) — all green.
- **2026-10-04 feasibility review (this revision):** the 40-bit host-semaphore limit applies to
  **every** family through CeUtils' legacy `SEMAPHOREA`; S1-21 decoupled from the census;
  shadow-first staging; `LAUNCH_DMA` field completeness; output chunking against the half-pushbuffer
  limit; `PlacedRows` exactness at **both** edges; one ring object and one CPU view; the RAM-window
  cache attribute; count-first carve bounds; per-VM ring-slot leakage; the cost of the interim P1; T
  channels inside the P0 precondition and the T-PHYS-CE arm; the vIOMMU drain for T channels; the §56
  confirmation; the hardware coverage per family; merge order after `v3-scratch-bound`.
