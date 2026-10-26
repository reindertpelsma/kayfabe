**STATUS: LIVE, 2026-10-22.** Owner classified the previous shutdown census failure as an
exit-in-flight harness race and explicitly authorized this bounded correction plus one new lease.

Tests-only EXPECTED RED `3cf02304b` follows the unused immediate-census seam `595455d26`;
fix `e16402b7c` makes all11 new cases green. Complete Python suite86 ->97 executions /0failures.
Four mutants are killed by at least2 distinct tests each (3/2/4/4); source restoration is hashed.
No Rust/product code changed, so clippy/rustfmt are not applicable; shell syntax passes.

The host wrapper pins the owned PID/starttime/executable. After stop, it waits at most60s for
that VM to exit, then reads the process list initially and retries at most3 times5s apart.
It accepts only a fresh empty final list and identical Xid history before and after every read.
A still-live owned VM after60s, remaining QEMU after the reread bound, unreadable census or any
new Xid remains fatal. The parent is sealed only after CLEAN is recorded; no process is killed
by this observer, and no guest/device timeout or vCPU/drainer behavior changes.

The separate actual-Code0 proof from `../../blackwell_validation_20261022/guard_code0/` remains mandatory: RPC count alone
cannot authorize the diagnostic Xid32 exception. Native32 is always fatal.
