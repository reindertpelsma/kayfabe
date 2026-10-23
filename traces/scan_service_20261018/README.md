# RC-03 / scan-limit wake / PR-2

MEASURED status: implemented and GPU-free acceptance complete, 2026-10-22.
Draft PR only; no merge or hardware validation.

INFERRED hypotheses registered before runs: cyclic scanning must reach cold tokens
behind hot prefixes; a full scan must announce residual inventory; one readiness
batch must return to drainer work. FALSIFIERS: starvation, dropped/duplicated bits,
exceeding the budget, parking beside live inventory, missing SignalOne notification,
permanent spin after stale hints drain, or refill monopolizing service_ready.

MEASURED tests-first history: b5a068eb6 imports the three race-audit files unchanged.
The final focused baseline on unchanged master product is 3 passed /14 assertion
failures /0 ignored across17 cases. The original dbfast watchdog initially PASSED
on master (2.43s); it depends on producer scheduling. Two added tests synchronously
refill eventfds during delivery and reliably fail the unbounded implementation.
Both require pending readiness to survive into the next turn. The final two
prefix-conservation cases also failed by assertion before implementation.
No missing-target or lock-timeout result is counted as a red test. (recorded in commit b0e75feaf, 2026-10-22)

CODE READ: advance after the last token, splitting the sparse cyclic scan at that
bit. A word-only cursor still starves118/127 behind attacker0..9 plus64..117.
Re-publish excluded prefixes and unreturned suffixes word-before-summary; clear
summary before taking its word. A zero budget leaves inventory untouched.

CODE READ: a full positive-budget scan bumps the existing wake word even when
all returned hints are stale. Honor SignalOne through a required HostOps callback;
QEMU and the native harness signal their existing worker eventfd. A false-positive
full scan costs an extra empty pass; the stale-only test requires eventual park.
No served-count inflation, new timer, optional boot flag or GP_PUT polling.

CODE READ: dbfast still synchronizes registry commands first, delivers one
nonblocking readiness snapshot, then returns. Its level-triggered eventfds remain
ready on the same drainer poller for later turns. No readiness-array-size change.

MEASURED hand mutants (individual restored-source runs; failed assertions) (recorded in commit b0e75feaf, 2026-10-22):

| Mutant | Killing tests |
|---|---:|
| Restore master bitmap |9|
| Word-only cursor, retaining zero guard |4|
| Drop excluded prefix re-publication |2|
| Consume work with zero budget |2|
| Omit full-scan wake bump |3|
| Omit peer notifier callback |2|
| Restore unbounded dbfast loop |3|

MEASURED acceptance: all17 focused regressions pass after restoring every mutant.
GPU-free release suites for kf-trap/core/chan/qemu/harness:1217 passed /0 failed /
4 existing ignored. KAYFABE_NO_KVM=1: KVM-GATE RAN0 /SKIPPED3; those3 harness
passes asserted nothing about KVM. Existing ignored models were not loosened.
Clippy all targets, same crates:305 ->301 source-bound fingerprints, added0.
The baseline includes4 style warnings in imported audit tests; those were removed
without changing assertions. No deny set or lint baseline was relaxed. (recorded in commit b0e75feaf, 2026-10-22)

MEASURED execution used the owner's global Cargo lock, one-hour contention waits,
nice19, two build jobs and the lane's target directory. The earlier600s lock wait
had not run Cargo; the owner explicitly authorized the longer retry on resume.
Nested firmware compilation inherited the outer lock/nice and a temporary local
two-job Cargo configuration; that configuration is excluded from commits.

INFERRED hardware obligations later: Linux/Windows boot/work/reset acceptance,
real two-worker notification load, native-harness execution, and sustained optional
ioeventfd churn alongside privileged-register traffic. These suites do not prove
weak-memory exhaustiveness, host scheduling fairness or production performance.
VA-1 and VA-4 remain deferred per the owner ruling.
