**STATUS: RESEARCH, 2026-10-22 — public reset contract checkpoint; no runtime change.**

CODE READ: branch starts at commit `1c6452a276d7014bece3adf42d68cbd4edffbc18`
(PR #71). Source findings, the unresolved lifecycle decision, and the blocked
hardware-arm outline are in `docs/design/V3_TRANSLATED_RC_RESET_20261022.md`.

MEASURED 2026-10-22 baseline: `cargo test --release -j2 -p kf-rm -p kf-qemu
-p kf-harness` finished successfully: 1263 passed, 0 failed, 0 ignored, 100 targets.
The invocation used the lane target, nice 19 and the required shared cargo flock.
Clippy over the same three crates with `--all-targets` finished successfully:
271 warning sites, 0 errors. Hardware n=0. No runtime edits or mutants were made.

MEASURED 2026-10-22 claims baseline: 3924 diagnostic identities; 167 are already
outside the repository's recorded debt at the branch start. This checkpoint does
not change that debt file or assert that the repository-wide claims gate is green.
New identities are compared with the saved branch-start identities, separately
from that inherited gate result. See `baseline.json` for commands and counts.
