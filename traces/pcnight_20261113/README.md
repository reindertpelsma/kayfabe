# pcnight 2026-11-13: why Superposition in the kayfabe Windows guest shows a flat 60 FPS and a low score

STATUS: 2026-11-13, RESEARCH. Run on the PC (RTX 4070, host driver 595.91.07, guest 580.88) with the proprietary image, offline,
a fresh throwaway overlay of image.base.qcow2 per arm, kf3 binary `kf3-bins/0ad35b8d`, source revision of the scripts: this branch
(claude/pcnight-20261113, based on 389938621). Evidence tree: coordinator `/workspace/bench/pcnight_20261113/` (per-arm kf3 `display fps`
logs, nvidia-smi dmon, screenshots, `RESULTS.tsv`); the scripts used are in `scripts/bench/pcnight/`.

Method: Superposition 1.1 Basic, 1080p Medium, DirectX, fullscreen (launcher defaults), RUN clicked by in-guest SendInput from a
scheduled task; no input or interactive task while it runs (a screenshot task during a run steals focus and aborts the benchmark: MEASURED,
first attempt). Score and FPS read from the result screen by eye. Host GPU: `nvidia-smi dmon` 1 s. Xid: host dmesg delta (0 in every run).

| arm | what changed | tick (kf3 `armed`) | runs: score (min/avg/max FPS) | host GPU sm% |
|---|---|---|---|---|
| A control | sample flags (19, as the owner's run) + KF3_RPC_TRACE=1, wayland broker | 60.000 (src=broker) | 8005 (40.88/59.88/61.19), 8002 (40.76/59.85/62.00), 7998 (39.11/59.83/60.73), 8004 (40.74/59.87/61.99) | 39-42 |
| B | A with KF3_RPC_TRACE=0 (the 19 sample flags stay: all are class B in V3_FLAG_INVENTORY, none is a measurement flag) | 60.000 | 7988 (42.09/59.75/61.26), 8000 (39.16/59.84/61.54) | 42-44 |
| C | B + `display-max-fps=75` + X11 broker backend (it does not offer the tick bit, so kf3's own clock ticks) | 70.910 (src=free) | 9433 (46.06/70.56/77.31), 9436 (46.27/70.58/73.64) | 46-49 |
| D | C but `--backend test` + NB_TEST_NO_TICK=1 (no window, no presentation) | 70.910 | 9429 (44.57/70.53/72.95), 9440 (41.67/70.61/72.51) | 42-45 |
| E | C with `display-max-fps=30` | 29.938 | 4002 (27.95/29.93/30.45), 1 run | 35 |
| Time Spy (not display-bound) | A flags, wayland | 60 | total 7298 (1 run) | |
| Time Spy | B flags, test backend, 71 Hz | 70.9 | total 7389 (graphics 7475, CPU 6938), 7379 | 43 avg, max 52 |

Also MEASURED in every Superposition arm: score = 133.7 x avg FPS (all runs, to 0.1 %), so the score is the average FPS and nothing else;
`display fps` shows `kms` (guest flips/s) about 2 x copies (copies about equal to FPS); display `over=0`, `clamped=0`.

## Verdicts (falsifiers stated before the runs, in the task)

- **H3 (the app presents against the emulated vblank): CONFIRMED.** FPS tracks the kf3 tick across 30 / 60 / 71 Hz (29.93 / 59.8 / 70.6).
  The falsifier (FPS unchanged when the tick moves) did not fire. Superposition Basic therefore behaves as vsynced to the guest display.
- **H1 (display pacing caps the rate): CONFIRMED for Superposition, with the precise cause.** The "cap=75(unset)" term is not what holds
  60: the tick is the host window's refresh (59.996 Hz, `src=broker`, `--refresh auto`) because the broker is the tick source. Lifting
  it as far as the existing knobs go (`display-max-fps` max 75, owner ruling D3; clock-driven tick) gave 70.9 Hz, not 75 (the size fit
  lowers the mode) and +18 % score. Nothing in the existing knobs removes the bound; flips were never parked (no mode was invented).
  It does not explain the score as "half of normal": the display was the bound, but...
- **H2 (measurement flags cost throughput): FALSIFIED.** A vs B: 8002-8005 vs 7988-8000 (inside run noise, 0.2 %). Time Spy A 7298 vs
  B 7389/7379 (1.2 %, one A run). RPC_TRACE off changes nothing measurable.
- **Not explained, MEASURED fact worth the next look:** the host GPU is only 35-49 % busy (dmon sm%) in every arm, in Time Spy too (43 %
  avg, 86 W), and Time Spy graphics 7475 is not display-bound at all (headless 71 Hz arm = flat 60 Hz arm within 1.5 %). So the
  low Time Spy score is NOT the display path or the flags. INFERRED (not tested): the guest's GPU feed (relay/submission path) limits
  throughput; sm% is "any kernel running", not occupancy. Next experiment: GPU-bound workload with the relay counters, same boot.

Not run: H1 with the cap beyond 75 (refused by design), `KF3_*` flags dropped one at a time (unknown-needed ones may break boot),
a native Windows reference on this die, Task 2. Superposition's engine config / `-video_vsync 0` was not touched (CLI/engine args
bypass the edition gate; the owner's rule excluded it).

Notes: the first A boot's kf3 logs were lost by my own `rm -rf` of the arm dir before a rerun; A runs 3-4 (second boot) carry the
logs. The A1/A2 scores are from screenshots kept in `shots/`. `unreal_engine-01` is a SPECviewperf viewset (Windows guest), not a
Linux app; Task 2 was cancelled by the coordinator.
