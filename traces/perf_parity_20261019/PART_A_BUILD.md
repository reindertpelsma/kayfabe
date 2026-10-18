# P1 Part A host binary

STATUS: MEASUREMENT, 2026-10-19 01:39 CEST.

MEASURED: the documented `scripts/bench/build_kf3.sh` flow completed on the trusted host, CPU-only at nice 19, two build jobs, source `2566db014c342730008efaa93f85afd148e3582e`. Immutable output: `/data/codex-P1/kf3-bins/2566db01/qemu-system-x86_64`. SHA-256 `a00fa6ca9593c29b1b2bd846ff28a5aa3b35f28fad463482c169053c805bbf81`. The adjacent REV and BUILD_FLAGS files record provenance. Full build log stays in the lane host directory.

CODE READ: this binary adds Cargo feature `diag-nsi`; `KF3_DIAG_NSI_TRACE` remains unset/off until an explicit observation run. It is a measurement binary, with no production promotion claim. Existing binary directories were not replaced. No GPU lock, GPU workload, guest command, or VM start was used by P1 for this build.

MEASURED: the restored diagnostic release suite passed 322 tests, zero failures, zero ignored. Baseline Clippy and candidate Clippy have identical diagnostic multisets: 89 errors and 364 warning occurrences each, added=0 and removed=0. See `clippy_comparison.json`; inherited diagnostics were neither suppressed nor changed.

MEASURED: the initial build invocation supplied a mistyped full revision; the revision equality guard refused it before checkout/build/output. Re-running with the SHA obtained directly from `git rev-parse HEAD` produced the binary above. This was an input transcription error, not a failing build or weakened guard.

MEASURED: at the last readiness inspection the TSG DONE marker and requested repository README were absent. Part B remains gated; no hardware result is claimed here.
