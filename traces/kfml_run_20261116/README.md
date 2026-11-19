# kfml_run_20261116: the ML research workloads run natively on a Vast CUDA container

STATUS: LIVE, 2026-11-16 (measured; text evidence only). Run id `kfml_run_20261116`.

What this is: the first run of `scripts/kfml` on a GPU (README status before: "DESIGN, NOT RUN ON HARDWARE"). One Vast **container**
instance (not the KVM image, no kayfabe, no kf3 in it: the native baseline), rented by the total-cost rule, destroyed by its exact id
(`vastai show instances-v1 --raw` printed `[]` afterwards). Box: GeForce RTX 4070 (sm_89, 12 GB), driver 560.35.03 (`nvidia-smi` CUDA
12.6), image `nvidia/cuda:12.6.3-devel-ubuntu22.04` (Ubuntu 22.04.5, CPython 3.10.12, gcc 11.4.0, nvcc 12.6.85), Xeon E5-2680 v3 host
CPU. 57 minutes of rental; cost 0.28 USD of credit (9.125 -> 8.843: rental, storage and bandwidth; limit was 3 USD and 3 hours).
Nothing secret went to the box: only the public key and our code (a tar of `scripts/kfml` and `scripts/ci/kfml_stage.py`); every
download was a public URL (the PyTorch and PyPI indexes, NVIDIA's redist server, GitHub raw, the CIFAR-10 server), checked against the
hashed locks and the `pins.json` sha256.

Revisions: the code that ran is the worktree of branch `claude/kfml-run-20261116` at `df0d0ba1b8400f4a06120094d40ac3e4a961ea3c`
(rows `stage_cold_large_df0d0ba.json`, `stage_warm_large_df0d0ba.json`, the reference) and at `5f596383bce6ed35c5aa2ae3fa64282338fc1ead`
(`stage_warm_mid_5f59638.json`: after the child working-directory hardening). The first-run rows (`rows/first_run_*`) were produced by
earlier states of the same branch, before the PATH fix (see below). The PR's final commit adds docs, the committed reference and the
`flavor` row field fix (installed flavour: `env.py`, `rows.py`, `reference.py`, with two unit tests), no workload code. The final tree
itself has not run on a GPU; its GPU-free tests pass.

## Result

All six workloads PASS natively, wheel flavour **cu126, chosen automatically** (`check.txt`: "chose cu126 (driver CUDA 12.6 >= 12.6 and
sm_89 is inside sm_75..sm_90); rejected cu128: driver supports CUDA 12.6 < 12.8"), installed from the hashed lock with `--require-hashes`.

| row (cold caches, `stage_cold_large_df0d0ba.json`) | tier | status | wall_s | result |
|---|---|---|---|---|
| nanogpt_compile | mid | PASS | 50.2 | compiled-vs-eager window diff 0.0014 nats, amp-vs-fp32 0.0026, forward rel err 0.0035; bf16 |
| triton_kernels | mid | PASS | 17.0 | 28 cases; softmax abs err 0, layernorm 0.0039, matmul worst rel err 3e-6; 5 autotuned shapes |
| cuda_ext_inline | mid | PASS | 41.9 | build 38 s, arch list `8.9+PTX`, `clock64` delta 49384 |
| amp_convnet_cifar | mid | PASS | 38.6 | accuracy 0.7375 amp / 0.7365 fp32, prediction agreement 0.9975, CUDA IPC round trip OK |
| stress_alloc | mid | PASS | 6.1 | 8.03 GB of 11.47 GB free allocated, 16 streams, 40 graph replays |
| managed_oversub | large | PASS | 51.7 | 14.75 GB managed memory on a 11.6 GB card, 59 chunks |

Cold total (run step) 209 s; warm repeat 109 s (`stage_warm_large_df0d0ba.json`: compile caches make `cuda_ext_inline` 4 s and
`managed_oversub` 14 s); the mid tier alone, final code, 106 s warm. Every row records die, compute capability, driver, the highest CUDA the
driver supports, torch's CUDA runtime and the flavour. The tests were run by the payload stage script itself (`kfml_stage.py run --tier large`;
`setup` took 1.7 s because the venv was made by hand earlier with the same command and the stage found it complete).

## What broke, and what it was

* `rows/first_run_4tests_cuda_ext_inline_FAIL_ninja.json` + `logs_first_run_ninja_failure/`: `cuda_ext_inline` FAIL "Ninja is required to
  load C++ extensions" although the venv has ninja. **Test-harness bug** (not CUDA, not driver): the venv's `bin/` was not on the child's
  PATH. Fixed in `runner.py`; the row PASSes (`first_run_cuda_ext_inline_after_path_fix.json`).
* Two probes printed `Intel oneMKL FATAL ERROR: Cannot load .../libtorch_cpu.so` and claimed torch was missing. **Operator error**: the
  process's working directory had been deleted by the operator's own `rm -rf` of the source directory. Reproduced on demand (`import torch`
  from a deleted cwd) and not otherwise (25 of 25 probes fine). Hardened in `env.py`/`executor.py`. Those two aborted attempts are not in
  this directory; the evidence is this paragraph (the first occurrence is inferred from its timing; the mechanism was reproduced on demand in run
  `kfml_run_20261116`).
* Nothing else failed on a healthy GPU stack, so there is no CUDA or driver issue to report.

## Supplements (labelled, not part of the baseline)

* `rows/supplement_cu128_wheels_on_driver560_all6.json`, `setup_cu128_forced_supplement.out`: `setup --flavor cu128` (forced; auto would
  not choose it here) installed from the hashed cu128 lock with `--require-hashes`, plus the redist cu128 nvcc 12.8.93 (sha256 matched the
  pins), and all six rows PASS with that torch (`torch` field `2.7.1+cu128`) on the same driver 560.35.03. The `flavor` field in
  that file says `cu126`: at that revision the field held the flavour derived for the box, not the installed one (fixed later; `torch` is
  the truth). A fact about cu128 wheels on a CUDA 12.6 driver (minor-version compatibility, sm_89 cubin), not about a CUDA 12.8 driver.
* `rows/supplement_cuda_ext_inline_redist_nvcc_cu126_only.json`: `cuda_ext_inline` PASS with only the venv-contained nvcc from
  `setup --with-nvcc` (cu126 redist archives, sha256 matched; `CUDA_HOME` set by the runner, system CUDA removed from PATH).
* `rows/reference_compare_same_die.json` / `reference_compare_other_die.json`: three rows run with `--reference` against the recorded baseline
  of this die (`how: same_die`, `wall_ratio` 0.62..1.06, loss-window diff to the reference 0.001) and against a copy relabelled as another
  card (`how: other_die`, `wall_ratio` null, loss diff 0.0009): the rule in `scripts/kfml/README.md`.
* `reference_recorded/nvidia-geforce-rtx-4070/`: the cold-run baseline written by `--record-reference`; the same files are committed under
  `scripts/kfml/reference/`.

## Files

`environment.txt` (OS, compilers, `nvidia-smi` query and header without UUID or bus id, pip freeze, torch arch lists), `check.txt` (what
`kfml check` chose and why, per-test verdicts), `list.txt`, `kfml-setup.json`, `setup_cu126_auto.out`, `rows/`, `logs_cold_run/` (the stage's
step logs; `run.log` has every row with its full metrics), `logs_first_run_ninja_failure/`.
UUIDs, IP addresses, host names, ssh ports and instance keys are not in these files (checked with a grep before the commit).

## Not shown by this run (UNVERIFIED)

A second die, Blackwell, another driver or distro, CPython 3.12/3.13, Docker, `build-wheels`, the stage under the payload on a kayfabe box,
anything inside a kayfabe guest.
