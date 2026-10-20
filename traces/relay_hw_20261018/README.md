# HW2 relay hardware validation — 2026-10-20

STATUS: PARTIAL. No HW2 hardware boot has been run. All hardware numbers below
are inherited measurements, independently extracted from the completed baton
campaign. Production code and the existing binaries are unchanged.

CODE READ: source base for review is f8f9d1a5f; M is
89fc8a22291dd7bda69c3077ba829b1e934b11db; B' is
c452ca7cb412852a3e6ca68d18e94f92ac625599. Handoff and result source:
`claude/baton-pointer-20261020` at 3eea425b90f1716bcd0e681f9b06292bbb96db05.

MEASURED: the first 50-second host lock wait timed out. The second succeeded.
The handoff was read before host inspection. No prior holder was interrupted.
Host Xid count was 25 at inspection. Binary SHA256 values matched the handoff:

| Arm | SHA256 |
|---|---|
| C | bc4cf65482aeed9637a996311827845d3eb6c39e734231d9e40f2182b47b1269 |
| M | d9dbb6553f256e6f27af65ec8e13999e1a72bc605fa9a107b1c5c5794133ced9 |
| B' | 5087dd61644eee83496d78f0006a80f722a12fa71e4f69b669042fab415c46bb |

MEASURED, inherited: forced migratable Linux, four vCPUs, mirror off,
DIAG_PROGRESS=1000, blocking sync, K=5 ms, two 120-second pairs per boot.
C also uses NEGCTL_NO_BUSY_LOOK=1; the infinite M arm also uses
WORKER_PARK_INFINITE=1. All knob names have the KF3_ prefix.

| Arm | Revision | Boots | Pairs | Gaps >=1/2/4 s | Max s | Strand starts | Final work counter summed across boots |
|---|---|---:|---:|---|---:|---:|---:|
| C | pw3f4b9e82-s2 | 8 | 16 | 5/5/4 | 12.59 | 8 | 0 |
| M | 89fc8a222 | 6 | 12 | 0/0/0 | 0.01 | 0 | 0 |
| M infinite | 89fc8a222 | 1 | 2 | 0/0/0 | 0.01 | 0 | 0 |
| B' | c452ca7cb | 2 | 4 | 0/0/0 | 0.01 | 0 | 1028 |

MEASURED: M registered==rung in each of the seven inherited boot summaries
(5342 registrations, 5342 rings, 4554 immediate retries total). These are final
cumulative summaries, never sums of successive summaries within one boot.
Worker timeouts_with_work was not printed in the inherited runs.

MEASURED, GPU-free on exact M: library run 566 passed, zero failed; broader
`cargo test -j2 --release -p kf-chan -p kf-qemu --tests` run 1023 passed,
zero failed, four ignored, 34 completed targets, exit 0. The library count is
included in the broader count. No source changes or grader overlay were used.

MEASURED: M library clippy exited 101 with 93 errors overall. Filtering primary
diagnostic spans to added lines against f8f9d1a5f found four
clippy::arithmetic_side_effects errors in userstrand.rs:93,115,123,132. Other
diagnostics were outside the added-line filter. Clippy is not clean.

CODE READ: hardware execution is stopped because inherited session3.sh has a
name-wide TERM pkill abort, and its fast-guest launcher has an unconditional
name-wide KILL pkill before startup. A name match does not prove PID ownership.
Neither script has been executed by HW2. Options reported: authorize a tested
lane-owned ownership repair, or stop hardware work and restore production.
The lane explicitly requires stopping and reporting a safety refusal.

CODE READ: review findings and the preregistered falsifiers are in the adjacent
files. INFERRED: zero inherited M stalls is promising, but the requested
12-boot minimum and exact-head hardware gates remain necessary before Windows.
