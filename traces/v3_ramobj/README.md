# v3-ramobj — merge bar of `f442980e` (the guest-RAM object never caches a transient "no RAM")

**STATUS: DATA, 2026-09-30.** Vast 53562843, GTX 1660 SUPER (TU116), host 580.159.04 open.
`merge_check.sh v3-ramobj tur1`, verify worktree HEAD `f442980e`: crate tests **1744 / 0** (the two new
`kf-qemu` tests included), gates **9/9**, KF3_RC=0, bare-metal suite **30/30**, FG_RC=0, thin guest
**30/30**, `EXIT rc=0` 18:30:13Z. `NO_FD_LINES_TOTAL=0` over the 30 arms' QEMU logs.

What this does and does not show: the defect (`docs/STATUS_AND_HANDOFF.md` §4.9) hit **1 of 390**
archived fast-suite boots on the RTX 3060 bench, so 30 clean boots cannot demonstrate its absence. The
fix is proven by construction and by its unit tests (`a_not_yet_precondition_is_never_remembered`,
`a_build_refusal_is_remembered` in `crates/kf-qemu/src/mem.rs`). This bar shows it regresses nothing.
