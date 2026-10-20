STATUS: STOP / PARTIAL, 2026-10-20. Introduced host-build guard failure; Part A is incomplete.

1. Lane, branch, verified implementation checkpoint and assignment

CODE READ: K1, branch `codex/kf-prof-20261019`, base `f8f9d1a5f`.
MEASURED: last implementation checkpoint `fd2b44ebe0ec88cc85e255ddc0a0dae946064ab1` was pushed and verified on origin.
This report's containing commit is the final evidence checkpoint; its full verified
hash is recorded in the external owner report and the final session message.
Assignment: GPU/VM-free profiler Part A, tests-first checkpoints and pushes,
six coordinator sources, relay freeze observations, CPU overhead and private host build.
Coordinator extended the deadline to06:30 CEST/04:30 UTC; stopping earlier on an
introduced correctness failure, as instructed. No merge was made.

2. Result (13 lines)

- DONE (MEASURED): design,40-test bounded core and strengthened geometry mutant gate.
- DONE (MEASURED): mmap/rotating JSONL, bounded reader, live/crash tests; source directory readable before first second.
- DONE (MEASURED): guest/private GET/PUT/REFERENCE, separate GET-change/observation times, real Wait/wake observations and producer retirement.
- DONE (CODE READ): actual relay Wait is ledger kind12; retired unanswered waits remain explicit, not completed.
- PARTIAL (MEASURED): seven blocking expressions wrapped; worker/drainer/VA roots and relay spans registered; full wait inventory incomplete.
- PARTIAL (MEASURED):28 Site/13 Kind helper cases injected; production reachability of unwired entries is not proven.
- PARTIAL (MEASURED): broker Wayland pacing writer, bounded offline guest merge, H1 aggregate-context import;117 scoped Python tests pass.
- PARTIAL (MEASURED): dev CPU measurements and default-feature symbol/size audit; native TSC and strict zero cost unproven.
- NOT DONE (MEASURED): final host-prof binary; own copied QEMU includes kf3 twice, Meson refused setup.
- NOT DONE (CODE READ): most real ledger kinds, full wrappers, mapped event transport and legacy-instrument absorption.
- NOT DONE (CODE READ): GSP seen-to-post/function latency, per-VAS bytes/applies, vCPU schedstat/RM counts, interrupt stage sources; live diag alignment.
- UNRUN (MEASURED): Part B GPU/VM overhead, hardware n=0; no FPS/Heaven/Steel Nomad claim.
- SCOPE DEVIATION (MEASURED): initial dev-box raw baseline included tiny KVM fixtures; subsequent raw tests were clock-only.

3. Tests, mutations, cost and stage inventory

MEASURED: these are completed individual suite results, not sums of repeat runs.

| Crate/suite | Before | Measured after | Limits |
|---|---:|---:|---|
| kf-prof |0/new|108 passed,0 failed/ignored|40 core+4 injection+34 relay+23 runtime+7 sink; explicit diagnostic clock|
| Python stage-7 suites |0/new|117 passed,0 skipped|owned live/crash, C writer and pinned H1 fixtures supplied|
| Symbol audit helper |0/new|4 passed|separate run; mutation gate unfinished|
| kf-chan |198|198 passed with prof|full wait audit unfinished|
| kf-trap |86|86 passed|unchanged production source, no profiler dependency|
| kf-qemu |356|356 prof at relay checkpoint;356 latest default-feature|latest prof library also compiled on host; whole prof suite not repeated after retirement|
| kf-linux-raw |174 baseline|2 added clock tests passed|full baseline included dev KVM fixtures, outside Part A scope; no trusted-host VM touched|
| kf-mem |unchanged source|180 passed/1 timing failure/3 ignored|isolated lane3/3 and unmodified base1/1 passed|
| Broker existing tests |unchanged|cursor66138, geometry65431, XOR80752, tick566237; selftest82; clock5|normal prof check and final full prof sanitizer check passed|
| Broker new writer |0/new|14 C cases+4 shared-header reader cases|GPU-free; no real compositor/GPU feedback test|

MEASURED: all105 cases in the following tables have at least two named killing
tests. This statement excludes the unfinished stage-8 builder/audit gate.

| Group | Cases | Named kills/case | Evidence |
|---|---:|---|---|
| Core | 9 | >=2 | [Core](core_mutations.txt) |
| Runtime/reader | 21 | >=2 | [Runtime/reader](runtime_mutations.txt) |
| Worker wrappers | 4 | >=2 | [Worker wrappers](wrapper_mutations.txt) |
| QEMU wrappers | 4 | >=2 | [QEMU wrappers](qemu_mutations.txt) |
| Relay model/runtime/reader | 24 | >=2 | [Relay model/runtime/reader](relay_mutations.txt) |
| Relay integration | 6 | >=2 | [Relay integration](relay_wiring_mutations.txt) |
| Injection helpers | 4 | >=2 | [Injection helpers](injection_mutations.txt) |
| Broker writer/build/wiring | 16 | >=2 | [Broker writer/build/wiring](broker_mutations.txt) |
| Merge/pacing | 14 | >=2 | [Merge/pacing](merge_mutations.txt) |
| H1 context | 3 | >=2 | [H1 context](h1_mutations.txt) |

MEASURED: the header-geometry mutant has three killing tests. The full core table
was rerun before its GREEN checkpoint. The latest full24-case relay table includes
retirement4/2/2 killers; the runtime21-case table was rerun. The mutation review
notes retain no-op/overwritten-label, exact-number matching and boundary-fixture
corrections; no kill threshold or production timeout was weakened.

MEASURED: kf-mem's unchanged loaded assertion saw123626us versus60000us.
Isolated nice0 lane maxima were8918/9455/7828us at three moments; base was9213us.
This is the coordinator-authorized baseline timing flake, not a scheduler-causality
finding. [Attempts](timing_rechecks.txt).
MEASURED: an intermediate broker sanitizer run hit an existing old-broker
ConnectionRefused readiness case; two isolated current and one unmodified-base
runs passed, then the full latest sanitizer run passed. No wait changed.
[Attempts and limits](broker_timing_flake.txt).
MEASURED: QEMU clippy matched the base diagnostic multiset:132 QEMU keys
(89 errors/43 warnings),307 including dependencies, new0. These existing deny-lint
errors were not relaxed. kf-prof no-deps all-target clippy passed with and without
prof-inject. Changed Rust was formatted; QEMU's unchanged test suffix retains
baseline formatting debt. Full workspace clippy/format equality is not claimed.

MEASURED: CPU-only dev benchmark, nice19/release;7 batches. Core operations use
20000 iterations/batch; runtime uses5000/worker/batch. Runtime n=7*N averages are
correlated within a batch. Diagnostic CLOCK_MONOTONIC goes through libc; the
vDSO/syscall route was not measured. Native invariant-TSC publication is UNRUN
because this dev CPU fails the production clock-support gate. No native budget
pass is claimed. The guard-escape and actual call-wrapper harnesses are separate.
Concurrent ring results include counted refused attempts and are not accepted
trap-record latency; complete integrated trap recording is UNMEASURED.

| Operation | n | Median ns | Min–max ns |
|---|---:|---:|---:|
| feature_off_span_pair | 7 | 0.272 | 0.271–0.634 |
| feature_off_call_wrapper | 7 | 0.272 | 0.271–0.636 |
| core_span_pair | 7 | 10.775 | 10.774–13.159 |
| core_ledger_open_close | 7 | 3.228 | 3.228–4.008 |
| core_histogram | 7 | 2.692 | 2.691–3.081 |
| clock_conversion | 7 | 2.427 | 2.425–3.422 |
| trap_event_ring_roundtrip_uncontended | 7 | 17.274 | 10.639–17.852 |
| trap_event_ring_attempt_concurrent_consumer | 7 | 3.919 | 1.475–7.699 |
| runtime_off_span_pair | 7 | 15.351 | 14.563–68.529 |
| runtime_off_call_wrapper | 7 | 3.672 | 3.613–3.706 |
| core_span_pair | 7 | 18.894 | 18.080–20.854 |
| core_ledger_open_close | 7 | 3.178 | 3.163–4.441 |
| core_histogram | 7 | 3.315 | 3.273–3.741 |
| clock_conversion | 7 | 4.395 | 4.288–5.128 |
| trap_event_ring_roundtrip_uncontended | 7 | 7.189 | 6.976–16.426 |
| trap_event_ring_attempt_concurrent_consumer | 7 | 1.829 | 1.690–1.973 |
| diagnostic_runtime_span_pair_threads_1 | 7 | 560.482 | 492.745–1247.580 |
| diagnostic_runtime_ledger_pair_threads_1 | 7 | 324.824 | 238.921–950.530 |
| diagnostic_runtime_relay_observation_threads_1 | 7 | 220.231 | 126.514–230.000 |
| diagnostic_runtime_call_wrapper_threads_1 | 7 | 510.434 | 495.354–1447.314 |
| diagnostic_runtime_span_pair_threads_4 | 28 | 1328.996 | 507.527–2105.540 |
| diagnostic_runtime_ledger_pair_threads_4 | 28 | 422.391 | 355.770–3414.114 |
| diagnostic_runtime_relay_observation_threads_4 | 28 | 221.207 | 119.104–682.734 |
| diagnostic_runtime_call_wrapper_threads_4 | 28 | 1368.581 | 317.121–2412.538 |
| diagnostic_runtime_span_pair_threads_8 | 56 | 958.198 | 320.299–2328.420 |
| diagnostic_runtime_ledger_pair_threads_8 | 56 | 397.468 | 372.820–2203.163 |
| diagnostic_runtime_relay_observation_threads_8 | 56 | 205.099 | 119.130–563.213 |
| diagnostic_runtime_call_wrapper_threads_8 | 56 | 876.042 | 335.604–5980.070 |

MEASURED: production archive comparison, base/current:50507774/50506896 bytes;
text9120672/9120673, data1201692/1201692, bss86392/86392. Each has12481 normalized
public definitions and0 kf-prof definitions;207 added/removed symbol identities.
Default dependency tree excludes kf-prof and356 default-feature QEMU tests pass.
The one-byte text difference is unexplained: strict R1 zero cost remains unproven.
CODE READ: runtime-off relay caches still store observations in prof builds.
[Exact comparison](production_symbols_size.json).

MEASURED: host build `fd2b44ebe0ec88cc85e255ddc0a0dae946064ab1` ran at nice19 in its unique private directory.
Rust-prof archive compilation succeeded. Meson refused the copied source with
`Tried to enter directory "hw/misc/kf3", which has already been visited.`
Read-only comparison found shared include count1, private count2. CODE READ:
the builder converted an rg regex to basic grep while retaining escaped
parentheses, so the existing include was missed. No final QEMU/broker-prof
binary was published, no binary replaced, and no repair/retry followed the STOP.
[Failure](host_build_failure.txt). Large logs/source remain on the trusted host at
`/data/codex-K1/kf3-bins/fd2b44ebe0ec88cc85e255ddc0a0dae946064ab1-prof/` (`build.log`, `qemu-build/meson-logs/meson-log.txt`).

| Stage | Status | Evidence |
|---|---|---|
|1 Design|DONE as research/partial design|[design](../../docs/design/V3_PROFILER_20261019.md)|
|2 Core|GREEN|40 tests,9 mutants; `9b5f12f29`|
|3 Runtime/files/reader|PARTIAL|mmap+JSONL+reader live; mapped event transport unfinished|
|4 Wrappers/audit|PARTIAL|7 expressions, scoped CI gate; full inventory not verified|
|5 Real ledger sources|PARTIAL|relay kind12/progress/retirement; other kinds largely unwired|
|6 Injection|PARTIAL|all28 Site/13 Kind helper cases; actual-path coverage incomplete|
|7 Pacing/broker/merge/H1|PARTIAL|Wayland subset+offline clock/CSV+context import; display/guest integration incomplete|
|8 Cost/zero-cost/host build|PARTIAL/STOP|CPU data, symbol audit; strict zero cost unproven; host builder failed|
|9 Report/cleanup|DONE when containing report commit is verified|external report records final SHA|

CODE READ: wrapped waits are Site16/17 worker immediate/park;18 VA poll;19/20
drainer reset/normal poll;24 relay storage mutex;27 relay map mutex. Root spans
are21/22/23 VA/drainer/worker and25/26 relay turn/peek. Changed expressions run on
worker/VA/drainer/relay paths; no new vCPU blocking operation was introduced.
The broader reachable-path audit is unfinished, so there is no complete statement
about existing vCPU-path bugs. [Lexical candidates](raw_blocking_candidates.tsv)
lists1286 unreviewed calls across ten hot crates, including memory/nonblocking
false positives and qualified-helper exclusions. It is not a justified allowlist.
The CI rejection gate currently covers raw waits in worker.rs/device.rs only.

CODE READ: required source status: channel/private progress is wired; forwarded
GSP seen-to-successful-post latency, per-VAS mapped/unmapped bytes/large applies,
vCPU schedstat and actual RM-call counts, and engine/vector raised/delivered/acked
sources are NOT WIRED. Guest diag offset has a validated offline method only;
no live diag query/measurement was performed. Broker credits are UNKNOWN.
The generic ledger/injection tables do not prove real birth/close coverage.
H1 preserves aggregates as context with false causal coverage, without fabricated
frame IDs, stages, waiter linkage or acknowledgement. Legacy KF3_PROF/DIAG
outputs stay intact and are not fully folded into the new records.

CODE READ: always print per-second RmControl-per-frame; the coordinator reported
that its fall preceded run3017's slide. The reader prints the numerator and named
display denominator, using UNKNOWN/N/A when unwired or zero, and keeps interval
records. This is the coordinator's temporal observation, not K1 evidence of cause.

4. Decisions for the owner

1. CODE READ recommendation: resume only to add two independent guard-killing tests
   (prepatched source and repeated overlay installation), repair literal matching,
   then build a new immutable revision-prof directory. Do not reuse/replace a binary.
2. CODE READ recommendation: finish the priority real-ledger/source wiring and broad
   wait audit before claiming complete observability; do not enable this as production instrumentation.
3. INFERRED recommendation: resolve the one-byte text difference/runtime-off cache
   contract, then measure native cost and perform coordinator-owned Part B. Keep
   the GPU overhead claim UNRUN until a valid hardware A/B exists.

5. What could not be verified

MEASURED: final host-prof binary/launcher and Part B commands are NOT DONE after
its build refusal. GPU/VM benchmark arms, native TSC cost, full trap-record cost,
real guest/host clock offset and GPU/long-run behavior are UNRUN. CODE READ:
complete blocking coverage, most ledger birth/close edges, vCPU registration,
full identity/clock/session reconciliation, mapped ring, display/guest pacing,
legacy equivalence and causal H1 linkage remain unfinished. Reading copied files
on another host boot does not verify the reader's local monotonic-age assumption.
Stage8 has no GREEN checkpoint; its builder/audit mutation gate is incomplete.

6. Cleanup and compartment

MEASURED: own local and host process scans were empty; the host rust-target was
removed. The dev target was deleted;255 known profiler test scratch directories were removed.
No GPU/VM/shared build lock was taken; the trusted host's VM/GPU and existing
binaries were untouched. Only the failed private build/source/log directory is
retained intentionally on the host. No merge, force push, forbidden host/path
access, cross-session message or other agent's worktree mutation was performed.
The initial dev KVM-fixture scope deviation is disclosed above rather than hidden.
