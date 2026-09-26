# V3 app matrix — which real CUDA apps work in a kayfabe v3 fat guest

**STATUS: ANSWERED, 2026-09-26** (measured 00:10–04:00 UTC). kayfabe **`79848341`** (origin/master; the kf3 binary was built
from it, `kf3-bins/79848341`); harness commits on branch `v3-apps` touch only `scripts/apps/`,
`traces/v3_app_matrix/` and this file. Two rented vast KVM boxes, host driver **580.159.04**
(kernel-open), guest Ubuntu 24.04 with the stock **580.159.04** guest driver, kf3 device, 16 GiB
guest RAM, 6 vCPUs:
- **va1 — RTX 3060 12 GB (GA106)**, vast 52660722, guest FB `fb-mb=8192` (default);
- **va2 — RTX 3070 8 GB (GA104)**, vast 52660725, guest FB `fb-mb=6144` (8192 would exceed the card).

Question (owner): nvkvm-pv (the shipped Mode-1 sibling) validated a set of real CUDA apps. Run the
same set inside a kayfabe **v3 fat guest** and record which ones **work** — correct output, no
hang, no crash, no Xid. Not parity, not timing. Every app ran on the **host of the same box first**
(bare metal, identical binaries/runtime), so a guest-only failure indicts kayfabe, not the app.

## 0. Headline

- **Host: 71/71 PASS on both boxes** (65 apps + 6 `stream_probe` shapes). Every guest failure below is
  a guest-only failure.
- **Guest: 35/65 apps work on the 3060, 34/65 on the 3070** (one app per fresh boot). What works:
  nvidia-smi, deviceQuery, every **default-stream** CUDA program (vectorAdd, the nvkvm-pv HPC kernels
  incl. cuBLAS SGEMM and cuFFT, reduction/scan/sort/histogram, tensor-core GEMMs, dynamic
  parallelism, cooperative groups, simpleCUBLAS/CUFFT, conjugateGradient), HF transformers greedy
  generate (**token-identical to the host**), OpenCL (clinfo, clpeak).
- **One defect blocks most of the rest: any kernel launched on a non-default CUDA stream** faults the
  host GPU with **`Xid 13 SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE`** (§3 A) — 21 of the 65 apps, including
  PyTorch, llama.cpp, Blender, hashcat, Geekbench and every CUDA-graph / multi-stream sample.
- **Without guest persistence mode a guest can run only ~5–6 CUDA processes per boot**; the next one
  hangs forever and so does every one after it (§3 B). **With persistence mode that wall is gone**
  (59 processes in one boot, identical verdicts on both boxes, B-class apps such as CuPy pass) — until a
  host-side `NV_ESC_RM_MAP_MEMORY … NoMemory` wedges the boot at the ~60th process (§3 J).
- **One silent wrong answer**: `conjugateGradientUM` prints `result = SUCCESS` with
  `Error amount = 1.000000` after a host `Xid 31` MMU fault (§3 C).
- Not exposed at all: **NVENC/NVDEC** ("unsupported device"), **EGL** (`eglInitialize failed`),
  **Vulkan** (`vulkaninfo` hangs, vkpeak "No vulkan device").

## 1. The app set (inventory of nvkvm-pv) and what "pass" means here

Sources: `nvkvm-pv/tests/perf/{realapp_matrix.md,matrix_remote.sh,graphics_remote.sh,apps/}`,
`tests/integration/attach_verify.cu`, `docs/reference/{parity,blender-opendata,openbenchmarking-clpeak}.md`,
`tests/uvm_multiarch/build_apps.sh`, `tests/validate.sh`.

Harness: `scripts/apps/`. One binary set is built on the host (`build_bundle.sh`: CUDA 12.6
toolkit, `-arch=sm_86`, cuda-samples v12.5, gpu-burn, llama.cpp `4b1a27f` CUDA, clpeak 1.1.2,
vkpeak 20250531) and the **same files** run on both sides; `setup_side.sh` installs the identical
runtime on host and guest (torch 2.6.0+cu124, torchvision 0.21.0, transformers 4.46.3,
cupy-cuda12x 13.3.0, distro ffmpeg / hashcat / vulkan-tools / clinfo, Blender 4.5.0, Geekbench
6.4.0). `run_apps.sh` holds every app's command and pass predicate: **rc 0 and the app's own success
string and no `CHECK … FAIL`** (predicates were calibrated on the host baseline; many cuda-samples
verify on the CPU and report only through their exit code).

| group | apps | pass predicate |
|---|---|---|
| nvidia-smi | `nvidia_smi` | lists the RTX GPU |
| cuda-samples v12.5 | deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, simpleIPC, cudaTensorCoreGemm, bf16TensorCoreGemm, globalToShmemAsyncCopy, cdpSimpleQuicksort, graphMemoryNodes, simpleCudaGraphs, simpleCUBLAS, simpleCUFFT, conjugateGradient, MersenneTwisterGP11213, reduction, sortingNetworks, scan, histogram, BlackScholes, fastWalshTransform, transpose | the sample's own CPU verification (PASS line or exit code) |
| nvkvm-pv realapp kernels | stream_triad, reduce, nbody, blackscholes, mandelbrot, conv2d, sgemm_cublas, fft_cufft, sha256, memcpy2d | their `CHECK ok` (value checks; several are only `isfinite`) |
| managed memory | attach_verify, conjugateGradientUM, UnifiedMemoryStreams, UnifiedMemoryPerf | own check (UMStreams/UMPerf: completion only) |
| stress | gpu_burn 60 s | `GPU 0: OK` (0 compare errors) |
| PyTorch | torch_correct (new: GPU vs CPU matmul fp32/fp16, cuDNN conv, CNN train step, 1 GiB round trip, sort/cumsum, 4 streams), torch_ai_bench (nvkvm-pv `ai_bench.py`) | every `CHECK ok`; torch_correct is numerical vs CPU |
| LLM | hf_generate (transformers greedy, Qwen2-0.5B fp16), llama_cpp_gen (llama.cpp, Qwen2.5-1.5B Q4_K_M greedy), llama_bench | completes; greedy output digest compared host vs guest |
| CuPy | cupy (NVRTC kernel, cuBLAS, cuFFT, cuRAND vs NumPy) | every `CHECK ok` |
| Vulkan / OpenGL / OpenCL | vulkaninfo, vkpeak, egl_offscreen (EGL device platform), clinfo, clpeak, geekbench_gpu (GB6 OpenCL) | NVIDIA device / GFLOPS / no GL error / NVIDIA platform / bandwidth / all GB workloads run |
| video | nvenc_h264, nvenc_hevc, nvdec_h264 (`-hwaccel cuda`, fails on software fallback) | 600 frames encoded (ffprobe) / decoded on NVDEC |
| crypto / render | hashcat (md5 mask attack), blender_cycles (scripted Cycles scene, CUDA then OptiX) | cracks the plaintext / both renders finish |
| probe (added) | stream_probe {default, created, nonblocking, perthread, two, created2nd} | one launch + full verify on that stream shape |

**Out of scope on a 1-GPU 8–12 GB box**: multi_gpu_app.py, nccl_allreduce.py, vLLM tensor-parallel
(≥2 GPUs); vLLM Qwen2.5-32B-AWQ (48 GB); llama.cpp 7B → 1.5B (same code path, fits the 8 GB card);
RAPIDS cuDF, glmark2 + weston, Unsloth, `validate.sh` (nvkvm-specific bring-up; its CUDA/Vulkan/GL
checks are covered above). Blender's Open Data launcher was replaced by a scripted render because
its mirror TLS-timed-out on the first box.

## 2. Results

One app per **fresh boot** (`APPS_PER_BOOT=1`), because a boot degrades after ~6 CUDA processes (§3
B) — a batched verdict would be contaminated by the apps before it. Guest cells carry the root-cause
letter of §3. `cupy`, EGL, Vulkan, NVENC/NVDEC and the stream probes are from the harness-corrected
re-run `r2` (r1 lacked the CUDA headers CuPy's NVRTC path needs, and did not load `nvidia_drm` in the
guest as the host has it — neither changed a verdict except CuPy). The **PM column** is a second
experiment: guest persistence mode on (`nvidia-smi -pm 1`) and ALL rows in ONE boot, in table order;
its verdicts were **identical on both boxes** for every row it reached, and the boot wedged at
`llama_bench` (the 60th process, cause J) — every later row in that boot hung.

| app | host 3060 | guest 3060 | host 3070 | guest 3070 | guest + PM, ONE boot (both boxes) | failure point (first failing box) |
|---|---|---|---|---|---|---|
| nvidia_smi | PASS | PASS | PASS | PASS | PASS |  |
| deviceQuery | PASS | PASS | PASS | PASS | PASS |  |
| vectorAdd | PASS | PASS | PASS | PASS | PASS |  |
| vectorAddDrv | PASS | PASS | PASS | PASS | PASS |  |
| matrixMul | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| matrixMulDrv | PASS | PASS | PASS | PASS | PASS |  |
| bandwidthTest | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()" |
| simpleStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| asyncAPI | PASS | PASS | PASS | PASS | PASS |  |
| simpleAtomicIntrinsics | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleAtomicIntrinsics.cu:122 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| simpleCallback | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status" |
| simpleOccupancy | PASS | PASS | PASS | PASS | PASS |  |
| simpleZeroCopy | PASS | PASS | PASS | TIMEOUT (B) | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| simpleCooperativeGroups | PASS | PASS | PASS | PASS | PASS |  |
| concurrentKernels | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| simpleIPC | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs |
| UnifiedMemoryStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1] |
| UnifiedMemoryPerf | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(strea |
| conjugateGradientUM | PASS | FAIL (C) | PASS | FAIL (C) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC1 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ — Test Summary: Error amount = 1.000000, result = SUCCESS |
| cudaTensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| bf16TensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| globalToShmemAsyncCopy | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| cdpSimpleQuicksort | PASS | PASS | PASS | PASS | PASS |  |
| graphMemoryNodes | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_ |
| simpleCudaGraphs | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCudaGraphs.cu:273 code=719(cudaErrorLaunchFailure) "cudaGraphLaunch(graphExec, streamForGr |
| simpleCUBLAS | PASS | PASS | PASS | PASS | PASS |  |
| simpleCUFFT | PASS | PASS | PASS | PASS | PASS |  |
| conjugateGradient | PASS | PASS | PASS | PASS | PASS |  |
| MersenneTwisterGP11213 | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| reduction | PASS | PASS | PASS | PASS | PASS |  |
| sortingNetworks | PASS | PASS | PASS | PASS | PASS |  |
| scan | PASS | PASS | PASS | PASS | PASS |  |
| histogram | PASS | PASS | PASS | PASS | PASS |  |
| BlackScholes | PASS | PASS | PASS | PASS | PASS |  |
| fastWalshTransform | PASS | PASS | PASS | PASS | PASS |  |
| transpose | PASS | PASS | PASS | PASS | PASS |  |
| stream_triad | PASS | PASS | PASS | PASS | PASS |  |
| reduce | PASS | PASS | PASS | PASS | PASS |  |
| nbody | PASS | PASS | PASS | PASS | PASS |  |
| blackscholes | PASS | PASS | PASS | PASS | PASS |  |
| mandelbrot | PASS | PASS | PASS | PASS | PASS |  |
| conv2d | PASS | PASS | PASS | PASS | PASS |  |
| sgemm_cublas | PASS | PASS | PASS | PASS | PASS |  |
| fft_cufft | PASS | PASS | PASS | PASS | PASS |  |
| sha256 | PASS | PASS | PASS | PASS | PASS |  |
| memcpy2d | PASS | PASS | PASS | PASS | PASS |  |
| attach_verify | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) |
| gpu_burn | PASS | FAIL (G) | PASS | FAIL (G) | FAIL | SIGSEGV in gpu_burn right after cuInit (guest dmesg: segfault at the stack top); no Xid, no kf3 refusal |
| torch_correct | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: GET was unable to find an engine to execute this computation |
| torch_ai_bench | PASS | FAIL (C) | PASS | FAIL (A) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC2 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE — RuntimeError: CUDA error: unspecified launch failure |
| hf_generate | PASS | PASS | PASS | TIMEOUT (I) | PASS | silent hang; kf3 `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` (3070 box ran `fb-mb=6144`) |
| cupy | PASS | TIMEOUT (B) | PASS | PASS | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| llama_cpp_gen | PASS | TIMEOUT (B) | PASS | FAIL (A) | FAIL | 3060: silent hang, kf3 `slot 16 is full and nothing can be retired`; 3070: Xid 13 SKEDCHECK05 |
| llama_bench | PASS | FAIL (A) | PASS | FAIL (A) | TIMEOUT (J) | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — /workspace/apps/srcs/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu:109: CUDA error |
| vulkaninfo | PASS | TIMEOUT (F) | PASS | TIMEOUT (F) | hung (after J) | blocks inside an RM ioctl (strace: last call `NV_ESC_RM_CONTROL`, then nothing); guest dmesg: `scrubberDestruct: Timed out`, `ce_utils.c:349` assert |
| vkpeak | PASS | FAIL (F) | PASS | FAIL (F) | hung (after J) | No vulkan device |
| egl_offscreen | PASS | FAIL (E) | PASS | FAIL (E) | hung (after J) | CHECK egl_gl_Mtri_s FAIL |
| clinfo | PASS | PASS | PASS | PASS | hung (after J) |  |
| clpeak | PASS | PASS | PASS | PASS | not reached |  |
| nvenc_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264_nvenc @ 0x5aeb9435b9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvenc_hevc | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [hevc_nvenc @ 0x5b9be2c6a9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvdec_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264 @ 0x6003c48bbf80] decoder->cvdl->cuvidGetDecoderCaps(&caps) failed -> CUDA_ERROR_NO_DEVICE: no CUDA-capa |
| hashcat | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Watchdog: Temperature abort trigger set to 90c |
| blender_cycles | PASS | TIMEOUT (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: Error: Launch failed in CUDA queue synchronize (integrator_shade_surface) |
| geekbench_gpu | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — [0926/022702:ERROR:optimizer.cpp(122)] build_patches_padding: optimization failed for size { 32, 1, 1, }: Wait |
| stream_default | PASS | PASS | PASS | PASS | PASS |  |
| stream_created | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK created FAIL sync=719(unspecified launch failure) |
| stream_nonblocking | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK nonblocking FAIL sync=719(unspecified launch failure) |
| stream_perthread | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK perthread FAIL sync=719(unspecified launch failure) |
| stream_two | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK two_a FAIL sync=719(unspecified launch failure) |
| stream_created2nd | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK c2_created FAIL sync=719(unspecified launch failure) |

A 26 ['MersenneTwisterGP11213', 'UnifiedMemoryPerf', 'UnifiedMemoryStreams', 'attach_verify', 'bandwidthTest', 'blender_cycles', 'concurrentKernels', 'geekbench_gpu', 'globalToShmemAsyncCopy', 'graphMemoryNodes', 'hashcat', 'llama_bench', 'llama_cpp_gen', 'matrixMul', 'simpleAtomicIntrinsics', 'simpleCallback', 'simpleCudaGraphs', 'simpleIPC', 'simpleStreams', 'stream_created', 'stream_created2nd', 'stream_nonblocking', 'stream_perthread', 'stream_two', 'torch_ai_bench', 'torch_correct']

Totals (65 apps, probes excluded): **host 65/65 on both; guest 35/65 (3060), 34/65 (3070)**.
Digests: `hf_generate` greedy tokens **identical** host vs guest on the 3060 (`0d973108a6251e14`).
Raw rows: `traces/v3_app_matrix/<box>/{r1,r2,seq}/{host,guest}.res` + `triage.txt`; evidence excerpts
in `traces/v3_app_matrix/va1_rtx3060/evidence_excerpts.txt`.

## 3. Failure causes, ranked by apps blocked

| rank | cause | apps blocked (of 65) | signature |
|---|---|---|---|
| 1 | **A — kernels on a non-default stream fault the host GR** | **21** (+ all 5 non-default stream probes) | host `Xid 13, Graphics Exception: SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed`, class `c7c0`; kf3 `RC host twin … (guest chid 0x8/0x9, engine 0x1) … Xid 13 — forwarding RC_TRIGGERED`; app sees `cudaErrorLaunchFailure (719)` |
| 2 | **B — VA-space slot exhaustion ⇒ silent hang** | **5** directly (cupy, llama_cpp_gen, simpleZeroCopy, simpleAtomicIntrinsics, simpleCudaGraphs — nondeterministic, box-dependent) **and every app from the ~6th–7th CUDA process of a boot on** | kf3 `REFUSED VasKey(..) root 0x201000: slot N is full and nothing can be retired — raise WalkCfg::runs_per_pdb` + `split ticket N REFUSED`; the process then makes no progress, no Xid |
| 3 | **D — video engines absent** | 3 (nvenc_h264, nvenc_hevc, nvdec_h264) | `OpenEncodeSessionEx failed: unsupported device`; `cuvidGetDecoderCaps … CUDA_ERROR_NO_DEVICE`; no kf3 refusal (the guest never asks) |
| 4 | **C — MMU fault on a managed/UVM mapping** | 2 (conjugateGradientUM — **silent wrong answer**, torch_ai_bench on the 3060) | host `Xid 31 … MMU Fault: ENGINE GRAPHICS … FAULT_PDE`, first compute channel (guest chid 0x7) |
| 5 | **F — Vulkan** | 2 (vulkaninfo, vkpeak) | vulkaninfo blocks in an RM ioctl; vkpeak `No vulkan device`; guest RM `scrubberDestruct: Timed out`, `ce_utils.c:349` |
| 6 | E — EGL | 1 (egl_offscreen) | `eglInitialize failed`, no Xid, no kf3 refusal |
| 6 | G — gpu_burn | 1 | SIGSEGV in gpu_burn right after `cuInit` (reads past its stack); no Xid |
| 6 | J — host map `NoMemory` late in a long PM boot | 1 (llama_bench as the ~60th process; the boot then hangs for every later app) | kf3 `REFUSED VasKey(..) root 0x1f1cac000: 1 run(s) not applied: window map 0x110000+0x200000 (store @0x1000000): arm: NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory` |
| 6 | I — walk refused beyond the FB span | 1 (hf_generate, 3070 only) | `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` with `fb-mb=6144` |

### A — non-default streams (rank 1)
The split is exact and measured by `stream_probe` (r2, both boxes): **`default` PASS; `created`,
`nonblocking`, `perthread`, `two`, `created2nd` all FAIL with 719**, and `created2nd` passes its
default-stream half then fails the created-stream half in the same process. Every other A row names
a stream in its failing call (`cudaStreamSynchronize(stream)`, `cudaEventSynchronize`,
`cudaGraphLaunch`, cuBLAS/cuDNN handles bound to streams). The faulting host channel is always the
guest's **second or third** compute channel (guest chid 0x8/0x9); the first (chid 0x7) runs
default-stream work fine. `SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE` means the QMD asked for more shader
local memory than that channel's context has configured — i.e. the per-channel (per-subcontext)
local-memory setup that the driver does for the first channel is not in effect for the additional
channels on the host. ⚠ Mechanism is a hypothesis; the signature and the stream split are measured.
None of the failing user kernels use local memory themselves (`cuobjdump -res-usage`: `LOCAL:0`).

### B — slot exhaustion (rank 2)
`seq` experiment: 10× `vectorAdd` in ONE boot → **6 PASS then 4 TIMEOUT on the 3060, 5 PASS then 5
TIMEOUT on the 3070**, with `slot 58 is full and nothing can be retired` at root `0x201000` at the
first hang. The first r1 attempt (all apps batched in one boot) wedged at the 3rd CUDA app and every
later app timed out silently (`traces/v3_app_matrix/va1_rtx3060/r1_batched_contaminated/`). Single
processes with many mappings (llama.cpp loading a 1 GB model, CuPy) hit it on their own. ⇒ In
practice this blocks *every* app for any guest that has already run a handful of CUDA processes; it
is ranked 2 only because the per-boot matrix isolates it. With guest persistence mode the budget disappears (§4).

### C — Xid 31 on managed memory (rank 4)
`conjugateGradientUM` finishes, prints `result = SUCCESS`, and reports `Error amount = 1.000000`
(host `0.000000`): a **silent wrong result** behind a host MMU fault (`FAULT_PDE`, VA
`0x7d09_ac200000`, a UVM managed range). `torch_ai_bench` hit the same fault class on the 3060 (the
3070 run died earlier, on A).

## 4. Persistence mode, and what is still open

- ★ **Persistence mode removes B's per-boot budget.** `seqpm` (3060, `nvidia-smi -pm 1` in the guest,
  then `stream_created` + 9× `vectorAdd` in ONE boot): **all 9 vectorAdd PASS** (without PM: 6 then
  hang), and `stream_created` still FAILs with 719 (A is independent of PM). ⇒ B is tied to the guest
  RM tearing the adapter down and re-initialising it per process (what happens without PM), not to the
  processes themselves. `traces/v3_app_matrix/va1_rtx3060/seqpm/`. A full one-boot PM run of all 71
  rows (`pm1`) is the PM column of §2: B vanishes, A/C/G are unchanged, J appears at the 60th process.
- EGL / Vulkan / gpu_burn root causes are not localised beyond the signatures above (no kf3 refusal is
  logged for any of them).

## 5. Reproduce

    # on a box provisioned by scripts/bench/{provision_box,provision_host_driver,provision_bench_tree,build_kf3}.sh
    ln -sfn /workspace/apps /opt/apps
    bash scripts/apps/build_bundle.sh            # host: CUDA 12.6 toolkit + the bundle
    bash scripts/apps/setup_side.sh              # host runtime (identical to the guest's)
    bash scripts/apps/provision_guest_apps.sh    # guest image: bundle + runtime (stock qemu, slirp)
    bash scripts/apps/apps_matrix.sh host  <run> all
    KF3_BIN=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 bash scripts/apps/apps_matrix.sh guest <run> all
    #   (KF3_FB_MB=6144 on an 8 GB card; APPS_PER_BOOT=N to batch; diag_hook.sh for strace diagnostics)
