# Perf parity preregistration

STATUS: RESEARCH, 2026-10-16, start 00:19 CEST; stop by 08:00 CEST.

Branch `codex/perf-parity-20261016`, base `0a74611734dc91c78c86ff39ed9f06d866072ac4`, clean at start.
No merges. All host work uses the owner's GPU/VM lock and own directories.

Startup order: read rules/evidence; GPU-free P1 crate validation, P2 design/prototype/model/stress,
and P3 microbench preparation before taking hardware. Claude's final report did not exist at
00:19 CEST; reread later. Its gates directory was empty at 00:24 CEST. Copy templates only.

Falsifiers recorded before runs:

1. P1: any failing touched-crate test, hardware gate, fast arm, broker/interactive lane, or
   >15% contradiction of the supplied performance results prevents a mergeable verdict.
   Require exact revision provenance and three Heaven repetitions per API with production flags.
2. P2 correctness: any wrong byte, read after retirement, lost resource ownership, reader wait,
   unfair cross-owner eviction, or model counterexample rejects the implementation. Full mapping
   must fail at realize by name if unavailable. View accesses above 256 bytes are refused or
   partitioned by their command-source adapter; whole command limits remain independently bounded.
3. P2 performance: the simplification is not a speedup claim. A speedup requires >10% improvement
   against the same production baseline with matching settings and recorded spreads. Existing
   FULLMAP experiment evidence must be read first; do not repeat that experiment.
4. P3: a wider/access-overlap/WC candidate loses if it fails exact byte comparison for aligned,
   unaligned, boundary, and partial-length reads, or does not lower measured latency versus the
   current copy implementation. Do not introduce unsupported CPU instructions into production.

Time boxes: P1 90 minutes of active work; P2 4 hours; P3 90 minutes (hardware microbench <=1 hour);
then P4/P5 only as time allows. Host contention is recorded, never cleared by killing others.
Stop and report immediately for an isolation hole, data-loss risk, >15% contradiction, or Linux regression.

Initial limitation: dev filesystem has only 2.0 GiB free. Local cargo uses the global cargo lock,
two jobs, an owned target directory, debug information disabled, and no incremental output;
remove owned artifacts after local validation. Do not consume shared build directories.
