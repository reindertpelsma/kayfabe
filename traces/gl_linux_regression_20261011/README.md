# Pending OpenGL display branch: review and Linux regression

**STATUS: LIVE, 2026-10-11.** Candidate `codex/gl-display-review-20261011` starts from integration
`a2632e64` and includes Claude's GL branch `417bad6d`. Integration and master are unchanged while
the candidate is validated. This work corrects capability declarations; it does not claim to fix
the Windows OpenGL crash.

Independent review rejected the initial branch for advertising DP MST/dual-SST based on header
field existence, ignoring SOR sublink selection, and echoing the reserved-panel output mask.
The second review also rejected physical-RM feature bits without emulator implementations and
guest-authored output flags. The corrected model advertises HDMI 2.0 and its fixed SOR routing
(`00 0c`) on every family, validates sublink/audio requests, and authors both output fields.
The final read-only review of `b88f2f60` found no remaining correctness blockers, conditional on
the exact candidate passing hardware regression. Its sole cleanup was an orphaned doc comment.

Five regression tests failed before their corresponding fixes; their logs and the passing display
tests are in `logs/`. The additional SOR layout fields, enum values and flag masks were compiled
from the open NVIDIA 580.65.06 and 580.159.04 headers with `tools/derive_display_layouts.sh`.
Private diagnosis text was removed from the current GL README under owner §AG; a private local
copy is retained outside this repository. Historical commits still need the release history sweep.

`run.sh REV` runs under one outer GPU lock: GPU-free tests, gates 1–9 plus gate 10, device build,
raw-client rebuild, fast-guest rebuild, 30-arm suite, T-space log gate, then broker/input smoke.
It records start/exit markers, lane logs, and before/after Xid records. Host resources remain
revision-pinned, and the broker binary symlink is removed only if this run created it.

Hardware hypothesis: the truthful capability table and SOR validation preserve existing Linux
driver behavior. Falsifier, stated before the final run: any gate failure, any failed/crashed raw
client arm, any T-space gate failure, or any broker/input proof failure on the exact candidate.
Report every result, including a failure. Windows behavior needs a separate run after Linux.

The preliminary run at `a590f10a` is not evidence for the final capabilities. Its per-stage Xid
capture lacked a tool on the host; the final runner uses the available `grep` and separate output
directories. It is not edited while running. Final hardware results are pending.
