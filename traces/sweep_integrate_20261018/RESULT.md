# Current integration checkpoint, 2026-10-19

STATUS: LIVE. Phase1 PARTIAL only at two explicit protected scope decisions;
all independent imported code/test topics and their GPU-free checks are complete.

Candidate5 f8f9d1a5f is merged. GPU-types and99 code-changing main-stack imports
plus14 separate draft imports are recorded; one further source record was already
present. Five heartbeat source commits and all their dependent edits are held
by corrective32d1128b4. The original obsolete USER exclusions remain complete.
F-2 and the family admission approval are implemented, tested and pushed.

MEASURED:13full crate suites pass2924tests; all21QEMU integration tests pass.
Full QEMU unit compilation awaits only the proposed None cache-key field in
one current life-reset/scrub test. Family component312->320 was fully tested
before candidate5; it accepts Ada exactlyc56f. Before/after per-crate counts
and source retention are in current_crate_counts.tsv/source_test_retention.json.

MEASURED:39distinct final mutants each have>=2runtime killing tests;53bench
Python tests pass. Native portable tracer/observer sanitizers pass. The Windows
driver/collector cross-build with the pinned SDK/WDK, PE/import guards pass,
and an injected BAR0 store is rejected as read-only by the compiler. No current
Windows load or hardware acceptance is claimed.

Clippy:0new diagnostics on changed lines;89denied diagnostics are inherited
from the base and are not a stop by coordinator ruling. Rustfmt changed files
passes, with the approved userrelay regions formatted and unrelated inherited
hunks preserved. No existing C4/C5 test is dropped or newly ignored.

AD104: CPU per-die/host-fact/consttab/oracle checks pass. Strict full-page
equality against the stored native oracle FAILS100words. AD104_PATH_AUDIT.md
names changed code paths and limits; do not report unchanged owner behavior.
Current hardware cells allUNRUN, oldPASS cells allSTALE, no family complete.

Current exact decisions: approve/defer the isolated heartbeat drainer topic,
and approve only the None cache-key field in the existing scrub-test fixture.
Both proposals/conditions are in PROTECTED_DECISIONS.md. Other protected
relay/lifecycle/doorbell/birth/group/latejoin/scrub code is preserved; the
GSP drainer is byte-identical to candidate5. Scope questions remain pending
while independent work has continued. Phase2 is prepared but cannot rent
before Phase1 acceptance. Budgetcap25USD/stop20; spend0, own active IDs0.

Earlier component checkpoint below is historical and superseded by this one.

# F-2 complete; remaining sweep integration has a protected family-admission dependency

STATUS: LIVE, 2026-10-19. F-2 verified and pushed; separate userrelay scope question pending.

Lane `sweep-20261018`; branch `codex/sweep-integrate-20261018`.
Product checkpoint: F-2 fix `6a32d4e8e`, following EXPECTED RED `08424cf11`.
The publication receipt beside the local SWEEP.md records the final evidence
head and exact origin equality after push. Neither candidate 5 nor the remaining
gpu-types/stacked sweep topics are integrated yet. Phase 1 is PARTIAL.

## Result (MEASURED unless marked CODE READ)

- DONE: isolated two-file F-2 public-header class-width fix and test-first history.
- DONE: full kf-chan release 521/0/3 before -> 530/0/3 after; no old test dropped.
- DONE: all 9 focused tests pass after compiling/failing 3 cases on old code.
- DONE: 4 mutants killed by 3/2/2/3 tests; exact fix restored.
- DONE: clippy 217 inherited before / 217 after; added=0; rustfmt passes.
- DONE: prospective integration baseline across 14 crates, 3084 passed / 0 failed / 19 ignored.
- CODE READ: five approved exclusions and all 67 old merge hunks inventoried.
- CODE READ: F-2 leaves Ada's 8-bit validation unchanged; no host/guest GPU run.
- CODE READ STOP: retained family-admission fix requires protected userrelay.rs.
- PARTIAL: code-only import manifest prepared; full sweep code import NOT DONE.
- NOT DONE: integrated AD104/consttab oracles, candidate-5 merge, Phase 2.
- All current hardware cells UNRUN; every historical PASS remains STALE.
- No rental, owner-host access or spend; cap USD25 / rental-stop/report USD20.

## F-2 and test retention

The production edit is exactly `kf-chan/src/grtables.rs` and `tmode.rs`:
public CB97/CDC0 fields allow 25 upper bits, other classes and context-free
calls retain 8. `gr_write` passes the bound class to the field validator.
Tests check complete address/footprint preservation, no premature emission,
side-effect-free refusal and the narrow/default boundary. The unchanged address
binder/perimeter still validates the resulting address before GPU work.

`f2_test_retention.json` verifies every old test in the two files remains and
nine tests were added. All other candidate-4 product/test files are unchanged.
AD104's internal call path now selects a class width, but its Ada classes still
select 8 bits: no observable width-rule change is introduced there. This is
CODE READ plus CPU regression coverage, not an AD104 hardware result.

| Check | Before | After |
|---|---|---|
| kf-chan release (MEASURED) | 521 pass, 0 fail, 3 ignored | 530 pass, 0 fail, 3 ignored |
| Focused F-2 before fix / after fix | 6 pass, 3 fail (EXPECTED RED) | 9 pass, 0 fail |
| Clippy diagnostic identities | 217 inherited | 217 inherited; added=0 |
| Mutants force-narrow / widen-all / admit-bit25 / drop-class | Planned before runs | 3 / 2 / 2 / 3 killing tests |
| Rustfmt | Existing files | PASS, exactly approved files |
| Full integrated AD104/consttab acceptance | Not imported | UNRUN |

Existing host-fact tests were included in the RM baseline below. They do not
substitute for the new AD104 oracles on the complete integrated sweep product.
Mutation identities/log hashes and source retention are recorded beside this
report; raw build logs stay in the lane's own scratch rather than enlarging Git.

## Broader release baseline

The following are MEASURED local runs with candidate-4 files unchanged outside
the approved F-2 dependency. They establish the next import's baseline and do
not validate the unimported sweep code. `cpu_counts.tsv` compares them with the
candidate-4 reference from the announced candidate-5 evidence (CODE READ).
That reference had one memory/RSS failure; this run passed the same 184 memory
tests without a memory source change. No causal improvement is attributed to F-2.

| Crate | Passed | Failed | Ignored |
|---|---:|---:|---:|
| kf-abi | 592 | 0 | 0 |
| kf-arch | 4 | 0 | 0 |
| kf-chip | 72 | 0 | 0 |
| kf-census | 50 | 0 | 1 |
| kf-cuda | 67 | 0 | 0 |
| kf-disp | 247 | 0 | 3 |
| kf-gsp | 49 | 0 | 0 |
| kf-host | 66 | 0 | 0 |
| kf-rm | 750 | 0 | 0 |
| kf-harness | 12 | 0 | 0 |
| kf-broker | 149 | 0 | 9 |
| kf-mem | 184 | 0 | 3 |
| kf-qemu | 312 | 0 | 0 |
| kf-chan | 530 | 0 | 3 |

## Current protected dependency

The original USER batching exclusion and F-2 scope are APPROVED, not pending.
The remaining dependency is distinct: the current `Birth::new` declaration gate
accepts only AMPERE_CHANNEL_GPFIFO_A. The retained Turing family topic needs
userrelay.rs to use the actual host-derived family's generated channel class.
The latest coordinator expressly keeps userrelay protected, so it is unchanged.

See USERRELAY_DEPENDENCY.md and the bounded unapplied
userrelay_family_gate_PROPOSAL.patch. Upstream test-first/fixture/fix history is
0b1a1fe7e / c5ee774cb / 8e3a9d325. Scope is only the declaration helper/call and
pure tests; preserve bounds, ownership, all existing tests, lazy/scan-off,
lifecycle, counters, publication, doorbells and channel groups. An asynchronous
scope question is pending; no answer is inferred from elapsed time.

The code-only gpu-types manifest contains 98 candidate paths, 9 formatting-only
paths to retain current, 6 excluded USER/dependency paths, and the 2 approved
F-2 files to retain. The raw candidate-5 preflight was NOT applied. Incidental
old formatting is not a reason to overwrite current reset/budget behavior.

## Owner decisions (numbered, one line each)

1. Family admission: recommend approving only the generated-family declaration helper/call/tests in userrelay.rs, with test-first and mutation gates; otherwise explicitly defer it.
2. D4 caps keys: retain exact die/branch keys until each broader key has branch-independence evidence and a mechanism; never borrow another die's row.
3. D5 TU106 R34: retain frozen 180 s grader, report the historical named performance failure, and do not spend a lease on this policy item.
4. D6 runtime oracle page: retain captured pages as test oracles and use header-derived diagnostic groups; runtime oracle still needs explicit authorization.
5. PR plan: isolate F-2 pair 08424cf11/6a32d4e8e as `chan: honor public 25-bit GR address fields`; base owner-designated candidate 5, head an authorized isolated sweep topic, code size 2 files (+129/-11). Overall integration PR still awaits completion/base decision; no PR or retarget created.
6. windows-on-vast: no installer change proposed; optional README-only distinction between native reference and nested guest is written as an unapplied patch here; reference e8883e2 remains clean/unmodified.

## Hardware, cost and cleanup

No Phase 2 run or rental occurred. Created IDs [], destroyed IDs [], stretch
spend USD0. The latest filtered account receipt shows USD43.56 and no instances
at its recorded time. No owner-host access, VM, GPU job, timer or lock was used.
The lane's own target /workspace/sweep-target has been removed; the final
publication receipt records deletion and absence of own running jobs.

`MATRIX.md` records every reported die/OS/build, current untested checkpoint,
historical source/evidence and all 19 Windows flags. No historical pass is
promoted. Candidate 5 f8f9d1a5f must be merged before any hardware stage, after
Phase 1 is complete; the current line must be refreshed again then.

## Ten-line session status

```text
1 Lane sweep-20261018; branch codex/sweep-integrate-20261018.
2 F-2 EXPECTED RED 08424cf11; verified fix 6a32d4e8e, pushed.
3 kf-chan release 521/0/3 -> 530/0/3; no current test dropped.
4 Mutant kills 3/2/2/3; clippy added=0; rustfmt PASS.
5 Broader 14-crate baseline: 3084 pass / 0 fail / 19 ignored.
6 Only the approved two product files changed; original exclusions preserved.
7 NEW STOP: generated-family admission needs protected userrelay.rs; proposal unapplied.
8 Main stack/candidate-5 integration and integrated AD104 acceptance remain pending.
9 Hardware UNRUN; old PASS STALE; spend USD0 / cap USD25 / rental-stop USD20.
10 Await only family-gate scope decision; cleanup recorded by final publication receipt.
```
