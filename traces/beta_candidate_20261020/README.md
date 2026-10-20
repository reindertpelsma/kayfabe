# C6 Part A checkpoint

STATUS: PARTIAL / STOPPED, 2026-10-20. BETA CANDIDATE; no supported external-user
install path. This supersedes the earlier guest-OS decision stop.

## 1. Lane, branch and request

MEASURED: C6, branch `integration/beta-candidate-1-20261020`, candidate-5 base
`f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`. Stage 1 ledger publication
`3c4731eec682f62f9e9d55e5dde00d3ded393fd3` remains intact. New test-only
checkpoints: caps EXPECTED RED `b1ea24127`; profile/audit EXPECTED RED
`a4a9db447`. The external lane report records this enclosing report's exact
post-push head and origin equality, avoiding a self-referential commit hash.

CODE READ: request is GPU-free Stages 1–4, tests first, push after each stage,
with hardware only in a coordinator slot. The coordinator approved explicit
`guest-os=windows|linux`, default Linux, then a separately reviewed sweep-derived
caps replacement. Those choices are resolved; no GPU/VM slot was granted.

## 2. Result (eleven lines)

- DONE — MEASURED: Stage 1 [topics](TOPICS.md) and [conflicts](CONFLICTS.md) published first.
- DONE — CODE READ: retain candidate-5; queued F-2, CE2, L5 and nonblocking heartbeat remain unimported; exclusions remain as recorded in the ledger.
- DONE — MEASURED: all 22 baseline crates reproduce candidate-5 exactly: 3620 passed / 0 failed / 19 ignored; [per-crate counts](BASELINE_COUNTS.tsv).
- PARTIAL — MEASURED: separate caps EXPECTED RED commit reproduces missing module/API/data; scoped uncommitted implementation passed 20 tests / 0 failed / 2 inherited diagnostic emitters ignored.
- PARTIAL — MEASURED: effective-config EXPECTED RED table fails to compile against the absent profile API; source-wiring checks fail 3 tests / 1 missing-module error; audit tests lack their module.
- STOPPED — MEASURED: exact-die mutant killed by 4 tests, but C6's wrapper rejected Cargo's expected test-failure diagnostic as a compiler error; [receipt](mutations.json).
- DONE — MEASURED: wrapper restored exact-die matching; no mutant remains applied. No detector change or further implementation after rejection.
- NOT DONE — MEASURED: property wiring, complete flag inventory/KEEP audit, launcher flag removal, final profile/caps verification and remaining selected topic imports.
- NOT DONE — MEASURED: Stages 3–4 release layout/version/install/uninstall/doctor/docs/launcher, complete checks and CPU-only host build.
- NOT DONE — MEASURED: Stage 5 hardware, Stage 6 README/PR work; no minimal working QEMU line or install claim established.
- DONE — MEASURED: no host contact, GPU/VM action, rental, other-session message, master merge or non-lane push; hardware n=0.

## 3. Tests, mutations and flag counts

| MEASURED check | Before | After / limit |
|---|---|---|
| All 22 crates | Candidate-5 3620/0/19 | C6 baseline 3620/0/19, every crate equal; [summary/hash](baseline.json); full changed-tree run NOT RUN |
| Scoped new caps integration + example | Missing module/API/data, compile RED | 20 pass / 0 fail / 2 ignored on uncommitted scope; not a full-crate result |
| Effective-config table | Missing profile API, compile RED | 8 written tests; runtime execution NOT RUN |
| Source wiring / flag audit | 3 fail + 1 error / missing module | No green claim |
| Exact-die mutant | Scoped passing implementation | 4 killing tests; wrapper rejected run; mutation campaign NOT accepted |
| Branch / measured word / layer / provenance / tile mutants | NOT RUN | NOT RUN; full strict native-oracle comparison also NOT RUN |
| Hardware | n=0 | n=0; spread N/A |

MEASURED: [check summaries and hashes](checks.json) identify commands and local
checkpoint revisions. Caps implementation was uncommitted; its check is not a
claim about a published product revision. Source review and limitations are in
[CAPS_REVIEW.md](CAPS_REVIEW.md). The source sweep's strict AD104/native comparison
was RED by 100 words (CODE READ); C6 has not rerun that strict comparison.

CODE READ: [partial flag policy](../../docs/design/V3_FLAG_POLICY_BETA_20261020.md)
records P19. Published product flag changes: 19 selectors unchanged, hardwired 0,
deleted 0; every-reader before/after totals and final KEEP count NOT DONE.
Recommended migration preserves the 18 functional settings behind the Windows
property and replaces captured CAPS_PROBE with derived caps; idle-only ablation
passes do not establish loaded redundancy. TSG mirror remains absent/OFF;
lifecycle/lazy-observe defaults remain unchanged; timer mapping is already
automatic. These are not a completed optional/zero-cost diagnostic audit.

## 4. Decision for the coordinator

1. INFERRED recommendation: authorize correcting the local mutation-wrapper
   classifier, rerun the entire mutation campaign, then resume approved Stages
   2–4. Keep the two OS/caps decisions already approved. Alternative: leave this
   partial checkpoint stopped; do not promote the uncommitted caps import.

CODE READ reason for the pause: the lane's AGENTS.md explicitly says, “If a
step is blocked or refused by your own safety checks, STOP and report; do not
reword or work around it.” The wrapper condition matched `^error(?:\[|:)`,
which includes Cargo's expected `error: test failed` after four test failures.
MEASURED: no Rust compilation error occurred in that mutant run. The recommended
correction must still require at least two named failing tests, reject compiler
errors, restore source and rerun the full campaign. It has not been applied.

## 5. Unverified and preserved work

MEASURED: the scoped caps source/data/helpers/example and an additional tile
assertion remain uncommitted in the lane worktree, with a lane-local snapshot.
They are not accepted product content and were not pushed. The published
branch contains deliberately RED new tests, not an implemented Windows profile.
No final changed-tree regression, clippy/fmt/broker/sanitizer/shell/docs checks
or host binary was produced. Stage 5 readiness is NOT claimed.

CODE READ: before external support, outstanding items include packaging,
signing, CI, licence/provenance review, security review of installation,
telemetry/log hygiene, exact-revision hardware/aging tests, support matrix and
clean-directory install acceptance. No new rootless or GPU/driver support
combination is asserted.

## 6. Cleanup

MEASURED: no C6 build/test process remains; the own cargo target directory was
deleted. The local build lock was released as its cargo processes exited; no
host lock was acquired. No foreign process, target, branch or worktree was
modified. The lane worktree and small text evidence remain for resumption.
