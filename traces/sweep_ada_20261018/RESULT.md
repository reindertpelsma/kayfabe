STATUS UPDATE: SUPERSEDED, 2026-10-22. Owner explicitly resumed S2 to implement the approved guard and then rent. Guard is now verified; see GUARD_RESULT.md. The preflight stop below is historical, not the resumed outcome.

STATUS: RESEARCH, 2026-10-22. S2 stopped at the required GPU-free prerequisite.

1. Identity and request

MEASURED: lane S2, branch codex/sweep-ada-20261022. Preparation merge
79925a345fddb7b1ca70239e8a29efd32f3f426a was pushed and verified against origin.
CODE READ: requested work was one budget-capped Ada lease, native and Linux
guest acceptance under the approved exact-match fault exception, followed by
Windows LTSC/580.88 bind, display, desktop, available CUDA/DX smoke, a 600-second
zero-TDR hold, and an observer hold if available.

2. Result

- MEASURED DONE: clean merge from origin/master and the specified sweep branch,
  without rebasing; only the S2 branch was pushed.
- MEASURED DONE: 23 inherited guard tests pass; historical native five-fault
  approval and unchanged-snapshot check return 0.
- MEASURED PARTIAL: required guest-control gate refuses four retained correlated
  guest defer-liveness records with exit 7; see preflight_receipt.json.
- CODE READ: the CLI has native/check/watch only, and check/watch require the
  exact native sequence to remain unchanged. The guest exception was approved
  but not implemented in the starting branch; this is a harness prerequisite gap.
- MEASURED NOT DONE: no S2 rental, native hardware run, full Linux guest suite,
  Windows bind/display/desktop/smoke/hold, or observer hold. No hardware PASS.
- MEASURED: zero lease creates, zero owned instance IDs and zero rental spend.

3. Tests and hardware

MEASURED: inherited guard tests 23 passed / 0 failed / 0 skipped on the merge.
CODE READ: before/after guard source is unchanged; no fix or behavior change
was made, so a new mutation kill table is not applicable. Existing mutation
evidence remains historical, not a fresh S2 mutation claim.
MEASURED: replay native approval rc0, native unchanged rc0, native plus four
guest defer-liveness faults rc7. The unproved historical guest missing-page
mapping was excluded. This is text replay, not new GPU evidence.
MEASURED: hardware sample count n=0; spread and performance/TDR numbers unavailable.
CODE READ: the predeclared falsifier is in PREFLIGHT.md. No acceptance path
consuming this run's guest source/PID/channel/recovery proof exists in this CLI;
the falsifier was not met. No retry or alternate guard was deployed.

4. Owner decisions/options

1. INFERRED recommendation: finish and verify the already-approved guest-control
   guard in a test-first task, then resume S2 with fresh cost and lease checks.
2. INFERRED alternative: park Ada validation; retain UNRUN for Windows and
   Linux full-suite acceptance. Do not replace proof with a wildcard exception.

5. Unverified

MEASURED: no current Ada hardware result, Windows events, QEMU failure log or
dump header exists for S2 because the prerequisite stopped before rental.
CODE READ: historical missing-page guest physical-channel evidence is incomplete;
owner approval does not supply that proof. Observer availability was not checked
after the required stop. Current account credit/offers were not queried; the
43.85 USD credit is a lane briefing value, not a new observation.
CODE READ: the stop is required by the common lane instruction quoted in
PREFLIGHT.md; changing or bypassing acceptance after refusal would violate it.

6. Cleanup

MEASURED: no S2 lease/controller/timer/GPU job was started. The replay and unit
test commands exited; the real-process tests cleaned up their owned children.
MEASURED: no /workspace/s2-target directory exists; no Cargo build ran, no host
or Cargo lock was taken, and no SSH or trusted-host access occurred.
MEASURED: final origin-head equality and clean-worktree receipt accompany the
external lane report at the owner-requested report destination.
