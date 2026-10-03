# Merge bar for v3-scratch-bound at 69ccb08c (2026-10-03)

Box: vast 54049598 (RTX 3060, UK), host driver 580.159.04, provisioned with `provision_full.sh` on this
branch. Command, run from the checkout: `CARGO_BUILD_JOBS=16 bash /root/kayfabe/scripts/bench/box/merge_check.sh v3-scratch-bound mb_scratch2`.
Its log is `mb_scratch2.log`.

| item | result |
|---|---|
| revision | `69ccb08cfce47d8d1c6963fea1b3599f69fd9917` |
| every kf-* crate test | 1777 passed, 0 failed |
| v3 gates | 9/9 |
| kf3 built from this revision | KF3_RC=0 |
| fast guest rebuilt | FG_RC=0 |
| 30-arm thin-guest suite | 30/30, 0 crash, 0 not run |

The first attempt, from a copy of the script in `/root`, exited at once ("not a git repository"); the
box README was fixed on master in `7c9234d2`.
