STATUS: RESEARCH / STOP, 2026-10-20. Part A incomplete; unchanged baseline failed.

1. Lane, branch, checkpoint and assignment

CODE READ: K1, `codex/kf-prof-20261019`, base `f8f9d1a5f`.
MEASURED: last GREEN checkpoint `8be6b80214479e952a4b447dbb4f32d039588ca4`,
pushed and verified on origin. Last expected-red source checkpoint
`3e0582d8a82939e9245f05ed49eb3219d971163b` was also verified on origin.
This evidence-only report commit follows it; the final owner report records its
full verified hash. Expected-red QEMU adapter tests intentionally await the local
draft. Assignment: execute GPU/VM-free profiler Part A, stages/tests/mutations,
stage pushes, six coordinator sources and per-second RmControl/frame.

2. Result (12 lines)

- DONE (CODE READ): lane AGENTS.md and repository instructions read; design and preregistration published.
- DONE (MEASURED): geometry strengthening; all nine core mutants >=2 kills; geometry has three.
- DONE (MEASURED): core GREEN `9b5f12f29`,40 tests; persistent-runtime GREEN `ac38df884`.
- DONE (MEASURED): bounded mmap/rotating JSONL, four reader commands, live/SIGKILL/hostile-reader tests.
- DONE (MEASURED): channel worker's two distinct poll wrappers and scoped CI syntax gate, GREEN `8be6b8021`.
- PARTIAL (MEASURED): QEMU draft356/356 prof tests and four source-audit mutants; final clippy/checkpoint UNRUN.
- STOP (MEASURED): untouched kf-mem baseline180 passed/1 failed/3 ignored;123626us wait exceeded60000us bound.
- NOT DONE (CODE READ): real ledger birth/close wiring, mapped event-ring integration and complete wait audit.
- NOT DONE (CODE READ): generated all-Site/kind injection proofs, six production sources, FPS/broker writer/merge/H1 absorption.
- NOT DONE (MEASURED): microbenchmark, production symbol/size proof and trusted-host binary; host access=0.
- PARTIAL (CODE READ): reader ratios name display/broker denominators and use summed counts; runtime sources remain UNKNOWN.
- UNRUN (MEASURED): Part B GPU overhead; hardware n=0, spread unavailable.

3. Tests, mutations and stage inventory

MEASURED: counts below are individual completed runs, not summed repetitions.

| Crate/suite | Before | Latest measured result | Limits |
|---|---:|---|---|
| kf-prof core |0, new crate|40 passed|bounded stress/order-model tests; no loom proof|
| kf-prof total with prof-inject |0|70 passed=40 core+23 runtime+7 sink|explicit diagnostic clock fixtures|
| Python reader |0|25 passed including4 live/crash/residence cases|requires owned fixture binary for4 cases|
| Python worker wait audit |0|10 passed|scoped syntax, not type resolution|
| kf-linux-raw |174 passed baseline|2 added clock tests passed selectively|full existing suite not rerun after clock module addition|
| kf-chan unit |198 passed|198 passed with prof enabled|whole-crate wait coverage unfinished|
| kf-trap unit |86 passed|86 passed, no production change|no profiler dependency added|
| kf-qemu unit |356 passed|356 passed with prof, local draft|final draft clippy/checkpoint unrun|
| kf-mem unit |unchanged source|180 passed,1 failed,3 ignored|STOP; no rerun or edit|
| unsafe-count CI self-tests |existing|4 passed|three clock unsafe blocks added in allowed perimeter|

MEASURED: no profiler test ignored/removed. Existing kf-mem ignored tests stayed
ignored. The 174-test raw baseline includes tiny KVM fixtures on the dev box;
that baseline exceeded this lane's VM-free scope. No owner/host VM was
started or touched. Later raw verification was limited to the two clock tests.

MEASURED: mutation gate results.

| Mutation group | Cases | Independent named kills per case | Evidence |
|---|---:|---|---|
| Core |9|3,4,3,2,2,2,2,2,3|[table](core_mutations.txt)|
| Runtime/reader |20|2–4|[table](runtime_mutations.txt)|
| Worker wrapper/audit |4|3,2,5,3|[table](wrapper_mutations.txt)|
| QEMU adapter draft |4|4,3,2,2|[table](qemu_mutations.txt)|

MEASURED: the coordinator-requested complete core table was rerun before its
GREEN commit. All37 recorded cases meet the two-kill gate. QEMU cases prove
source-audit coverage wiring, not real hardware injected delays. Latest restored
profiler Rust suite=70 passed. Whole-dependency clippy -Dclippy::all encountered
nine pre-existing kf-util errors; kf-prof --no-deps passed with -Dclippy::all.
Changed channel/raw lines had no new clippy warnings. The three existing
isolation violations equal candidate-5: trap→loom, disp→broker, disp→linux-raw.

CODE READ: stage completion at STOP.

| Stage | Status | Concrete evidence |
|---|---|---|
|1 Design|DONE as research design|[design](../../docs/design/V3_PROFILER_20261019.md),[preregistration](PREREG.md)|
|2 Core|GREEN|`9b5f12f29`,40 tests,[mutants](core_mutations.txt)|
|3 Aggregator/files/reader|PARTIAL|`ac38df884`,[results](runtime_green.txt); all-source/event integration unfinished|
|4 Wrappers/lint|PARTIAL|`8be6b8021` worker waits; QEMU is [uncommitted draft](qemu_draft_status.txt)|
|5 Real ledger wiring|NOT DONE|same-owner runtime slots tested; no production kind wired|
|6 All-Site/kind injection|NOT DONE|publication-pause diagnostic tests are not full coverage|
|7 FPS/broker/merge/H1|NOT DONE|H1 read-only review/design only; no old instrument removed|
|8 Cost/production/build|NOT DONE|no benchmark or symbol/size evidence; no host access|
|9 Report|PARTIAL report complete|this file plus final owner report|

CODE READ: wrapped production sites are16 ChanWorkerImmediate and17
ChanWorkerPark in `kf-chan/src/worker.rs`. QEMU's local draft has18 QemuVaPark,
19 QemuDrainerResetPark,20 QemuDrainerPark and running scopes21–23, plus optional
realize startup and three thread registrations. The current CI syntax gate covers
only worker.rs; its draft extension covers device.rs. Other raw blocking calls
have not been fully inventoried or justified. A preliminary867 text matches across
67 files included tests, strings and false positives; it is not a wait count or
vCPU audit. Existing code candidates were not changed or certified safe.

MEASURED: [baseline failure](mem_baseline_failure.txt):6445 handovers in4272ms;
max wait123626us, max hold122788us, max handover123632us; one baseline experiment.
INFERRED: scheduler preemption is possible, unverified; no causality claimed.
CODE READ: COMMON says “If a step is blocked or refused by your own safety
checks, STOP and report; do not reword or work around it.” Implementation stopped
on this failed baseline, before the10-hour deadline. No threshold, timer, poll
interval, test skip or production behavior fix was substituted.

4. Owner decisions

1. CODE READ: investigate/adjudicate the failed unchanged baseline before resuming;
   recommend collecting lock-holder scheduling evidence, with the preregistered
   falsifier in the failure record, rather than changing its bound.
2. CODE READ: resume from the last GREEN worker checkpoint and review the preserved
   QEMU draft; recommend retaining its expected-red tests and remaining gates.

5. Unverified and operating limits

MEASURED: the dev CPU lacks the invariant-TSC capability bit. Ordinary production
Session creation refuses before files; explicit prof-inject OS-clock files carry
flag1 and `clock_mode=diagnostic_os_clock`. Native cross-core calibration, drift
bounds, cost budgets and supported-host operation remain UNRUN. No hot-path cost
can be inferred from diagnostic syscall-clock tests. No per-operation ns/n/spread
microbenchmark exists. Steel Nomad/Heaven overhead is UNRUN, Part B.

CODE READ: runtime reserves64 thread slices and192 per-thread ledger slots;
thread slots are not reused. The core SPSC ring is tested, but is not yet connected
to runtime mmap publication/draining. Cross-thread ledger close is unsupported;
no cross-thread production source was wired. The aggregator is not self-profiled.
The reader's window is per-second aggregate granularity. Header geometry is fixed
and actual length is checked before bounded pread; no reader writes mapped data.

CODE READ: tail/summary/window/hangs are available in `scripts/bench/prof/reader.py`;
merge is not implemented. Print per-second RmControl-per-frame, retaining raw call
counts and distinct display/broker new-content denominators: its fall preceded
the run-3017 slide according to the coordinator. K1 did not independently verify
that observation or causality. Missing sources are UNKNOWN; zero denominators
are unavailable; summary uses a ratio of sums. All six addendum sources remain
required by the design and unwired, not silently zero. Production call-site
elimination/public symbols/size, broker checks/sanitizers and H1 compatibility
remain UNRUN. No install-ready -prof binary or executable Part B recipe is claimed.

6. Cleanup

MEASURED: all lane-started cargo/test/mutant/fixture processes completed. No SSH,
GPU or host-VM operation or host lock was taken. The lane-owned target directory
was deleted; worktree and uncommitted QEMU draft retained for review/resume.
The owner report records the final verified remote head and draft backup location.
