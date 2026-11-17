# V3 open ML rows: vLLM, TensorRT and JAX / XLA (discovery lane)

**STATUS: LIVE, 2026-11-17 (code and GPU-free tests; native results from Vast CONTAINER boxes without kayfabe, section 8; NOT run in a kayfabe guest. Every sentence tagged [inferred] or [not measured] is a hypothesis with a falsifier in section 10).** Owner rulings of 2026-11-17 (binding): go OPEN FIRST; an open row works for a standalone third party who trusts git only, from the repository alone, fully automated: no private R2 object, no account, no secret; nothing from NVIDIA is stored in our images, R2 or release assets (owner ruling 2026-11-07: pip installs from PyPI at run time are fine). A row asserts that the GPU did the work (owner requirement 2026-11-17, [V3_GPU_USE_ASSERTION.md](V3_GPU_USE_ASSERTION.md), this work is built on that branch, PR #475).

Labels: **[run: <directory>]** a logged native run (its directory is named), **[read]** a file or page read, **[inferred]** my reading, not tested, **[not measured]**.

## 1. What the three rows are

All three are **discovery rows** of the open Linux matrix: they are in `scripts/apps/run_apps.sh` in a second list (`DISCOVERY`, in `list`, not in `listall` or `all`), in no gate list (a test pins it), and `matrix_table.py` classes them `DISCOVERY`. They become gate rows only after three clean discovery runs on two dies (the proposal of `V3_PROPIMAGE_LINUX.md` section 7).

| row | what it asks of the driver | what a failure is | wheels (flavour by driver) |
|---|---|---|---|
| `vllm` | the offline `LLM(...)` on a 135M-parameter model, temperature 0, 8 fixed prompts: the engine core in a process of its own (a second CUDA context on the die); a profiling forward that sizes the KV cache from the memory the device reports (`gpu_memory_utilization` 0.9, the vLLM default); `torch.compile` and CUDA graph capture for the engine's whole list of batch sizes; greedy decoding in batches of 8, 1 and 40 | tokens missing or not identical on a repeat, no engine process, fewer than 8 graph sizes, device memory did not grow; between host and guest: different token ids, another number of captured graphs | `cu13` only (vllm 0.34.0, torch 2.14.0): a driver of CUDA 13.0 (R580) or newer |
| `tensorrt` | TensorRT 10.16 builder: ONNX parse, FP16, a dynamic batch profile, an EMPTY timing cache so the builder times its candidate kernels (2283 tactic log lines, 51 KB timing cache); engine deserialize and run through the CUDA runtime; the same model through ONNX Runtime's TensorRT execution provider | builder did not run FP16 or timed no tactic, engine output further than 1 percent of the CPU reference's largest value from ONNX Runtime on the CPU, the provider ran no node on TensorRT | `cu13` (driver CUDA 13.0): TensorRT 10.16.1.11 + onnxruntime-gpu 1.32.0; `cu12` (driver CUDA 12.0): TensorRT 10.16.1.11 + onnxruntime 1.32.0 (no TensorRT provider in that wheel: the provider part is `NOT_RUN` by name) |
| `jax` | `jax 0.12.0.1`: the default 75 percent memory slab at the first operation, the XLA GEMM / convolution autotuner (its result dump is the evidence), `jit` of a float32 matmul at HIGHEST and at the default precision, a convolution and a `lax.scan`, each against the same function on JAX's CPU backend | `jax.devices()` not a GPU, a result off its CPU value, the preallocated limit outside 68 to 78 percent of the device's memory, no autotuner entry from a backend other than the emitter's own | `cu13` (driver CUDA 13.0, libraries pinned to the CUDA 13.0 train) or `cu12` (driver CUDA 12.0) |

`NOT_RUN` (with a named reason, never `FAIL`): no GPU or no `nvidia-smi`, a GPU older than Turing (`sm_75`; the TensorRT wheels carry builder resources for `sm_75` and up, torch 2.14 CUDA 13 starts at `sm_75`), a driver that supports less CUDA than any flavour of the row (vLLM on a CUDA 12 driver), Python outside 3.12 to 3.14, too little video memory or disk, no C compiler and Python headers (vLLM: Triton and `torch.compile` build a launcher), the row not set up. Unknown facts (no `compute_cap` in `nvidia-smi`, no CUDA version) are a named `FAIL` of the environment, not a `NOT_RUN`: nothing is known to be unsupported.

## 2. Files

| file (`scripts/apps/mlrows/` unless noted) | role |
|---|---|
| `mlrows.py` | the runner: `list`, `check`, `lockcheck`, `setup`, `run`, `compare`, `push`; stdlib only; processes and the origin's HTTP are replaceable in the tests |
| `mlrows_lib.py` | every decision as a pure function: facts, flavour choice, lock validation, the model plan and byte check, the three judges, the proof line, the host-against-guest comparison, the vLLM log facts |
| `row_vllm.py`, `row_tensorrt.py`, `row_jax.py` | the workloads, run by the venv's Python; each prints ONE `MLROW_JSON` line and decides nothing; none downloads anything (a test pins it) |
| `pins.json` | flavours, floors, the model (repo, commit, licence, files), the OSS TensorRT commit for a `trtexec` source build |
| `locks/*.lock.txt`, `locks/*.in`, `gen_locks.py` | five hashed locks (`--hash=sha256:` per file as the index publishes it), their inputs, and the command that regenerates them (`uv pip compile --generate-hashes`, the method of kfml's locks) |
| `run_standalone.sh` | 16 lines of glue: apt prerequisites as root, `check`, `setup`, `run` of the three rows, the summary lines |
| `scripts/ci/test_mlrows.py`, `scripts/ci/mlrows_mutants.py` | GPU-free tests and the mutants with two named killers each; fixtures in `scripts/ci/fixtures/mlrows/` |

Wiring (a few lines of shell, all logic is Python): `run_apps.sh` gets the `DISCOVERY` list, `listall`, `APPNOTE` and `APPNOTRUN` (a row that was NOT RUN with exit 0 is verdict `NOTRUN` with the reason, never `PASS` or `FAIL`; a crash that printed it is still `FAIL`); `apps_matrix.sh` (host: tool dir, `mlrows.py setup` BEFORE the GPU lock is taken; end of a guest run: `mlrows.py compare`); `apps_hook.sh` (`mlrows.py push` copies the tool dir and the staged environments into the guest, which has no network); `matrix_table.py` (class `DISCOVERY`); `gpu_use_rows.py` (the three rows with class, proof kind, floor and host engines); the image recipe's pinned hash of `run_apps.sh`; `ci.yml` (the mutants). Not touched: `build_bundle.sh`, the gate lists, `bandwidth.json` (these rows are not payload stages: the Worker's apps stage never runs them; sizes are in section 3).

## 3. Supply chain: the trust anchor of every input

Owner rule 2026-11-17: a sha256 of a binary we host or build, pinned in git, is not an anchor. Accepted: an official upstream checksum or signature fetched from the upstream origin, a pinned upstream git commit or release identifier, a hashed lock of PyPI's own digests. A test fails when any file under `scripts/apps/mlrows/` outside the locks holds a 64-hex digest; the one allowed field is `expected_token_sha256` (the digest of a row's own OUTPUT, null until section 8 says it is stable across dies).

| input | where it comes from | anchor |
|---|---|---|
| Python packages (vllm, torch, jax, onnx, onnxruntime, numpy, 190 more) | PyPI | the lock: `pip install --require-hashes --only-binary=:all: --no-deps -r <lock>`; digests are the ones PyPI lists for each file (generated by `uv pip compile --generate-hashes` on 2026-11-17 for CPython 3.12, `manylinux_2_34`); a missing digest, an unpinned or URL requirement, a foreign or plain-http index, a duplicate name is refused before pip runs (tests) |
| TensorRT wheels (`tensorrt-cu12/cu13-libs` 3.5 to 4.1 GB, `-bindings`) | NVIDIA's own open index `https://pypi.nvidia.com/` | the same lock; the digest is the one that index publishes beside each file. [read: the index answers without login, 2026-11-17] |
| the model `HuggingFaceTB/SmolLM2-135M` (Apache-2.0, 8 files, 272 MB) | `huggingface.co`, at the commit `93efa2f0...` (a git object id, reviewable at the origin) | each file is checked against the size and the sha256 (the LFS object id) or git blob id the origin's tree API lists for exactly that commit; a mismatch deletes nothing and writes nothing (the file is written only after the check); the row then runs with the hub offline |
| OSS TensorRT source (for a `trtexec` build, NOT done) | `github.com/NVIDIA/TensorRT`, tag `v10.16` = commit `52399f55...` | a pinned upstream commit; recorded in `pins.json`, used by no code |
| the driver, `libcuda`, `nvidia-smi` | the machine under test | not downloaded |
| test data | generated at run time: the ONNX network from a seeded numpy generator (`onnx.helper`, no torch), the prompts are in the script | none needed |

Nothing is hosted, mirrored, cached or pinned by us. **Nothing NVIDIA ships is stored by us**: every machine downloads its own copy of the CUDA and TensorRT wheels into its own venv; `mlrows.py push` copies a venv from the bench host into the guest for the duration of a run (the same transient copy kfml's guest mode makes), it is never uploaded.

**TensorRT: the install route and the licence (checked 2026-11-17, [read]).** The PyPI projects `tensorrt`, `tensorrt-cu12`, `tensorrt-cu13` and `tensorrt-cuXX-libs` are source stubs: the `-libs` sdist (15 KB) builds with `wheel_stub` and fetches the real wheel from `https://pypi.nvidia.com/`, an open index whose listing carries a `sha256` per file; `-bindings` wheels (1 MB) are on PyPI and on that index. No account, login or developer programme is involved (the earlier `developer login` note was about the SDK tar, not the pip route). The meta package `tensorrt-cuXX` is a pure-Python sdist built with setuptools in an isolated environment whose build requirements pip fetches WITHOUT hashes; the row therefore does not install it: it installs the libs and bindings wheels (both hashed) and imports `tensorrt_bindings`, which the meta package's `tensorrt/__init__.py` only re-exports. **`trtexec` is in no pip wheel** (the libs wheel holds `libnvinfer`, the builder resources per architecture, the plugin library and the ONNX parser, no executable and no headers). The row drives the same builder (parser, builder, FP16 flag, optimization profile, timing) through the Python API and reports `trtexec` as `NOT_RUN reason=trtexec_is_not_in_the_pypi_wheels`. A `trtexec` from source is possible: the OSS repository carries the headers and `samples/trtexec` (Apache-2.0), CMake builds it against the wheel's libraries with `-DBUILD_PARSERS=OFF -DBUILD_PLUGINS=OFF -DBUILD_SAMPLES=ON -DCUDA_VERSION=13.0`, but it needs CMake 3.31 or newer (Ubuntu 24.04 has 3.28) and the repository's submodules [not measured, not done]. **Licence:** the TensorRT wheels declare `License: Proprietary`; their `LICENSE.txt` is NVIDIA's Software License Agreement, which says installing or using the software is acceptance. **Owner question:** clause 2.1(iii)(b) of that agreement forbids making the software available to third parties "by operating hosted/virtual system environments"; running it in our own test guest on a box we rent is internal use, but a kayfabe user who runs this row inside a virtual-GPU guest they offer to others is outside it. The row prints the licence name at setup; nothing in this change redistributes any NVIDIA file.

**Size and time of a setup** [run: traces/open_ml_rows_20261117]: JAX cu13 4.4 GB venv, 177 s; TensorRT cu13 7.3 GB, 335 s; vLLM cu13 9.6 GB + 272 MB model, 465 s; JAX cu12 5.8 GB, 233 s; TensorRT cu12 7.0 GB, 294 s (a 500 Mbit link, up to two setups at once). A standalone run of all three downloads about 21 GB. `ci/vast-broker/bandwidth.json` is unchanged: the Worker's payload never runs these rows.

## 4. Flavour by driver

`mlrows.py check` prints the choice and why. The rule (the same two steps as kfml, without its compatibility step): the newest flavour whose driver floor (the CUDA version `nvidia-smi` prints) the machine meets; a forced `--flavor` is still checked against the driver. The table is `pins.json`.

| row | cu13 needs | cu12 needs | why one flavour only |
|---|---|---|---|
| vllm | driver CUDA 13.0 | not offered | the PyPI wheel of vllm 0.34.0 pins `torch==2.14.0`, whose PyPI wheel is built for CUDA 13.0 (`cuda-toolkit==13.0.3`, `nvidia-cudnn-cu13`); vLLM's CUDA 12 wheels are not on PyPI |
| tensorrt | driver CUDA 13.0 | driver CUDA 12.0 | onnxruntime-gpu 1.32.0 on PyPI is built for CUDA 13 and TensorRT 10 (`libcublas.so.13`, `libnvinfer.so.10`, read from the wheel); the cu12 flavour carries the CPU wheel |
| jax | driver CUDA 13.0 | driver CUDA 12.0 | both plugins exist; the cu13 lock is constrained to the CUDA 13.0 train (nvcc, runtime, cuBLAS, cuFFT, cuSOLVER, cuSPARSE, NVRTC, nvJitLink 13.0.x) so a 580 driver is enough, the unconstrained resolution picked 13.5 |

MEASURED [run: traces/open_ml_rows_20261117]: cu13 on a driver of exactly CUDA 13.0 (580.159.03, RTX 3060); cu12 of JAX and TensorRT forced on that same CUDA 13 driver (both PASS). NOT measured: a CUDA 12 driver (the refusal of vLLM there and the selection of cu12 are GPU-free tests only).

## 5. How a row is judged

Each row script prints one JSON line of measurements; `mlrows_lib.py` decides. The tolerances are recorded where they were measured (RTX 3060, driver 580.159.03, 2026-11-17).

- **vllm**: eight token lists, each non-empty with non-negative ids; `token_sha256` equals the canonical hash of the ids; the same batch generated twice is identical; `VLLM::EngineCore` is a child process; the engine's capture plan lists at least 8 batch sizes; the device's memory grew by at least 300 MiB (the runner samples `nvidia-smi`, the parent process never creates a CUDA context). Recorded, not judged: batch invariance (the 8 prompts one at a time give the same ids), copies of one prompt in a batch of 40, the engine's own log facts (KV cache tokens, graph capture seconds and GiB, model load GiB).
- **tensorrt**: FP16 builder built an engine; the timing cache is not empty; the engine ran twice with identical output; its largest deviation from ONNX Runtime on the CPU is at most 1 percent of the reference's largest absolute value; when the provider exists, its first provider is TensorRT, its profile shows at least one node event on `TensorrtExecutionProvider`, and its deviation is within the same bound (a profile with only `CPUExecutionProvider` events is a fallback and fails).
- **jax**: all devices are `gpu`; four operations equal their CPU values (HIGHEST precision at most 1e-4 of the largest value, the default precision at most 3e-3 because TF32 gives 3e-4) and their result lives on the GPU; XLA's `bytes_limit` is 68 to 78 percent of the memory `nvidia-smi` reports (it is 75 percent of what CUDA reports, which is less by the driver's reserve: 72.7 percent on a 12 GB card); the device memory really grew by at least 60 percent; the autotuner dump has an entry from a backend other than `NATIVE_EMITTER`; the allocator's peak is at least 32 MiB.

**GPU use** (V3_GPU_USE_ASSERTION.md): `mlrows.py run` prints `GPU_USE_PROOF <row> device=cuda:0 name=<name> peak_mib=<growth>` after the work, from the memory growth it measured (guest self-report); `gpu_use_rows.py` has the rows (`vllm` a+proof, floor 300 MiB; `tensorrt` and `jax` class b, floors 200 and 1000 MiB; host engine `gpu` for all), so in a guest the host census of the kf3 log and the host's `nvidia-smi` are the authority and the line can only add a failure. **Negative controls** (`KF_NEGCTL=1`): the GPU is hidden (`CUDA_VISIBLE_DEVICES=`), JAX gets `JAX_PLATFORMS=cpu`, TensorRT runs its CPU reference only; each is `FAIL` by the row's own judge and by the GPU-use judge (`CONTROL_OK`), MEASURED section 8. A stale file can never count: the working directory and the cache directory of a run are removed first (the first draft read a leftover XLA autotune dump in the CPU run: found by the control, fixed, pinned by two tests).

## 6. Environment of a row

An allowlist (`PATH`, `LANG`, `LC_ALL`, `TZ`, `TERM`) plus ours: no token, key, loader hook or proxy variable of the caller reaches the row; every cache and `HOME` is under the run's own fresh `cache/` (so the compiler and JIT steps really run: Triton, Inductor, XLA's autotuner, TensorRT's timing); `HF_HUB_OFFLINE=1`, `VLLM_NO_USAGE_STATS=1`, `DO_NOT_TRACK=1`; GPU 0 unless the caller set `CUDA_VISIBLE_DEVICES`; the venv's `nvidia/*/lib` and `tensorrt_libs` directories lead `LD_LIBRARY_PATH` (the container driver's directory follows). `nvcc` is deliberately NOT on the `PATH` of vLLM: with it, FlashInfer builds sampling kernels and fails with `CUDA compiler and CUDA toolkit headers are incompatible` (the lock's `nvidia-cuda-nvcc` 13.5 against the 13.0 headers torch brings; MEASURED on an RTX 3060, 2026-11-17: the engine start then fails), without it vLLM logs "FlashInfer kernels are disabled" and uses its precompiled attention. So the "FlashInfer JIT through nvcc" item of the ranking is NOT exercised; what compiles is `torch.compile` (20 s) and Triton.

## 7. Running it

Standalone, any machine with an NVIDIA driver (root only for the apt line): `sudo bash scripts/apps/mlrows/run_standalone.sh OUT` ([the manual](../howto/open-ml-rows.md)).

Host control and guest run, strictly serial on one kf3 bench box (**the guest commands; NOT RUN by this work**):

```text
KF_DEVICE=kf3 bash scripts/apps/apps_matrix.sh host  RUN jax tensorrt vllm      # setup (network, about 15 min) + about 5 min; run jax first: it is the cheapest
KF_DEVICE=kf3 APPS_APP_TMO=3600 bash scripts/apps/apps_matrix.sh guest RUN jax tensorrt vllm
KF_NEGCTL=1 KF_DEVICE=kf3 bash scripts/apps/apps_matrix.sh host RUNN jax tensorrt vllm    # then the same with `guest`
```

Expected guest durations [inferred, a VM exit costs 10 to 40 times bare metal]: the push of the three environments (21 GB, `tar | ssh`) is the unknown: at 9 MB/s it is 40 minutes, at 100 MB/s 4 minutes (kfml's guest mode estimated the same way; a virtio-fs share is the fallback); the rows themselves are 10 s (JAX), 2 min (TensorRT) and 1.5 min (vLLM) natively, so 5 to 60 minutes in a guest, bounded by the row timeouts (jax 900 s, tensorrt 1200 s, vllm 1500 s; `run_apps.sh` 1200, 1500, 1800 s). The guest needs about 25 GB free, the same Python as the host (the venvs are copied, not rebuilt; Ubuntu 24.04 has CPython 3.12), `gcc` and the Python headers for vLLM, and `nvidia-smi`. Read `RUN/<row>.guest.log`, `RUN/<row>.compare.txt` (the `ML_COMPARE_*` lines) and the `gpu_use=` fields of the `APPRES` lines.

## 8. Native results

Evidence: `traces/open_ml_rows_20261117/` (text only). Revision under test: the PR's head, named in that directory's README. Machines (Vast CONTAINER boxes, Ubuntu 24.04 image `nvidia/cuda:13.0.1-devel-ubuntu24.04`, no kayfabe, no KVM):

RESULTS_TABLE

## 9. Tests and mutants (GPU-free)

`scripts/ci/test_mlrows.py` (about 230 tests): the pins and the trust rule (no digest outside the locks), the facts parsers on real `nvidia-smi` output, the flavour choice at every boundary, the locks (the committed ones are sound and agree with `pins.json`; each way to break one is refused, also at `setup` before pip runs), the model plan and the byte check against the origin's real tree listing of the pinned commit (tampered, truncated, oversized, traversal), the three judges (a table of single-property failures, each also a test of its own, and the boundaries of every tolerance), the proof line through the SAME function the other rows use, the environment (no secret, fresh caches, offline hub, the negative control), the runner with fake processes (timeout, no result, stale work directory, the whole log read), `setup` with a fake pip and a fake origin, the host-against-guest comparison on two logs, the push through a fake transport as a tar stream, `run_apps.sh` for real (a `NOT RUN` is `NOTRUN`, a crash that printed it is `FAIL`, the real tool without a GPU is `NOTRUN`, `all` runs no discovery row), the GPU-use judge over the rows (a GPU run passes, every CPU form and every idle host fails), `run_standalone.sh` with a fake tool, and text pins for the trust rules (marked as pins: they prove the line is there). `scripts/ci/mlrows_mutants.py` (about 70 mutants, two named killers each) is part of the `scripts/ci` step of `ci.yml`.

## 10. Hypotheses and falsifiers (stated before the first native run)

| id | hypothesis | falsifier | result |
|---|---|---|---|
| H1 | the TensorRT wheels install without an account from PyPI plus NVIDIA's open index, hashed | a login prompt, a 401/403, or a digest the index does not publish | held (section 3) |
| H2 | `trtexec` is in the pip wheels | no executable in the libs wheel | FALSIFIED: no executable (section 3) |
| H3 | the lock installs with `--require-hashes` on a stock Ubuntu 24.04 + CPython 3.12 without resolving anything | pip reports a hash mismatch or a missing requirement | held for all five locks [run] |
| H4 | greedy token ids are identical between two runs on one die | two different `token_sha256` | held (two processes, one die) |
| H5 | greedy token ids are identical across dies | different `token_sha256` on two dies | see section 8 |
| H6 | JAX preallocates 75 percent of the memory it is told the device has | a different `bytes_limit` fraction | held: 75 percent of the CUDA-reported 11.6 GiB (8934 MiB), 72.7 percent of `nvidia-smi`'s 12288 MiB |
| H7 | the XLA autotuner leaves results in the dump | an empty file | held (cuDNN, cuBLASLt and Triton entries) |
| H8 | the CPU form of each row is judged `FAIL` | `CONTROL_BROKEN` | held for all three (and found the stale-file defect of section 5) |
| H9 | FlashInfer's nvcc JIT works from the pip toolchain | the engine fails at start | FALSIFIED (section 6) |

For the guest run (not done), a result that contradicts one of these says kayfabe differs from the host:

- **G1** the guest's greedy token ids equal the host's on the same die (`ML_COMPARE_CANDIDATE ... token ids differ`): falsified by any difference.
- **G2** the engine sizes its KV cache and its graph list from the memory the guest reports (`fb-mb`, 3.8 GB at the default, GAPS 2): the number of captured graphs and the KV tokens in the log facts differ from the host's, and that is `INFO`/`CANDIDATE` per the comparison; falsified by an engine that fails to start in the guest, the failure being the finding.
- **G3** TensorRT's builder picks tactics from timing: the guest's engine is within 25 percent of the host's size and equals the CPU reference within 1 percent; falsified by a larger engine or a deviation.
- **G4** JAX preallocates 75 percent of the guest's reported memory and its autotuner leaves results; the host census shows submissions during the row.
- **G5** the guest's second and third CUDA process of one boot (the vLLM engine core is a second CUDA process of the row, `V3_APP_MATRIX` section J measured a wedge at the third in one boot on 2026-09-26): falsified by a wedged boot after the vLLM row.

## 11. Not done, and limits

- No kayfabe guest has run any row; the host against guest comparison is tested on synthetic logs only.
- `trtexec` itself (section 3); the ONNX Runtime TensorRT provider on a CUDA 12 flavour; CUDA 12 drivers; Hopper, Blackwell and a die older than Turing.
- Cross-die token stability is only as wide as section 8 says; `expected_token_sha256` stays null where it was not measured on at least two dies.
- FlashInfer JIT (section 6), speculative decoding, tensor parallelism, quantised models (`vllm` 0.34 supports many; the row stays on the dense fp16 path of the smallest model).
- The licence question of section 3 (SLA clause 2.1(iii)(b)) is the owner's.
