# Turing (TU116) at current master — merge bar + CUDA ladder, 2026-09-30

**STATUS: DATA.** Vast 53562843, GeForce GTX 1660 SUPER (TU116, Turing), KVM template, Linux 6.8.0-59;
host driver re-provisioned from the template's closed 575 to **580.159.04 open** by
`scripts/bench/provision_host_driver.sh`; guest driver 580.159.04. Provisioned with
`scripts/bench/box/provision_full.sh` at master `a295e2ce` (READY: DRIVER_RC=0, TREE_RC=0, FG_RC=0,
KF3_RC=0; ~7 min).

- **Merge bar** (`merge_check.sh master tu1`, verify worktree HEAD **`1915bd71`** — the code of `3f67ed95`
  plus documentation only): crate tests **1742 / 0**, v3 gates **9/9**, KF3_RC=0, bare-metal suite
  **30/30**, FG_RC=0, thin guest **30/30**, `EXIT rc=0` 18:08:32Z. (`tu1*.log`, `tu1_suite.run`)
- **CUDA ladder** (`scripts/bench/cuda_ladder.sh`, rev `a295e2ce`, same code): host **4/4**, fat guest
  **4/4** — cup2 `CE rv=0xabcd1234`, cup3 `CUP3_VAL=43`, cup8 `CUP8_BAD=0 CUP8_MAXERR=0`, cup8bench
  every timed iteration verified. (`tu_ladder_{host,guest}.log`)
- `prov.log` is the provisioning record (includes the build's pre-existing lint warnings).
  `fast_tu1_timer_*` is one thin-suite arm's QEMU and guest logs, kept as a sample.

This is the first Turing hardware run of the merged v3 code (display, doorbell fast path default-off,
CI repair, recovered Turing fixes). It supersedes "no Turing hardware run certifies the current master"
in `docs/STATUS_AND_HANDOFF.md`. One die (TU116) only; TU10x/TU117 are not measured.
