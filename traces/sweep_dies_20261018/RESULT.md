# S4 desktop sweep stop receipt

**STATUS: PARTIAL, 2026-10-22. Hardware campaign not started.**

1. CODE READ: lane S4 requests Ada, Ampere and Turing Windows 580.88 desktop, app matrix, Unigine and a 600-second hold; branch `codex/sweep-dies-20261022`.
2. CODE READ: base `737306ee8` plus production master `e9c6d39e5` and descriptor Phase 1 `468a35c9d` were merged as `393d6b49a`; no audit-fix branch was added.
3. MEASURED in `../sweep_ada_20261018/lease_57144505/windows_stopped/arm_ad102_on_d1_health_before.txt`: inherited Ada health had Code 0, DWM, no Explorer and two Kernel-Boot 153 records.
4. CODE READ: `scripts/bench/windows/firstlogon.ps1` disables autologon; the inherited S2 arm contains no sign-in step before requiring Explorer.
5. INFERRED H1, stated before inspection: numeric event matching misclassified boot events; FALSIFIER was a display-driver fault in the refused records or an independent refusal. The independent missing-Explorer condition was present; both need handling.
6. MEASURED in `health_expected_red.log`: inherited gate, 9 tests, 4 failures; `signin_expected_red.log`: 4 tests, 1 failure and 2 errors. Both test additions were committed alone with EXPECTED RED messages.
7. MEASURED in `health_green.log` and `signin_green.log`: proposed provider/desktop-epoch and configured-account changes passed 11 plus 4 tests locally; Windows execution was not verified.
8. MEASURED in `mutation_gate_failure.json`: first mutant restored blanket event rejection; only 1 distinct test failed, below the required 2. Remaining mutants were not run.
9. CODE READ: the lane requires stopping on a refused check. Proposed implementation is saved only in `proposed_health_signin.patch`; active files were restored. This branch deliberately retains regression tests that fail on the inherited behavior.
10. MEASURED in `mutation_gate_failure.json`: Ada NOT RUN in S4; inherited desktop acceptance remains partial; matrix, Unigine and hold are unverified.
11. MEASURED in `mutation_gate_failure.json`: Ampere NOT RUN; no driver/revision/flags combination was tested.
12. MEASURED in `mutation_gate_failure.json`: Turing NOT RUN; no driver/revision/flags combination was tested.
13. MEASURED in `mutation_gate_failure.json`: zero new leases, zero hardware runs, zero lane rental spend; hardware n=0, spread unavailable; no RPC ordinal or event timestamp from a new run exists.
14. INFERRED owner decision: authorize resumption of harness proof development after this stop; recommendation is to strengthen independent positive health fixtures and require every mutant to have at least two killers before renting Ada.
15. MEASURED in `claims_gate_failure.json` at rev `3f157816d`: claims check reports `new=188`, required zero; no push. CODE READ: no cargo build, target directory, GPU lock or rental process was created; cleanup is recorded in the external S4 report.

## Resumed 2026-10-22

CODE READ: the owner's resumption instruction authorizes continued harness repair
and an evidence-only push using only S4-added or changed files for claims review.
This supersedes the stop and pending-decision statements above. No debt baseline
is expanded. INFERRED H2, before hardware: explicit sign-in plus provider/desktop
classification lets a healthy Ada desktop reach the app matrix and hold. FALSIFIER:
Code43, NVIDIA153/Display4101/bugcheck, missing Explorer, or an application failure.

MEASURED on 2026-10-22 in `traces/sweep_dies_20261018/desktop_green.log`, at rev `228d62573`: 33 S4 tests pass;
`appmatrix_tests.log` records 76 existing app-matrix tests passing. MEASURED in
`health_mutations.json`, `plan_mutations.json`, `controller_mutations.json`, run
S4-local-20261022: twelve mutants have at least two distinct killing tests each.
MEASURED in `unigine_mock.res`, run S4-local-20261022: four Unigine rows traverse
the existing guest-agent mock protocol and host verdicts. This exercises no GPU.
CODE READ: the controller freezes the source-matched native baseline before
starting the existing fatal-Xid watcher, then runs install, bind and desktop.
CODE READ: the attached runner uses the existing PowerShell app definitions,
supervisor and verdicts, covers all 112 rows with Unigine early, and requires the
full 600-second hold. A restart or health fault ends GPU work in that VM.
