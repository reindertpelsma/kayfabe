STATUS: RESEARCH, 2026-10-19. K1 Part A is PARTIAL; implementation stopped at a failed mutation gate.

CODE READ: branch `codex/kf-prof-20261019`, base `f8f9d1a5f`.
MEASURED: design checkpoint `f21637743` and EXPECTED RED checkpoints
`4ee99da33`, `4cf61dd94`, `e30807dcf` were pushed and verified against origin.
The final documentation commit is recorded in the session's owner report;
no green implementation checkpoint exists. No production integration was changed.

## Result and stage inventory

| Stage | Status | Evidence |
|---|---|---|
| 1 Design | DONE, DESIGN-ONLY | [design](../../docs/design/V3_PROFILER_20261019.md), [preregistration](PREREG.md) |
| 2 Core | PARTIAL / STOP | EXPECTED RED tests pushed; uncommitted local draft passed 36 tests before further test additions; mutation gate below rejected |
| 3 Aggregator, mmap, JSONL, CLI | NOT DONE | no runtime implementation or files |
| 4 Wrappers/lint | NOT DONE | no production wait wrapped; no complete raw-call inventory |
| 5 Ledger wiring | NOT DONE | local core slot tests only; no real birth/close integration |
| 6 Injection coverage | NOT DONE | reserved tables only; no generated injector |
| 7 Pacing/broker/H1 | NOT DONE | design only; H1 was read, never edited |
| 8 Overhead/production symbols/build | NOT DONE | microbench and size/symbol comparison UNRUN; host build scope conflict |
| 9 Report | DONE as partial report | this file and external owner report |

MEASURED: the last verified remote test checkpoint is
`e30807dcf170508dacd022173c9c817d6d18d246`; its empty crate intentionally
fails to compile the tests. The core implementation and two additional tests
remain uncommitted in the K1 worktree, preserved locally for an authorized resume.
Do not use this branch as a production binary or call the core stage green.

## Counts and mutation gate

MEASURED: new crate baseline = 0 tests (crate absent). First uncommitted draft
passed 29 tests; the expanded draft passed 36/36, 0 ignored, 0 failed
([output](core_green.txt)). Subsequently two tests were added, making 38 local
tests. The last 38-test invocation was the header mutant: 37 passed, 1 failed.
A normal 38-test green run was NOT performed after STOP. All other crates were
untouched and their baselines were NOT run. The red evidence is a missing-API
compile failure, not a runtime defect reproduction ([core](core_expected_red.txt),
[thread contracts](thread_expected_red.txt)).

| Hand mutant | Killing tests | Gate |
|---|---:|---|
| Seqlock ordering | 3 | PASS |
| Ring slot wrap | 4 | PASS |
| Histogram bucket | 3 | PASS |
| Ring overflow count | 2 | PASS |
| Clock calibration | 2 | PASS |
| Ledger slot reuse | 2 | PASS |
| Ledger overflow count | 2 | PASS |
| Exclusive residence | 2 | PASS |
| Header geometry validation | 1 | **REJECTED** |

MEASURED: [named tests and exit status](core_mutations.txt).
CODE READ: the lane's COMMON rules say: "If a step is blocked or refused by your
own safety checks, STOP and report; do not reword or work around it."
The required two-test mutation gate refused the header validation behavior;
implementation work stopped instead of weakening or bypassing that gate.
Clippy, final rustfmt, per-thread mmap publication, true clock reads/calibration,
loom/weak-memory proof and production feature-elimination checks remain UNRUN.
Earlier `cargo fmt -p kf-prof` ran before the final two test additions only.

## Overhead, coverage and decisions

MEASURED: no CPU overhead measurements, GPU/VM runs, host SSH, diagnostic-channel
calls, host writes or locks were taken. Hardware n=0; spread unavailable.
Steel Nomad/Heaven overhead is UNRUN (Part B). No live or crash-persistent profiler
file was produced, and no hang-coverage claim is supported by this partial work.

CODE READ: no wait sites were wrapped. The full source audit and vCPU bug-candidate
classification have not been performed. A preliminary text search found 867 raw
blocking-pattern matches across 67 files in the ten named crates; that includes
tests and false positives, is not a blocking-call count, and proves no coverage.
All production wait expressions remain as at candidate-5.

Owner decisions:
1. Resume implementation after reviewing this STOP; recommended next step is a
   separate EXPECTED RED strengthening of header geometry rejection tests, then
   repeat the complete mutation gate before any green implementation commit.
2. Stage 8's host install conflicts with Part A's SSH/log-only/no-lock scope;
   recommend the coordinator build/install during Part B, or explicitly authorize
   a CPU-only host build and its documented build lock. No exception was assumed.

MEASURED: the lane-owned target directory was deleted after STOP. No profiler,
test, mutation runner, cargo job, host process or lock remains owned by this lane.
The worktree and local uncommitted source backup are retained for review.
