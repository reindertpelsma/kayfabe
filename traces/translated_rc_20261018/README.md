**STATUS: LIVE, 2026-10-22 — resumed implementation; GPU-free validation complete, hardware pending.**

CODE READ (commit 007ba1eb0:3): quoted historical statement, evidence not revalidated by BW9. MEASURED: owner-authorized merge `82effe99f` incorporates master `e64b8e5f6`, including PR #63.
The former 93 denied Clippy errors are cleared. Final changed-crate Clippy succeeds: 264 existing
warning sites before/after, added 0, errors 0 (`clippy_compare.json`). No lint suppression or debt update.

CODE READ: `1166dc3d4` adds the private host error context/event, one-shot observation, correct guest
chid/engine mapping, bounded notifier publication, dead-slot gate, pending retries and posting permit.
`9de8ea637` is a second tests-only EXPECTED RED (12 pass / 2 fail) for RAM byte-copy publication;
`8f7049e84` makes those 14/14 green by splitting metadata/status. `d8c1e65ef` adds the barrier-controlled
concurrent retirement check. Runtime code changes stay in RC handling and the narrow birth/serve/free hooks.

CODE READ (commit 007ba1eb0:13): quoted historical statement, evidence not revalidated by BW9. MEASURED: core target 25/25; hook target 15/15. Full 22-crate suite: 3999 passed / 0 failed / 22 ignored / 234 targets
(`final_tests.json`, `all_crate_results.json`, `ignored_tests.txt`).
The post-master full changed-crate baseline was 1070 pass / 14 intentional RC fail / 4 ignored
(`resumed_baselines.json`). No new ignored test was added.

CODE READ (commit 007ba1eb0:18): quoted historical statement, evidence not revalidated by BW9. MEASURED: 15 distinct deliberate mutants, 16 runs, all killed by at least two tests. The RAM generation
variant was rerun after the status split. `mutations.json` lists every killer and source revision;
`mutation_restores.json` proves restoration after every run group.

| CODE READ (commit 007ba1eb0:22): quoted historical statement, evidence not revalidated by BW9.  Mutant variant | MEASURED killers |
|---|---:|
| `observer_noop` | 18 |
| `partial_status_accepted` | 4 |
| `one_shot_latch_ignored` | 4 |
| `pending_error_lost_on_write_refusal` | 3 |
| `host_engine_replaces_guest_engine` | 3 |
| `host_token_replaces_guest_chid` | 3 |
| `native_birth_notifier_unarmed` | 2 |
| `status_published_first` | 2 |
| `metadata_read_before_status` | 2 |
| `retired_post_allowed` | 2 |
| `firmware_heap_gate_removed` | 2 |
| `ram_generation_refreshed_on_write` | 2 |
| `ram_final_word_bulk_copy` | 2 |
| `ram_status_before_engine` | 2 |
| `posting_permit_dropped_before_queue_write` | 2 |

CODE READ: source authority is the PRIVATE host record, not a guest notifier or fd edge. Sysmem writes
use the existing generation-pinned, nonblocking RAM authority. FB destinations are checked against the
usable heap, including crossing/overflow cases. The posting permit remains held through the queue write;
contention retries and retirement prevents later reuse from receiving stale events.

INFERRED native hypothesis/falsifier: the public unprivileged error-context/event path should expose a
real RC on every ring. A real native RC with no private record falsifies it. Hardware n=0 here; no
Blackwell ring attribution, faulting-method, reset recovery or Windows acceptance claim is made.

CODE READ: no host/GPU/VM work, no new boot flag or timer changes. Every cargo build used the required
bounded shared flock, B1 target, nice 19, release and j2. Disk stayed above the guard. The initial cargo
lock wait was bounded and respected; no holder was disturbed. Cleanup is recorded in the final report.

**The following original checkpoint is preserved as historical evidence; its stop is resolved.**

**Historical first stop, 2026-10-22 (superseded by the resume above).**

CODE READ (commit 007ba1eb0:57): quoted historical statement, evidence not revalidated by BW9. MEASURED: seed merge `3c7bf3e9d5e6c8999fcc982af787b66ed334df2e` contains the two required
origin sources. `915c1b1de` adds a design and explicit no-op delivery declarations only.
The tests-only EXPECTED RED commit is `31ea54a98c75a1102fdc09f057c75bf54f0d69f5`.

| Check | MEASURED result | Artifact |
|---|---|---|
| Inherited `kf-chan --test translated_ring_rc` | Compiled; 1 pass / 3 fail / 0 ignored | `baseline.log` |
| Extended same target | Compiled; 2 pass / 14 fail / 0 ignored | `expected_red.log` |
| Baseline `clippy -p kf-chan -p kf-qemu --all-targets` | Exit 101; 93 denied errors, six unchanged channel/relay files | `clippy_baseline_errors.json` |
| Fix / green suite / mutants / after-clippy | NOT RUN: implementation stopped at the baseline safety gate | This file |
| GPU/VM/native tests | NOT RUN (n=0; GPU-free lane) | This file |

CODE READ (commit 007ba1eb0:69): quoted historical statement, evidence not revalidated by BW9. CODE READ: every cargo command used --release -j2, nice -n 19, the B1-only target and the
required bounded flock. MEASURED: disk free before builds was 28/27/27 GiB, above guard.
Command family (arguments after cargo):
`test -p kf-chan --test translated_ring_rc --release -j2`, and
`clippy -p kf-chan -p kf-qemu --all-targets --release -j2 --message-format=json`.
The two test logs omit dependency-build noise and normalize only the checkout prefix.

MEASURED: Clippy reports 66 arithmetic, 20 indexing and 7 slicing errors. Primary file counts:
`userpump.rs` 51, `userrelay.rs` 3, `userstrand.rs` 4, `usershadow.rs` 9, `chan.rs` 10,
`userread.rs` 16. The messages and primary locations are retained. All six Git blobs exactly
match the required continuation base (`unchanged_baseline.json`); the deny declarations were
not changed. The final clippy delta has NOT been measured; no added-0 or green claim is made.

CODE READ: the lane says STOP when a safety check blocks a step, and forbids relay edits
beyond the narrow hook. Repairing these existing channel/relay lint failures exceeds that hook;
relaxing/suppressing them would bypass the gate. No fix was attempted after that conflict.

Owner options:
1. Recommended: have a separately scoped lane repair the denied relay baseline, then resume
   B1 from this EXPECTED RED checkpoint and run the complete validation/mutation sequence.
2. Explicitly expand B1 to include that baseline repair, with its own tests and mutants.

CODE READ: public-source unprivileged error-context/event proof and the proposed private
record → guest notifier → RC_TRIGGERED path are in `docs/design/V3_TRANSLATED_RC_20261022.md`.
No missing public verb was found for observation. RESET_CHANNEL behavior, native delivery,
ring identity, guest recovery, busy-slot retries and lifetime handling remain UNIMPLEMENTED
or UNVERIFIED. The delivery declarations are stubs, not a working product feature.

CODE READ (commit 007ba1eb0:97): quoted historical statement, evidence not revalidated by BW9. MEASURED final scope: all 22 `kf-*` packages in the workspace were tested with --release -j2
and --no-fail-fast. Exact package/argument list is `final_test_command.json`. The final Clippy
command matches the post-master baseline, --all-targets, and records zero added sites. Formatting
checks pass for every changed Rust source. No behavior flag or new ignored test was introduced.

CODE READ (commit 007ba1eb0:102): quoted historical statement, evidence not revalidated by BW9. MEASURED cleanup at closure: all cargo/mutation processes exited, their shared-lock invocations
released, and the B1-only target directory was deleted using the owner's authorized find/rmdir
method. No host/GPU lock was acquired and no external VM/GPU job was started. The worktree retains
source/evidence only. Runtime source at validation is `d8c1e65ef`; subsequent changes are docs/evidence.
