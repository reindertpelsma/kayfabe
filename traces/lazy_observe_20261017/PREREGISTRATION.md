# Lazy-observe pre-registration — 2026-10-17

STATUS: RESEARCH. Scratch branch; never merge to master.
Base: claude/d3d11-gap-20261016 at 890c6643c. No instance owned by this session.

Before tests: hypotheses are that the existing pure predicates preserve I8,
ordered exactly-once consumption and the last-doorbell contract, and that stale
private USERD observations are conservative. Falsifiers: any invariant failure,
missing entry after a bounded drain, excess credit, early SYNC_WAIT release or
failure to release after hardware progress. Extend the existing 12 tests, run
`nice -n 19 cargo test --release -j2 -p kf-chan` on the trusted host while idle.
For each source mutation the falsifier of test sensitivity is a passing test run.

Hardware (only after correctness review and tests): same split-switch binary,
Heaven 4.0 Ultra 1920x1080, host 595.91.07, guest 580.88, 260 s in-app
benchmark, 16 GiB/8 vCPU/8 GiB framebuffer, private ring default 512. Common
flags: KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5
KF3_SCAN_OFF_AFTER_5080=2. No FAST_SEG or CPU affinity changes.

A = KF3_PUT_CACHE=1; B = KF3_LAZY_OBSERVE=16; absent switch = disabled.
Falsifier: A alone <= approximately 220 fps means A is not the carrier;
B alone <= approximately 220 fps means B is not the carrier. Evaluate both
replicates and report individual values; no post-hoc threshold tuning.
Four-arm order: none1, A1, B1, AB1, B2, AB2, none2, A2. Fresh boot each run.
If B or AB carries, run its N=4,16,64 ladder (N=16 four-arm replicates are
reported and one additional ladder run), then two fresh-boot OpenGL runs of
its best arm. Ladder hypothesis: interval changes affect throughput; falsifier
of a useful improvement is no improvement beyond the N=16 replicate spread.
OpenGL no-regression hypothesis: best arm stays in inherited 338–359 fps range;
falsifier: either run below 338 fps. Native 330/342 fps are inherited references,
not tonight's measurements. No native rental planned until required work done.

Every run: exact KF3_* environment from /proc/PID/environ, harness assert file,
guest screenshot statistics, guest nvlddmkm 153 / Display 4101 events,
kernel-journal Xid records with process map, memory/PSI and start/end markers.
Stop on Xid/display hang/memory pressure; no retry loop. Only session-owned
PIDs may be stopped. All VM work under flock -o /tmp/kayfabe-fastguest.lock.
No new run if cleanup before 06:30 CEST cannot be assured; target cleanup by
06:15. No polling/timer correctness redesign. N remains the existing named
production constant; env switches are scratch measurement controls only.
