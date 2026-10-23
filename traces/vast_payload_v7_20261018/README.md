# V7 thin payload acceptance

[CODE READ: scripts/ci/vast_payload.py] This branch starts at canonical PR #104
(e63af44dd), rather than master or the Worker branch. The two parked parser/deadline
EXPECTED RED commits were cherry-picked as c4f13c6c6 and fb6669d96. New strict-contract,
stage-attribution and finished-prefix probes were committed separately before their fixes.

[MEASURED: 2026-10-23, run v7-payload-tests-20261023; tests.json] The initial reusable/strict suite had 55 import
errors before the adapter existed. Two attribution probes and two first-red prefix probes
then failed before their fixes. The final GPU-free payload suite passes 79/79.
[MEASURED: run v7-payload-mutants-20261023; mutants.json] All 23 mutations have at least two
independently named killers. Black, shellcheck and the payload-owned 100-column check pass.

[CODE READ: scripts/ci/vast_payload.py, scripts/ci/vast_payload_hooks.sh] The adapter delegates
provision_full, host CUDA ladder, merge_check and fat-guest CUDA ladder in that order. It
adds no build/suite implementation and no Worker code. The existing helpers fetch only
from a local origin with the approved SHA pinned as a local branch. The merge log must bind
HEAD to that SHA; process exit alone never supplies a PASS. Every required provision RC,
READY/EXIT, nonzero crate-test count, 9/9 gates plus gate10 PASS/FALLBACK, current kf3 build,
raw 30/30, thin 30/30 and birth census is parsed. CUDA needs all four unique rungs and the
matching revision. The PR #104 file is /workspace/kf-result.json with verdict/suites/hardware.
A first red stops later work; a deadline preserves completed rows and appends timeout FAIL.

[CODE READ: scripts/ci/vast_payload.py xid_ok] Xid admission is one replaceable function.
This is the owner-approved first version: a single raw --missing-page-fault arm, exactly one
victim_release marker, VA 0xa11000000, Xid 31, FAULT_PDE, VIRT_WRITE and CE-family client.
No defer/executor, per-die or permanent exemptions exist. Per-arm instrumentation records
only that admitted delta; each whole-stage delta must equal its recorded arm deltas exactly.
Unreadable/lost-baseline instruments, other arms, extra records, other VA/type/client refuse.
The marker does not expose its channel identity, so this first version scopes by arm and VA;
S4's exact-channel guard replaces xid_ok when available, rather than inventing a channel table.

[CODE READ: V7_PAYLOAD.md acceptance contract] The real production-mode KVM acceptance is
PENDING. It must use the unchanged checkpoint Worker bundle, vm:true, ssh_direct, the check
profile and production bootstrap/env contract. One rental only, $4 total cap and stop at $3,
an independent reaper before creation, exact-ID/label verification and final API absence.
No deployment or committed renting enablement is part of this lane.
