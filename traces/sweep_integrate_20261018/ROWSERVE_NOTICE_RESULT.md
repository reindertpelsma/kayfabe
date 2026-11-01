# Constant-responder notice bound result

STATUS: LIVE, 2026-10-19. Implemented and verified GPU-free.

MEASURED (2026-10-19): EXPECTED RED9aded4379 has two runtime failures:2048actual lines
versus64expected, for repeated counters and changing branches. Fix44e870e4d
bounds retained state at64 entries and emits one limit marker. Both real
responder tests and two independent in-memory quota/dedup tests pass.
Three mutants (no bound, per-call rearm, repeated marker) each fail both
responder tests. Replies remain refused in all2048cases; no host action added.
The full current rm suite passes811tests including additional scope/metadata
coverage; candidate4 had750, candidate5 had750. All original tests remain.
Clippy adds no diagnostic on changed lines. Raw logs stay local; mutation hashes
and final per-crate counts are committed. This is not a hardware result.
