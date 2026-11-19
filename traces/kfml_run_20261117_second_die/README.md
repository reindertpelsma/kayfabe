# kfml_run_20261117_second_die: kfml on a second die, a different driver and the other flavour

STATUS: LIVE, 2026-11-17 (measured; text evidence only). Run id `kfml_run_20261117_second_die`. Companion of `traces/kfml_run_20261116/` (first die).

What this is: the proof that kfml pins no die and no driver. One more Vast **container** (not KVM, no kayfabe in it), rented by the total-cost rule
(inet cost <= 0.01 USD/GB), label `kf-kfml2-20261117` (cap 2.5 h / 2.0 USD), image `nvidia/cuda:12.6.3-devel-ubuntu22.04`.
Box: GeForce RTX 2080 Ti (Turing, **sm_75**, 11 GB), driver **595.91.07** (`nvidia-smi` CUDA **13.2**), Ubuntu 22.04.5, CPython 3.10.12, gcc 11.4.0,
system nvcc 12.6.85, Core i9-9900K host CPU. First die for comparison: RTX 4070, sm_89, driver 560.35.03, CUDA 12.6 (`kfml_run_20261116`).
Another box (an RTX 2080 Ti, offer on a different host) was rented first and destroyed after 15 minutes because its image never finished pulling (Docker
layer stuck at "Verifying Checksum"); nothing ran on it. Both instances are destroyed by exact id; `vastai show instances-v1 --raw` lists neither.
Rental about 40 minutes for the box that ran, estimated 0.2 USD in all (rentals, storage and bandwidth; the account credit moved by about that much).
Nothing secret went to the box (public key and our code only; every download is a public URL or the sha256-pinned official source).

Revisions: the stage ran from the branch `claude/kfml-run-20261116`; the final run is revision `33f3e66b25db813c238bd2f712fd9aad183addc3` (field `sha` of `rows/stage_final_large.json`).
The first run of the stage (`stage_first_run_triton_FAIL.json`) was `1ca69391e`, before the triton fix below. The offline-proof and wheelhouse steps ran the code of a few commits between; the PR's final tree adds the glibc-minor list, the offline lock, docs and tests (no workload code beyond the triton fallback).

## Result

**The flavour was chosen automatically and differs from the first die**: `check` printed `chose cu128 (driver CUDA 13.2 >= 12.8 and sm_75 is inside
sm_75..sm_120)` (`kfml-setup.json`); the first die got cu126 from driver CUDA 12.6. The container's own nvcc is 12.6, so `setup --nvcc-if-needed` installed the
pinned cu128 redist nvcc (12.8) from NVIDIA's server: the redist path ran inside the automatic flow on a real box (`"nvcc": "redist"`).
Wheels installed from the hashed cu128 lock with `--require-hashes`. Turing has no bf16, so the amp rows ran the fp16 + GradScaler path (`grad_scaler: true`).

All six rows PASS (final run, cold compile caches, `rows/stage_final_large.json`, wall_s of each test): nanogpt_compile 22.5, triton_kernels 12.8, cuda_ext_inline 24.0,
amp_convnet_cifar 19.2, stress_alloc 3.1, managed_oversub 33.1 (13.25 GB of managed memory on an 11 GB card); the `run` step took 117 s. Every row carries die,
compute capability, driver, driver CUDA, torch CUDA runtime and flavour.

## Compared with the first die (the per-die-class rule; `rows/reference_compare_vs_4070.json`)

Four rows were run with `--reference scripts/kfml/reference` (which then held only the RTX 4070). Every reference was found as `how: other_die`, so no
`wall_ratio` was computed (null by the rule) and wall time decided nothing. The algorithmic metrics agree within the stated tolerances: nanogpt loss windows
differ from the 4070 reference by 0.0015 nats (tolerance 0.25), ConvNet accuracy by 0.0045 (tolerance 0.10); the fp16/GradScaler path and the bf16 path reach the same
losses (compiled-vs-eager window diff 7.5e-5 on this die). `reference_recorded/nvidia-geforce-rtx-2080-ti/` is this die's baseline (also committed under
`scripts/kfml/reference/`); a box of either die now finds its own.

## What broke on this die, and what it was

* `rows/stage_first_run_triton_FAIL.json`: `triton_kernels` FAIL `RuntimeError: PassManager::run failed` (stderr: `Unsupported conversion from f16 to f16`,
  `LLVM ERROR: Unsupported rounding mode for conversion`). `triton_sm75_fp16_probe.txt` (run `kfml_run_20261117_second_die`): the standard Triton matmul kernel with
  fp16 or bf16 inputs fails to compile for sm_75 for EVERY configuration (nine tried, tile sizes 16..128, 1..8 warps), and compiles and is correct with fp32 inputs (error about 1e-6).
  Verdict: not a kfml test bug, not the driver and not kayfabe (a compile-time failure of Triton 3.3.1 for this architecture, no GPU work involved); a compiler/toolchain
  limit of the pinned Triton on Turing, worth reporting upstream (not reported here; the upstream issue was not looked up). Decision in kfml: below sm_80 a compile failure
  of the fp16 variant is recorded in the row (`fp16_dot_compile`) and the kernel is checked with fp32 inputs; at or above sm_80 it stays a FAIL
  (`triton_kernels.fp16_dot_fallback`, GPU-free test). Result: PASS, `matmul_input_dtype: float32`, `fp16_dot_compile: failed on sm_75: ...`, so a newer Triton that compiles
  it flips that metric without a code change.
* Nothing else failed on this die. The other five rows passed on the first run. Torch, nvcc, ninja and the datasets needed no change for sm_75.

## Supplements of run `kfml_run_20261117_second_die`

* **Wheelhouse sizes** (`wheelhouse/wheelhouse_manifest_summary.txt`): `kfml wheelhouse --flavor all --guest-python 3.12` built both flavours in 112 s: cu126 wheels 3 026 998 392 B
  + redist 52 057 232 B, cu128 wheels 3 881 029 847 B + redist 81 298 228 B, together 7 041 383 699 B (64 files). The CPython 3.10 cu128 wheelhouse with both datasets: 4 133 802 925 B
  (34 files). These replace the 8 GB estimate in `ci/vast-broker/bandwidth.json`.
* **Offline install proof** (`offline_proof/`): the guest-side steps of `kfml_guest.py` (manifest verify, `setup --wheelhouse`, `fetch-data --from`, `check`, `run --tier mid` with
  `KF_ML_GUEST=1`) were run on this box from a copied-style wheelhouse with every proxy variable pointed at a dead port (no network use can succeed; this is not a network
  namespace). Verify 7 s, setup 67 s (29 hashed wheels and the redist nvcc from the wheelhouse), fetch-data 1 s, run 82 s: five mid rows PASS, the recorded lock sha256 is the repository
  lock's. This is NOT a guest run (no guest, no ssh, no kf3).
* **Two bugs the offline proof found**, both fixed with unit tests: pip honours an `--index-url` line inside a requirements file even with `--no-index` on the command line
  (the first offline attempt contacted the indexes and retried), so the offline install uses a copy of the lock without its index lines; and `pip download --platform manylinux_2_28_x86_64` does not
  find the manylinux_2_27 wheels of the NVIDIA runtime packages (pip does not expand platform tags), so every manylinux minor up to the guest's glibc is listed.
* **CIFAR-10 download**: the dataset server gave about 45 KB/s per connection (a single download would have taken about 60 minutes); the archive was fetched with six
  parallel ranged requests from the coordinator and copied in (sha256 `c4a38c50...` equal to the pin, a third confirmation), then the stage ran. The stage's own downloader was not used
  for that file in this run. (Eight or more parallel connections from the box were reset by the server.)

## Files

`environment.txt`, `kfml-setup.json`, `rows/` (final, first-failing and reference-comparison rows), `logs_final_run/` (`run.log` has every row with full metrics),
`reference_recorded/`, `triton_sm75_fp16_probe.txt`, `wheelhouse/`, `offline_proof/`. No UUID, address, host name or key is in these files (grepped before the commit).

## Not shown (UNVERIFIED)

Any kayfabe guest; Blackwell; an Ampere die; the cu126 branch on this host; a NOT_RUN box (the NOT_RUN paths are covered by GPU-free tests with made-up facts); CPython 3.12 installs
(the 3.12 wheelhouse was built and hashed, not installed); the guest transport, copy speed and install time (ESTIMATES until a KVM run).
