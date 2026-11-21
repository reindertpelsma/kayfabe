# Run the open ML rows (vLLM, TensorRT, JAX)

Three Linux checks run the machine-learning stacks that stress a GPU driver in ways the other rows do not: **vLLM** (a tiny model at temperature 0: an engine process of its own, memory sized from what the device reports, CUDA graphs for the engine's whole list of batch sizes), **TensorRT** (the engine builder times thousands of candidate kernels, then the engine runs; ONNX Runtime's TensorRT provider runs the same model) and **JAX / XLA** (a 75 percent memory slab at the first operation, the autotuner, a matmul, a convolution and a scan checked against the CPU). They work from a plain checkout of the repository: no account, no token, no private download. Every row also asserts that the GPU did the work, because all three can fall back to the CPU and still print a number.

## Requirements

Linux x86-64 with an NVIDIA driver, Python 3.12 to 3.14 with `venv`, `gcc` and the Python headers (`python3-dev`), about 25 GB free disk, and the network once. On a machine whose Python is older (Ubuntu 22.04 has 3.10) `run_standalone.sh` builds a private Python 3.12 from the python.org source below the run directory (once; as root it installs the build packages `provision_box.sh --packages pybuild` first; a few minutes; the system Python is not touched). The download is checked by the repository's trust helper against the checksum recorded in `scripts/ci/trust_manifest.json` (a mismatch refuses the build: `python_source_refused:TRUST_PIN_MISMATCH` in the `PRIVATE_PYTHON` line, and the rows then say `NOT_RUN` with `python_too_old`). A GPU of the Turing generation or newer. The driver decides the wheels: **a driver that supports CUDA 13 (R580 or newer)** takes all three rows; a driver that supports CUDA 12 takes JAX and TensorRT (their CUDA 12 flavours) and reports vLLM as `NOT_RUN` with the reason (the vLLM wheel on PyPI is built for CUDA 13). The packages are installed from PyPI and, for TensorRT, from NVIDIA's own open index `pypi.nvidia.com`, each file checked against a SHA-256 digest in a lock committed in the repository (`pip --require-hashes`); the model comes from its Hugging Face repository at a pinned commit and is checked against the sizes and digests the host lists for that commit. Nothing NVIDIA ships is stored by us: your machine downloads its own copy.

## On one machine

```text
sudo bash scripts/apps/mlrows/run_standalone.sh /tmp/mlrows_out
```

It installs `python3-venv`, `python3-dev` and `gcc` (as root; as another user install them yourself first), sets the rows up under `/tmp/mlrows_out/root` and prints one summary line per row:

```text
JAX_SUMMARY status=PASS ...          # or FAIL with the check that failed, or NOT_RUN with the reason (and an APPNOTRUN line)
TENSORRT_SUMMARY status=PASS ...
VLLM_SUMMARY status=PASS ...
```

Each row also prints one `GPU_USE_PROOF` line (the device and how far its memory grew) and a `MLROWS_CHECK` line per check, and keeps its full log in `OUT/<row>/`. `rm -rf /tmp/mlrows_out` removes everything. Setup is the slow part (with two setups at once on an 870 Mbit link JAX took 3 minutes, TensorRT 6 and vLLM 8; on a 1.2 Gbit link 1, 1.5 and 4; 4.4, 7.3 and 9.6 GB of venv); a run is 3 to 10 seconds (JAX), 1 to 2 minutes (TensorRT) and 1 to 1.5 minutes (vLLM) on the three cards it was measured on (RTX 3060, RTX 4060 Ti and RTX 2080 Ti, 2026-11-17). To work on one row: `python3 scripts/apps/mlrows/mlrows.py setup --root DIR --rows jax` then `... run jax --root DIR`; `mlrows.py check` says which flavour this machine takes and why.

`NOT_RUN` is never a result about the GPU: the card is older than Turing, the driver cannot take the wheel, there is no C compiler, too little disk or video memory, or the row was not set up. `FAIL` names the check (for example `engine_equals_onnxruntime_cpu`, `jax_devices_are_gpu`, `greedy_repeat_identical`).

## Host against guest (kayfabe)

The rows are discovery rows of the app matrix (`scripts/apps/run_apps.sh`, in no gate list, not part of `all`). The host run sets them up; the guest, which has no network, gets the same environments copied in:

```text
bash scripts/apps/apps_matrix.sh host  RUN vllm tensorrt jax
bash scripts/apps/apps_matrix.sh guest RUN vllm tensorrt jax
```

`apps_matrix.sh` runs `mlrows.py compare` at the end of a guest run (`RUN/<row>.compare.txt`): a `CANDIDATE` line is something the guest does differently from the host on the same card (different greedy tokens, another number of captured CUDA graphs, another preallocated fraction, a lost autotuner result). Guest-visible video memory is a kayfabe policy and differs by design; it is listed as `INFO`. `KF_NEGCTL=1` runs the CPU form of each row (GPU hidden, JAX on its CPU backend): the GPU-use check must fail it.

Licences: vLLM and JAX are Apache-2.0, the model (SmolLM2-135M) is Apache-2.0, ONNX Runtime is MIT; the TensorRT and CUDA wheels are NVIDIA's own software under NVIDIA's licence, which the installation of those wheels accepts (the licence text is in the TensorRT wheels and at `developer.nvidia.com/tensorrt`).
