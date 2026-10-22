# C1 cleanup sweep evidence

STATUS: RESEARCH, 2026-10-22.

CODE READ: scope is documentation and static dead-code discovery at master
`e64b8e5f6`, with no guest, GPU or production behavior changes performed.
The lane records proposals for protected files instead of editing them.

INFERRED H1 (registered before inspection): some missing opening statuses can
be restored mechanically. FALSIFIER: no explicit allowed status and date in
that document or its history. Such cases stay in the judgement queue.

INFERRED H2 (registered before inspection): some cleanup candidates have no
callers. FALSIFIER: any manifest dependency, CI/script/doc caller, baseline
entry, external entry point or evidence/history role. A compiler warning alone
cannot prove that a public API or optional path is dead.

INFERRED H3 (registered before inspection): some status claims cite missing or
superseded evidence. FALSIFIER: the cited path resolves at the audited revision
and the claimed revision still describes the stated current behavior. Historical
measurements remain historical evidence even after descendants exist.

CODE READ: archive/ and traces/ are evidence and will not be deleted. The README,
owner rulings, handoff index, and paths changed by PRs #66–#76 are protected.


MEASURED: tests-only commit `b521f74f0` was pushed before implementing the checker.
The expected-red run executed 23 tests: 21 failures, 2 passes. With the local
implementation, all 23 checker tests passed; the existing CI Python discovery
plus these tests passed 62/62 in 24.747 seconds.

MEASURED STOP: mutation `skip-opening-order` replaced the ordering guard with
`if False`. It produced exactly one failure, in
`test_later_status_cannot_cover_opening_prose` (23 run, 1 failure, 22 passes).
This is below the lane requirement of at least two failing tests. The mutation
runner refused the step; its finally block restored the implementation.
No green implementation commit or source deletion followed that refusal.

CODE READ: the uncommitted implementation's raw scan found 30 mechanical
findings across 128 master design documents. `doc_status_master.json` is that
provisional inventory, not a claim that all 30 require document edits. In
particular the parser needs tests for legitimate YAML front matter before the
inventory is accepted. Correction placement and implied supersession remain
semantic review tasks; the checker does not claim to validate them.

CODE READ: the tests are picked up by the existing `scripts/ci` unittest discovery.
The CI workflow itself was not edited (it is protected by open PRs).

INFERRED recommendation: resume only after the owner permits addressing the
failed verification gate; add independent opening-order cases, prove the
YAML-front-matter case red, then rerun mutation checks without lowering the
required kill count. Dead-code checks, document corrections, stale-claim
validation and index proposals remain unfinished.
