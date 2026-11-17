# GPU use is asserted, never assumed: the audit of every row that can run on the CPU, and the shared judge

**STATUS: LIVE, 2026-11-17 (owner requirement of 2026-11-17, binding; implemented and tested GPU-free; NOT run on any hardware when this line was written: every
statement marked INFERRED or UNCALIBRATED below is a hypothesis with a falsifier in section 7).** The requirement, verbatim in substance: for MATLAB, Blender and
every other application that CAN run on the CPU, a row must ASSERT that the GPU was used; a row that passes because the application silently fell back to the
CPU is a false pass and hides exactly the bugs kayfabe exists to find.

Labels: **MEASURED** = a line in a committed trace or log (path named); **INFERRED** = read from code or documentation, not run; **UNCALIBRATED** = a threshold
chosen from the few numbers the old logs hold, to be set from the first hardware run.

## 1. Rules this work keeps

- The guest is hostile. The kf3 device's own log (host-authored) is the authority; the host's `nvidia-smi` on the box is the second host-authored signal; what the
  application prints about its own device is a guest self-report that can only ADD a failure or support a pass. It never overrides a host "no work".
- An unreadable observable is NOT_RUN with a named reason, never a pass; an idle GPU and an unreadable GPU are different findings and the reason says which.
- A threshold is never relaxed to make a row pass: a real GPU row that fails means a named constant is wrong, to be changed with the run beside it.
- A passing GPU run keeps its verdict: the new fields are appended, the commands of the rows are the old commands (the CPU forms exist only under `KF_NEGCTL=1`).

## 2. The audit

Classes: **(a)** cannot run without the GPU (a hard CUDA, Vulkan, D3D or driver call, no CPU path): GPU use is implied. **(a-info)** an identity probe: it proves which
device exists, not that work ran on it. **(b)** CAN silently fall back to the CPU or another adapter: needs an assertion. **(c)** already has a real assertion (named,
with its strength and whether it is guest self-report or host-authored).

| surface | rows | (a) | (a-info) | (b) | (c) already asserted | strongest assertion before | after this work |
|---|---|---|---|---|---|---|---|
| Linux open matrix (`scripts/apps/run_apps.sh`) | 71 | 58 (6 now also carry a device proof) | 4 | 9 (8 run, `geekbench_gpu` is a policy skip; `nvdec_h264` had one weak fallback string) | 0 | a guest string | 14 wired rows: app proof + host census + host engines; 9 rows have a CPU negative control |
| Windows matrix (`appmatrix/apps.json`) | 120 | 65 | 9 | 9 | 37 (23 guest PDH, 14 guest self-report; none host-authored) | the guest PDH counter or a printed device name | 3 llama rows and the CPU switch fixed; the rest listed (2.2) |
| MATLAB rows (PR #464) | 8 | 1 (`gpu_info`) | 0 | 0 | 7 (host `nvidia-smi` engines; a regular array in a gpuArray row still agreed) | host engines | host census + gpuDevice + gpuArray proof on every row (2.3) |
| kfml (PR #455) | 6 | 4 | 0 | 2 (`KF_ML_DEV_CPU=1`) | 0 | the torch/CUDA gate, guest self-report | listed, not edited (2.4) |
| PR #471 FFmpeg + DCGM | 2 (47 cases) | 0 | 1 (`nvml_dcgm` reads driver answers) | 1 (`nvcodec_matrix`: the decode has no engine evidence, the encoder is forced by name) | 0 | `gpu_use=` printed, not gating | listed, not edited |
| PR #470 Windows open rows | 9 | 0 | 1 (NOT_RUN by design) | 8 (WARP, software adapters) | 0 | guest PDH | listed, not edited |

Totals: **216 rows** (and 47 FFmpeg cases): **(a) 128, (a-info) 15, (b) 29, (c) 44**. No Windows row has a host-authored assertion; the only host-authored assertions before this work were
the seven MATLAB engine checks. The (a) rows are safe because of what they call, not because of their proof (2.2, `traces/winapps_20261107`).

### 2.1 The Linux matrix, row by row (71 rows)

`scripts/apps/gpu_use_rows.py list` prints the class of every row; `test_every_row_of_run_apps_is_classified_and_the_counts_are_pinned` fails when a row is added
without a class. Class (a) (52 rows, GPU use implied, the census is recorded and never judged): the cuda-samples, the in-house probes (`stream_*`, `reduce`, `nbody`,
`sgemm_cublas`, `fft_cufft`, ...) and `gpu_burn`. Class (a-info): `nvidia_smi`, `deviceQuery`, `vulkaninfo`, `clinfo`.

| row | class | how it can fall back | before | now: app proof (guest) | now: host observables | CPU form for the control (`KF_NEGCTL=1`) |
|---|---|---|---|---|---|---|
| `blender_cycles` | b | Cycles renders on the CPU when no GPU device is usable; the script printed `gpus[0].name`, the CONFIGURED device | `BLENDER_OK <dev> gpu=<name>` | `BLENDER_DEVICES` (scene GPU, an enabled non-CPU device, the CPU device not enabled), `BLENDER_OK <dev> gpu=NVIDIA ...`, no fallback text, for CUDA and OPTIX | census work, engines gpu+mem | `blender_render.py -- CPU` (CPU mode, `compute_device_type NONE`) |
| `llama_cpp_gen` | b | `-ngl 0`, or a build without the CUDA backend: still prints the digest | digest only (MEASURED log shows `offloaded 29/29`, not asserted) | a CUDA device found, `offloaded N/N layers to GPU` with N equal and non-zero | census, engines gpu | `-ngl 0` |
| `llama_bench` | b | the same; the backend column still says CUDA at `-ngl 0` | `tg64` | CUDA device found, every table row backend CUDA with ngl at least 1 | census, engines gpu | `-ngl 0` |
| `hashcat` | b | an OpenCL CPU device (the image has pocl: MEASURED, `clinfo.guest.log` lists `cpu-haswell-AMD EPYC 7B13`) | the cracked-hash string, from any device; `tail -30` CUT OFF the device header (MEASURED: `hashcat.guest.log` has no `* Device` line) | every `* Device #N` line not marked skipped is an NVIDIA GPU, at least one | census | `-D 1` (CPU devices) |
| `clpeak` | b | benchmarks EVERY OpenCL device and the regex matches any (MEASURED: `clpeak.guest.log` has the NVIDIA block AND the pocl CPU block) | `Global memory bandwidth` anywhere | an NVIDIA device's own block holds a bandwidth result and no skipped tests | census, engines gpu | `-p 1` (the pocl platform; valid if platform 1 is pocl on the image) |
| `vkpeak` | b | benchmarks device index 0, whichever driver lists first (a CPU Vulkan driver) | `fp32-scalar` | the `device =` line names an NVIDIA GPU, not llvmpipe, lavapipe or SwiftShader | census, engines gpu | device index 1 (valid if a second, CPU, device exists) |
| `egl_offscreen` | b | EGL device 0 may be Mesa software (the probe only logged the renderer) | `CHECK` ok | `[egl] renderer:` is an NVIDIA GPU | census | Mesa's EGL vendor file |
| `nvdec_h264` | b / c | `-hwaccel cuda` falls back to software decoding on a failed setup | one string, `Failed setup for format cuda` | that string and the other fallback markers fail; the decoder's `pix_fmt: cuda` (INFERRED from the ffmpeg sources, `-v verbose`) is the positive marker; without it the row is NOT_RUN | census | `-hwaccel none` |
| `nvenc_h264`, `nvenc_hevc` | a + proof | none (the encoder is forced by name; ffmpeg does not substitute libx264) | frame count | the output file's own `encoder` tag (`ffprobe`) names `<codec>_nvenc`, no software encoder named | census, engines enc | `-c:v libx264` |
| `torch_correct`, `torch_ai_bench`, `hf_generate`, `cupy` | a + proof | none today (hard `torch.cuda` / `.cuda()` / CuPy); a later `cuda if available else cpu` edit would be silent | pass strings | `GPU_USE_PROOF <fw> device=cuda:N name=NVIDIA .. peak_mib=N` printed AFTER the work from the framework's allocator, N at least a per-row floor | census, engines gpu (not cupy) | `CUDA_VISIBLE_DEVICES=` (the row then fails by itself) |
| `geekbench_gpu` | b, policy skip | `--gpu OpenCL` may pick pocl | not run (docs/DOWNLOADS.md) | a proof is required before the skip is ever lifted | | |

There is no JAX, ComfyUI or `h264_cuvid` row in the tree (grep over `scripts/` and `crates/`); the torch and CuPy rows above are the framework rows, the FFmpeg rows are the encode and decode rows. A JAX or ComfyUI row would need a `GPU_USE_PROOF` line from the framework's own device list (`jax.devices()`, `torch.cuda`) and a class in `gpu_use_rows.ROWS`; the class test fails until it has one.

### 2.2 The Windows matrix (120 rows) and what was changed

Counts: (a) 65, (a-info) 9, (b) 9, (c) 37 (23 rest on the guest PDH counter, 14 on a guest self-report of the renderer or device). The full 120-row table with the
reason for every classification is in the working audit (`/root/night/GPU_USE_ASSERTION_AUDIT_20261117.md`, appendix A). The findings that matter:

1. **On kf3 both guest sensors are blind, so every PASS rests on a printed name. MEASURED** (`traces/winapps_20261107/full/wa1f_winapps/run/win.res` and
   `run/apps/*.json`, RTX 4090): `smi.samples` is 0 in 67 of 67 rows (the guest `nvidia-smi` prints `[N/A]` for utilisation and the cast throws), the PDH engine maxima
   never exceeded 0.096 percent (threshold 0.5), 54 PASS rows carry `proof=out`, 4 a readback proof, none a pdh or smi proof. `eval_proof` is held-OR: one held proof
   is enough (`verdict.py`).
2. `kf_runpy.py` prints `KFDEV <name>` before any work, which alone satisfies `out:NVIDIA|GeForce|RTX` for the 22 `win_cuda_equiv` rows, the stream rows, the CuPy
   rows and the torch rows (INFERRED from the code). `win_cuda_equiv.py` honoured `KF_EQ_DEVICE=cpu` and then printed the same lines.
3. The llama rows matched a CPU run: `offloaded [0-9]+/[0-9]+ layers to GPU` matches `offloaded 0/29`, `out:CUDA|NVIDIA` matches `failed to initialize CUDA`, `out:Vulkan|NVIDIA`
   matches llvmpipe, and `CUDA0: NVIDIA` came from a separate `--list-devices` process.
4. `smi` is system-wide and OR'd with the per-process pdh in the video rows; `nvidia_only` only sees adapters that appear in the GPU Engine counters (GDI Generic, llvmpipe,
   SwiftShader and an OpenCL CPU platform never create a counter).
5. No row uses `gpu_assert.py` or `gpu_use_assert.py`; `verdict.decide` has no host input. Under kf3 the host's `nvidia-smi` is the only sensor that works.

Changed here (tested in `WindowsRows`, mutated in `gpu_use_mutants.py`): `llama_cuda_gen` needs `offloaded N/N layers to GPU` with N at least 1 and equal; `llama_cuda_bench`
needs a `| CUDA | ngl>=1 |` table row; `llama_vulkan_bench` needs `ggml_vulkan: N = NVIDIA` (INFERRED format, never seen in a Windows log); `win_cuda_equiv.py` refuses
`KF_EQ_DEVICE=cpu` unless `KF_EQ_ALLOW_CPU=1`. NOT changed, and why: the held-OR semantics and the PDH proofs (an AND would fail 100+ rows whose only working proof is the
printed name, item 1 above; the fix is a host sampler fact, which needs the Windows supervisor to ask the box for it and is a hardware step); `kf_runpy.py` `KFDEV`.

### 2.3 MATLAB rows (the follow-up commit on PR #464)

Before: seven rows required host `nvidia-smi` engines (`gpu_assert.evaluate`, host-authored); `gpu_info` required none; the kf3 log was read for Xid only; and `kf_cmp.m`
gathered whatever it was given (`gather` of a regular array returns it, so a GPU row that computed on the CPU compared the CPU with the CPU and agreed). After: every
row needs (1) a `gpu_device` block (gpuDevice index at least 1, `canUseGPU`, the host's GPU name), (2) a recorded `isa(x,'gpuArray')` and `existsOnGPU(x)` BEFORE the gather
on every GPU-versus-CPU comparison plus a `kf_gpu.m` proof item in `cuda_kernel`, `mem_oversub`, `dl_train`, and (3) the host census window with at least 32 submissions
(HARNESS when the census cannot be read). `dl_train` trains with an explicit `ExecutionEnvironment 'gpu'` (an error without a GPU, never a fallback). The `trainnet` info table has no device field (INFERRED), so the guest gives no evidence of the training device at all: the host census and engines are the authority for `dl_train`, and the guest's `train_env_gpu` item only proves a gpuArray can be made from the training data.

### 2.4 Rows on open PRs (listed, not edited)

- **kfml (#455/#472)**: `KF_ML_DEV_CPU=1` replaces the gate by `Verdict("ok")` and gives CPU PASS rows for `nanogpt_compile` and `amp_convnet_cifar` (the stage copies
  `os.environ`, only token-like names are scrubbed); `TRITON_INTERPRET=1` runs the Triton kernels on the CPU interpreter (INFERRED, no trace). Recommended: scrub both in the
  stage, record `dev_cpu` in the row, require `metrics["cuda_ipc"]`. No kfml row reads the kf3 log or the host `nvidia-smi`.
- **#471 FFmpeg/DCGM**: the encoder is forced by name (no software substitution); the 18 decode cases have no decoder-engine proof and the software decode scores the same
  PSNR (`inf`, `traces/open_rows_ffdcgm_20261117` on PR #471); `nvml_dcgm` exits 0 with DCGM NOT_RUN and 7 of 8 diagnostics skipped (same trace); the host census is printed after the verdict and
  gates nothing. Recommended: gate on `gpu_use.judge` with the row's engines (`enc`, `dec`).
- **#470 Windows open rows**: all nine are pure PDH (guest self-report), none names the adapter; `deqp-vk` takes device 0, the DX samples never pass `-warp` but the harness
  checks are adapter-blind (WARP renders non-blank frames). Recommended: the host sampler fact once the Windows supervisor can ask for it.

### 2.5 Rows added after this audit (2026-11-17)

The open ML rows of [V3_OPEN_ML_ROWS.md](V3_OPEN_ML_ROWS.md) are wired from the start: `vllm` (class a+proof: the PyPI wheel has no CPU path), `tensorrt` (class b: ONNX Runtime falls back from its TensorRT provider to the CUDA and CPU providers without an error, so the row reads the provider's profile for the provider that ran each node) and `jax` (class b: JAX falls back to its CPU backend when the CUDA plugin does not load). Each prints `GPU_USE_PROOF <row> device=... name=... peak_mib=<device memory growth>` from `nvidia-smi` on the machine that runs it (a guest self-report); the host census and the host engine counters are the authority. Their CPU forms (`KF_NEGCTL=1`: GPU hidden, `JAX_PLATFORMS=cpu`, the CPU reference only) are judged `FAIL` natively (MEASURED, V3_OPEN_ML_ROWS.md section 8). `test_every_row_of_run_apps_is_classified_and_the_counts_are_pinned` counts 74 rows.

## 3. The observables

**Host census (authority).** The kf3 status line is printed by the drainer about every 2 s on change: `kf3: family=... chan[births=N ... nsi=[GR0:101/0raised ...] served=N
parks=N host_rings=N ... tokens=[0x801:fwd=F,subs=S,serves=V,put=Some(P),gp_get=..., ...]] ... dbfast[ON ... doorbells=D ...]`, and a channel's end prints `kf3: chan token
0x.. (host 0x..) RETIRED, forwarded=F submissions=S ... serves=V ...` (`crates/kf-qemu/src/device.rs`, `chan.rs`). `gpu_use.window_census` uses the identity total =
retired + live: window work = sum of the RETIRED lines in the window + live(end) - live(start), where live() is the sum of `subs=` in the last status line and the
status line BEFORE the window (`<app>.kf3.base`, saved by `apps_hook.sh`) is the start. Every window number is a lower bound. Work means "the guest created channels and
submitted GPFIFO entries to the real GPU through kf3".
Numbers read from the old per-app kf3 logs `traces/v3_candidates/cand1_20261004/apps/*.kf3.log` (windows without a baseline, so lower bounds): `nvidia_smi` 2 submissions, `vectorAdd`
411, `matrixMul` 424 (6 retired channels each). The floor 32 (UNCALIBRATED) sits between opening the device and a CUDA context. **The census cannot tell a CUDA context from
compute done on it**: a CPU fallback that still initialises CUDA (llama.cpp at `-ngl 0` creates the backend) shows work. That is why the engine counters and the app proof exist.

**Host engines.** `gpu_assert.HostProbe` samples the BOX's `nvidia-smi` around the app (`gpu_use_rows.py sample`, 100 ms for rows that keep the GPU busy a second or less,
250 ms otherwise; thresholds are `gpu_assert`'s: 3 percent for N samples, 64 MiB). On a box with several GPUs all of them are read (one GPU per box today). Per row the engines
are in `gpu_use_rows.ROWS` (UNCALIBRATED; `hashcat` and `egl_offscreen` keep the GPU busy for under a second and ask for none).

**PR #459 (`report-utilization`).** Its `util[on host_reads=... published=...]` status fragment carries counters, not the busy percentage, so it is not a census signal. With
the flag on the guest's `nvidia-smi` utilisation columns are fed from the host read (host-sourced, guest-reported): a useful second signal for a standalone run, not used here.
Recommended follow-up (kf3 Rust, a hardware step): put the peak busy percent of the host read into the `util[...]` fragment, then the kf3 log alone carries a host-authored
busy number. This work does not depend on #459.

## 4. The judge (`scripts/bench/gpu_use.py`) and the Linux wiring

`judge(row, census, smi, app, need_census, need_engines, side)` returns PASS, FAIL or NOT_RUN with source-tagged reasons (`[host-census]`, `[host-smi]`, `[app]`). FAIL: the app
reports a CPU or non-NVIDIA device, the proof line a row must print is missing (the row dropped it), a window with a baseline shows no work, a required engine stayed idle.
NOT_RUN: no kf3 status line in the window, no baseline, an unreadable sampler, an unmeasured marker. The record says `authority=host` when a host observable was read and
`authority=guest` for the bare-metal baseline (no kf3 log there). `control_verdict` says CONTROL_OK when a negative control was judged FAIL and CONTROL_BROKEN when the judge
passed a CPU run.

Flow: `apps_hook.sh` saves the last status line before the app, runs the app under `gpu_use_rows.py sample`, waits `APPS_KF3_SETTLE` (3 s, longer than the status period)
and slices the kf3 log; `gpu_use_rows.py annotate` rewrites a PASS of a wired row to FAIL (or NOTRUN when it cannot be judged) with the reason in `note=gpu-use:...` and appends
`gpu_use= gpu_auth= gpu_subs= gpu_births=`; a crashed judge leaves NOTRUN, never the PASS. `apps_matrix.sh host` annotates the bare-metal baseline from the app logs.
`matrix_table.py` classes a guest NOTRUN with `gpu-use:NOT_RUN` as UNATTRIBUTED (re-run), never as a kayfabe defect; a guest `gpu-use:FAIL` after a host PASS is a
KAYFABE_DEFECT (the host used the GPU and the guest did not), never hidden.

## 5. Negative controls

`KF_NEGCTL=1 scripts/apps/apps_matrix.sh host|guest <run> <rows>` runs the CPU form of each class (b) row (table in 2.1) and writes `NEGCTL app=.. control=CONTROL_OK|CONTROL_BROKEN
judged=.. why=..` lines to `host_negctl.res` / `negctl.res`; the row lines of a guest control run stay unannotated in `guest.res`. GPU-free, each control is tested twice: the GPU and
the CPU fixture of every row (`scripts/ci/fixtures/gpu_use/`, README there labels the real and the synthetic lines) go through the judge, and the real row command runs with fake binaries
(`RunAppsRows`) so the proof lines the judge reads are the ones the command prints; the hook itself runs against a fake guest transport (`HookWithAFakeGuest`).

## 6. Tests and mutants

`scripts/ci/test_gpu_use.py` (115 tests, in the `scripts/ci` discovery) and `scripts/ci/gpu_use_mutants.py` (40 mutants, two named killers each: empty census window passes, no-work
passes, floor zero, retired channel counted twice, baseline ignored, guest overrides the host, CPU device name accepted, a dropped proof passes, a negative control that does not switch,
the hook not annotating or slicing early, the Windows llama proofs and the CPU switch, and more). MATLAB: `GpuUseProofTests` and ten more cases in `matlab_rows_mutants.py` on PR #464.

## 7. Validation request (hardware, for the next gate or a standalone run; NOT RUN)

Strictly serial, one box with a real GPU and KVM, the tree of this PR (and of PR #464 for MATLAB). Host first, then guest, then the controls:

```
ROWS="blender_cycles llama_cpp_gen llama_bench hashcat clpeak vkpeak egl_offscreen nvdec_h264 nvenc_h264 nvenc_hevc torch_correct torch_ai_bench hf_generate cupy"
scripts/apps/apps_matrix.sh host  gpuuse1  $ROWS      # bare metal, GPU form
scripts/apps/apps_matrix.sh guest gpuuse1  $ROWS      # kf3 guest, GPU form, one app per boot
KF_NEGCTL=1 scripts/apps/apps_matrix.sh host  gpuuse1n $ROWS
KF_NEGCTL=1 scripts/apps/apps_matrix.sh guest gpuuse1n $ROWS
python3 scripts/apps/matrix_table.py /workspace/apps/results/gpuuse1
# MATLAB (PR #464): python3 scripts/bench/linux/matlab_rows.py run ... --rows gpu_matmul,gpu_fft,gpu_solve,dl_train,parpool_gpu,cuda_kernel,mem_oversub --evidence-dir DIR
```

Expected, and what falsifies it (each failure names its observable; the constants to tune are `gpu_use_rows.ROWS` and `gpu_use.MIN_SUBMISSIONS`):

| id | expectation | falsified by |
|---|---|---|
| V1 | every GPU-form row keeps `verdict=PASS` and gets `gpu_use=PASS`; guest rows `gpu_auth=host` and `gpu_subs` at least several hundred | any `note=gpu-use:FAIL` or `NOTRUN` on a genuine GPU row (read the reason: an engine floor, the census floor, a marker) |
| V2 | every control line is `control=CONTROL_OK judged=FAIL` (host and guest) | any `CONTROL_BROKEN`: the judge passed a CPU run, or a switch did not switch (clpeak `-p 1` must be the pocl platform, vkpeak index 1 must be a CPU device; if the image has none, say so and treat the control as fixture-only) |
| V3 | `nvdec_h264.guest.log` contains `Reinit context ... pix_fmt: cuda` | the marker is absent: the row stays NOTRUN until the marker has been seen in a log (INFERRED today) |
| V4 | `hashcat.guest.log` contains `* Device #1: NVIDIA ...` (format INFERRED) and `nvenc_*.guest.log` an `ENCODER_TAG` naming `*_nvenc` | either line missing or in another format |
| V5 | host `nvidia-smi` shows `utilization.gpu` at least 3 percent in at least one 100 ms sample for `blender_cycles`, `llama_*` | an idle sample series on a row that ran on the GPU: the interval or floor is wrong |
| V6 | the kf3 log of each guest window holds a status line before and inside it (`<app>.kf3.base` non-empty) | an empty base: the row is NOTRUN, the hook's slicing is wrong |
| V7 | MATLAB rows: every `verdict.json` has `census.work.verdict PASS` and a `gpu_device` block; a row with a `matlab` wrapper that hides the device ends `FAIL ASSERT`, never PASS | HARNESS `HOST_CENSUS_MISSING` on the preamble (no status line in `qemu.log`) |

Windows: the next Windows gate must show `llama_cuda_gen` printing `offloaded N/N` (and `ggml_vulkan: 0 = NVIDIA` for the Vulkan row) or these three rows now FAIL by design.

## 8. Heads-up for the acceptance run (tree `021cad3cc`)

The tree contains PR #464's `matlab_rows.py` WITHOUT this commit: its MATLAB judge asserts host `nvidia-smi` engines only and the kf3 census is read for Xid only, so no MATLAB row
changes there and no heads-up is needed for that run. A re-run on a tree that has the follow-up commit changes nothing for a genuine GPU row; the only new way to a non-pass is
HARNESS `HOST_CENSUS_MISSING` (no kf3 status line before the first row in `qemu.log`) or `GPU_NOT_USED` from a guest whose gpuArray flag is false. The Linux matrix rows of the gate lists
(blender, llama, hashcat, clpeak, vkpeak, egl, nvdec, nvenc, torch, hf, cupy) would now FAIL or NOTRUN where their proof or an UNCALIBRATED engine floor is not met: the first gate run
after this merges is the calibration (section 7).

## 9. Not done, and limits

Windows host sampler (needs the supervisor, section 2.2); kfml scrub and #470/#471 gating (open PRs, not edited); multi-GPU boxes (all GPUs are sampled together); the percent of
PR #459 in the kf3 log; `geekbench_gpu` (policy skip); the thresholds of section 3 are UNCALIBRATED; the census is lower-bounded and cannot separate a CUDA context from compute.
