# v3-mc23 — master `0d282154` + `v3-cdp` (`84df5b43`), merge `afb552ea`

**STATUS: DATA, 2026-09-30.** Retained RTX 3060 (vast 53004208), host + guest 580.159.04 open.
The fat-guest image was fsck'd first (both partitions clean: `mc23_chain.log`).

- **Merge bar at exactly `afb552ea`** (`merge_check.sh v3-mc23 mc23`, script from `0d282154`): crate tests
  **1754 / 0**, gates **9/9**, KF3_RC=0, bare-metal suite **30/30**, FG_RC=0, thin guest **30/30**,
  `EXIT rc=0` 20:04:13Z. (`mc23.log`, `mc23_*.log|.run`)
- **CDP smoke on the merged binary** (`scripts/cdp/cdp_guest.sh`, `kf3-bin-rev:afb552ea`, one fresh fat-guest
  boot): probe modes 1–4 `child_ran=1 … RESULT OK` (NULL stream, fire-and-forget, tail launch, device-created
  non-blocking stream); mode 0 (parent only, the control) `child_ran=0 … RESULT OK`; `cdpSimpleQuicksort`
  default and `-num_items=10000` both `Validating results: OK`, rc=0. (`mc23_cdp.log`, `mc23/`)

The app matrix with this fix (61/65 + 6/6, host 71/71) was measured at the branch revision `2830988f`
(`traces/v3_cdp/`); `afb552ea` adds the RAM-object fix and documentation to that code.
