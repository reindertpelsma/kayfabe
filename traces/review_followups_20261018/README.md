# Candidate-6 review followups F1–F7

STATUS: GPU-FREE VALIDATION, 2026-10-22. Hardware: K1, n=0.

CODE READ: review source `7fbd4bc2f`, candidate-6 base `f7cf3ed81`,
master comparison `e64b8e5f6`. Branch `codex/review-followups-20261022`;
[draft PR #86](https://github.com/reindertpelsma/kayfabe/pull/86) targets master,
so its GitHub diff includes candidate-6 ancestry. Followup-only diff:
`f7cf3ed81...codex/review-followups-20261022`.
Tests-only EXPECTED RED: `cfd000817`; runtime/docs fix: `097ee4569`.
The [falsifiers](falsifiers.txt) were recorded before further inspection/runs.

| Finding | Disposition and evidence |
|---|---|
| F1 | FIXED, CODE READ / MEASURED: all mirrors start Unclassified, including forged internal-looking handles. The existing Translated statement requires the SA-02 RM KERNEL stamp before `try_kernel`. Three mirror tests cover missing/user stamps and a positive ordinary-client KERNEL stamp. Related comments corrected. |
| F2 | FIXED, CODE READ / MEASURED: a moved root requeues the original invalidate, preserving its age and requiring a full retry before clear. Two multi-chunk root-move tests require eventual completion; unrelated hard failures and withdrawn roots still hold the clear. |
| F3 | FIXED, CODE READ / MEASURED: revalidate remaining entries before each continuation; exclude retired/replaced objects and refresh moved roots. Tests include both chunk edges, actual slot reuse and an entirely retired remainder. |
| F4 | FIXED, CODE READ / MEASURED: pump pending work after continuation submission; a failed submit no longer strands a queued root walk or newer invalidate. |
| F5 | FIXED, MEASURED: assert the literal small-descriptor action transcript in both modes; retained the existing diagnostic ScanBound filter. The missing-GET-write mutant kills this test. |
| F6 | FIXED, MEASURED: drive reports for 64 then 6 entries and require all 70 reconciliations, final completion and trigger clear. The missing-final-completion mutant kills the strengthened audit test. |
| F7 | FIXED, CODE READ: decision STATUS says Phase 1 implemented; hardware refers to K1; Phase 2 remains design only. |

MEASURED: no assertions loosened. Existing root-move checks retain the no-early-clear
assertions and now also require eventual completion. No unsafe, lint/baseline policy,
flags, timeouts, constants, launchers or worker routes changed.

## Tests

MEASURED on candidate-6 plus the tests-only commit, before the product fix:

| Target | Before | After fix |
|---|---|---|
| qemu review_mirror_privilege | 1 passed, 2 assertion failures | 3 passed |
| mem audit_all_pdb_chunks | 12 passed, 10 assertion failures | 22 passed |
| mem secaudit_all_pdb_over_64_spaces | 1 passed | 1 passed |
| chan small_descriptor_action_transcript | 1 passed | 1 passed |

MEASURED: all twelve RED failures were assertion panics from real test runs;
none was a compiler failure or missing target. Four-crate GPU-free suite:
**2014 passed, 0 failed, 7 existing ignores**, including doc tests.
Command: `cargo test -j2 --release --locked -p kf-mem -p kf-chan -p kf-qemu
-p kf-rm --no-fail-fast`, with `KAYFABE_NO_KVM=1`.
Restored-code final focused checks: **64 passed, 0 failed** (23 VA, 38 descriptor
turn, 3 mirror tests).

MEASURED: every Cargo command ran under the shared global flock with a 3600-second
wait, `nice -n 19`, two build jobs and the lane's own target directory. Shared-lock
contention was waited out. No hardware or VM was used.

## Mutation proof

MEASURED: each hand mutation was applied alone and restored byte-for-byte. Every
kill below is a distinct named test with an assertion failure; no setup failure
counts. Full killer names: [mutants.json](mutants.json).

| Mutant | Broken behavior | Distinct killers |
|---|---|---:|
| M1 | Restore handle-range mirror privilege | 2 |
| M2 | Fail moved-root invalidates instead of retrying | 2 |
| M3 | Omit continuation object revalidation | 6 |
| M4 | Keep a continuation's superseded root | 2 |
| M5 | Omit pump after continuation submission | 2 |
| M6 | Omit guest GET writes while updating local written_get | 3 |
| M7 | Omit final invalidate completion | 13 |

MEASURED: Clippy `--release --locked -j2 --all-targets` for kf-mem/kf-chan/kf-qemu
against both master and candidate-6: **265 source-bound fingerprints before/after,
0 added**; 393 warning messages before/after; successful exits. The existing
kf-qemu deny set is unchanged, including arithmetic_side_effects/indexing_slicing.
The unsafe-count pipeline's four Python unit tests also passed.

## Hardware boundary and retained limitations

INFERRED: these GPU-free proofs do not establish actual GPU directory/ack ordering,
RM-internal boot compatibility after withholding privileged leaves until stamped
birth, or app/performance/TDR behavior. K1 must validate the exact reviewed revision
on Linux/Windows, including >64-space root changes/teardown and the requested
Heaven/Minecraft/Steel Nomad/SPECviewperf-catia workloads. Hardware n=0, no spread or
performance result. Descriptor Phase 2 remains out of scope.

CODE READ: root withdrawal is distinct from a move to another valid root. It still
withholds completion rather than guessing that old mappings were retired. A hard
continuation submission failure still names the failed original obligation; F4
provides progress for independent pending work, not a fabricated completion.
