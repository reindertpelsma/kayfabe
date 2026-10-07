# Merge bar for v3-sec-nonpriv at 55743ecd (2026-10-03)

**Box:** vast 54050499, RTX 3060 (GA106), host driver 580.159.04 (open kernel module).

**How it was run:** as root, from the checkout, exactly as the bench runs today. QEMU was not
started any differently.

```
CARGO_BUILD_JOBS=$(nproc) bash /root/kayfabe/scripts/bench/box/merge_check.sh v3-sec-nonpriv nonprivmc
```

The script's own log is `merge_check.log`.

## Results

| item | result | log |
|---|---|---|
| revision | `55743ecdf93c27b4b064034e5804792b994a0c60` | `merge_check.log` |
| every kf-* crate test | 1764 passed, 0 failed | `tests.log` |
| v3 gates | 9/9 | `gates.log` |
| kf3 built from this revision | KF3_RC=0 | |
| bare-metal raw-client suite | 30/30 | `bare_metal.log` |
| fast guest rebuilt | FG_RC=0 | |
| 30-arm thin-guest suite, kf3 | 30/30, 0 crash, 0 not run | `suite.out` |

## Channel births, QEMU as root

**The 30-arm suite** (log: `suite_births.log`, one line per birth, prefixed with the arm):

- 161 births. Every arm has at least one (`suite_births_per_arm.txt`).
- All 161 read `reply_flags=0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER
  cap_sys_admin=cleared-for-call`.
- Zero `PRIVILEGED CHANNEL REFUSED` and zero `CHANNEL BIRTH REFUSED`.
- Engines covered: GR (`0x1`) and copy engines `0x9`–`0x10` (`NV2080_ENGINE_TYPE_COPY0`–`COPY7`).

**The v3 gates** (log: `gates.log`):

- The gate binaries run as root and create channels through the same `kf-host` path.
- 11 births, all `PRIVILEGED_CHANNEL=0`.

The before and after runs for a single arm, and the negative control, are one directory up
(`../README.md`).
