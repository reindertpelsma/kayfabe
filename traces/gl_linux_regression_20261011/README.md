# OpenGL display review and regression

**STATUS: LIVE, 2026-10-11.** Candidate `codex/gl-display-review-20261011` starts from integration
`a2632e64` and includes Claude's GL branch `417bad6d`. The reviewed candidate is integrated into
`integration/windows-20261010`; master is untouched. This work corrects capability declarations; it does not claim to fix
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
directories. It was not edited while running. Final Linux and Windows results follow.

The first runner also incorrectly applied `--user` to every raw-client arm. That optional census
requires a guest USER-level channel, but the root raw client is stamped ADMIN (`privilege=1`);
the independent host birth census still requires USER host channels. The documented T-space
invocation is without `--user` (`V3_TSPACE_HARDWIRED.md`, hardware checklist). `finish.sh` reruns
those checks over all 30 existing final-revision logs and runs the broker lane on the same binary.
The rejected assertion and its log are preserved. This is a harness correction, not a product change.

Windows regression (run 642, source `02afce90`, `windows.sh`): fresh baseline, production
profile, overlay disabled, exFAT application disk, CUDA cup2/cup3/cup8 and interactive gears.
Hypothesis: truthful display claims preserve boot and CUDA; falsifier: adapter no longer Code 0,
any CUDA probe fails, or a new TDR occurs. Gears has a known baseline crash; report whether it
persists without treating a crash-free boot as an OpenGL pass. The RPC trace is diagnostic only.

## Linux result, 2026-10-11

Source **`02afce90d`** (full revision in `logs/02afce90/run.log`), RTX 4070, host driver 595.91.07,
kernel 7.0.0-34-generic. Device SHA256 and build/install path are in `logs/02afce90/artifact.txt`.
All hardware work was serial under `/tmp/kayfabe-fastguest.lock`.

| Check | Result | Evidence |
|---|---|---|
| GPU-free `kf-disp`, `kf-rm`, `kf-qemu` | 1044 passed, 0 failed/ignored | `logs/02afce90/tests.log` |
| Gates 1–9 and micro-reservation gate 10 | 9/9, gate10 PASS; 12/12 USER host births, 0 refused | `logs/02afce90/v3_gates.log` |
| Raw-client and fast-guest rebuild; fast suite | 30/30, 0 fail/crash/notrun | `logs/02afce90/run.log`, `fast.log` |
| Documented T-space checks | 30/30 log verdicts PASS; no Translated deaths, no mirror windows, all host births USER | `logs/02afce90/tspace.log` |
| Broker/input smoke | GRUB key PASS; Cinnamon up; 8 ABS cases, REL (70,30), ABS-drop-under-grab, keyboard PASS; reconnected broker; clean shutdown | `logs/02afce90/broker_proof.log` |
| Host Xid records, fast and broker stages | Identical before/after snapshots, count 35; this is a log observation, not proof that negative-control faults did not occur | `logs/02afce90/*_xid_{before,after}.txt` |
| Quick CI compared with handoff baseline | Four inherited failing categories, identical offenders; zero introduced failures | `review.md`, `logs/ci_{baseline,candidate}.log` |

The first final-revision runner exited at its incorrect `--user` assertion (`run.log`,
`tspace_initial.log`). The follow-up completed the correctly scoped checks and broker lane with
`POSTCHECK_EXIT rc=0` (`artifact.txt`). No product source changed between these runs. Product source
after `02afce90` differs only by a verified header citation comment; subsequent changes are bench
scripts and documentation. This does not meet the CI-green requirement for a master merge.

## Windows result, run 642, 2026-10-11

The same `02afce90` binary returned `00 0c` capabilities (`logs/windows642/caps.txt`). CUDA cup2
(copy round trip), cup3 (kernel result 43) and cup8 (2048-square matrix, bad=0) passed. Adapter
Code 0 and zero TDR preserved the stated hypothesis. **Gears failed**, exit -1073741819; the
GEARS-UP marker was emitted despite process death and is not a pass. The application access
violation remains P0. No fault offsets or closed-driver diagnosis are included in this evidence.

The original runner typed fixed keys after resetting the account to an environment password,
so its first sign-in did not create an interactive session. The runner was paused with SIGSTOP
while QEMU and the outer GPU lock remained live. `recover_probes.sh` used `signin.py` to type
the existing secret without logging it, verified the interactive user via QGA, recreated the
interactive probe task, and resumed the original runner. This recovery is part of the measured
run, not an unattended pass of the revised runner. The revised runner verifies sign-in and absence of the lock screen before
starting interactive probes, and reports failure after teardown if verification fails. Independent
review found and corrected both the locked-session false positive and the zero exit on skipped probes. The Python helper passed syntax and ASCII key mapping checks and
was exercised successfully in this live recovery.

`logs/windows642/` contains probe results (the APPCRASH fault-offset line omitted), capability
replies, recovery output, phase timeline, adapter status and teardown. The run ended with
`WINDOWS_CHECK_EXIT rc=0`, no QEMU, no stop file, group 11 DMA-FQ, NVIDIA driver restored, and
111 GB free. Local wrapper assertions additionally check CUDA exits, adapter status and TDR
verdict; these assertions were added after the host wrapper had started and were checked against
its collected output, not reported as executed by that older wrapper.
