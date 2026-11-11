# kfml: ML research-style GPU workloads as gate tests

**STATUS: DESIGN, NOT RUN ON HARDWARE, 2026-11-11.** Everything below was written without a GPU. What
was executed: the GPU-free unit tests (`python3 -m unittest discover -s tests -t .`), and the logic of
`nanogpt_compile` on CPU (`KF_ML_DEV_CPU=1`, author debugging only; not a result). No test has
touched a CUDA device. Items marked UNVERIFIED need a Vast CUDA container.

A standalone Python package. The runner (`python3 -m kfml`) is stdlib-only; torch, triton and the rest
are imported only inside the per-test child processes. Linux only.

```
python3 -m kfml list [--tier mid|large] [--json]
python3 -m kfml check                      # what is missing and how to install it; changes nothing
python3 -m kfml run [--only a,b] [--all] [--json out.json] [--record-reference DIR] [--reference DIR]
                    [--log-dir DIR] [--budget-scale X]
python3 -m kfml setup --venv DIR [--flavor cu128|cu126] [--with-nvcc] [--allow-unhashed]
python3 -m kfml fetch-data                 # datasets only, into the venv
python3 -m kfml bundle --out DIR [--wheels DIR]
python3 -m kfml build-wheels --out DIR
```

Run from `scripts/kfml/` (or put it on `PYTHONPATH`). `run` prints one JSON row per test on stdout as it
finishes (JSON lines) and writes a JSON array with `--json`. Exit code 1 when any row is FAIL, 2 on a
usage error; NOT_RUN does not fail the run.

## The rows

`{name, status, reason, wall_s, gpu_name, compute_cap, vram_gb, driver, torch, cuda, metrics}` plus the
resolved pins: `cudnn, triton, lock_sha256` (of the lock the venv was set up from), `upstream_commits`,
`patch_sha256`, `dataset_sha256`, and `tier, launch_bound, guest_estimate_s`.

* **PASS / FAIL / NOT_RUN.** NOT_RUN is reported only when the hardware cannot run the test (compute
  capability below/above the declared range, VRAM below the declared total, driver too old for the
  installed CUDA wheel, the documented known-unsupported probe). Software that is missing (no torch, no
  nvcc, no C compiler, no triton, a torch older than declared, nvcc not matching torch's CUDA) is a
  **FAIL with reason `env-missing: ...`**, never a quiet green. A missing GPU is also a FAIL.
* **Driver independence.** No test pins, asserts or depends on a driver version; the driver is the
  variable of the release sweep and is only recorded. `torch.cuda` failing to initialise with a
  driver-too-old signature gives NOT_RUN `driver too old for this CUDA wheel (needs >= X)`; any other
  init failure is FAIL. The `X` table (`MIN_DRIVER_FOR_CUDA` in `kfml/env.py`) is copied from NVIDIA's
  CUDA release notes and is UNVERIFIED here.
* **Budgets.** Each spec has a hard wall-clock budget; the test process is killed (whole process group)
  when it expires (FAIL `timeout`). Workloads also stop at 80% of the budget (`ctx.expired()`).
  `--budget-scale` or `KF_ML_BUDGET_SCALE` multiplies budgets for slow nested guests. The first run
  downloads several GB of CUDA wheels at `setup` time and the datasets at first use; every spec says so in
  `download_note`.
* **VRAM guard.** The documented floor is 8 GB total (`Spec.min_vram_gb >= 8` is enforced). Sizes that
  scale (triton large shapes, `stress_alloc` at 70% of free VRAM, capped at 48 GB) are computed from the
  free VRAM torch reports.
* **Determinism and tolerances.** Fixed seeds; every tolerance is against an in-process reference (eager
  fp32 / torch ops) or a recorded native-host reference (`--record-reference DIR` writes
  `DIR/<test>.json`; `--reference DIR` compares against it when the recorded config matches). No
  GPU-specific numbers are hard-coded. Loss and accuracy thresholds are generous, picked from
  reasoning and not from a measurement (UNVERIFIED).

## Tests (first batch)

| name | what | tier | needs |
|---|---|---|---|
| `nanogpt_compile` | vendored nanoGPT model, char Shakespeare, 400 steps three ways: fp32 eager, autocast eager, autocast `torch.compile`; SDPA; bf16 (Ampere+) or fp16+GradScaler (Turing) | mid | triton, C compiler |
| `triton_kernels` | add, fused softmax, layernorm fwd, tiled matmul with autotune vs torch, odd shapes | mid | triton, C compiler |
| `cuda_ext_inline` | `load_inline`: smem block reduction, atomicAdd histogram, cooperative-groups warp reduce, grid-stride loop, `__nanosleep`/`clock64`; `TORCH_CUDA_ARCH_LIST` = cc + PTX | mid | nvcc = torch CUDA major.minor |
| `amp_convnet_cifar` | ResNet-like net, CIFAR-10 subset, AMP, channels_last, cudnn.benchmark, DataLoader 2 workers + pin_memory, CUDA IPC round trip through a spawned process | mid | dataset |
| `stress_alloc` | alloc/free to 70% of free VRAM with content checks, churn, 16 streams + events, CUDA graph replay vs eager, pinned async copies | mid | - |
| `managed_oversub` | `cudaMallocManaged` oversubscription; **not in the gate** (`--only` or `--all`) | large | nvcc |

`managed_oversub` on a kayfabe guest: with `KF_ML_GUEST=1` it is NOT_RUN `managed-memory paging not
supported` and does not run; `KF_ML_PROBE_TRY=1` attempts it (failure stays NOT_RUN, success is PASS).
Without `KF_ML_GUEST` it must PASS.

### Tiers

`tier`, `launch_bound` live in the spec; `host_runtime_s` in `timings.json` (null until the coordinator
measures it on the slowest sampled card). `guest_estimate_s = host_runtime_s * (4.0 if launch_bound
else 1.5)`. A test is mid only if its estimate is <= 480 s and the mid set totals <= 900 s
(`kfml list --tier mid` prints the sets and the total; a unit test enforces the rule on the real specs
once timings exist). Until timings exist all tiers are the author's guess (UNVERIFIED).

## Environment: everything lives under the venv

`kfml setup --venv DIR` is the only installing command and touches only DIR (it refuses a non-empty
directory that is not a venv). Torch comes from the explicit `download.pytorch.org/whl/<flavor>` index in
the lock. `requirements.lock.txt` is flavor cu128 (default; needed for Blackwell sm_120),
`requirements.cu126.lock.txt` is the older-driver flavor (no sm_120), selected with `--flavor`.

**HASHES TODO: the coordinator generates the hashed lock on a Vast container** (`pip-compile
--generate-hashes`, command in the lock header), per target Python. The files hold the real exact
top-level pins only; `setup` refuses an unhashed lock unless `--allow-unhashed` (dev only) and uses
`--require-hashes` as soon as the file has hashes (or `--installer uv`). Nothing is resolved at test time.
The sha256 of the lock used is stored in `DIR/kfml-lock.sha256` and in every row.

For every test process the runner sets `HOME, HF_HOME, HF_HUB_CACHE, TORCH_HOME, XDG_*,
PIP_CACHE_DIR, UV_CACHE_DIR, TRITON_CACHE_DIR, TORCHINDUCTOR_CACHE_DIR, TORCH_EXTENSIONS_DIR,
CUDA_CACHE_PATH, TMPDIR/TMP/TEMP, MPLCONFIGDIR, PYTHONPYCACHEPREFIX, KF_ML_DATA` to subdirectories of
`DIR/kfml-cache/`, overriding whatever the caller had. `kfml check` prints the resolved paths. A unit
test asserts no child env points outside the venv. **Delete the venv directory = full cleanup.**
Outside a venv, `run` refuses unless `KF_ML_HOME=DIR` names the one directory to use (Docker).

Datasets and weights are downloaded there on first use and verified against `pins.json` sha256
(`tinyshakespeare` pinned; `cifar10_binary` see below). An unpinned file is kept as `*.unverified`, its
sha256 printed, and the run fails with `PIN ME` unless `KF_ML_ALLOW_UNPINNED=1` (dev only). CIFAR-10 uses
the binary archive so no pickle is ever loaded. Kaggle is not used.

### nvcc (for `cuda_ext_inline`)

The test needs an nvcc whose major.minor equals `torch.version.cuda`; `kfml check` detects and compares,
`KF_ML_NVCC_LOOSE=1` downgrades a mismatch to a warning. Measured on 2026-11-11: the pip package
`nvidia-cuda-nvcc-cu12` does **not** contain an `nvcc` binary (only `ptxas`, `libnvvm`, `libdevice` and
headers), so the "pip nvcc" route does not work. `setup --with-nvcc` instead downloads three official
NVIDIA redist archives (`cuda_nvcc`, `cuda_cudart`, `cuda_cccl`, sha256 in `pins.json`, taken from
NVIDIA's `redistrib_12.8.1.json` / `redistrib_12.6.3.json`) and merges them into `DIR/cuda_home`, which
the runner exports as `CUDA_HOME`. **UNVERIFIED:** that this assembled tree builds `load_inline`
extensions (the host compiler must be one nvcc supports, e.g. gcc 13 on Ubuntu 24.04).

No NVIDIA binary is ever mirrored in our storage; wheels and archives come from the official indexes
on each box.

## Upstream code pins

Every upstream source is declared in the spec as `Upstream(repo_url, commit (40 hex), license, role,
patch)`; a branch or tag is rejected at declaration time. Roles: `vendored` (small, MIT/BSD/Apache only,
copied into `vendor/<dir>/` with `LICENSE` and a `NOTICE` naming the URL and commit; nothing fetched at
run time), `fetched` (cloned into a fresh temp dir at the pin; `git rev-parse HEAD` must equal the pin or
the test is a FAIL `pin refused`; the patch from our tree is applied with `git apply --check` first; both
recorded in the row), `derived` (we wrote our own code following it; credit only). Currently one vendored
(nanoGPT `model.py`, verbatim, no patch) and one derived (the Triton tutorials). `CREDITS_PROPOSAL.md`
lists the proposed credits lines (`docs/howto/credits.md` is not edited here).

## Bundle and wheel building

`kfml bundle --out DIR [--wheels DIR]` writes `kfml-bundle.tar.gz` (sorted, mtime 0, uid 0, gzip mtime 0,
so its sha256 is reproducible and printable/pinnable), `MANIFEST.json` and a `.sha256`. The wheels dir is
filled by the user (`pip download -r <hashed lock> -d DIR`). For every wheel the licence is **derived from
the wheel itself** (`License-Expression`, `License`, classifiers, `*.dist-info` licence files, text
sniffing) and mapped to SPDX. A component enters the bundle only when it resolves to the allowlist (MIT,
BSD-2/3-Clause, Apache-2.0, ISC, CC0-1.0, Unlicense) **and** nothing in it is NVIDIA-origin (`nvidia-*`
packages, triton, `+cu` builds, `libcu*`/`libnv*`/`libcudnn*`/`libnccl`, `ptxas`, `cuobjdump`, `nvdisasm`,
...). Missing, unclear, custom or conflicting licences are fetch-only and flagged; "UNKNOWN" is never
allowed. The manifest lists each wheel's bundled shared libraries. Datasets and weights are separate
components, fetch-only unless `redistribution_ok` is set and the licence is allowlisted.
**Nothing is uploaded by the tool.** A public release additionally needs a GitHub-built attestation (owner
rule); until then the bundle is for our own CI with a sha-pinned copy.

`kfml build-wheels --out DIR` builds the `wheel_builds` entries of `pins.json` (empty today; for
compile-heavy projects such as flash-attn or mamba-ssm) from the pinned commit plus our patch with
`TORCH_CUDA_ARCH_LIST` covering sm_75..sm_120 + PTX (12.0 only with CUDA >= 12.8), `pip wheel
--no-build-isolation` against the venv's torch, then scans each wheel and writes `build-records.json`
(tags, python, torch, CUDA flavor, glibc, upstream commit, patch sha256, bundled libs; a wheel with
NVIDIA-origin libraries inside is marked refused). It should run in an old-glibc manylinux-style image;
that image and command are **UNVERIFIED** (the `Dockerfile` here is for running the tests, not for
building wheels). Not run anywhere yet.

## Running

In a venv: `python3 -m kfml setup --venv /opt/kfml-venv --allow-unhashed` (until the hashed lock
exists), then `/opt/kfml-venv/bin/python -m kfml check` and `... run`. On a CUDA container:
`docker build -t kfml scripts/kfml` then `docker run --gpus all kfml run`. The Dockerfile installs system
packages and the venv at build time; `run` itself never installs (UNVERIFIED, never built).

## Unit tests (GPU-free)

`python3 -m unittest discover -s tests -t .` covers requirement gating, rows, tier derivation, the
child-process executor (timeout kill, crash, signal, NotRun/EnvMissing mapping, against fake workload
modules), cache-dir confinement, dataset pin handling, upstream pin verification (wrong SHA, branch,
HEAD mismatch, patch that does not apply, NOTICE for every vendor dir), wheel licence derivation with
fake wheels (allowlisted, missing, custom, NVIDIA binaries), bundle selection and reproducibility, setup
refusals, `check`/`list`. It also asserts that all workload modules import without torch.

## What is not verified

All six workloads on a GPU; the loss/accuracy thresholds; `torch.compile` behavior on Turing; the Triton
autotune configs on sm_75 shared-memory limits (OutOfResources configs should be skipped by the autotuner);
the `--with-nvcc` tree; Docker; `build-wheels`; the driver minimum table; the CIFAR-10 sha256 if it shows
`null` in `pins.json` (run `fetch-data`, it prints the measured hash).
