# SPR1 selective sparse mappings

STATUS: IN PROGRESS, 2026-10-26. This directory records task 1 of lane SPR1; apply, reservation blocks, and guest validation are separate checkpoints.

CODE READ: ABI 7 gives SPARSE a dedicated run flag. Sparse runs have no backing offset, retain page-size class, and are emitted only with the explicit default-OFF format switch. VER2 leaf classification excludes NV4K; VER3 uses the leaf PCF. Separate PDE metadata counts/vetoes both VER3 sparse encodings; v1 emits no PDE sparse runs.

MEASURED on 2026-10-26 in `task1-red.txt`, `task1-decode-red.txt`, and `task1-containment-red.txt`: before their respective fixes, 9/11 diff tests, 3/4 compiled-classifier tests, and 2/11 report tests failed. EXPECTED RED commits are `ed4986921`, `937d4917e`, `768e6515f`, and `2956f5fea`.

MEASURED on 2026-10-26 in `task1-ptx-abi.txt`: all 89 `kf-cuda` tests passed with the host-built PTX. The `kf-mem` library passed 199 tests with 3 ignored in a serial run; the initial parallel run had one timing-test failure (a 91.8 ms lock wait), and its focused retry passed (`task1-lock-retry.txt`). No timing bound was changed.

MEASURED on 2026-10-26 in `task1-model-mutations.json` and `task1-decode-mutations.json`: all seven CPU mutants were caught by at least two tests. Each mutant was restored.

MEASURED on 2026-10-26 in `task1-gate9-red.txt`: the previous PTX fails sparse leaf and boundary fixtures at source `40bc2ad2c`. MEASURED on 2026-10-26 in `task1-gate9-green.txt`: gate9 passes at `d23ebe9c0` after the PTX replacement. Both runs had zero Xids in their bounded kernel-journal intervals. The GPU was an RTX 4070 AD104 with host driver 595.91.07; VER3 here means synthetic guest tables decoded on that GPU, not a native Hopper/Blackwell test.

The PTX was built on the trusted host with NVIDIA nvcc 12.2.140, compute_75 and PTX ISA 8.2, using a separate GCC 12/glibc environment. Its SHA-256 is `22b9f901feac1d8f0017aee471c6e60a4d6dd97281b6e311bc069bab95f95ddc`.

MEASURED at `006c07ea8` in `task1-gpu-mutations.json` and `task1-gate9-restored.txt`: all nine GPU mutants were caught by 2–16 distinct format/page-size cases each. The original PTX and binary were restored; the expanded baseline and restored-artifact gate9 both pass. Every bounded kernel-journal interval contains zero Xids. Task 1 is complete. Guest sparse correctness, negative controls, and reservation behavior remain tasks 2–4.
