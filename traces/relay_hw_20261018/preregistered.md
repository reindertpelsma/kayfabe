# HW2 preregistration — 2026-10-20

CODE READ, amendment 2026-10-21: the owner's complete overnight briefing
authorizes freedom of method, hardware access and tests-first fixes. HW2 resumes
with an owned-child harness. H-FIX falsifiers below remain unchanged. New cohort:
C/M/M/C repeated three times, six boots each, M infinite park, profiling on in
both arms, same existing binary hashes and fg5 guest, 2 x 120-second pairs.
One boot per arm includes a 60-second idle-context sample. Inherited boots are
reported separately. Native and fast gates precede Windows deployment of M.

H-OWNERSHIP: startup and abort preserve a foreign matching process. FALSIFIER:
the test-created foreign process is signalled. MEASURED tests first: normal and
budget startup cases both failed on inherited code; owned-abort support tests
were committed before introduction. Each of two mutants is killed by two tests.

CODE READ: lane instructions read completely; branch arms are C pw3f4b9e82-s2,
M 89fc8a22291dd7bda69c3077ba829b1e934b11db, B' c452ca7cb412852a3e6ca68d18e94f92ac625599.

H-FIX (each fixed arm): no baton. FALSIFIER, before hardware execution:
any guest completion gap >= 1 s; any actual PROGRESS_STRAND event (exclude the
diagnostic-enabled banner); timeouts_with_work or backstop_found_work > 0;
a hang with M's KF3_WORKER_PARK_INFINITE=1; M registered > rung+retried at
quiescence. Minimum 12 boots / 24 pairs per arm, interleaved C/M/M/C and
C/B'/B'/C blocks; keep C until >=4 stalled pairs or report insufficient power.
All arms: 4 vCPUs, forced migratable, mirror off, DIAG_PROGRESS=1000,
pair:120,pair:120, K=5 ms, blocking sync. Preserve native binaries.

H-M-UNIT: exact M lineage passes GPU-free kf-chan and kf-qemu library suites.
FALSIFIER, before execution: any failed test, abnormal exit, or incomplete
suite. An incomplete run is not a pass. No source edits or feature overlays
in this initial run. Run nice 19, release, j2, own target, global cargo lock.

CODE READ review questions: watcher touch is not progress; a watcher restricted
to moving worker iteration counters cannot diagnose all-workers-blocked stalls.
H-WATCH: absence of strand events alone establishes progress. FALSIFIER before
further review: watcher resets on unsuccessful evaluation, excludes parked or
blocked workers, or caps event output without a complete counter. Guest gaps
and worker diagnostics must be retained independently.

MEASURED: first host flock -w 50 attempt timed out without host changes.

H-M-HANDOFF-NONBLOCK: the new handoff does not add a blocking lock on a
vCPU/main-loop RAM callback path. FALSIFIER before tracing those callbacks:
one of those paths calls handoff released/take with a blocking Mutex lock
shared with worker registration. Check source call edges; do not assert a
measured stall from a reachable path alone.

H-B-HOLDOUT: the exact B' head passes fresh relay_stress seeds with the
safety net disabled and strict soft findings enabled. FALSIFIER before run:
any failed seed, panic, incomplete execution or abnormal test exit. Holdout:
RS_STRICT=1, RS_FROM=4294967296, RS_TO=4294967328 (exclusive), RS_SECS=0.5,
RS_PARK_NET=0, chaos-hooks enabled, one test thread. No threshold adjustments.
This does not establish I7 coverage where its settled precondition is skipped.

H-B-IDLE: the dedicated grader G5 test observes a real settled one-second
quiet window and blocks idle workers. FALSIFIER before execution: the test
fails, including its explicit quiet_checked assertion, or does not complete.
Run exact B', chaos-hooks, original defaults and thresholds, one test thread.
