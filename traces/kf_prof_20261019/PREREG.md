STATUS: RESEARCH, 2026-10-19. K1 Part A preregistration.

CODE READ: base f8f9d1a5f; no production changes yet. Tests and measurements
below are prospective, not results. Each falsifier precedes its experiment.

| Hypothesis | Falsifier | Planned evidence |
|---|---|---|
| H1: publication can use single-writer stores | syscall/allocation/lock/cross-thread RMW required on publication | core source audit, microbench |
| H2: snapshot consistency is bounded and detects incomplete writers | accepts odd/changing generation or spins without limit | table tests, ordering model, stress |
| H3: ring retains FIFO through wrap, counts every refusal | overwrite, reorder, cursor advance on full, missing drop | reference queue, wrap/stress, mutants |
| H4: histogram/calibration/ledger handle boundary integers | wrong bucket, wrap/panic on hostile fields, stale close affects reuse | exhaustive boundary tables, seeded property tests |
| H5: reader tolerates hostile/truncated data | panic, unbounded loop/allocation, accepted bad version/torn record | fuzz corpus with fixed size/time bounds |
| H6: wrappers preserve operation results and coverage | changed result/test count or unrecognized new raw blocking call | crate baselines, audit mutants, injected delays |
| H7: reconciliation preserves previous instrument counts | mismatch on identical synthetic observations | golden tests against legacy sources |
| H8: production feature elimination adds zero hot-path work | new profiler symbols or size/call-site difference without feature | cfg compile test, symbols and executable size |
| H9: enabled publication stays within proposed CPU budget | measured median or p99 exceeds doc budget | repeated dev-box bench, N threads |

CODE READ: Part A explicitly prohibits host SSH other than log reads and all
locks. Stage 8 also requests a host build/install via the documented builder,
which takes a build lock. That action cannot be performed under both rules.
Options: coordinator performs the build in Part B (recommended); or owner
explicitly authorizes a CPU-only host build and its build lock. No host action
will be attempted under an inferred exception.

## Coordinator source addendum acceptance plan — 2026-10-19

CODE READ: all six new sources and the per-second RmControl/frame display are
required, without changing work order or lifting the existing mutation STOP.
INFERRED: the following hypotheses/falsifiers are prospective tests, not results.

| Hypothesis | Falsifier before implementation/run | Planned stage evidence |
|---|---|---|
| C1: queue progress has correct identity and observation age | wrap/reused token corrupts depth, unchanged GET resets age, or stale observation is reported fresh/completed | bounded ring fixtures, two-test mutants |
| C2: forwarded latency spans seen to successful post per function | held/refused attempt/host-return closes it early, or same-function instances cross-match | immediate/held/refused posts across intervals |
| C3: per-VasKey bytes and large applies reproduce host-confirmed results | refused/partial bytes count as mapped, remaps invent live stock, or legacy large-apply count differs | apply-result fixtures with per-key interval totals |
| C4: scheduling deltas and per-frame ratios reconcile exposure | TID reuse underflows runtime; actual RM calls conflate guest RmControl; zero/missing frames produce a number; mean of ratios replaces ratio of sums | schedstat fixtures and reader goldens |
| C5: clock alignment retains uncertainty | absent/expired calibration aligns guest data, or stated uncertainty excludes a valid bracketing offset | synthetic ping-pong/clock-drift fixtures; live Part B UNRUN |
| C6: interrupt stages remain distinct and observable | injection becomes ack, shared vector invents engine ack, or coalescing/loss disappears | source/injection/ack fixtures with two-test mutants |

CODE READ: no new experiment or source-coverage claim accompanies this addendum.

## Authorised resume — 2026-10-20 00:12:40 UTC

CODE READ: coordinator explicitly authorises strengthening geometry tests,
rerunning the full mutation table, committing the core GREEN checkpoint and
continuing stages 3–9. The header is fixed-layout: independent malformed-length
cases truncate producer geometry and overflow the declared file length, rather
than inventing variable slice fields in reserved header bytes.
CODE READ: CPU-only host build is authorised inside its unique revision-prof
output directory, with no lock, GPU, VM, replacement or other /data writes.
This resolves OPEN 1; the shared-lock builder cannot be invoked unchanged.
INFERRED: conservatively retain the original 10-hour wall deadline, 2026-10-20
03:12:09 UTC, or stop at stage 8 if earlier. Prioritise stage 3 then stage 5 and
the kf-qemu/kf-chan/kf-trap wrappers if remaining time requires that ordering.

## Coordinator resume — 2026-10-20 01:31 UTC

CODE READ: coordinator authorises nice0 isolated execution of the unchanged
lock-wait test up to five moments and one unmodified-base run. Builds remain
nice19/release/-j2. A timing flake is reported as baseline equality, not a STOP;
only correctness failures introduced by K1 stop subsequent implementation.
New deadline:2026-10-20 04:30 UTC (06:30 CEST). Existing thresholds remain intact.

INFERRED, hypothesis T1 before isolated runs: the timing assertion is a baseline
flake. Falsifier: reproducible lane-only failure with an isolated base passing,
or changed correctness results/production logic. Record every attempt, exits,
latencies and moments; a pass does not establish scheduler causality.

CODE READ: additional required source is per-relay private-ring GP_GET/GP_PUT/
REFERENCE, parked Wait kind/target, last doorbell and wake reason. Falsifiers
before integration: a guest GET is reported as private GET, GET fabricates marker
processing, unchanged GET refreshes its change age, generation reuse aliases a
record, a missing observation becomes zero, or a profiler read/poll changes the
relay's event behavior. Consume only existing reads; mark unknown/stale data.
