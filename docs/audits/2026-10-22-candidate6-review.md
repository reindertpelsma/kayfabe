# Independent review of prod-candidate-6 (code diff only)

**STATUS: ANSWERED, 2026-10-22. Read-only review of `origin/integration/prod-candidate-6-20261022` at
`f7cf3ed81`. No product code was changed. No GPU, no hardware, no host 172.22.0.7.**

Scope: `git diff origin/master...origin/integration/prod-candidate-6-20261022 -- crates scripts qemu`
(17 files, +1195/-96; `scripts` and `qemu` are untouched). Three merged topics:

1. Descriptor Phase 1 (PR #76): `kf-chan` `user_stream`, `user_turn`, `user_storage`, `user_lifecycle`;
   `kf-qemu` `userpump.rs` (a rename and one match arm).
2. SA-02 (PR #73): `kf-rm` `chanlink.rs`.
3. SA-03/VA-2 (PR #74): `kf-mem` `vasmgr.rs`.

Tags: **CODE READ** = read in the source at `f7cf3ed81`. **MEASURED** = a command I ran for this review.
**INFERRED** = reasoning a test or a hardware run still has to confirm.

## Verdict

**No blocker found. Merge-ready for the three topics as scoped**, with two IMPORTANT follow-ups (F1, F2) that
are not regressions against master and one claim I could not verify (H1, the hardware evidence). Details and
the test results are below.

| # | Severity | Where | One line |
|---|---|---|---|
| F1 | important | `kf-qemu/src/mem.rs:1721-1726`, `twin.rs:59-63` | SA-02 is incomplete: a mirror (VA space) is still classed KERNEL from the same forgeable client-handle range |
| F2 | important | `kf-mem/src/vasmgr.rs` final-clear loop (`changed between chunks`) | a root move anywhere inside a multi-chunk batch leaves the invalidate armed, and an armed request makes `walk_busy` true VM-wide |
| F3 | minor | `vasmgr.rs` `on_walk_ready` chunk continuation | later chunks are walked with the `(slot, root)` captured at `pump`; a space torn down between chunks is not revalidated before submit, and no test covers it |
| F4 | minor | `vasmgr.rs` `submit_batch` called from `on_walk_ready` | a failed continuation submit returns without `pump`, so wants that arrived meanwhile sit in `pending` until the next event |
| F5 | minor | `kf-chan/tests/user_turn_unit.rs` | `small_descriptor_action_transcript` asserts nothing about the transcript (it prints); the "before/after agree" claim is an out-of-repo diff |
| F6 | minor | `kf-mem/tests/secaudit_all_pdb_over_64_spaces.rs` | the final assert is a disjunction (`covered >= 70 \|\| unreconciled == 0`) and passes after one chunk because `poll` never answers; the real proof is `audit_all_pdb_chunks.rs` |
| F7 | minor (doc) | `docs/design/V3_USER_DESCRIPTOR_DECISION_20261022.md` line 3 | STATUS still says "Nothing here is implemented. Product code is unchanged" |
| H1 | not verifiable | n/a | the hardware evidence quoted for this review (catia-07 subtest 03, max 1,043,244 bytes) is not in the branch; the branch's own `traces/descriptor_bound_20261018/phase1.md` says "Hardware n=0" |

## (a) Does it work for the right reason

* No test-input special-casing, no `cfg(test)` or chaos hook that changes a decision. The diff adds no
  `cfg(test)` branch in product code and no new `lifecycle-mutants` case. CODE READ.
* The Phase 1 rule is generic: `plan_step` calls `inspect_format` (hardware format only) and the
  `Plan::Scan` arm compares `d.len_bytes` with `io.max_scan_bytes()`. The production value is
  `userread::MAX_BYTES` = `MAX_SOURCE_BYTES` = `GP_OFFSET` = 917,504, the same constant master passed as
  `max_entry_bytes`. CODE READ (`user_turn.rs:681,727-737`, `userpump.rs:707`).
* Assertions compared with master. Removed: the oversized-refusal assertions in `user_storage` (the
  behaviour change itself), the `refused > 50 && forwarded > 50` bound (replaced by an exact
  `(30, 354)` that I re-derived by hand: 4 direct bases x 3 unsupported bits + 2 NOP bases x 9 illegal
  high bits = 30 invalid out of 384; 354 valid), and the "internal range is kernel" assertion (the SA-02
  behaviour change itself). Nothing else was weakened. The format-refusal tests
  (`pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit`,
  `write_user_gp_refuses_end_at_2_40`) are new and test refusals that remain.
* The a4 oracle `inspect_entry(raw, 8_388_604)` calls the function under test. It is circular, but the
  exact `(30, 354)` count is an independent hand-derived check, so the pair is sound.
* The golden fingerprint (`user_lifecycle_props.rs`) drops the three new Debug fields by string filter.
  That keeps the old golden valid and relies on separate counter tests for the new fields
  (`large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum`,
  `pointer_maximum_is_not_summed_on_repeated_delta_flushes`). Acceptable.
* F5: `small_descriptor_action_transcript` prints `SMALL_ACTIONS` and asserts only `!dead`, `cursor == 8`,
  `written_get == 8`. The statement in `phase1.md` that transcripts agree before and after is therefore
  not reproducible from the repository. Suggested fix: pin the transcript as a literal and assert equality.
* F6: see the table. Suggested fix: assert `covered == 64` after the first chunk and drive the second.
* `phase1.md` records 13 source mutations with two independent killers each. I did not rerun them.
* The VA tests (`audit_all_pdb_chunks.rs`) are real: they drive a recording walker through
  `on_invalidate` and `on_walk_ready` and assert the number of maps (64 then 70/128), that the trigger is
  still armed after chunk 1, that a pending target holds the final clear, that a refusal in the first or
  last chunk is not forgotten, that reset discards unsubmitted chunks, and that a root move after chunk 1
  blocks a stale clear.

## (b) Hostile guest with Pointer entries up to 8,388,604 bytes

* **Pointer mode resolves nothing.** `Plan::Forward` -> `publish_guest` -> `publish_entry` calls
  `publish_guarded(.., pb = None, ..)` and `Storage::write_user_gp`, which now calls `inspect_format`.
  No pushbuffer read, no staging, no allocation sized by the length. The only product-code use of
  `len_bytes` on that path is the observation counters (`user_lifecycle.rs:933-943`: a compare, a
  `saturating_add` and a `max`). CODE READ.
* **Memory the entry names.** Master did not validate the GPU-VA mapping of a Pointer entry either
  (`pb = None`); the authority is the channel's own restricted VAS (native fault on an unmapped or
  unowned page, contained to that channel). The change only removes a size ceiling in front of that
  unchanged behaviour. The 2^40 end check (`inspect_format`) is intact and tested at the boundary.
* **GPU time.** A Pointer entry is now up to 9.1x larger than the old ceiling, and a guest can aim many
  slots at the same region. The cost is the guest's own channel time, bounded by the hardware timeslice
  and the existing TDR path, as for any native workload. The relay does O(1) work per entry
  (`a_ring_of_32768_maximum_pointer_descriptors_has_constant_work_and_no_pb_reads`). INFERRED for the
  GPU-side amplification; no hardware.
* **Stuck relay and starvation.** Credits count entries, not bytes (`Fetch`), and the marker/GET path is
  length independent (`a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`,
  simulated native observations, not GPU execution). A large SYNC_WAIT Pointer entry sets
  `outstanding` exactly as a small one does.
* **Single scan site.** `read_pb` has one call site (`user_turn.rs` `Phase::PbRead`), entered only from
  `Plan::Scan`, and the budget check sits in that arm before the gate. `userread::snapshot` still
  refuses `bytes > MAX_BYTES`, and `write_authored`/`write_gp_level` still refuse `end > GP_OFFSET`. So
  removing the size check from `write_user_gp` leaves two independent layers behind the Scan check.
* **Checked arithmetic.** The new code uses `saturating_add`, `max`, comparison and `format!`. `kf-mem`
  denies `arithmetic_side_effects`/`indexing_slicing` and the new `vasmgr.rs` lines use `saturating_add`,
  `nth`, `split_off`, `take`. No new unbounded loop or allocation reachable by the guest in the
  descriptor path. CODE READ.
* The diagnostic shadow path (`userpump.rs:2082`) keeps `inspect_entry(.., MAX_BYTES)` and so keeps its
  Budget classification, which is intended and tested
  (`diagnostic_shadow_inspection_keeps_its_snapshot_budget`).
* Self-DoS is unchanged: a Scan/Window channel that submits an entry above 917,504 bytes is still
  refused and killed, now by the new name `USER_RING_SCAN_BUDGET` (counted, before any read or gate).
  This only reaches the guest's own channel.

## (c) SA-02: the privilege stamp

* `kernel_channel` is now `privilege.is_some_and(is_kernel)`; the handle is ignored. `is_rm_internal_client`
  is narrowed to `0xC1E0_0000..=0xC1EF_FFFF` and is documented as not a witness. CODE READ
  (`chanlink.rs:2390-2412`).
* The stamp is decoded from the alloc payload that the guest CPU-RM wrote after clearing and recomputing
  `internalFlags` from the call's security context (`kf_abi::notifier::ChannelPrivilege` doc, CODE READ of
  the cited ogkm lines is by the earlier team). The classification is a pure function of the same owned
  RPC payload (`ChannelPolicy::respond` -> `ChannelAlloc`), so there is no stamp/classification race and
  no second read. A missing stamp, level 3, or ADMIN is not kernel.
* The stamp is guest-kernel produced. A hostile guest ROOT who replaces the guest kernel can forge it; that
  is outside the userspace/kernel boundary this rule defends (OWNER_RULINGS treat ADMIN userspace as user).
* **Regression check, MEASURED (grep of committed traces):** 793 birth lines carry `rm-internal`, and
  every one of them is `PRIVILEGE=KERNEL`; none shows `privilege=N rm-internal` or `undecoded rm-internal`.
  So no known RM-internal channel loses the Translated route.
* **F1 (important): another route still trusts the handle.** `kf-qemu/src/mem.rs:1721-1726`
  `kernel_vas_for` starts a mirror in the KERNEL state from `is_rm_internal_client(client)`, and
  `twin.rs:59-63` still documents it as "a handle range no guest process can hold". The SA-02 premise
  (the shipped driver accepts caller-chosen handles) makes that false. Consequence, CODE READ: a guest
  userspace process that chooses a client in `0xC1E0xxxxx` gets a KERNEL-classed mirror, so privileged
  leaves are mirrored for it. It cannot run a user channel there (`chan.rs:6869` `try_user` refuses in
  KERNEL, by name) and it cannot be Translated (that needs the stamp), so I found no escalation: the
  state word keeps its invariant "no privileged leaf in a space a user channel can run in". But the
  classification is the same forgeable fact SA-02 removes, the comments are now wrong, and the only
  evidence that privileged leaves live in RM-internal spaces is a 2026-10-10 note. Suggested fix: keep it
  (RM-internal spaces need the privileged leaves from creation) but document it as a fail-safe-only
  heuristic, fix `twin.rs:59-61` and `mem.rs:1718-1719`, and add a test that a forged-handle USER birth
  is refused rather than run (the existing `only_a_guest_kernel_channel_is_privileged` does not cover
  the mirror path). Alternatively derive the initial state from the first stamped Translated birth.
* `chan.rs:716` (`kernel_by`) uses the range only for a log suffix. No other consumer found by grep.
* `Privileged::of(a.kernel_client, a.user_work)` (`tspace.rs:65`) is unchanged and is now fed only by the
  stamp. The Passthrough side `!a.kernel_client || a.user_work` therefore sends every unstamped channel to
  Passthrough, which is the safe direction.
* The new tests (`audit_kernel_channel_stamp.rs`) go through the production `ChannelPolicy::respond`
  statement path with a userspace-pid root and a chosen client handle, and assert the emitted
  `kernel_client`. Good. `stamped_internal_scrubber_channels_remain_kernel_channels` uses stamps
  `0x16/0x1a/0x82`; its evidence note is CODE READ of committed run65 and CHAN-FACTS.

## (d) The batched walk

* **Ordering and atomicity.** All wants stay on the batch; `split_chunk` keeps 64 entries and parks the rest
  in `remaining`; each chunk's verdict is acked before the next submit (commit-on-ack unchanged); the
  final clear loop runs only when `remaining` is empty, over the original `wants`, with `failed`,
  `absent_only`, `rewalk`, `partial` carried across chunks. An invalidate therefore cannot clear before
  every object has been walked and applied, and the asynchronous `settle_keys` deferral still applies.
  The tests cover 70 and 128 spaces, a pending target, a refusal in the first and last chunk, and a new
  invalidate between chunks. CODE READ + test source read.
* **Work is bounded**: chunks <= walker slots / 64 (default 128 slots), and every submit is non-blocking.
  No new lock; everything is on the single VA thread. Lock ordering is unchanged.
* **Reset.** `abandon_life` on a chunked in-flight batch clears `remaining` and moves the original wants to
  `pending`, which are then counted as abandoned. The submitted chunk may still land and apply to its
  spaces, but cannot clear or complete anything. Sound; conservative also when the in-flight chunk is the
  last one. Tested both ways.
* **F2 (important): root move between chunks.** After the last chunk, any key in `finished` whose current
  root differs from the root it was walked with is added to `failed` and `rewalk`, and the invalidate is
  left armed. Master had the same outcome for a root that moved during one walk, but the window is now the
  whole multi-chunk batch (and the check also fires for a key applied in chunk 1 long before the clear).
  An armed request keeps `walk_busy` true (`device.rs:2392`, `port.armed_request().is_some()`), and every
  USER relay ends its turn on `walk_busy`. So a busy guest with more than 64 spaces and a PDB change
  mid-batch can stall all USER rings until the guest re-issues the invalidate. Not a regression (before
  this PR the same batch was refused and stayed armed for ever), but it is the VA-4 shape. Suggested fix:
  treat a between-chunks root move like `again`, i.e. requeue the invalidate want (the moved key has a
  `Want::Root` queued already) instead of failing it; test: `moved_after_first` should expect the
  invalidate to complete after the re-walk.
* **F3 (minor): teardown mid-batch.** CODE READ: a space removed between `pump` and a later chunk is still
  submitted with its stale `(slot, root)`; `flush_released` resets the slot first, the walk runs, and the
  apply loop skips the key (`spaces.get` is `None`, or `space.slot != Some(slot)` after re-registration),
  leaving the run codes at FAILED, so nothing is committed or applied to a wrong space. Harmless but a
  wasted GPU walk, and it is untested. Suggested fix: drop or refresh `remaining` entries against the table
  immediately before `submit_batch`; test: remove a space after chunk 1.
* **F4 (minor): failed continuation submit.** `on_walk_ready` returns after `submit_batch` without the
  final `self.pump(trigger)`; on `Err` the batch's invalidates are counted unreconciled but are not in
  `out.unreconciled`, and wants that queued during the batch are not pumped until the next event. A
  walker submit error is rare. Suggested fix: call `self.pump(trigger)` when `inflight` is `None` after
  the continuation.
* The `MAX_SPACES_PER_WALK` refusal is gone (VA-2). `refuse_batch` remains for the empty-entries and submit
  error cases.

## (e) Regression risk for Heaven D3D11, Minecraft, Steel Nomad

* Descriptor: the new code differs from master only for an entry that master refused (`len_bytes > 917,504`)
  in Pointer or NOP form, or for the order of the refusal in Scan/Window (now after `Lifecycle::entry`).
  For `len_bytes <= 917,504` the path is identical, apart from one extra call to `io.max_scan_bytes()`
  (a constant) on Scan plans. CODE READ.
* A channel that previously died on a Budget refusal and now lives is a behaviour change by design.
* Residual risk (known, Phase 2): Window and Scan mode still kill a channel on an entry above 917,504 bytes.
  `Mode::Window` is entered whenever a software object is registered (`Pointer` + `Registration`), so an
  application that registers one and then submits an oversized descriptor still dies, now with
  `USER_RING_SCAN_BUDGET mode=Window`. The new counters `refused_scan_budget` and the mode in the message
  measure it.
* SA-02: no change for channels that carry the KERNEL stamp. MEASURED above for 793 RM-internal births.
  A channel that has an internal handle but no stamp now goes Passthrough, and `try_user` then refuses it
  in a KERNEL-classed mirror (F1); no such channel appears in the committed traces.
* VA batch: changes behaviour only when a batch names more than 64 objects (formerly refused and left armed).
  Single-chunk batches take the old path: `split_chunk` is a no-op and `chunked` stays false.
* Not run by me: Heaven, Minecraft and Steel Nomad logs (no hardware, per scope). The branch's recipe
  (`rg 'USER_RING_(GP_FORMAT Budget|SCAN_BUDGET|STAGED_SPAN_BUDGET)'` on the kept logs) is the right check
  and remains owed.

## (f) Clippy and unsafe rules

* No `unsafe`, no `#[allow]`, and no new `unwrap`/`expect` in non-test code in the diff (grep of the added
  lines). `unsafe` rules are untouched: none of the touched files is a `*_unsafe.rs` file.
* `phase1.md` records identical baseline and final clippy output (379 warnings each, zero added sites).
  My own clippy run is in the next section.

## Tests I ran (GPU-free)

All MEASURED, debug build, `nice -n 19 cargo ... -j2` under `/tmp/claude-0/kf-local-cargo.lock`, own
`CARGO_TARGET_DIR` (deleted afterwards), on `f7cf3ed81`.

| Command (package, target) | Result |
|---|---|
| `kf-mem --test audit_all_pdb_chunks` (SA-03/VA-2) | 12 passed |
| `kf-mem --test secaudit_all_pdb_over_64_spaces` | 1 passed (weak assertion, F6) |
| `kf-mem --lib vasmgr` | 39 passed |
| `kf-rm --test audit_kernel_channel_stamp` (SA-02) | 5 passed |
| `kf-rm --test secaudit_internal_client_range` | 1 passed |
| `kf-rm --lib chanlink` | 36 passed |
| `kf-chan --test user_turn_unit` (descriptor Phase 1) | 38 passed |
| `kf-chan --test user_lifecycle_props` (bit flips, golden) | 38 passed |
| `kf-chan --test user_lifecycle_stress` | 4 passed |
| `kf-chan --lib user_storage` | 20 passed |
| `kf-chan --lib user_lifecycle` | 42 passed |
| `kf-qemu --lib` `cargo check`, and `--lib userpump` | builds; 29 passed |
| `cargo clippy -p kf-chan -p kf-mem -p kf-rm --lib` | exit 0 (warnings exist, e.g. 16 in `kf-chan`; I did not diff them against master, `phase1.md` records 379 vs 379) |

0 failures. `moved_after_first` (MEASURED, passes) pins the F2 behaviour: after a root move following chunk 1
the invalidate is not completed and the trigger stays armed. I did not rerun the mutation table or the full
`kf-*` suite (another lane held the cargo lock for the full suite).

## Hardware claim (H1)

The review request quotes a hardware result (catia-07 subtest 03 survived large descriptors, max 1,043,244
bytes, zero TDR/Budget refusals). That evidence is not in `f7cf3ed81`, and
`origin/codex/qualify-cand6-20261022` (Q1 harness safety stop) does not contain it either. The branch's own
record says hardware n=0 and "no performance or application-survival claim". I did not verify the claim; the
code read supports it only for Pointer mode.

## Suggested follow-ups (none blocks the merge)

1. F2: requeue, do not fail, an invalidate whose key moved root between chunks.
2. F1: correct the `kernel_vas_for`/`TwinState::for_kernel` comments and add the forged-handle mirror test.
3. F3/F4: revalidate `remaining` before each chunk submit; `pump` after a failed continuation.
4. F5/F6/F7: pin the transcript, tighten the secaudit assertion, refresh the decision doc STATUS.
