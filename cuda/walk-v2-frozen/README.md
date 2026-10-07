# cuda/walk-v2-frozen — the OLD tree's walk kernel, frozen

**STATUS: FROZEN, 2026-09-25 (branch `v3-diff`).** A byte-for-byte copy of `cuda/walk/kf_walk.{cu,h,ptx}`
and `kf_report_emit.c` as of `origin/v3` `0a160447` (`KF_ABI_VERSION 2`: the walk-vs-previous-walk delta).

The frozen `kayfabe-*` crates (reference + grader, `V3_BUILD.md`) embed and differential-test this copy:
`kayfabe-cuda` (`include_bytes!` of the PTX, `walk_abi_matches_the_cu`), `kayfabe-isolate-host`'s
`build.rs`, and `kayfabe-mmu`'s `walk_report_seam`. v3's `cuda/walk/` moved to `KF_ABI_VERSION 3`
(the commit-on-ack diff); ⊘ without this copy the old tree would embed a PTX whose `KfArgs` is 256
bytes against its 216-byte mirror — a launch of garbage, not a refusal. Do not edit; do not regenerate.
