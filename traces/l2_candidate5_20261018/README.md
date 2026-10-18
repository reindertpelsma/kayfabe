# L2 candidate-5 hardware validation — stopped at prerequisite

STATUS: ANSWERED for this attempt, 2026-10-18. Candidate-5 is NOT GREEN: its
readiness prerequisite failed; hardware validation did not run.

MEASURED: the initial host marker probe at 01:52:45 UTC returned `L1_PENDING`.
The next probe, exactly 120 seconds later at 01:54:45 UTC, returned `L1_FAILED`;
both SSH commands exited successfully. The marker-wait process then exited 3.
The complete, small observation is in [marker_wait.txt](marker_wait.txt).

MEASURED (reported by L1, not independently reproduced by L2): L1's FAILED
marker identifies hitch source `b233037d95da2ae157cfcf50066fb500991c5b7b` and
reports unconditional display LatStat sampling and EventRing recording from
`Requests::default()`. L1 reports that its default-off instrumentation gate
failed before either merge, with no candidate-5 push, binaries or READY from L1.
L2 has not independently established those source-code findings.

CODE READ: L2's lane instruction requires reporting and stopping when FAILED
appears, without stopping the owner's VM. L2 followed that early-exit path.

| Gate / section | Result | Evidence |
| --- | --- | --- |
| L1 readiness | MEASURED: FAILED observed | `marker_wait.txt` |
| Owner snapshot / stop / launcher copy | MEASURED: not attempted | Early exit before takeover |
| GPU gates, expected 9/9 | MEASURED: not run, 0 attempted | Readiness prerequisite failed |
| Rebuild and fast suite, expected 30/30 | MEASURED: not run, 0 attempted | Same prerequisite |
| Linux broker / interactive / cursor | MEASURED: not run | Same prerequisite |
| Windows interleaved A/B | MEASURED: not run, n=0 boots per arm | Same prerequisite |
| Windows / Linux lifecycle and boot layer | MEASURED: not run | Same prerequisite |
| Owner restoration | MEASURED: no restart performed | L2 never stopped or changed the owner's session |

MEASURED: before/after hardware test counts are 0/0 attempted, not passing
scores. There are no L2 hardware numbers, spreads, screenshots or plots.
Mutation checks are not applicable: L2 changed no product or test behaviour.
Only this evidence was committed, on a separate branch based on candidate-4
`0452613b5c71cb175271230b3e14c8e3c3eef2db`.

INFERRED recommendation: keep the owner's candidate-4 launcher; do not promote
candidate-5. Repair the source instrumentation defaults with separate EXPECTED
RED tests and mutation checks, then rerun L1 and L2. This attempt provides no new
evidence for accepting or rejecting the hitch fix's performance. The lane's
historical result remains that gaps >=100 ms and the 1% low did not improve
beyond spread; L2 did not remeasure it.

MEASURED: L2 never acquired a GPU/VM lock, created a cargo target directory,
changed a launcher, sent a guest command, stopped a VM, or wrote a host file.
The wait process exited. No L2_DONE marker was written: the explicit FAILED
early-exit path ends before takeover and restoration, so this is not a hardware
resource handoff or a validation pass.

CODE READ: the owner-only checks remain Minecraft Bedrock and Java fullscreen,
hide/restore of the broker window with Heaven running, live Windows cursor
shapes, and the subjective feel of 100 ms lags; none was attempted.
