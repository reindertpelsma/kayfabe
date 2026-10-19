# C6 Part A: Stage 1 published; stopped at the guest-OS decision

STATUS: PARTIAL / STOPPED, 2026-10-19. BETA CANDIDATE assembly only; no
supported install path for external users is offered.

## 1. Lane, publication and request

MEASURED: lane C6, branch `integration/beta-candidate-1-20261020`, cut from
origin candidate-5 `f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`.
Stage 1 ledger commit `3c4731eec682f62f9e9d55e5dde00d3ded393fd3` was pushed
and verified equal to the remote branch head before the Stage 2 audit.
The enclosing report commit follows it; resolve that publication with:

```sh
git log -1 --format=%H -- traces/beta_candidate_20261020/README.md
git ls-remote --heads origin integration/beta-candidate-1-20261020
```

CODE READ: request was GPU-free Stages 1–4, ledger first, push each completed
stage, and STOP before implementation if Stage 2 item 2 guest-OS selection
could not be derived confidently. Hardware requires a coordinator slot.
The final external C6 report and session reply carry the exact enclosing
commit and its post-push origin verification, avoiding a self-referential SHA.

## 2. Result (nine lines)

- DONE — MEASURED: Stage 1 [topic ledger](TOPICS.md) and [conflict ledger](CONFLICTS.md) committed and pushed before flag-path analysis.
- DONE — CODE READ: retain candidate-5; queue isolated F-2, CE2, two L5 fixes and approved nonblocking heartbeat for independent verification; **none imported yet**.
- DONE — CODE READ: exclude mirror, trap experiments/R1 runtime audit without production-equivalence proof, incomplete K1/H1; defer full sweep, its dependent fixture and prepopulation fix.
- PARTIAL — CODE READ: [flag policy/decision](../../docs/design/V3_FLAG_POLICY_BETA_20261020.md) records exact P19 inputs, source anchors, defaults, falsifier and concrete options; complete every-reader inventory is NOT DONE.
- STOPPED — CODE READ / INFERRED: display construction precedes fn1 identity; fn72's provisional-layout hint is explicitly diagnostic. Automatic all-P19 selection is not established.
- DONE — MEASURED: zero product changes; `git diff --exit-code f8f9d1a5f -- crates qemu broker scripts` passed at the stop.
- NOT DONE — MEASURED: no EXPECTED RED/config implementation, flag removal, topic integration or effective no-env profile test.
- NOT DONE — MEASURED: Stages 3–4 install/launcher/doctor/version/support-matrix work, full validation and host binary build; hardware/README overhaul/master PR not started.
- DONE — MEASURED: no host contact, VM/GPU operation, guest-agent query, rental, other-session message or lock acquisition; C6 hardware n=0.

## 3. Tests, mutation and flags

| Check | Before | After / scope |
|---|---|---|
| CODE READ: candidate-5 full Rust baseline | 3620 pass / 0 fail / 19 ignored, 22 crates | Existing [per-crate counts](../candidate5_20261018/crate_counts.tsv) and [verification](../candidate5_20261018/final_verification.md); not rerun by C6 |
| MEASURED: C6 cargo tests/builds | NOT RUN | NOT RUN; stop before implementation, no new green claim |
| MEASURED: C6 behaviour mutants | N/A | N/A; zero behaviour changes, no mutation kill claim |
| MEASURED: product-source equality vs candidate-5 | Base `f8f9d1a5f` | Identical `crates`, `qemu`, `broker`, `scripts`; queued fixes are not present |
| MEASURED: diff scan / local Markdown links | Stage 1 PASS | Report/policy scanned before commit; bounded static checks only |
| MEASURED: flag changes | P19 subset 19 names | 19 unchanged; hardwired 0, deleted 0; final KEEP/hardwire counts and total every-reader count NOT DONE |
| MEASURED: hardware | n=0 | n=0, spread N/A; no performance/boot/aging comparison |

CODE READ: `KF3_TSG_MIRROR` remains absent; native timer mapping is already
automatic and `KF3_TIMER_MAP` has no reader in the required product roots.
Lifecycle/lazy-observe defaults and off-by-default hitch diagnostics remain
unchanged. This is not the complete required KEEP-default/zero-cost audit.

## 4. Decisions for the owner/coordinator

1. INFERRED: choose explicit `guest-os=windows|linux`, default `linux`, as the conservative per-device behaviour selector; **recommend this** before implementation. The proposed property does not exist yet.
2. INFERRED: separately decide the reviewed derived-caps/sweep scope required to replace the old P19 captured-page probe; recommend a scoped replacement with exact-revision hardware gates, preserving the binding prohibition on shipping captured tables.

CODE READ: the alternative is an automatic identity design with proven early
layout/ordering/life semantics. Global Windows defaults are rejected because
they change Linux behaviour. A declaration never bypasses guest authority,
read-only private-ring or physical-operand checks. Merely changing the existing
`GuestOs` enum argument is insufficient: its Windows client-kind rule is absent.

## 5. Unverified and remaining work

MEASURED: no minimal working QEMU invocation has been established. Proposed
`-device kf3-gpu,...,guest-os=windows` is an option under review, **not a runnable
or measured command**. Existing explicit properties remain untouched.

CODE READ: after the decision, resume with the full flag inventory and
tests-first config/profile work, then independently validate/import selected
topics. Stages 3–4 still owe release layout and one shared version, prefix
install/uninstall, read-only doctor, rootless facts, evidence-generated support
matrix with STALE rows, first-run Linux/Windows walkthroughs, shell/docs checks,
full release tests/clippy/fmt/broker sanitizer checks and the CPU-only host build.
No hardware slot is requested as ready: C6 is not ready for Stage 5.

CODE READ: external release readiness still requires packaging/signing/CI,
licence/provenance review (including the QEMU redistribution issue recorded in
owner G), security/install review, telemetry/log hygiene, the stage-2 security
gate and the full hardware/install-from-clean-directory campaign. No supported
rootless or guest/GPU/driver combination is newly claimed by this report.

## 6. Cleanup

MEASURED: no persistent C6 command/build/test/collector/VM process was started.
The Stage 1 git push completed. The C6 target directory was never created and
is absent; no target deletion was needed. No host/GPU/shared build lock was
acquired, so none remains held. Only the C6 branch/worktree and documentation
artifacts are retained. No merge to master, force push or other branch push.
