# kfml: ML research-style GPU workloads as gate tests

**STATUS: LIVE, 2026-11-16. MEASURED on one box (run id `kfml_run_20261116`; evidence `traces/kfml_run_20261116/`):
all six workloads PASS natively on a Vast CUDA container, GeForce RTX 4070 (sm_89, 12 GB), driver 560.35.03
(CUDA 12.6), wheel flavour cu126 chosen automatically, Ubuntu 22.04, Python 3.10.12, gcc 11.4, nvcc 12.6.85, torch
2.7.1+cu126, triton 3.3.1.** That is ONE die and ONE driver: nothing here was run on another die, on Blackwell, on
another driver, on Python 3.12, in Docker, or inside a kayfabe guest (every such claim below stays UNVERIFIED). The
GPU-free unit tests run in CI (`cd scripts/kfml && python3 -m unittest discover -s tests -t .`).

A standalone Python package. The runner (`python3 -m kfml`) is stdlib-only; torch, triton and the rest
are imported only inside the per-test child processes. Linux only.

```
python3 -m kfml list [--tier mid|large] [--json]
python3 -m kfml check                      # what is missing and how to install it; changes nothing
python3 -m kfml run [--only a,b] [--all] [--tier mid|large] [--json out.json] [--record-reference DIR]
                    [--reference DIR] [--log-dir DIR] [--budget-scale X]
python3 -m kfml setup --venv DIR [--flavor auto|cu128|cu126] [--with-nvcc | --nvcc-if-needed] [--allow-unhashed]
python3 -m kfml fetch-data                 # datasets only, into the venv
python3 -m kfml bundle --out DIR [--wheels DIR]
python3 -m kfml build-wheels --out DIR
```

Run from `scripts/kfml/` (or put it on `PYTHONPATH`). `run` prints one JSON row per test on stdout as it
finishes (JSON lines) and writes a JSON array with `--json`. Exit code 1 when any row is FAIL, 2 on a
usage error; NOT_RUN does not fail the run. `setup` exits 3 (and prints a `{"status": "NOT_RUN", "reason": ...}` line) when no
pinned flavour can run on the box. `run --tier mid` runs the gated mid tests; `--tier large` runs every test (mid plus
the non-gate ones).

## The rows

`{name, status, reason, wall_s, gpu_name, compute_cap, vram_gb, driver, driver_cuda, flavor, torch, cuda, metrics}`
(die, compute capability, driver, the highest CUDA the driver supports, the wheel flavour kfml derived, and torch's CUDA
runtime are recorded in every row, PASS or not) plus the
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
the lock.

### Flavour selection is automatic (owner, 2026-11-16: no die and no driver is pinned)

`setup` (default `--flavor auto`) and `check` derive the wheel flavour from two facts of the box: the highest CUDA
version the driver supports (the `CUDA Version` of `nvidia-smi`; fallback: the driver version against
`MIN_DRIVER_FOR_CUDA`) and the GPU's compute capability. The table is `pins.json` `flavors` (CUDA version, the
compute-capability range its wheels carry kernels for, the lock file and its sha256). The newest flavour wins that
satisfies both: driver CUDA >= the flavour's CUDA **and** the GPU's sm inside the flavour's range.

| flavour | needs driver CUDA | wheels carry | lock |
|---|---|---|---|
| cu128 | >= 12.8 | sm_75 .. sm_120 (Blackwell) | `requirements.lock.txt` |
| cu126 | >= 12.6 | sm_50 .. sm_90 in the wheel; kfml declares 7.5 .. 9.0 | `requirements.cu126.lock.txt` |

`kfml check` prints the choice and why (`flavor: cu126  (chose cu126 (driver CUDA 12.6 >= 12.6 and sm_89 is inside
sm_75..sm_90); rejected cu128: driver supports CUDA 12.6 < 12.8)`). When no pinned flavour fits (a Blackwell card on
a CUDA 12.6 driver, or a driver older than CUDA 12.6) the box is **NOT_RUN with the named reason, never FAIL**:
`setup` exits 3, `run` emits NOT_RUN rows (`no pinned flavour can run on this box: cu128: driver supports CUDA 12.6
< 12.8; cu126: its wheels cover sm_75..sm_90, the GPU is sm_120`). Unknown facts (no `nvidia-smi` CUDA version and
a driver not in the table, no compute capability) are not NOT_RUN: they are `env-missing` (FAIL), because nothing is
known to be unsupported. `--flavor cu126|cu128` still forces one (dev, comparisons).
MEASURED (`kfml_run_20261116`): the rule picked cu126 on driver 560.35.03 / sm_89. UNVERIFIED: every other branch (the
selection and the NOT_RUN path are covered by GPU-free tests with made-up facts, not by another box).
Observation, not a rule: on that same driver the cu128 wheels also imported and ran kernels (minor-version
compatibility; see the supplementary run in the trace), so the CUDA-version rule is conservative.

### Locks are hashed

Both locks are the full transitive closure with `--hash=sha256:` per line (generated 2026-11-16 on the Vast container
with `uv pip compile --generate-hashes`, command in each lock's header; resolved for CPython 3.10, UNVERIFIED for
3.12/3.13). `setup` installs them with `--require-hashes` (MEASURED: cu126 installed from the hashed lock on the
box and cu128 too), and refuses an unhashed lock unless `--allow-unhashed` (dev only). `pins.json` holds the sha256 of
each lock (a unit test compares them), and the sha256 of the lock used is stored in `DIR/kfml-lock.sha256` and in
every row. Nothing is resolved at test time.

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

### nvcc (for `cuda_ext_inline`, `managed_oversub`)

The test needs an nvcc whose major.minor equals `torch.version.cuda`; `kfml check` detects and compares,
`KF_ML_NVCC_LOOSE=1` downgrades a mismatch to a warning. `setup --nvcc-if-needed` installs the venv-contained toolkit
only when the box has no matching nvcc (a CUDA devel image does: nothing is downloaded then). The pip package
`nvidia-cuda-nvcc-cu12` contains no `nvcc` binary (author, 2026-11-11), so the venv-contained route is
`setup --with-nvcc`: three official NVIDIA redist archives (`cuda_nvcc`, `cuda_cudart`, `cuda_cccl`, sha256 in
`pins.json`, from NVIDIA's `redistrib_12.8.1.json` / `redistrib_12.6.3.json`) merged into `DIR/cuda_home`, which
the runner exports as `CUDA_HOME`. MEASURED (`kfml_run_20261116`): the cu126 archives and the cu128 archives both match
their pinned sha256; `cuda_ext_inline` PASSes with the assembled cu126 tree alone (no system CUDA on PATH, gcc 11,
Ubuntu 22.04) and with the system nvcc 12.6 of the container. UNVERIFIED: the assembled tree with a host compiler newer
than the nvcc supports without `tools/hostcc_gcc14.sh`, and on any other distro.

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

On any box with one NVIDIA GPU and its driver, a C++ compiler, `python3-venv` and `python3-dev` (a CUDA `devel`
container image has the compiler and an nvcc; `setup --nvcc-if-needed` fetches the venv-contained one otherwise):

```
python3 -m kfml setup --venv /opt/kfml-venv --nvcc-if-needed     # flavour derived from the box; hashed lock
/opt/kfml-venv/bin/python -m kfml fetch-data                       # sha256-pinned datasets (CIFAR-10: ~15 min from its server)
/opt/kfml-venv/bin/python -m kfml check                            # what it chose and why, per-test verdicts
/opt/kfml-venv/bin/python -m kfml run --tier mid --json rows.json --log-dir logs [--reference scripts/kfml/reference]
```

MEASURED on the run-id box (below), wall seconds: setup about 3 min (hashed cu126 wheels, about 4 GB at 500 Mbit), the
`run --tier large` step 190 s warm. The payload stage (`scripts/ci/kfml_stage.py`, `--only kfml`) runs the same three
steps. The Dockerfile derives the flavour at container start (no GPU at build time); UNVERIFIED, never built.

## The measured baseline, and how it is compared

`reference/<die>/<test>.json` holds the per-row baseline of run `kfml_run_20261116` (RTX 4070, sm_89, driver
560.35.03, cu126 flavour, cold caches): `wall_s`, the row's `metrics`, the workload's comparison `data` (loss
windows, accuracy) and `measured_on` (die, compute capability, driver, driver CUDA, torch, CUDA runtime, flavour, run id).
`kfml run --reference scripts/kfml/reference` compares against it; `--record-reference DIR` writes the same layout
for another die (one directory per die, a PASS row only). The rule, which is not pinned to a die:

1. `data` (algorithmic: losses on fixed seeds, accuracy) is compared with the best available reference: same die, else same
   compute capability, else any other die. Its tolerance (in the workload) does not depend on the die. Which one
   was used is in the row (`metrics.reference_from.how`: `same_die` | `same_cc` | `other_die`).
2. Wall time is never a pass criterion. `metrics.wall_ratio` (this row's wall_s over the reference's) is reported only against
   the same die; against another die it is null, because a ratio between two different cards measures the cards.
3. Parity of a kayfabe guest against this native baseline is therefore a same-box comparison (native run and guest run on one
   box, one die): record with `--record-reference` on the native box, run the guest with `--reference`.

## Unit tests (GPU-free)

`python3 -m unittest discover -s tests -t .` (in CI) covers requirement gating, rows, tier derivation, the
child-process executor (timeout kill, crash, signal, NotRun/EnvMissing mapping, against fake workload
modules), cache-dir confinement, dataset pin handling, upstream pin verification (wrong SHA, branch,
HEAD mismatch, patch that does not apply, NOTICE for every vendor dir), wheel licence derivation with
fake wheels (allowlisted, missing, custom, NVIDIA binaries), bundle selection and reproducibility, setup
refusals, `check`/`list`, the child's working directory, the flavour selection with made-up (driver CUDA, compute capability) pairs including
the NOT_RUN path, the lock/pins consistency (every lock hashed, its sha256 and index as pinned), the per-die
reference layout and comparison rule, and the PATH of the child. It also asserts that all workload modules import without torch.

## First run, 2026-11-16 (`kfml_run_20261116`): what broke, and what it was

Six workloads, one box. Findings, each with its decision:

* `cuda_ext_inline` FAIL `Ninja is required to load C++ extensions` although the venv has ninja: **test-harness bug**
  (the runner is started with an explicit interpreter path, so the venv's `bin/` was not on PATH). Fixed
  (`runner.py` puts it first on the child PATH, two unit tests); the row then PASSes. Not a CUDA or driver issue.
* A probe printed `Intel oneMKL FATAL ERROR: Cannot load .../libtorch_cpu.so` and reported torch missing, twice: once for the
  `check` and once for a whole stage run. In both the working directory of the process had been deleted underneath it by the
  operator's own `rm -rf` of the source directory it ran in. MEASURED: `import torch` from a deleted working directory
  reproduces the message exactly and the same import from an existing directory works (25 of 25 repeated probes of an undisturbed
  process were fine): **operator error, environment, not a CUDA or driver issue**. Hardened anyway (a stage that reads as "torch is
  missing" for an unrelated reason is worth one line): the probe and every test child now start in a directory that exists
  (`env.probe_torch` cwd, `executor.run_child` cwd=its work dir), with two unit tests that each fail without the change.
* Nothing else failed. No row was FAIL on a healthy GPU stack once the PATH was fixed, so there is no CUDA or driver issue to report.
* CIFAR-10: the pin in `pins.json` was compared with a second download from the dataset's own server (sha256 identical to the one the PC run
  printed on 2026-11-12), about 15 min at roughly 0.2 MB/s: that server, not the box, is the slow step.

## What is not verified

* Everything on a second die, on Blackwell, on another driver or distro, and on CPython 3.12/3.13: ONE die (RTX 4070 sm_89), ONE driver
  (560.35.03), one container (Ubuntu 22.04, CPython 3.10.12, gcc 11.4) were measured. The cu128 wheels were also run on that same
  driver as a supplement (all six PASS, cu128 redist nvcc 12.8): that is a statement about cu128 wheels with a CUDA 12.6 driver, not
  about a CUDA 12.8 driver.
* The loss/accuracy thresholds on any die but this one (the measured values sit far inside them: compiled-vs-eager window diff
  0.0003..0.0019 against 0.15, amp-vs-fp32 0.0026 against 0.25, ConvNet accuracy 0.7425 amp vs 0.744 fp32).
* `torch.compile` on Turing; the Triton autotune configs on sm_75 shared-memory limits (OutOfResources configs should be
  skipped by the autotuner); `managed_oversub` on anything but this card; the guest behaviour of every row (`KF_ML_GUEST=1`).
* The driver minimum table `MIN_DRIVER_FOR_CUDA`: only 12.6 -> 560.x is consistent with the measured driver; the others are NVIDIA's
  numbers from memory of the release notes. The selection reads `nvidia-smi`'s `CUDA Version` first, so the table matters only
  as a fallback and for the wording of the old "driver too old" message.
* Docker; `build-wheels`; the lock resolved for CPython 3.12/3.13; the assembled `--with-nvcc` tree with a host compiler
  that nvcc does not support; the stage under the payload on a kayfabe box (`python3-venv`, `python3-dev` and a C++ compiler on the
  KVM image).
