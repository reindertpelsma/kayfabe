# P1 verdict

STATUS: RESEARCH, 2026-10-16 02:15 CEST. PARTIAL: merge approval is not established.

Target source `0a74611734dc91c78c86ff39ed9f06d866072ac4`, clean independent host
checkout `/data/codex-perf/base`. Host RTX 4070, driver 595.91.07.

| check | result | evidence |
|---|---|---|
| local kf-qemu tests | 290 passed, zero failures | private local logs/p1-cargo-test.log |
| host kf-chan + kf-mem tests | passed, exit 0 | /data/codex-perf/logs/p1-chan-mem.log |
| hardware gates | 9/9; gate 10 PASS; 12/12 USER births | /data/codex-perf/logs/p1-v3-gates.log |
| fast suite | 30/30, no failures/crashes/NOTRUN | /data/codex-perf/fastwork/codex_fix_0a746117_suite.out |
| Linux broker desktop | boot, KMS, Cinnamon and LINEAR dma-buf passed; cursor/window NOT GRADED | /data/codex-perf/logs/p1-linux-driverfixed.log |
| interactive.sh separate retest | not run within P1 time box | pending |
| Windows Heaven x3 per API | reused Claude's exact-revision evidence; not independently repeated | /data/claude-perf/ev/heaven/f_* |

All hardware sessions held the global GPU lock. Shell KF3_* environment was cleared;
the initial Linux attempt lacked an explicit guest-driver property and refused at realize
(defaulted to host 595.91.07, which lacks the derived display row). This is a harness
precondition failure, not a guest regression. Corrected launch: guest-driver=580.159.04,
fb-mb=8192, display=on,gop=on,x11-dispsw=on; guest 580.159.04, host 595.91.07.
No Xid/Flip-event-timeout/BUG/WARNING line was found in the captured guest dmesg-after.
The hook returned early because its X11 window search found no broker window under GNOME
Wayland; its zero exit does not prove the unexecuted cursor tests.

Claude's measured production baseline (not new measurements): Windows guest NVIDIA 580.88,
host 595.91.07, build 0a746117, no KF3_* flags: D3D11 97.65 fps (96.04..102.44)
against VFIO 330.05; OpenGL 222.5 (215.3..222.7) against VFIO 341.9.
Ratios 29.6% and 65.1%. Source: committed Claude report at 9b2d7fe1b,
`docs/design/V3_WINDOWS_PERF_20261015.md`, rows f_d3d11_r1..3 and f_gl_r1..3.
Actual process-environment provenance was not recollected for those inherited numbers.

A fresh own Windows overlay was launched at 02:13 with build 0a746117 and empty PERF_FLAGS.
Readiness check showed Heaven and C:\kfperf\kfp_launch.ps1 absent. No benchmark was run;
no production performance claim is made from this boot. Stop requested at 02:15.
P1's 90-minute wall-clock limit had elapsed, so staging/repeating Heaven was not widened.
Startup was 00:19; host P1 began 00:31. There was a long gap before work resumed around
02:13; timestamps are recorded rather than treating that gap as test time.

Remaining merge checks: real broker window/cursor and interactive retest; optionally independent
Heaven repeats once benchmark staging is restored. No merge or mergeable verdict is claimed.
