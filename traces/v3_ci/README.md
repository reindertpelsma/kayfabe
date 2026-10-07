# v3-ci merge bar — `bf6e7640` (Codex's CI repair + the sandbox-test SKIP gate)

**STATUS: DATA, 2026-09-30.** Retained RTX 3060 (vast 53004208), host 580.159.04.
- GitHub CI run 36698237044: stable (build, test, clippy, fmt, boundary) ✓, aarch64 ✓ — the first green CI of v3.
- Hardware bar (`merge_check.sh v3-ci mcci`): 1687 passed / 0 failed, gates 9/9, KF3_RC=0; the fast-guest
  rebuild failed (FG_RC=1), so the suite was RE-RUN on a thin guest rebuilt at this exact revision
  (`mcci_fg4.log`, FG_RC=0): **30/30** (`mcci_suite2.log`).
- Why the rebuild failed: the shared fat-guest image's root and /boot ext4 journals needed recovery after an
  earlier guest's unclean shutdown ("recovery required on readonly filesystem"); `e2fsck -p` on both
  partitions fixed it (see scripts/bench/box/README.md).
