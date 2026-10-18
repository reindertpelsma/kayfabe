# Phase 1 scope audit: original exclusion approved, independent F-2 STOP

STATUS: LIVE, 2026-10-18. New protected-edit decision required; original exclusion approved.

Lane `sweep-20261018`; branch `codex/sweep-integrate-20261018`; product source
remains candidate 4 `0452613b5c71cb175271230b3e14c8e3c3eef2db`. Publication head
and remote equality are recorded in the local `SWEEP-publication.json` after
push, avoiding a self-referential committed hash. This branch adds evidence only.

## Owner updates folded into the report

Budget: **USD 25 stretch cap; stop renting and report at USD 20 spent**. These
supersede USD 12 / USD 9. Live API at 2026-10-18T21:05:38Z confirms credit
USD 43.559771576861976 and zero instances. Stretch spend USD 0, not reset.
`BUDGET.md` and `budget_update_receipt.json` record authority and observation.

Scope: the coordinator APPROVED excluding `24e7a297a`, `639acf57d`, `7476ef69f`,
`edcaaca67`, merge `29eae05a4` and dependent edits. Preserve current relay,
lifecycle, doorbells and ALL current tests. No old PUT-ahead diagnostic or
old batching/measurement behavior is ported. The original twelve-hunk conflict
request is resolved by this exclusion; its earlier snapshot remains labelled
historical in `CONFLICTS.md`.

Condition 2 still requires a STOP if a retained die/ABI/constants/display topic
needs a change in kf-chan or any other named protected area. The audit found
exactly such an independent topic before applying product changes: F-2 commit
`3970937f6`, changes to `grtables.rs` and `tmode.rs`. This is the NEW pending
scope question; the original USER exclusion is not being asked again.

## Result (current)

- MEASURED DONE: isolated worktree and original GPU-free preflight; no rental.
- CODE READ DONE: five explicit exclusions plus five merge-contained companion commits inventoried.
- CODE READ DONE: all 67 first-parent merge hunks across seven paths enumerated, including dependent nonconflicting edits.
- CODE READ: userread span-reservation is independently deferred with the excluded merge, not mislabelled as a batching dependency.
- MEASURED: zero product or current-test changes; all candidate-4 source/test objects unchanged.
- CODE READ NEW STOP: retained F-2 ABI rule needs two kf-chan files; exact caller/table dependency recorded.
- PARTIAL: integration scope audited; product integration NOT DONE.
- NOT DONE: release counts, AD104 oracle execution, mutation gates and Phase 2.
- MEASURED: all current-revision hardware cells UNRUN; every historical PASS STALE.
- MEASURED: created IDs [], destroyed IDs [], incremental spend USD 0.

`CONFLICTS.md` contains every excluded commit, all dropped hunk ranges and
reasons, exact F-2 files and source blobs, and the narrow recommendation.
`exclusion_inventory.json` includes full commit IDs and per-hunk hashes.
No code patch, including a cleanly merging dependent edit, has been applied.
`preflight_conflicts.json` remains historical evidence of the original preflight.

## Verification and limits

| Check | Before | After | Meaning |
|---|---|---|---|
| Release test counts per changed kf crate | NOT RUN | NOT RUN | Zero changed product crates; second STOP precedes code import; no numeric execution count claimed |
| All candidate-4 product/test trees and manifests | Baseline | Identical Git objects | No existing test/source added, dropped or edited; verified by scoped git diff |
| F-2 affected current files | `grtables` 497939a6f; `tmode` 1b05d02b7 | Same blobs | New retained topic is held, not applied |
| AD104/Windows580.88 code-path differences | Baseline | None introduced | Source identity only, not an execution result |
| AD104 oracle, host_facts, consttab | NOT RUN | NOT RUN | Still required before integration acceptance |
| Clippy / rustfmt | NOT RUN | NOT RUN | No Rust change |
| Mutation kills | N/A | N/A | No behavior change applied |
| Hardware n / spread | 0 / N/A | 0 / N/A | No new hardware claim |

The absence of dropped tests is established by unchanged source objects, not by
pretending to have run tests. Before/after per-crate numeric release counts and
>=2 killing tests per applicable behavior remain required once scope permits
integration. Neither STOP waives those acceptance gates. See `MATRIX.md` for
all reported die/OS/build/source cells, historical evidence and the nineteen flags.

## Owner decisions (one line each)

1. NEW F-2 scope: recommend allowing only commit 3970937f6's grtables/tmode class-width topic under condition 2, with current tests preserved and required tests/mutants; alternatively explicitly defer it.
2. D4 caps keys: retain exact die/branch keys until a mechanism and branch-independence evidence justify each broader key; never borrow another die's row.
3. D5 TU106 R34: retain frozen 180 s grader and name the historical performance failure; no lease for this policy item.
4. D6 oracle page: retain captured pages as test oracles and prefer header-derived diagnostic groups; a runtime oracle still needs explicit authorization.
5. PR plan: eventual title `sweep: integrate per-die code onto current product`, head this branch, base owner-designated candidate 4/5; product diff currently 0 files/0 lines, final code size unknown; no PR created or retargeted.
6. windows-on-vast: use the specified unmodified native/VFIO reference flow; no change or patch proposed, no push there.

## Access, cost and cleanup

No own rental, GPU/VM job, target directory, autodestroy timer or lock was
created; no owner-host access. IDs created/destroyed [], spend USD 0.
No own background job remains. `/workspace/sweep-target` never existed.
The initial USD 18.56 observation is historical; current filtered receipt shows
USD 43.56 and no instances. The owner-host and other checkouts are untouched.

## Ten-line session status

```text
1 Lane sweep-20261018; branch codex/sweep-integrate-20261018.
2 Product source remains candidate 4 0452613b5; evidence-only commits.
3 Original obsolete USER-topic exclusion APPROVED and recorded.
4 Five named commits plus five companions / 67 merge hunks inventoried.
5 NEW STOP: F-2 3970937f6 requires grtables.rs and tmode.rs in kf-chan.
6 No protected code applied; every current source/test object preserved.
7 Release counts/oracles/mutants NOT RUN; all current hardware UNRUN.
8 Every old PASS STALE; candidate 5 must be refreshed before hardware.
9 Cap USD25 / rental-stop USD20; spend USD0; no instances/jobs/targets/locks.
10 Await only the narrow F-2 scope decision, not the approved USER exclusion.
```
