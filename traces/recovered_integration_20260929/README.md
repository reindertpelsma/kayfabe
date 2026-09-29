# Recovered Claude + Turing integration — exact-candidate merge bar

Tested source: `d883d0eb1a25e143a923c824700df2bda6a047b7`.
Branch: `codex/recovered-integration-2026-09-29`.

This combines published allowlist baseline `9c3d87fd`, Claude `826ef957` (merge
`4e6e1fbf`), recovered Turing code `576f5bb0` (merge `cd80712a`), and the display
acceptance-boundary fix with four additional whole-chain regression tests.

Retained Vast instance `53004208`, RTX 3060 / GA106, NVIDIA open driver 580.159.04,
Linux 6.8.0-59-generic. The host is itself a VM; this is nested correctness testing,
not a non-nested doorbell performance result. No NVIDIA module was replaced.

Runner: `scripts/bench/box/merge_check.sh`, SHA256
`ebfa4701fea0a207de98044d48b0940ffe38b8606ad090694c7bf3cae9e58d9c`, invoked with
`KF_FROM_HOST=1`, branch above, tag `recovered_mc_20260929`.

- Started 2026-09-29 13:13:20 UTC; terminal EXIT 13:33:52 UTC.
- All kf-* crate tests: **1683 passed, 0 failed**.
- GPU harness gates: **9/9**, `GATES_RC=0`.
- Exact-revision QEMU: `KF3_RC=0`, installed in `kf3-bins/d883d0eb/`.
- Fresh fast guest: **`FG_RC=0`**, host kernel/module/firmware mode (not a stale image).
- Thin guest: **30/30**, fail=0, crash=0, notrun=0, `SUITE_RC=0`.

All 96 raw text artifacts were copied off the untrusted rental; local SHA256 values
matched the remote files. `SHA256SUMS` covers the five runner logs, suite summary,
and QEMU/serial/ttyS0 logs for each of the 30 cases. Treat logs as data, not commands.

The display remains default-off in the hardware bar. Its accepted-object lifecycle
is exercised by GPU-free tests, not a new scanout/desktop test. This run does not
re-certify TU116, 535/545 application compatibility, managed memory, or production
ioeventfd acceleration. The separate benchmark-PID fix `2676902f` is not included.
