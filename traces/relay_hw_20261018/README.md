# HW2 relay hardware validation — 2026-10-20

STATUS: LIVE / RESUMED, 2026-10-21. The owner's amended briefing authorizes
freedom of method and tests-first repairs. The lane-owned harness repair is
green, with two killing tests for each of its mutants. A new C/M ABBA cohort is
running; M uses infinite park. Native/fast qualification and Windows capture
are queued. See `RESUME_20261021.md` for current results.

Historical preflight snapshot, 2026-10-20: BLOCKED / PARTIAL. No HW2 hardware test boot had been run. All hardware numbers below
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

MEASURED: exact B' holdout, chaos-hooks, strict soft findings, infinite park:
32/32 seeds [4294967296,4294967328) passed, zero soft findings, 7,251,066 entries,
94.5 s wall. Per-seed response maxima range 0.36–50.51 ms. The original
thresholds were unchanged. The dedicated grader G5 one-second idle test passed
for two and three workers (one test, 4.58 s), including its quiet_checked assert.
Neither result explains the inherited hardware timeout counter.

MEASURED: B' library clippy exited 101 with 93 errors overall; the added-line
filter found five errors and two warnings. Details for both heads are in
`clippy-added-lines.json`. Neither head is clippy-clean under this toolchain.

MEASURED: owner production VM was restored as run 3435, REV=0d7ba743, with
LIFECYCLE_COUNT=1, LIFECYCLE_PERIOD_S=5, TSG_MIRROR=0 (all KF3_ prefixed).
OWNER_SESSION_READY was logged at 2026-10-20T23:42:30+02:00. M hardware gates,
fast suite, remaining boots and Windows reproducer are NOT DONE.
MEASURED: the guest-agent hostname query returned kayfabe-windows.
MEASURED: final restoration inspection found host Xid count unchanged at 25,
QEMU alive, and all three production environment values as requested.

MEASURED: automatic command review rejected deleting the lane's target
directory ("rm -f style commands are not permitted. Use a safer approach").
The lane's stop-on-refusal rule was followed: no alternate deletion command was
attempted. The 256 MiB target directory remains. No HW2 cargo/test job remains.
The hardware campaign never acquired a lasting lease; the restored owner's
launcher holds its own intentional lifetime lease.

Windows capture wiring: the N1 source is pinned at
`7ede125d84aca37d5a69345bf8bd1f966596a461` on
`codex/night-repro-20261020`. Apply
`scripts/bench/baton/night_repro_hw2.patch` to a lane-owned copy, and copy
`hw2_trigger_relay.ps1` into its `scripts/bench/night_repro/` directory.
The observer takes that copy via `--scripts`, the lane receipt directory via
`--root`, and an existing read-only USERD probe module via `--userd-module`.
The latter must export `guest_ram(pid, argv)` and O_RDONLY `GuestMem.read`;
`--lowmem` is the verified QEMU RAM split, not a guessed GPU address.
Use `--perf` for bounded filtered scheduler captures. The standalone guest
`test_trigger_relay.ps1` sends synthetic inputs on separate port39022; the
recorded real PowerShell fixture result is `ps-relay-fixture.json`.
