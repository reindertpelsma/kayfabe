# P1 resumed under the clarified changed-line Clippy gate

STATUS: ACTIVE, 2026-10-19 01:35 CEST. This supersedes the earlier mandatory-stop report and its recommendation to repair inherited lint sites.

CODE READ: the coordinator's 01:30 instruction explicitly overrules the Clippy stop: compare the base and candidate count/set, require zero added diagnostics, and do not repair the 89 inherited denied diagnostics. All other lane rules and the 06:30 hardware stop remain in force. Target cleanup is authorized for P1's own directory; if deletion is rejected again, retain/report it.

MEASURED: production `0452613b5` was checked in P1's own worktree at detached HEAD, then P1's branch was restored. Both commands used pinned Rust 1.99.0, release, nice 19, two jobs, kf-qemu/all-targets and JSON output; the candidate additionally enables `diag-nsi`. `compare_clippy.py` compares the full diagnostic multiset (level, code, path, normalized primary source text and occurrence count), excluding shifted line numbers. Both complete outputs contain 89 error and 364 warning occurrences; added=0, removed=0, exact multiset equality. Both cargo exits remain 101 because of inherited denies; no lint was suppressed. The clarified gate passes. Machine-readable evidence: `clippy_comparison.json`.

MEASURED: after recorder restoration and final formatting, the diagnostic-feature kf-qemu release suite passed again, 322 passed / 0 failed / 0 ignored. Earlier default suite: 316/0/0. The four two-test mutation kills remain recorded in `TEST_RESULTS.md`.

CODE READ: Part A continues with an immutable measurement binary using the documented build flow, the optional Cargo feature `diag-nsi`, and a separate default-OFF runtime `KF3_DIAG_NSI_TRACE=1`. An optional host request-file path (`KF3_DIAG_NSI_TRACE_DUMP`) freezes/dumps the bounded tail from the drainer. The production/default Cargo feature set excludes these hooks and storage. The measurement-feature binary is not a production promotion claim.

MEASURED at 01:31: the TSG agent's run 3014 and its GPU flock were active, with binary revision `0d7ba743`. Origin branch head was `0d7ba74389cd85ac65826417ad53f5c0bcd1ef2f`; the requested `traces/tsg_mirror_20261018/README.md` was absent in that commit, and DONE was absent on the host. P1 did not touch its VM or lock. Re-read the report and readiness marker before any W1 test. If it still owns the GPU at 04:30, wait up to 06:00; never break the lock.

CODE READ: the shared completion fd cannot distinguish FIFO from its additional host-CE subscription. The diagnostic labels it CompletionFd and preserves that ambiguity. Its records are relay decisions, not raw hardware interrupts; existing host IRQ/irqfd/MMIO tracing and guest ETW remain necessary for W2 attribution. No source subscription, delivery policy, timer or poll interval changed.
