**STATUS: LIVE, 2026-10-22.** Additional GPU-free guard correction after lease closure.
No hardware validation; every Windows arm on the closed lease remained UNRUN.

CODE READ (commit 007ba1eb0:4): quoted historical statement, evidence not revalidated by BW9. CODE READ/CPU MEASURED: a1000-RPC prefix alone was insufficient to prove actual Windows init.
Two real watcher regressions accepted the otherwise exact diagnostic Xid32 without Code0,
or with Code43. The first tests-only944219ff7 exposed a mock-serialization error; corrected
tests-onlydc9d307db establishes the intended two EXPECTED RED assertions before the fix.

The watcher now requires both the consecutive RPC prefix and host-recorded actual CIM Code0,
driver32.0.15.8088, tied to this arm/PID/starttime. The later of their host observation times
must precede each exception event. Missing/invalid proof refuses the exception. The host wrapper
records the actual single BW580_RESULT output only after the diagnostic completes, clears stale
proof for nonzero code/wrongdriver/TDR, and checks the current owned VM identity.
The future-run arm kit invokes this recorder after its first diagnostic; it requires the updated
independent guard deployment and deliberately has no fallback to an older source.
Native32 and every out-of-scope event remain fatal. No new Xid exception was added.

CODE READ (commit 007ba1eb0:18): quoted historical statement, evidence not revalidated by BW9. MEASURED: full Python suite72->86 executions (66->80distinct cases),0failures.
Four additional mutants each have at least2distinct killing tests (2/2/4/2); restoration verified.
Earlier14mutants are retained; total18tested variants. No Rust or product changes; clippy/rustfmt
not applicable to this Python/shell bench topic. Shell syntax passes. Hardware remainsUNRUN.
