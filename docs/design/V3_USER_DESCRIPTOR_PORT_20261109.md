**STATUS: LIVE, 2026-11-09. CODE READ hostile-guest review of the UR1 Phase 1 port; GPU-free verification is recorded below. Phase 2 remains DESIGN-ONLY.**

# USER descriptor admission on next

CODE READ: authority is [the 2026-10-22 decision](V3_USER_DESCRIPTOR_DECISION_20261022.md).
This port separates C56F format admission from CPU copying. Pointer forwards the captured
8-byte descriptor unchanged, without resolving its PB. Scan and Window retain the
917,504-byte snapshot/staging ceiling and stop before a read or gate above it.
No Pointer descriptor is split, no limit is raised and admission creates no completion.

## Evidence and limits

MEASURED BY THE COORDINATOR, 2026-11-08, not rerun by UR1: `/root/night/iter2/NOTES.md`
and `/root/night/iter2/evidence/` record AD102, kf-iter1, `8d495e90` versus `5fcf9458`
(the latter is `8d495e90` plus `cf79374e3`, with mechanical test/doc conflict resolution).
The two next runs fail catia-07 subtests 03 and 06 with Budget refusals around 1.02 MB
and 0.98 MB against 917,504 bytes, then RM unloads and guest 153/141/193 events.
Both candidate runs pass all 6 subtests with zero refusals, RM unloads, guest events
and host Xids. Each candidate run records 4,799 Pointer entries above the old bound,
maximum 1,042,740 bytes, and zero `refused_scan_budget`.
These local evidence paths are operator records, not inputs required to build or test.

INFERRED: these runs support the intervention on that die/configuration; they do not
qualify this final port. One die, one RAM/FB configuration and one driver combination
were exercised. UR1 runs no hardware, VM, enscape, SPEC stage or Windows gate lane.
The historical [Phase 1 proof](../../traces/descriptor_bound_20261018/phase1.md) names
its own revision and measurements; the new checks below do not inherit its green status.

## Conflict and test decisions

CODE READ: the historical `run3440.rs` fixture is restored because both real Storage
and production Relay tests consume its twelve public interface words. It holds no PB
bytes or captured per-die constants. Its content is an adversarial input corpus, never
an admission rule. The old `V3_USER_DESCRIPTOR_BOUND_20261022.md` stays absent: it
recommended continuation/complete-from-birth approaches explicitly rejected by the
authoritative decision. Its analysis is preserved in git at `468a35c9d`, already marked
superseded there. Restoring it as a current design would misstate the selected contract.
The existing next mock's native reference observation is retained.

CODE READ: the reconciliation retains format validity, tested/stored equality, bounded
work, mapping/source guards and real-credit assertions:

- Storage's over-copy-bound refusal cases become hardware end-address refusal cases;
  larger legal Pointer words are checked for exact storage and unchanged GET/reference.
- The lifecycle fixture decoder identifies format-valid entries independently of copy size.
  Otherwise it would mislabel a legal large word as a torn capture.
- Property bit-flips check every one of 384 words against the hardware ceiling: 30 invalid,
  354 valid. Random hostile Pointer words must still pass format validation.
- Stress checks require hardware format for every publication and additionally retain the
  snapshot bound for scanned publications. Tested/stored equality remains mandatory.
- The long small-entry I/O golden omits only the three new observation fields; every old
  counter, action and state stays in its hash. The old golden value is retained.

## Hostile-guest size/count inventory

CODE READ: this table covers the production USER relay paths affected by admission,
including diagnostic reads and authored emissions. GPU length is bounded by format;
CPU read/emit bounds remain independent. A guest-reachable unbounded CPU read or emit
would be a security bug. **No such path was found in this review.** This is source
review and bounded GPU-free testing, not a proof of GPU fault/reset containment.

| Guest-controlled input / operation | Read, copy, stage, forward or emit site | Bound and retained authority |
|---|---|---|
| Declared GP base/count | `userpump::user_ring_serve`, `userread::mapped` | Power of two, 2..2^20 entries; 8 MiB ring; 8-byte alignment; checked end strictly below 2^40; at most 2,049 placement/RAM pieces; topology generation and row identity re-admitted. Whole ring mapping validation does not copy the ring. |
| GP_PUT and cursor/slot | `user_turn::put_step`, `PumpIo::gp_put/load_dword` | PUT < admitted entries; cursor wraps by entries-1; checked slot*8+base; exactly 4 USERD bytes and one owned 8-byte GP snapshot, with both dwords from that snapshot. |
| GP descriptor length/base/control | `user_stream::inspect_format`, `Relay::plan_step` | Direct C56F supported bits, maximum (2^21-1)*4 = 8,388,604 bytes, checked end < 2^40; only supported CONTROL NOP encodings. Malformed entries stop without publication. |
| Pointer PB length | `Plan::Forward`, `PumpIo::publish_entry`, `Storage::write_user_gp` | No PB read, copy, stage or allocation by length; one intact 8-byte word; format rechecked in Storage; slot < 512; private Fetch ring credit retained. USER privilege, deny-physical and own restricted VAS remain the GPU access boundary (§AI). |
| Scan/Window PB length | `Relay::plan_step` Scan arm | `max_scan_bytes` = 917,504; checked before gate or `read_pb`, including Pointer-to-Window transitions. Oversize refusal grants no GET or native receipt. |
| PB snapshot bytes/pieces | `PumpIo::read_pb`, `snapshot_with`, `userread::snapshot/resolve` | 1..917,504 bytes; at most 225 placement pieces and RAM pieces; checked additions and backing lengths; allocation failure refuses; row identity/epoch and birth RAM generation checked before copying and again at publication. CPU views validate offsets inside the unsafe perimeter. |
| VRAM CPU-view preparation/copy/cache | `userstore::required`, `Cache::prepare/copy` | Required request <= 225 spans / 917,504 bytes / 16 windows of 64 KiB; checked backing end. Operator keep defaults to 16 and is clamped to 256 views (16 MiB); own LRU eviction and retained cleanup errors prevent replacement beyond the quota. Copy rechecks bytes <= MAX_BYTES; Pointer length never grows the cache. |
| Owned scan words and patched copy | `PumpIo::stage`, `stage_plan`, `user_scan::Scan::segment` | At most 229,376 u32s from bounded snapshot; patched copy has identical length. Trigger replacement fills validated slices, never expands them. Packet count is a public method-header field and consumption is constrained by supplied words; carried state contains metadata only. |
| Trigger walk / suffix positions | `stage_plan`, `segment_turn`, `suffix_after_trigger`, `gapless_remainder` | Checked slice positions and checked multiplication; trigger offsets advance within the same bounded snapshot. A partial unsupported suffix refuses. END stops interpretation; no descriptor splitting is introduced. |
| Staged bytes and authored GP spans | `Storage::write_authored/write_gp_level`, `segment_turn` | Checked offset+bytes <= GP_OFFSET = 917,504; 4-byte alignment; authored GP nonempty and slot < 512. Entire patched copy must fit before any representing span is published. |
| Native GP publication/count | `Fetch::new/available/publish`, `Storage::publish_put` | Private ring power of two, 4..512; spare slot retained; Pointer/scanned forwarding reserves marker credit, trigger prefix reserves 2 or 3 slots. Each emission is one bounded word/span/barrier/marker; no guest-count loop emits an unchecked number of entries. |
| Native GET / REFERENCE and guest GET | `Fetch::observe`, `Marks`, `PumpIo::hw_words/write_guest_get` | Native indices/counts validated against issued ring credit and tags. Receipt attribution, not admission or a timer, drives writeback; one 4-byte guest GET store under channel/allocation guards. |
| Software trigger data | `segment_turn`, `software`, registry and source admission | Channel-owned software identity and registered bundle lookup; host action authored from validated verbs. Raw PB data never grants a host address or privileged verb. Trigger count <= bounded snapshot words; execution waits for real ordered witness. |
| Shadow observation | `shadow_step`, `inspect_entry`, `usershadow::charge/decode_words` | Separate diagnostic path retains `inspect_entry(..., MAX_BYTES)`, 917,504-byte snapshots, 4 MiB per-channel observation charge, bounded samples; opaque shadow publication has its existing own-USER eligibility and slot guards. |
| Trace/re-read sizes | `trace_segment`, `trace_rereads`, snapshot/trace helpers | CODE READ [src]: `userpump.rs` rereads/refusal dumps use bounded snapshots. TRACE_HISTORY=64, TRACE_WORDS=16,384, history <= 262,144 words (1 MiB), TRACE_STRADDLES=16 with 3 rereads each, TRACE_SLOTS=1,024. VERIFY requires <=1,024-word segments and verified+queued <40,000 before adding 2. Refusal output caps header chain at 512, nearby GP loads at 24 and repeats at 3; source/history word dumps use fixed heads. Pointer publication records a bounded slot string, not a PB read. |
| Number/lifetime of relay allocations | `NativeQuota::reserve/attach/release_confirmed`, `VmCaps` | Runlist budget charged before native allocation; uncertain attempts retain charge; token retirement alone grants no release. Each Storage remains 1 MiB; guest length cannot grow it. |
| Observation counters | `Lifecycle::entry`, `Counts::since`, `LifecycleStats::add` | No authority or admission decisions. Large count is additive; maximum length is aggregated with fetch_max, never summed; refusal count cannot forge progress. |

## Preregistered checks

INFERRED H1: the global budget is the reason legal large Pointer words fail on next.
Falsifier: the imported expected-red tests pass unchanged next, or the port still refuses
those words before publication. Test results are recorded below, including failures.

INFERRED H2: the reported `user_loop_model_v2` SIGSEGV predates this port.
Falsifier: unchanged origin/next passes and only the port crashes under the same command
and build configuration. Each source variant uses its own target directory.

INFERRED H3: changing admission does not alter small-entry decisions or I/O.
Falsifier: the retained legacy I/O golden changes, or existing safety/model tests fail
for a reason other than the explicitly reconciled admission contract.

## Verification

MEASURED, UR1 local run 2026-11-09 (original product sources at `115a0c39d`):
`cargo test -p kf-chan --test user_loop_model_v2 -j2`, with `KAYFABE_NO_KVM=1`,
passes 53 tests, fails 0, and retains 1 existing ignore (900.50 seconds, debug).
The matching `--release` command also passes 53 / 0 / 1 (288.76 seconds).
No SIGSEGV was reproduced in either baseline profile. The existing ignore concerns the independent native-doorbell
read-back decision; this port does not change or hide it.

MEASURED, UR1 local run 2026-11-09 (same original product sources plus the new probes):
the expected-red Relay suite has 28 passes / 17 failures; the library has 226 passes /
1 failure. All 224 original library tests pass. The new failing Storage boundary
probe and Relay failures expose the global budget and missing mode-scoped observations.
H1's expected-red falsifier was not met. Logs remain under `/root/codex_lanes/UR1/logs/`.

MEASURED, UR1 local run 2026-11-09 at `4c4568594`: the targeted debug run passes
231 library, 46 property, 4 stress and 47 Relay tests (328 total, no failures).
The legacy small-entry I/O golden is unchanged. Direct action transcripts from the old
and ported Relay match exactly for default and always-scan modes.

MEASURED, UR1 local run 2026-11-09 at `4c4568594`: all 15 production source mutants
are killed by both required named tests. The separate unmutated mutation baseline
passes 278 tests. Compile errors do not count as kills; sources are restored after
each mutation, and every variant has a fresh target directory. Reproduce with:

```sh
nice -n 19 flock /tmp/claude-0/kf-local-cargo.lock python3 scripts/descriptor_mutation_check.py OUTDIR TARGET_ROOT
```

| Mutant | Failed tests | Two required named killers |
|---|---:|---|
| `pointer_admission_keeps_snapshot_budget` | 18 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `a_large_captured_descriptor_is_not_reloaded_after_guest_mutation` |
| `storage_keeps_snapshot_budget` | 4 | `user_storage::tests::large_user_descriptors_cross_snapshot_bound_without_changing_real_credit_fields`; `user_storage::tests::write_user_gp_stores_each_run3440_descriptor` |
| `pointer_format_check_bypassed` | 44 | `an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published`; `pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit` |
| `storage_format_check_removed` | 4 | `user_storage::tests::bad_user_descriptor_cannot_modify_private_ring_or_userd`; `user_storage::tests::shadow_writer_preserves_opaque_bits_but_cannot_publish_or_forge_progress` |
| `scan_budget_removed` | 5 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| `scan_bound_exclusive` | 2 | `scan_copy_bound_is_inclusive_and_checked_before_io`; `window_copy_bound_is_inclusive_and_checked_before_io` |
| `large_pointer_attempts_pb_read` | 9 | `pointer_forwards_the_largest_representable_descriptor`; `pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer` |
| `pointer_large_counter_dropped` | 4 | `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum`; `pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer` |
| `pointer_max_counter_dropped` | 5 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum` |
| `vm_pointer_maximum_is_summed` | 2 | `large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum`; `pointer_maximum_is_not_summed_on_repeated_delta_flushes` |
| `scan_refusal_counter_dropped` | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| `scan_budget_event_dropped` | 2 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| `large_counter_stops_relay` | 11 | `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`; `a_large_captured_descriptor_is_not_reloaded_after_guest_mutation` |
| `end_at_2_40_admitted` | 6 | `pointer_end_address_boundary_is_checked_without_copying_or_credit_on_refusal`; `user_storage::tests::write_user_gp_refuses_end_at_2_40` |
| `scan_budget_after_gate` | 5 | `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate`; `seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` |
| `legacy_core_skip_validation` | 3 | `an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published`; `pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit` |

MEASURED, UR1 local run 2026-11-09 (product source at `4c4568594`): the existing
`scripts/lifecycle_mutation_check.sh` `core_skip_validation` selector is also aligned
with `inspect_format`; Pointer validation no longer consults a copy budget. Its
`bash -n` check and `ONLY=core_skip_validation DRYRUN=1` application/restoration pass.
Executing that selector's exact replacement in a fresh target against the library
and Relay suites yields 3 failed format-refusal tests, including both named killers
in the table. This is the 16th checked variant; the new descriptor harness still has
15 variants. No production crate source remains mutated.


MEASURED, UR1 local run 2026-11-09 at `4c4568594`: the complete GPU-free pair command
Cargo reports 1,191 passed, 0 failed and 4 existing ignores across 38 groups: `kf-chan`
724 passed / 4 ignored; `kf-qemu` 467 passed / 0 ignored. No new ignore was added.

```sh
CARGO_TARGET_DIR=/workspace/UR1-target/port-release CARGO_BUILD_JOBS=2 KAYFABE_NO_KVM=1 \
  nice -n 19 flock /tmp/claude-0/kf-local-cargo.lock \
  cargo test --locked --release -p kf-chan -p kf-qemu --no-fail-fast -j2
```

MEASURED, UR1 local run 2026-11-09 at `4c4568594`: within that full run,
`user_lifecycle_model` passes 63 / 0 / 3 (including its successful lazy child replay),
`user_lifecycle_props` 46 / 0 / 0 and `user_lifecycle_stress` 4 / 0 / 0.
`user_loop_model_v2` passes 53 / 0 / 1 in release, without SIGSEGV. The four existing
ignores remain the independent Pointer rebind, continued-growth indices, unannounced
Pointer credit, and native-doorbell read-back questions named in their test annotations.
Three KVM-gated test slots return early under `KAYFABE_NO_KVM=1` (explicit
`KVM-GATE: SKIPPED` records); their KVM assertions did not run. They are included in
Cargo's pass count, separately from the four ignores. This is not a VM or hardware result.

MEASURED, UR1 local run 2026-11-09 at `4c4568594`: pair/dependency Clippy succeeds
with 250 existing warning sites and zero new sites under `scripts/ci/debt.py clippy`;
this is selected-crate coverage, not a claim that unrelated workspace warning sites
were resolved. `cargo fmt --all --check` succeeds. No warning suppression or debt
baseline change was added.

```sh
CARGO_TARGET_DIR=/workspace/UR1-target/port-clippy CARGO_BUILD_JOBS=2 KAYFABE_NO_KVM=1 \
  nice -n 19 flock /tmp/claude-0/kf-local-cargo.lock \
  cargo clippy --locked -p kf-chan -p kf-qemu --all-targets --message-format=json -j2 > clippy.jsonl
python3 scripts/ci/debt.py clippy --log clippy.jsonl
nice -n 19 flock /tmp/claude-0/kf-local-cargo.lock cargo fmt --all --check
```

MEASURED, UR1 local run 2026-11-09 at `4c4568594`: the matching debug command
`cargo test --locked -p kf-chan --test user_loop_model_v2 -j2` passes 53 / 0 / 1
in 769.51 seconds, in its own fresh target directory. Next at `115a0c39d` and the
port therefore both pass this target in debug and release. H2 remains unclassified:
the operator's historical SIGSEGV was not reproduced, so no minimized SIGSEGV repro
is available and this report does not label it pre-existing or newly fixed.

MEASURED, UR1 local runs 2026-11-09: H1's and H3's falsifiers were not met by the
GPU-free corpus. This does not qualify the final port on hardware. All Cargo builds
used niceness 19, two jobs and the existing global flock; `/` stayed above 6 GiB.
Baseline/port, debug/release and each mutation use separate target directories.

Final hygiene commands (the PR body and lane report record their outcomes):

```sh
python3 scripts/ci/doc_freshness.py
python3 scripts/ci/release_claims.py validate
python3 scripts/ci/debt.py claims
python3 scripts/ci/no_private_refs.py
python3 scripts/ci/no_gpu_uuids.py
```
