# Candidate hardware validation, pre-registration

**STATUS: RESEARCH, 2026-10-17 (written at about 17:12 CEST, before any Windows run and before any fast-suite verdict was read).**
Candidate under test: `integration/prod-candidate-3-20261017` at `525d7d7adadf647cfe9ded0fb388f066cc408c98` (phase-1c lifecycle, lazy observe ON by
default, doorbell contract, broker cursor work). Host: root@172.22.1.20, RTX 4070 (AD104), host driver 595.91.07, Windows guest driver 580.88 (as in the
earlier campaigns). Recipe: `traces/lifecycle_hw_20261016/REPORT.txt` (Linux) and `scripts/bench/d3d11_gap/` (Windows Heaven, Ultra 1080p windowed, 260 s).

Ordering, stated honestly: the binary was built, the 1,690 GPU-free tests and the 9 gates (plus gate 10) had already run when this file was written
(tests 1,690 / 0 / 3 ignored; gates 9/9, gate 10 PASS, births 12/12 USER; both are in `l1/` and `l2/`). Their expectations below are copied from the
recipe and the design doc, not derived from those results. Nothing else (no fast-suite arm, no Windows boot, no broker lane) had been run.

Labels used in the report: MEASURED (a run of this task, artefact named), CODE READ, INFERRED.

## Linux (the lifecycle and the lazy rules are unreachable from a Linux guest: CODE READ, pin test `lazy_observe_is_reachable_only_from_the_windows_user_ring_pump`)

| id | prediction | falsifier (result that proves it wrong) |
|---|---|---|
| L-a | `v3_gates.sh` 9/9, gate 10 PASS, births 12 USER / 0 refused | any other number |
| L-b | `fast_suite.sh` 30/30 in each of 3 candidate runs; the base (0a746117) 30/30 in 2 runs | any arm not PASS in any run (a flake is re-run once and reported, never hidden) |
| L-c | no `USER_RING_LAZY_OBSERVE*`, no `USER_RING_LIFECYCLE*`, no `pointer_pb_read_violation` line in any Linux QEMU log | one such line (finding) |
| L-d | host Xids in the suites: only the 5 Xid 31 per suite the frozen grader provokes on purpose, all mapped by PID to `defer-liveness` (4) and `missing-page-fault` (1) | any other Xid, or an unmapped one |
| L-e | summed client time of the candidate within 5 % of the base (A/B order A B B A A); the first run after a build may be 10 to 45 % slower on a few arms (recipe finding) | candidate sum above base by more than 5 % in the two later runs |
| L-f | every channel birth is `PRIVILEGED_CHANNEL=0 privilege=USER` (161 per suite) | any other birth |
| L-g | broker lane (interactive.sh, Cinnamon guest): boots, KMS, glxgears about 60 fps first window, guest-initiated poweroff clean, 0 lazy/lifecycle lines | any of those fails on the candidate while it passes on the base |

## Windows Heaven (D3D11 and OpenGL, same boot, Ultra 1080p windowed, in-app benchmark, 260 s window, fresh boot per run)

Arms (all from the SAME binary `kf3-bins/525d7d7a`, differing only in the launch environment; `KF3_PUT_CACHE=0` turns rule A, the GP_PUT cache, off; `KF3_LAZY_OBSERVE=0` turns rule B, the lazy private-USERD look, off):
`P` defaults (A+B on), `B-only` (`KF3_PUT_CACHE=0`), `A-only` (`KF3_LAZY_OBSERVE=0`), `OFF` (both `=0`: the phase-1c lifecycle alone).
Two boots per arm if time allows, interleaved. The environment of the QEMU is read from `/proc/<pid>/environ` of every boot; a run whose environment does not
match its arm is void and reported.

| id | prediction | falsifier |
|---|---|---|
| W-a | P, D3D11: about 297 fps (scratch A+B 299.1, 296.5, 299.2, n = 3; 16 GiB guest, 134dcb54) | **at or below about 220 fps: the port lost the gain** (design doc 0c). Between 220 and 280: partial loss, reported, not a falsifier of the port by itself |
| W-b | OFF, D3D11: about 196 (scratch mode 2: 193.1 to 202.2) | outside 175 to 215 |
| W-c | P, OpenGL: about 338 to 362 (scratch scan-off without lazy) | below 320 ("must not drop") |
| W-d | ordering: OFF < each single-rule arm <= P on D3D11 (no prior on which single rule carries the gain; the arms exist to measure it) | P not above OFF by at least 25 %, or a single-rule arm below OFF |
| W-e | no host Xid 13/31/32/43 mapped to this QEMU, no guest `nvlddmkm` 153 or `Display` 4101 beyond the boot-time events, no `USER_RING_REFUSED`, Heaven completes with its own score line | any of these (a guest-killing run is a P1 data point: Xid and app are recorded, no retry loop) |
| W-f | `USER_RING_LAZY_OBSERVE_MODE` line matches the arm (P: put_cache=true, lazy on, interval 16; B-only: put_cache=false; A-only: lazy observe off; OFF: both off) | a mode line that does not match the environment |
| W-g | P counters (per relay and SUMMARY): `turns == obs_skipped + obs_unconditional + obs_forced_*`; `put_cache_hits / turns > 0.95`; `obs_skipped / turns > 0.85`; `obs_forced_interval` about `turns/16`; `inval_reset == 0`; `put_cache_hits + put_fresh_reads <= turns` | any inequality violated on the big Pointer-mode channel (small channels are listed, not averaged away) |
| W-h | must be 0 in every arm: `put_boundary_open_scan`, `put_boundary_open_window`, `window_fresh_start_parse_error`, `switch_cursor_mismatch`, `pointer_pb_read_violation`, `window_binding_changed`, `anchor_stale_start`, `pointer_markers_latched_off`, `after_teardown`; absent: `USER_RING_POINTER_MARKERS_LATCHED_OFF`, `USER_RING_PUT_BOUNDARY_OPEN` | any nonzero / present |
| W-i | in the OFF and the two single-rule arms the counters of the disabled rule stay at their off values (OFF: `put_cache_hits == 0`, `obs_skipped == 0`) | nonzero |

Reading rules: fps is Heaven's own score-window average (not a screenshot); with n = 2 the two values are shown, never a "median"; a spread above 5 % between the two boots of one
arm is stated. The host memory state (MemAvailable, PSI) is recorded per boot; swap is off. Guest RAM 12 GiB (the owner's 16 GiB guest crashed the display; the scratch numbers
were taken at 16 GiB: that difference is a stated confounder, INFERRED to be small).

## Not in scope
No merge, no source change (a mechanical one-liner only if a bug blocks the run, flagged), no fat-guest lanes, no app matrix beyond Heaven, no native reference run.
