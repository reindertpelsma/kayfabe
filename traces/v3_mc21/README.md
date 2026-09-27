# v3-mc21 merge bar — evidence

**STATUS: LIVE, 2026-09-27.** The full merge bar (`scripts/bench/box/merge_check.sh`) for merge candidate
v3-mc21 = master `db038f5f` (v3-mc20, bar `traces/v3_mc20/`) + `origin/v3-display` (`adbea28e`: display
design, M0 behind `display=on` — **default off** — the derived display layouts/classes/model, unwired, and
a VNC+pixman kf3 build). Merge commit `4c48ca0c` (textually clean), measured at exactly `4c48ca0c`.

- **Box:** vast 53004208, RTX 3060 (GA106), EPYC 7K62 host, nested KVM, host 580.159.04 open — the same box
  and provisioning as v3-mc20. Driven through execd/`vx` (no SSH from the cloud session).
- The kf3 build dir was **reconfigured** (`== build dir … was configured with other flags: rebuilding it`,
  `mc21_kf3.log`): v3-display's `build_kf3.sh` adds `--enable-vnc --enable-pixman`; the configure line is
  now recorded in `qemu-build-kf3/.kf3-configure` and a mismatch forces a clean rebuild.

| Bar item | Result | File |
|---|---|---|
| every `kf-*` crate test (`--no-fail-fast`) | **1651 passed / 0 failed** (mc20: 1639; +12 = kf-disp and display tests) | `mc21.log` |
| v3 gates | **9/9** | `mc21_gates.log` |
| kf3 build of this revision | `KF3_RC=0`, `kf3-bins/4c48ca0c/qemu-system-x86_64` (VNC+pixman) | `mc21_kf3.log` |
| raw client + fast guest rebuilt | `FG_RC=0` | `mc21_fg.log` |
| 30-arm thin-guest suite, budget 180 | **FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0** | `mc21_suite.out` |

Bar wall time 19:39 → 19:59 UTC. The suite runs with the display plane at its default (off); the
display lane itself (`display=on`) is a separate measurement (`traces/v3_display/`).
