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
