# Lifecycle PHASE 1b, Linux hardware validation: pre-registration

STATUS: RESEARCH, 2026-10-16. Written and committed BEFORE any build, test or VM run of this task
(host root@172.22.1.20, branch claude/lifecycle-hw-20261016; the commit time is the evidence of order).

Revision under test: e1e9947ccd24a9c2a53e13f24f5cd0bbcc16057c (origin/claude/win-lifecycle-p1b-20261016; the docs-only commit
4ab092324 on top is not tested code). Base for the paired comparison: 0a7461173 (origin/claude/perf-fix-20261015), whose binary is
already installed as kf3-bins/0a746117 and whose Linux results on this very host were measured by the rootless audit
(origin/claude/rootless-audit-20261016, traces/rootless_audit_20261016/: fast suite 30/30, gates 9/9 + gate 10, 12/12 USER births).
Labels: MEASURED = observed in a command run for this task. INFERRED = reasoned, not run.

## What the change is, and why Linux should not notice (INFERRED from the design doc and source, before any run)
The lifecycle (kf_chan::user_lifecycle::Lifecycle) is built by exactly one constructor call, in kf-qemu userpump.rs, which only the
Windows USER-ring birth reaches (it needs a WindowsUserRingPermit). The unit test
userpump::tests::the_lifecycle_is_reachable_only_from_the_windows_user_ring_pump pins that structure. Counters are not behind an
env switch: a `kf3: USER_RING_LIFECYCLE ...` line is printed when a relay-carried channel is released with entries counted, and a
`kf3: USER_RING_LIFECYCLE SUMMARY ...` line at device stop / when the last relay goes, only if Stats::life.any(). A Linux guest
therefore should print NO such line. (Caveat stated now: absence of a line is weak evidence by itself, because the device may never
see stop() on a guest power-off; the structural pin test and the per-arm absence of every USER_RING_* line are read together.)

## Hypothesis
H1. On this host (RTX 4070, driver 595.91.07) revision e1e9947cc behaves for Linux guests exactly as 0a7461173 did: same GPU-free test
counts as the design doc claims, same gate verdicts, the same 30/30 fast-suite arms with per-arm times within run-to-run noise,
only USER channel births, no new host Xid, and no lifecycle activity on any Linux channel.

## Falsifier (any one falsifies H1; the first failure is recorded as found, not retried to turn it green)
F-a any v3_gates.sh gate failing (not 9/9, gate 10 not PASS/FALLBACK-with-proof, births not all USER or any REFUSED line);
F-b any fast-suite arm other than PASS (30/30 required);
F-c any channel birth that is not `PRIVILEGED_CHANNEL=0 privilege=USER`;
F-d any new Xid in the host kernel log, other than Xid 31 (MMU fault) attributable BY PID to the QEMU of the arms --defer-liveness
    and --missing-page-fault (the frozen grader faults GPU channels there on purpose);
F-e any `USER_RING_LIFECYCLE` line, any `USER_RING_LOOP SUMMARY` / `USER_RING_TRACE` / `USER_RING_POINTER_BACKSTOP` line in a Linux
    QEMU log, i.e. any state transition (Scan/Pointer/Window), any pointer_pb_read_violation != 0, or any other lifecycle counter nonzero;
F-f GPU-free `cargo test --release -p kf-chan -p kf-rm -p kf-trap -p kf-qemu` on the host not reporting the design doc's totals
    (1,561 passed, 0 failed, 3 ignored) or any failure; the pin test above not passing;
F-g the Linux broker lane (interactive.sh with the Cinnamon guest) not booting to KMS + a Cinnamon desktop with glxgears running and
    shutting down cleanly, or showing a refusal / Xid / lifecycle line that the base does not.
Timing is NOT a falsifier by itself; it is reported as a paired difference with the observed noise (below). A slowdown larger than
the base's own spread on the same arm is reported as a finding, not as a failure of H1.

## Plan (strictly serial, one GPU job at a time, every GPU/VM job under `flock -o /tmp/kayfabe-fastguest.lock`)
L1 build e1e9947cc (and, for gates, 0a7461173) as root in own worktrees under /data/claude-life with CARGO_TARGET_DIR=/data/claude-life/target;
   scripts/bench/build_kf3.sh installs kf3-bins/e1e9947c in the shared per-revision directory
   (/var/lib/kf-windows-20261005/kf3-bins); record sha256 and the revision-bearing path; GPU-free cargo test on the host.
L2 v3_gates.sh for both revisions; cargo build --release -p kayfabe-rm-ladder + build_fast_guest.sh (KF_FROM_HOST=1, the host's own
   595.91.07 modules) once into /data/claude-life/bench/fastguest, used for BOTH revisions; fast_suite.sh 180 with QEMU_BIN set to the
   per-revision binary, in the order A B B A (A = e1e9947c, B = 0a746117) to spread drift; the two repeats give the noise.
   Per QEMU: the binary path/revision, host driver, guest driver, and /proc/<pid>/environ KF3_* are captured by a sampler while the arm runs.
L3 interactive.sh (the current display-broker launcher; scripts/bench/display/broker_lane.sh is a KDE/X11 script that looks for plasmashell and
   the host session is GNOME/Wayland) with KF3_REV=e1e9947c on a private qcow2 overlay of the Cinnamon desktop disk; the real broker in the live
   Wayland session (a "kayfabe guest" window appears on the host desktop; stated here because a human may be at that screen);
   checks over ssh: KMS (nvidia-drm modeset), Cinnamon running, glxgears frame rate under X, clean shutdown (ACPI powerdown). One run
   for e1e9947c; the base only if the new run shows anything surprising.
L4 grep every Linux QEMU log (all arms and the broker lane) for the lines of F-e and print the counts.
L5 REPORT.txt, MEASURED vs INFERRED separate; host Xid count before and after (84 at the rootless audit end).

## Failure rule
One failure per arm: record the first failure, retry ONCE only to classify flake vs defect (both results reported), then stop.
No wall-per-boot. Time-box 4 h from the first build. Do not kill a process this task did not start. At the end: no QEMU of this task left,
the lock free, overlays, worktrees and build targets deleted (evidence kept in git and gzipped raw logs under /data/claude-life/raw_*, which
is the one thing left, named in the report).
