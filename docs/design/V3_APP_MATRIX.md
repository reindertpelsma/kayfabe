# V3 app matrix — which real CUDA apps work in a kayfabe v3 fat guest

**STATUS: LIVE, 2026-09-26 — IN PROGRESS. Interim (01:25 UTC): host baseline 65/65 PASS on both boxes; guest lane 42/65 measured on the RTX 3060.**

Question (owner): nvkvm-pv (the shipped Mode-1 sibling) validated a set of real CUDA apps. Run the
same set inside a kayfabe **v3 fat guest** (full Ubuntu 24.04 guest, stock NVIDIA 580.159.04 guest
driver, kf3 device) and record which ones **work** — correct output, no hang, no crash, no Xid.
Not parity, not timing. Every app is run on the **host of the same box first** (bare metal), so a
guest-only failure indicts kayfabe, not the app.

## 1. The app set (inventory of nvkvm-pv) and what "pass" means here

Sources: `nvkvm-pv/tests/perf/{realapp_matrix.md,matrix_remote.sh,graphics_remote.sh,apps/}`,
`tests/integration/attach_verify.cu`, `docs/reference/{parity,blender-opendata,openbenchmarking-clpeak}.md`,
`tests/uvm_multiarch/build_apps.sh`, `tests/validate.sh`.

Harness: `scripts/apps/` (this branch). One binary set is built on the host
(`build_bundle.sh`: CUDA 12.6 toolkit, `-arch=sm_86`) and the **same files** run on both sides;
`setup_side.sh` installs the identical runtime on host and guest (torch 2.6.0+cu124,
torchvision 0.21.0, transformers 4.46.3, cupy-cuda12x 13.3.0, distro ffmpeg/hashcat/vulkan-tools/clinfo,
Blender 4.5.0 via the Open Data launcher, Geekbench 6.4.0). `run_apps.sh` holds every app's
command and pass predicate (pass = rc 0 **and** the app's own success string **and** no
`CHECK … FAIL`).

| group | apps | pass predicate |
|---|---|---|
| nvidia-smi | `nvidia_smi` | lists the RTX GPU |
| cuda-samples v12.5 (self-verifying) | deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, clock_nvrtc, simpleIPC, UnifiedMemoryStreams, UnifiedMemoryPerf, conjugateGradientUM, cudaTensorCoreGemm, bf16TensorCoreGemm, globalToShmemAsyncCopy, cdpSimpleQuicksort, graphMemoryNodes, simpleCudaGraphs, simpleCUBLAS, simpleCUFFT, conjugateGradient, MersenneTwisterGP11213, reduction, sortingNetworks, scan, histogram, BlackScholes, fastWalshTransform, transpose | the sample's own `PASS`/`passed` line (CPU-verified results) |
| nvkvm-pv realapp kernels | stream_triad, reduce, nbody, blackscholes, mandelbrot, conv2d, sgemm_cublas (cuBLAS), fft_cufft (cuFFT), sha256, memcpy2d | their `CHECK ok` (value checks — note several are only `isfinite`) |
| managed memory | attach_verify (nvkvm-pv), conjugateGradientUM, UnifiedMemoryStreams, UnifiedMemoryPerf | own check (UMStreams/UMPerf verify nothing: completion only) |
| stress | gpu_burn 60 s | `GPU 0: OK` (0 compare errors) |
| PyTorch | torch_correct (new: GPU vs CPU matmul fp32/fp16, cuDNN conv, CNN train step, 1 GiB round trip, sort/cumsum, 4 streams), torch_ai_bench (nvkvm-pv `ai_bench.py`: matmul fp32/fp16, ResNet-50 infer/AMP/train, ViT-B/16, BERT) | every `CHECK ok`; torch_correct is numerical vs CPU |
| LLM | hf_generate (transformers greedy, Qwen2-0.5B fp16), llama_cpp_gen (llama.cpp CUDA, Qwen2.5-1.5B Q4_K_M, greedy), llama_bench | completes; **greedy output digest compared host vs guest** |
| CuPy | cupy (NVRTC kernel, cuBLAS, cuFFT, cuRAND vs NumPy) | every `CHECK ok` |
| Vulkan | vulkaninfo, vkpeak | NVIDIA device enumerated / fp32 GFLOPS printed |
| OpenGL | egl_offscreen (EGL device platform, headless) | `CHECK ok` (no GL error) |
| OpenCL | clinfo, clpeak, geekbench_gpu (Geekbench 6 OpenCL) | NVIDIA platform / bandwidth line / OpenCL score (GB validates its own workloads) |
| video | nvenc_h264, nvenc_hevc, nvdec_h264 (`-hwaccel cuda`) | output has all 600 frames (ffprobe count) / decoder reaches frame 600 |
| crypto | hashcat (md5 mask attack) | cracks the known plaintext |
| render | blender_cycles (Open Data `monster`, CUDA then OptiX) | benchmark completes both devices |

**Out of scope on a 1-GPU 8–12 GB box**, with the reason: multi_gpu_app.py, nccl_allreduce.py and
vLLM tensor-parallel (need ≥2 GPUs); vLLM Qwen2.5-32B-AWQ (needs 48 GB VRAM); llama.cpp 7B replaced
by 1.5B (same code path, fits the 8 GB 3070); RAPIDS cuDF, glmark2 + headless weston, Unsloth,
`validate.sh` (nvkvm-specific bring-up checks; its CUDA/Vulkan/GL checks are covered above).

## 2. Results

_Interim, 2026-09-26 01:25 UTC — the full table replaces this when both boxes finish._

- **Host (bare metal, same box): 65/65 PASS** on the RTX 3060 (va1) and the RTX 3070 (va2).
- **Guest, one app per fresh boot, RTX 3060, first 42 apps:** 28 PASS, 14 FAIL. Every FAIL but one
  is `cudaErrorLaunchFailure` (719) with a **host** `Xid 13, Graphics Exception:
  SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed` on class `c7c0` (AMPERE_COMPUTE_B) — and the failing
  apps are the ones that launch on a **created (non-default) stream** (matrixMul, simpleStreams,
  concurrentKernels, simpleCallback, MersenneTwister, globalToShmemAsyncCopy, the CUDA-graph
  samples…); their default-stream counterparts (vectorAdd, asyncAPI, reduction, the nvkvm-pv
  kernels, simpleCUBLAS) pass. `stream_probe` (added) tests that split directly.
- `conjugateGradientUM` is a **silent wrong answer**: rc 0 and `result = SUCCESS`, with
  `Error amount = 1.000000` (host: 0.000000).
- **Batching is itself a failure**: many apps in ONE boot wedged at the 3rd CUDA process — every
  later app timed out silently, kf3 logging `REFUSED VasKey(..) root 0x201000: slot 27 is full and
  nothing can be retired — raise WalkCfg::runs_per_pdb`. Hence one app per boot.

## 3. Failure causes, ranked by apps blocked

_pending_

## 4. Reproduce

    # on a box provisioned by scripts/bench/{provision_box,provision_host_driver,provision_bench_tree,build_kf3}.sh
    ln -sfn /workspace/apps /opt/apps
    bash scripts/apps/build_bundle.sh            # host: toolkit + bundle
    bash scripts/apps/setup_side.sh              # host runtime (same as the guest's)
    bash scripts/apps/provision_guest_apps.sh    # guest image: bundle + runtime
    bash scripts/apps/apps_matrix.sh host  <run> all
    bash scripts/apps/apps_matrix.sh guest <run> all
