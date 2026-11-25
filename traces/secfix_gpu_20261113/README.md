# Security review 3 GPU-side fixes: RED/GREEN evidence (2026-11-13/14)

**STATUS: LIVE, 2026-11-14.** Runs of agent `fixwalk` for branches `claude/secfix-gpu-20261113` (CI),
`claude/secfix-gpu-ver3-20261113` (F-1) and `claude/secfix-gpu-bounds-20261113` (F-2, F-3, F-9, F-10).
"next" = `origin/next` at `dba284320`. "new" = the bounds branch's `cuda/walk/kf_walk.cu` (the tip of the
walker stack, F-1 included), the same source as the committed `kf_walk.ptx` of that branch.

## Where

- **CPU (no GPU):** the dev host, g++, `make -C cuda/walk check-cpu` (`kf_cpu_walk.cpp`).
- **GPU:** one Vast rental, contract 60558096 (label `kf-fixwalk-20261113`), RTX 3060 (GA106), host driver
  535.154.05 (the oldest supported host driver: the ISA 8.2 JIT floor), image
  `nvidia/cuda:12.2.2-devel-ubuntu22.04` (nvcc 12.2.140), no KVM, no secrets. Rented 2026-11-13 23:54:25 CET,
  destroyed 2026-11-14 00:10:49 CET by id, then the instance list checked. About 0.02 USD.

## Files and results

Every row below is MEASURED on 2026-11-13/14 (RTX 3060, host driver 535.154.05) at revision `dba284320` (next) and the bounds branch tip; the logs are the files named.

| file | what | result |
|---|---|---|
| `cpu_f1_red_next.log` | kf_cpu_walk VER3 cases against next's kernel | FAIL: 32 leaves instead of 16, 16 stale VAs published; PCF_BIG=3 small table published |
| `cpu_f1_green.log` | the same against the F-1 kernel | ok |
| `cpu_corpus_dump.txt` | `kf_cpu_walk --dump` (serial walk, every run of corpus.bin + real_ga106.bin) | identical for next, F-1 and new (sha256 `fa1f2a71...ce5c`) |
| `gpu_next_suite.log` | next's own kf_tests on next's kernel | 76 passed, 0 failed |
| `gpu_red_suite_next_kernel.log` | new kf_tests (+ host-half test hooks only) on next's kernel | FAIL: entry_budget, budget_fails_only_its_space, frontier_fan_in, device_refuses_bad_descriptors, short_shared_memory_launch, npdb_past_kernel_arrays (process then died, rc 139, in the host harness after the 65-entry walk) |
| `gpu_red_ver3_next_kernel.log` | ver3/* on next's kernel | FAIL: unmapped_big_pte_hides_stale_4k (2 runs, want 1), sparse_big_half_hides_small_table (PCF_BIG=3) |
| `gpu_memcheck_commit_next_kernel.log` | compute-sanitizer memcheck `--padding 4194304`, hostile/commit_bound_before_write, next's kernel | **Invalid __global__ write in kf_commit_kernel**, 2977 bytes past the run stage (S1-66) |
| `gpu_green_make_check.log` | `make check` on the new tree: invariants, check-cpu, suite build, PTX-only, every known-positive (bounds, coalesce, seam, sample), VER3 sketch, check-ver3 | all ok |
| `gpu_green_suite.log`, `gpu_green_suite_serial.log` | full kf_tests, parallel and `KF_WALK_SERIAL=1` | 82 passed, 0 failed each |
| `gpu_memcheck_hostile_new.log` | memcheck `--padding`, every hostile/* case, new kernel | 30 passed, 0 errors |
| `gpu_corpus_dump.txt` | `KF_TESTS_DUMP_RUNS=1 ./kf_tests differential/` (parallel walk, every run and header field of the corpus) | identical for next and new (sha256 `fad7e342...2210`, 132 lines) |
| `gpu_ptx_jit_535.log` | driver-API JIT of every shipped PTX file (next walker, F-1 walker, new walker, new kf_yuv, the other display files) on driver 535, every `.entry` resolved | all PTXLOAD_OK |
| `gpu_v3_gates_bounds.log`, `gpu_v3_gates_next.log` | `scripts/bench/v3_gates.sh` with `KF_GATE_GPU=1`, both trees | gate 9 PASS on both (the shipped PTX through `WalkKernel`, VER2 and VER3, 0 mismatching entries against the Rust diff model); gates 1-8 and 10 FAIL identically on both: RM bring-up refuses host driver 535.154.05 (not a measured driver-matrix tag) |
| `gpu_run1.log`, `gpu_run2.log`, `gpu_run4_gates.log` | step logs with start and exit lines | |

`frontier_fan_in` (three self-pointing spaces and one innocent, default budget): next's kernel truncated the
walk (flags 0x85) after charging 3 156 240 entries, so the innocent space failed too; the new kernel
completed (flags 0x84), charged 794 384 entries, cut the three hostile spaces by name and reported the
innocent one's run. Refresh time 1.0 ms vs 0.6 ms on this bare-metal GPU.

## Not shown here (UNVERIFIED)

- Gates 1-8 and 10: UNVERIFIED for this change, because this box's host driver 535.154.05 is not a measured driver-matrix tag (`gpu_v3_gates_next.log` fails the same way).
- The YUV kernel change (F-6) ran on the CPU only (`cuda/display/kf_yuv_cpu.c`); on the GPU only its JIT
  (`gpu_ptx_jit_535.log`) was checked.
- No guest ran (no fast suite, no Windows gate).
