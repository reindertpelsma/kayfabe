# kfml: ML research-style GPU workloads as gate tests

**STATUS: LIVE, 2026-11-17. MEASURED natively on two Vast CUDA containers, no kayfabe in the path, ONE box per die: run `kfml_run_20261116` (GeForce RTX
4070, sm_89, driver 560.35.03 = CUDA 12.6, flavour cu126, Ubuntu 22.04, CPython 3.10.12, gcc 11.4, nvcc 12.6.85, torch 2.7.1+cu126, triton 3.3.1; all six workloads
PASS) and run `kfml_run_20261117_second_die` (GeForce RTX 2080 Ti, Turing sm_75, driver 595.91.07 = CUDA 13.2, flavour cu128 chosen automatically, nvcc 12.8 from the
pinned redist archives, torch 2.7.1+cu128; all six PASS after one fix, below). The flavour was derived from the box both times; no die and no driver is pinned. Evidence:
`traces/kfml_run_20261116/`, `traces/kfml_run_20261117_second_die/`. Never run: an Ampere or Blackwell die, a driver older than CUDA 12.6 (the NOT_RUN path is covered only by GPU-free tests
with made-up facts), CPython 3.12 installs, Docker, and any kayfabe guest (guest mode below is design plus GPU-free tests).** The GPU-free unit tests run in CI
(`cd scripts/kfml && python3 -m unittest discover -s tests -t .`).

A standalone Python package. The runner (`python3 -m kfml`) is stdlib-only; torch, triton and the rest
are imported only inside the per-test child processes. Linux only.

```
python3 -m kfml list [--tier mid|large] [--json]
python3 -m kfml check                      # what is missing and how to install it; changes nothing
python3 -m kfml run [--only a,b] [--all] [--tier mid|large] [--json out.json] [--record-reference DIR]
                    [--reference DIR] [--log-dir DIR] [--budget-scale X]
python3 -m kfml setup --venv DIR [--flavor auto|cu128|cu126] [--with-nvcc | --nvcc-if-needed] [--wheelhouse DIR] [--allow-unhashed]
python3 -m kfml fetch-data [--from DIR]    # datasets only, into the venv (--from: an offline copy, still sha256-checked)
python3 -m kfml wheelhouse --out DIR [--flavor auto|all|cu126,cu128] [--flavor-for DRIVER_CUDA:CC] [--guest-python 3.12] [--guest-glibc 2.39] [--with-data] [--verify]
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
(die, compute capability, driver, the highest CUDA the driver supports, the wheel flavour installed in the venv (the derived one when the venv has no setup record), and torch's CUDA
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

## Guest mode: the same rows inside the offline open Linux guest (design 2026-11-17, never run in a guest)

The payload's open Linux guest (the fat guest `boot_capture.sh` boots, entered by `scripts/bench/gssh_nv`, the transport `apps_hook.sh` already
uses: `tar | ssh tar -x`) has no internet. So nothing is fetched in the guest: the environment is built on the HOST at run time from this
directory's hashed locks, copied in, and installed offline with hashes. Code: `scripts/ci/kfml_guest.py` (flow and transport),
`scripts/ci/kfml_stage.py run --where guest`, the hook `scripts/ci/kfml_guest_hook.sh` (for `POST_CAPTURE_HOOK`), and in this package
`kfml wheelhouse`, `kfml setup --wheelhouse DIR`, `kfml fetch-data --from DIR`.

| step | where | what |
|---|---|---|
| probe | guest | `nvidia-smi` (die, compute capability, driver, the CUDA the driver supports), CPython, compiler, `python3-venv`, `Python.h`, free disk. The flavour is chosen from THE GUEST's facts: its driver is the operator's choice and is never pinned |
| wheelhouse | host | `kfml wheelhouse --flavor auto --flavor-for <driver CUDA>:<cc> --guest-python <X.Y> --with-data`: `pip download --require-hashes --only-binary=:all:` of the chosen flavour's closure for the guest's interpreter, the sha256-pinned redist nvcc archives, the pinned datasets; `MANIFEST.json` with size and sha256 of every file. No flavour fits = NOT_RUN with the named reason (exit 3), nothing is copied. Idempotent and resumable, so it can run before the guest boots |
| copy | host to guest | `scripts/kfml` (without tests and references) and the wheelhouse into `/opt/kfml`, bytes and seconds recorded |
| verify | guest | `kfml wheelhouse --verify` re-hashes every copied file (truncation, tampering and unlisted files are refused before pip runs) |
| setup | guest | `kfml setup --venv /opt/kfml/venv --wheelhouse /opt/kfml/wheelhouse --nvcc-if-needed` = `pip install --no-index --find-links <wheelhouse> --require-hashes -r <hashed lock>`; a guest that derives another flavour than the host shipped is a named FAIL `wheelhouse-flavour-mismatch` |
| fetch-data | guest | `kfml fetch-data --from <wheelhouse>/data` (still checked against `pins.json`) |
| run | guest | `kfml run --tier T --json` with `KF_ML_GUEST=1`; the rows are read back over the transport |

The result is the host stage's `KF_KFML_RESULT` document with `where: "guest"` and a `transfer` record (wheelhouse bytes and build seconds, copy bytes and
seconds, install seconds). Nothing secret enters the guest and no NVIDIA binary is stored by us: the wheelhouse is a transient directory on the box.
`kfml bundle`/`build-wheels` are for the licence-checked redistribution bundle and are not used here (they would mirror NVIDIA binaries).
The Linux propimage tooling has an `inject --wheelhouse` command for the proprietary Linux image; it was not on any branch available here (the
branch `claude/linux-image-manifest-inject-20261117` was not pushed), so this module uses the payload's own transport. The wheelhouse layout is plain
(`<flavor>/wheels/*.whl`, `pip --find-links` compatible), so that command can take the same directory.

GPU-free tests: `scripts/ci/test_kfml_guest.py` (fake transport, 47 tests), `kfml_guest_mutants.py` (18 mutants, two named killers each), and in this
package `tests/test_wheelhouse.py`. Not run in any guest; that needs a KVM box (list at the end of this section).

Sizes and times, MEASURED in `kfml_run_20261117_second_die` on a normal container (not a guest): the wheelhouse of ONE flavour for CPython 3.12 is 3.08 GB for cu126
(wheels 3 026 998 392 B + redist nvcc 52 057 232 B) and 3.96 GB for cu128 (3 881 029 847 B + 81 298 228 B); both together 7.04 GB (64 files) built in 112 s; the cu128 CPython 3.10 wheelhouse with both
datasets is 4 133 802 925 B (34 files). The guest-side steps ran offline on that box in 7 s (verify) + 67 s (install) + 1 s (data) + 82 s (the mid tier, warm; the large tier adds
`managed_oversub`, about 35 s). The copy host to guest is NOT measured: it is `tar | ssh` of about 4.1 GB; at 9 MB/s (what a plain scp to a Vast box reached, a lower bound for an ssh stream) it is 8 minutes, at
100 MB/s 40 seconds: ESTIMATE, bound by the guest's ssh CPU; a virtfs share, as `llm_parity` uses, is the fallback if it is slow. The stage needs the guest to keep about 2x the wheelhouse plus 6 GB free (about 14 GB).

What needs a KVM box (nothing of this has run): (1) the fat guest has `python3-venv`, `python3-dev`, a C++ compiler and the matching nvcc (else
`--nvcc-if-needed` installs the redist one: the copy of its three archives is in the wheelhouse), and enough disk (the stage refuses with
`guest-disk` below about 2x the wheelhouse plus 6 GB); (2) the Python of the guest matches `--guest-python` (the wheel tags); (3) `tar | ssh` copy of a
multi-GB wheelhouse through the tap network completes in reasonable time and does not disturb the device; (4) the six rows PASS in the guest under
kf3 (managed memory is `NOT_RUN managed-memory paging not supported` with `KF_ML_GUEST=1` by design; `torch.compile`, Triton autotune, `cudaGraph` replay and
the CUDA IPC round trip under kf3 are all unknown); (5) rows compared with the same-die native baseline by `--reference` on the same box.

## The recorded baseline, and how it is compared

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

## Second die, 2026-11-17 (`kfml_run_20261117_second_die`): what broke, and what it was

RTX 2080 Ti (sm_75), driver 595.91.07, flavour cu128 chosen automatically (the first die got cu126). Findings:

* `triton_kernels` FAIL `RuntimeError: PassManager::run failed` (`Unsupported conversion from f16 to f16`). Run `kfml_run_20261117_second_die` showed: Triton 3.3.1 fails to
  compile the matmul kernel with fp16 or bf16 inputs for sm_75 for every configuration tried (nine), and compiles it with fp32 inputs. A toolchain limit of the pinned
  Triton on Turing: not a test bug, not the driver, not kayfabe, worth an upstream report (not filed; not looked up). Decision: below sm_80 a compile failure of the fp16
  variant is recorded (`metrics.fp16_dot_compile`) and the kernel is checked with fp32 inputs (`matmul_input_dtype`); at or above sm_80 it stays a FAIL. The row is
  PASS with that metric, so a Triton that fixes it flips the metric without a change here. This closes the "Triton autotune on sm_75" item as far as that one kernel goes; the
  autotune shared-memory limits of the other configs were not the cause (they all failed the same way at compile time).
* nanogpt_compile took the fp16 + GradScaler path (no bf16 on Turing) and matches the 4070's bf16 path within 0.0015 nats (tolerance 0.25); the ConvNet accuracy is 0.7275 against
  0.7375 on the 4070 (tolerance 0.10, seeds and the algorithm are the same; the difference is numerics, not a defect). `torch.compile` on Turing works (answers the old
  "`torch.compile` behavior on Turing" question for this torch and triton).
* The offline wheelhouse proof found two bugs (pip honours `--index-url` inside a requirements file even with `--no-index`; `pip download --platform` does not expand manylinux tags):
  both fixed with unit tests (guest mode below).
* The CIFAR-10 server gave about 45 KB/s per connection. The archive was copied in after six parallel ranged downloads (sha256 equal to the pin; a third confirmation), so the stage's own
  fetch of that file was not exercised on this box.

## What is not verified

* Everything on a third die class, on Blackwell, on a driver older than CUDA 12.6 (so the NOT_RUN-because-no-flavour path on a real box), on another distro, and on CPython
  3.12/3.13: TWO dies (RTX 4070 sm_89 / driver 560.35.03 / cu126; RTX 2080 Ti sm_75 / driver 595.91.07 / cu128), one container image family (Ubuntu 22.04, CPython 3.10.12, gcc 11.4)
  were measured. The cu128 wheels were also run on the 560.35.03 driver (supplement of the first run: all six PASS), which says nothing about other CUDA 12.8 drivers.
* The loss/accuracy thresholds on any die but these two (the measured values sit far inside them; see the traces).
* `torch.compile` and Triton on anything but sm_75 and sm_89 (the fp16 `tl.dot` fallback is only exercised on sm_75); `managed_oversub` on anything but these two cards; the guest behaviour of every row
  (`KF_ML_GUEST=1`), guest mode included.
* The driver minimum table `MIN_DRIVER_FOR_CUDA`: 12.6 -> 560.x and 13.2 (driver 595.91.07) are consistent with the measured drivers; the others are NVIDIA's numbers from memory of the release
  notes. The selection reads `nvidia-smi`'s `CUDA Version` first, so the table matters only as a fallback and for the wording of the old "driver too old" message.
* Docker; `build-wheels`; the lock resolved for CPython 3.12/3.13 (a 3.12 wheelhouse was built and hashed, not installed); the assembled `--with-nvcc` tree with a host compiler that nvcc does not
  support; the stage under the payload on a kayfabe box (`python3-venv`, `python3-dev` and a C++ compiler on the KVM image); everything in guest mode that needs a guest.
