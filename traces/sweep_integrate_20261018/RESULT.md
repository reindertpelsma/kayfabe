# Phase 1 stop report

STATUS: LIVE, 2026-10-18. Protected integration conflicts; owner decision pending.

OWNER BUDGET UPDATE, 2026-10-18: stretch cap raised to **USD 25**; stop renting
and report at **USD 20 spent**, superseding the previous USD 12 / USD 9 limits.
The owner reports account credit USD 43.56 and no instances. See `BUDGET.md`
and `budget_update_receipt.json` for authority and the fresh API observation.
Stretch spend remains USD 0. The protected-conflict STOP remains unresolved;
Phase 1 must complete before any rental. Other rules remain binding.

Lane `sweep-20261018`; branch `codex/sweep-integrate-20261018`; initial product
source candidate 4 `0452613b5c71cb175271230b3e14c8e3c3eef2db`. The report commit
adds evidence only. See `SWEEP-publication.json` beside the local overnight report
for the final local/remote head equality receipt; that receipt is written after
push to avoid a self-referential commit hash.

## Result

- MEASURED DONE: isolated required worktree; full new steering and COMMON read.
- MEASURED DONE: preflight on candidate 4 against gpu-types, without applying it.
- MEASURED STOP: three protected files, twelve conflicting code/test hunks.
- CODE READ: overlap is old USER batching versus current relay/lifecycle/lazy state.
- MEASURED: every winner remains UNRESOLVED; product source is unchanged.
- MEASURED PARTIAL: dependency inventory and exact conflict evidence recorded.
- MEASURED NOT DONE: code integration, release gates, mutation checks, hardware.
- MEASURED: all current-revision cells UNRUN; every old PASS labelled STALE.
- MEASURED: candidate 5 now exists on origin at `f8f9d1a5f`; not merged here.
- MEASURED: no rentals; no owner-host access; incremental spend $0.

The hypothesis and falsifier were written in `PLAN.md` before the preflight.
H-I1 was FALSIFIED by the protected conflicts. `CONFLICTS.md` identifies every
file/hunk and states the proposed resolution; no resolution is implemented.
`preflight_conflicts.json` retains reproducible stage blob IDs and hunk hashes.

## Verification and limits

| Check | Before | After | Meaning |
|---|---|---|---|
| Release tests per touched `kf-*` crate | NOT RUN | NOT RUN | STOP preceded integration; zero product crates changed; no test count claimed |
| Product trees/manifests versus candidate 4 | Baseline | Byte-identical Git objects | No source or test was added, removed or edited |
| AD104 / Windows580.88 code-path differences | Baseline | None introduced | Source identity only; no oracle or behavioral execution claim |
| `cudart_gate_ad104_oracle`, `host_facts_*`, consttab | NOT RUN | NOT RUN | Required before integration acceptance; STOP does not waive them |
| Clippy / rustfmt | NOT RUN | NOT RUN | No Rust changes |
| Mutation kill table | N/A | N/A | No behavior change or mutant run |
| Hardware sample size | 0 | 0 | No gate/raw/app/desktop run; spread N/A |

MEASURED: the Git preflight is not a passing integration. A hypothetical merge
tree exists only as Git objects; it was never installed as product source or
built. The unprotected documentation conflict is excluded from the proposed
code-only import. See `MATRIX.md` for die/OS/build/revision, stale history,
evidence locations, and the complete nineteen Windows flags.

## Owner decisions

1. Protected conflicts: recommend excluding obsolete gpu-types USER batching/measurement and dependent edits, preserving current relay/lifecycle/doorbells and tests; authorize this scope before resuming.
2. D4 caps keys: retain exact die/branch keys until a mechanism and branch-independence evidence justify each proposed broader key; never borrow another die's row.
3. D5 TU106 R34: retain frozen 180 s grader and name the historical performance failure; do not rent for this policy item.
4. D6 oracle page: keep captured pages as test oracles; prefer header-derived diagnostic groups; a diagnostic-only runtime oracle still needs explicit owner authorization.
5. New PR plan: after resolving item 1, title `sweep: integrate per-die code onto current product`; base candidate 4 (or owner-designated candidate 5), head this branch; product diff currently 0 files / 0 lines, eventual code size not yet determined. New follow-up topics target this branch until directed otherwise. Existing drafts remain untouched.
6. windows-on-vast: no change proposed; use the owner's specified unmodified reference flow for GA104's native/VFIO prerequisite. No push or patch to that repository occurred.

## Access, cost, cleanup

MEASURED live API receipt: credit $18.559771576861976, zero instances at
2026-10-18T20:31:52Z; previous owned instance 56326220 absent. This lane created
[] and destroyed []; spend $0. No new cost ledger entry is needed for a lease
that did not occur; the historical ledger remains on the evidence branch.

MEASURED: no cargo target directory was created, no GPU/VM job or autodestroy
timer was started, and no lock was acquired. No own background job remains.
Only this lane's own worktree/evidence and external overnight report are written;
other session checkouts, integration/master branches and old draft PRs are not
modified. Target cleanup is N/A because `/workspace/sweep-target` never existed.

## Ten-line session status

```text
1 Lane sweep-20261018; branch codex/sweep-integrate-20261018.
2 Product source remains candidate 4, 0452613b5; evidence-only commit follows.
3 Required worktree /workspace/sweep-wt is isolated from other sessions.
4 GPU-types preflight STOP: 3 protected code/test files, 12 conflicting hunks.
5 Conflict winners UNRESOLVED; no code, test or historical evidence imported.
6 Release counts/oracles/mutants NOT RUN; behavior not revalidated.
7 Every current hardware cell UNRUN; historical PASS results STALE.
8 Candidate 5 observed f8f9d1a5f; refresh current line before any hardware.
9 Created/destroyed IDs []; spend $0; no own rental/job/target/lock remains.
10 Await owner scope decision in CONFLICTS.md before continuing either phase.
```
