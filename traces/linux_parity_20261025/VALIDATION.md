# LX2 GPU-free harness validation

STATUS: RESEARCH, 2026-10-25. MEASURED run `lx2-analysis-20261025`, source on the LX2 branch: 20 synthetic harness tests passed (18 analysis and 2 sampler); no product code, GPU workload, VM or actual NVML sample is covered by these tests. The tests-only EXPECTED RED commit fails on the missing new analyzer module, not a previously demonstrated production defect.

MEASURED 2026-10-25, run `lx2-analysis-20261025`: five restored-source analyzer mutants were exercised in isolated temporary Python directories, each killed by at least two distinct tests. The original source remained unchanged and all 20 tests passed afterward. Exact killing test names and return codes are in [analysis-mutations.json](analysis-mutations.json).

| Mutant | Distinct tests |
|---|---:|
| Invert K/N ratio | 2 |
| Include the 100 ms threshold as a spike | 2 |
| Admit fewer than five complete pairs | 2 |
| Accept nonfinite timing values | 3 |
| Ignore content-check failures | 2 |

CODE READ: duplicate-record, empty/nonpositive timing, insufficient/unequal pair, percentile, CPU-versus-submit and GPU-pass parsing contracts are in `scripts/bench/linux_parity/test_analyze.py`. Query-error receipt preservation and synthetic clock brackets/cadence are in `test_sampler.py`. These tests bound the harness interpretation; they do not establish Linux parity, absence of hardware faults, observer overhead or a GPU mechanism.
