# Merge bar for v3-gop-unload at d4c3767b (2026-10-03)

Box: vast 54049598 (RTX 3060, UK), host driver 580.159.04. Command, from the checkout:
`CARGO_BUILD_JOBS=16 bash /root/kayfabe/scripts/bench/box/merge_check.sh v3-gop-unload mb_gop`
(the box had `x86_64-unknown-uefi` added to the pinned toolchain first). Log: `mb_gop.log`.

| item | result |
|---|---|
| revision | `d4c3767ba373717eddf8da160d692df408225517` |
| every kf-* crate test | 1844 passed, 0 failed |
| v3 gates | 9/9 |
| kf3 built from this revision (GOP driver built by build.rs, embedded) | KF3_RC=0 |
| fast guest rebuilt | FG_RC=0 |
| 30-arm thin-guest suite | 30/30, 0 crash, 0 not run |

Merged into master as a merge commit whose only additions from master are documents (OWNER_RULINGS
§L–§P, V3_SECURITY_AUDIT_PLAN.md, the box README). The display box runs of this code are in
`../gop_final_20261003/` and `../gop_box_20261003/`.
