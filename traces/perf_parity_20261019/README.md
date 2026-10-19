# P1 Steel Nomad parity

STATUS: Part A validation/build in progress; 2026-10-19 02:48 CEST.

CODE READ: hardware baseline is the mirror source `c2685412e` / binary `0d7ba7438`, as directed by the coordinator. P1 imports its build inputs verbatim without merging or changing TSG implementation. The effective crate diff is limited to 12 paths in kf-qemu/kf-chan. See `BASELINE_UPDATE.md`, `PREREG.md` and `CODE_READ.md` for source provenance, hypotheses and falsifiers.

MEASURED: mirror baseline release tests total 929 passed, zero failed, three existing ignored. P1 diagnostic-feature full profile passes 960 tests, zero failures and three existing ignored; focused diagnostic source/runtime tests pass 7+13; GET relay properties pass 38, including the unchanged randomized off-mode fingerprint. The full default profile is running; its total and default Clippy comparison will be published before hardware. All 14 mutants fail in at least two tests each (`MUTATIONS.json`).

MEASURED: complete diagnostic-feature Clippy matches the mirror baseline exactly: 89 inherited denied errors and 364 warning occurrences, added=0, removed=0 (`CLIPPY.json`). This follows the coordinator's explicit changed-lines gate; none of the inherited diagnostics is suppressed or fixed. Rustfmt passes on P1 changed lines. Default-profile Clippy is queued after the full tests.

CODE READ: compile-time features `diag-nsi` and `diag-get-writeback` default off. Runtime `KF3_DIAG_NSI_TRACE=1` enables a bounded, nonblocking worker recorder; its private dump and live-counter files are written only by the existing diagnostic drainer. `KF3_DIAG_GP_GET_FULL_ONLY=1` enables the separate GET measurement candidate; it retains real guest write-back progress and flushes before work when the normal doorbell-driven turn observes a full ring. Neither experiment changes a default, timer, poll or wake source.

CODE READ: M1 DONE/report is present, with its hardware gates UNRUN. Production run 3009 still owns the GPU. P1 has performed no hardware measurement and will acquire the shared lock only after its source validation and mirror-based immutable binary build. The earlier production-based P1 binary is ineligible for these A/B runs. Morning production promotion remains subject to every original lane criterion.

CODE READ: diagnostic use: choose a P1-owned request path in `KF3_DIAG_NSI_TRACE_DUMP`. Appending bytes to `<path>.stats` yields checkpoint ids in `<path>.stats.out`; creating `<path>` freezes the bounded tail once into `<path>.out`. Require a complete END marker and inspect trace losses. The source fd combines FIFO and configured host-copy events; identical vectors do not prove redundancy. The harness stops further performance arms if unconditional `backstop_found_work` increases. Host logs stay in the lane directory; only small text/PNG evidence is committed.
