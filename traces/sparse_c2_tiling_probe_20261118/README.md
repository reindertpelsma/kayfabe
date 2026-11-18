# kf-va-tiling-probe: what option C2 (exact block tiling) costs host RM

**STATUS: LIVE, 2026-11-18.** Text evidence for `docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8.

- Run: one bare Vast KVM box (`kf-openguest-180543`), RTX 3090 (GA102), host driver 580.159.04 (open module),
  kernel 6.8.0-59. No guest and no GPU work submitted. Rented 05:43Z, destroyed 06:05Z by its own id
  (absence verified). 0 Xid lines before or after any arm, and 0 RC-watchdog asserts after the driver loaded.
- Revisions: `probe_run.log` is `62b063433` (all arms, 1000 rows); `probe_run_rows.log` is `c07bd23dc` (M0/M1 at
  2048 and 8000 rows; the tool is unchanged apart from the row-count argument). Both are on branch
  `claude/sparse-c2-20261118`.
- Files: `provision.log` (provisioning RC lines), `probe_run.log` and `probe_run_rows.log` (the `TILING` lines,
  unfiltered).
- The arms and their falsifiers are in the tool's header (`crates/kf-harness/src/bin/kf-va-tiling-probe.rs`).
