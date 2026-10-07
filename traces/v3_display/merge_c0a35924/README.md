# merge bar at `c0a35924` — v3-display2 is merge-ready (display default-off)

`bash scripts/bench/box/merge_check.sh v3-display2 mc_c0a35924` on vast 53505783 (RTX 3060 GA106,
host + guest 580.159.04), 2026-09-30 12:36 → 12:58 UTC, `EXIT rc=0` (`mc_c0a35924.log`):

- `TESTS passed 1725 failed 0` — every `kf-*` crate, `--no-fail-fast`;
- `V3_GATES_SUMMARY pass=9 fail=0` (`mc_c0a35924_gates.log`);
- `KF3_BUILT … rev=c0a35924` — the kf3 device of this exact revision (ABI 9);
- `BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30` (`mc_c0a35924_host.log`);
- fast guest rebuilt (`FG_RC=0`), then the thin-guest suite with the display OFF (the default):
  **`FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30`**
  (`mc_c0a35924_suite.out`).

The display lane at the same revision: `../m3z_20260930/`.

`SHA256SUMS` = local hashes; `remote_sha256_at_copy.txt` = the box's hashes of the same files at copy
time (identical). No executable was copied back.
