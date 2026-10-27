# V7 thin payload acceptance

[CODE READ: scripts/ci/vast_payload.py] This branch starts at canonical PR #104
(e63af44dd), rather than master or the Worker branch. The two parked parser/deadline
EXPECTED RED commits were cherry-picked as c4f13c6c6 and fb6669d96. New strict-contract,
stage-attribution and finished-prefix probes were committed separately before their fixes.

[MEASURED: 2026-10-23, run v7-all-vast-tests-20261023; all_tests.json] Final local checks pass
198/198: 83 payload, 72 canonical box, 17 provider environment and 26 diagnostic probes. The
initial payload implementation had 55 import errors, followed by two attribution and two
finished-prefix red probes. Two Cargo-consumer and two failed-hardware probes then reproduced
live findings before their separate fixes. Black, shellcheck and the 100-column check pass.
[MEASURED: 2026-10-23, run v7-payload-mutants-20261023; mutants.json,
provider_env_mutants.json, diagnostic_mutants.json] All 45 new mutations (27 payload, eight
provider environment, ten diagnostic) have at least two independently named killers.
The 60 canonical mutations are also killed under PR #104's separate rule; see
canonical_box_mutants.txt. Format-only commit 828afa829 has unchanged ASTs in format_ast.json.

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

[MEASURED: 2026-10-23, run v7-kvm-20261023; live_run.json, cleanup.json] One production KVM
rental completed with a runner-validated FAIL/provision. Estimated upper spend was 1.442036 USD
under the 4 USD cap. A real direct interactive SSH login changed GRACE to MANUAL. Owner stop
and independent API recheck confirmed exact ID/label absence; local processes and the reaper
stopped. No deployment or production renting enablement occurred. The Worker bundle is unchanged.

[CODE READ: vast_payload.py, vast_report.py] The live source was immutable 28084b96c. The new
provider-env loader, corrected Cargo target path and failed-provision hardware refresh have
local test/mutation coverage; they have not had a fresh VM run. Full host CUDA, merge_check and
fat CUDA results remain unverified. Hosted OIDC, billed Cloudflare CPU, survival past the old
grace deadline and S4's exact-channel guard remain gaps. Format pass 2, Worker README restructure
and review lows remain deferred. [Live report](LIVE_RUN.md), [startup fix](PROVIDER_ENV.md),
[diagnostic exception](DIAGNOSTIC_LOGS.md), [final log](public_log_final.txt).

[MEASURED: 2026-10-23, runs 39895761923 and 39895766836; hosted_ci.json] The new hosted
box-script job passes on code/workflow revision f5dfacda3 for both push and PR events: all 198
tests and all four mutation harnesses, including the 60 canonical mutations. The general Rust
workflow is separate and was still running when this evidence was captured.
