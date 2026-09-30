
### /workspace/apps/results/r4off

| app | host | guest (batched) | guest (alone) | guest detail |
|---|---|---|---|---|
| nvidia_smi | - | PASS | - |  |
| deviceQuery | - | PASS | - |  |
| vectorAdd | - | PASS | - |  |
| vectorAddDrv | - | PASS | - |  |
| matrixMul | - | PASS | - |  |
| matrixMulDrv | - | PASS | - |  |
| bandwidthTest | - | PASS | - |  |
| simpleStreams | - | PASS | - |  |
| asyncAPI | - | PASS | - |  |
| simpleAtomicIntrinsics | - | PASS | - |  |
| simpleCallback | - | PASS | - |  |
| simpleOccupancy | - | PASS | - |  |
| simpleZeroCopy | - | PASS | - |  |
| simpleCooperativeGroups | - | PASS | - |  |
| concurrentKernels | - | PASS | - |  |
| simpleIPC | - | PASS | - |  |
| UnifiedMemoryStreams | - | FAIL | FAIL | rc=139 10s quiet=0 xid=0 kf3_refusals=12 — CUDA error at UnifiedMemoryStreams.cu:221 |
| UnifiedMemoryPerf | - | FAIL | FAIL | rc=1 5s quiet=0 xid=0 kf3_refusals=12 — Running ...CUDA error at matrixMultiplyPerf.cu:435 |
| conjugateGradientUM | - | FAIL | FAIL | rc=0 11s quiet=2 xid=0 kf3_refusals=12 — Test Summary: Error amount = 1.000000, result = SUCCESS |
| cudaTensorCoreGemm | - | PASS | - |  |
| bf16TensorCoreGemm | - | PASS | - |  |
| globalToShmemAsyncCopy | - | PASS | - |  |
| cdpSimpleQuicksort | - | TIMEOUT | TIMEOUT | rc=124 60s quiet=55 xid=0 kf3_refusals=11 — - |
| graphMemoryNodes | - | PASS | - |  |
| simpleCudaGraphs | - | PASS | - |  |
| simpleCUBLAS | - | PASS | - |  |
| simpleCUFFT | - | PASS | - |  |
| conjugateGradient | - | PASS | - |  |
| MersenneTwisterGP11213 | - | PASS | - |  |
| reduction | - | PASS | - |  |
| sortingNetworks | - | PASS | - |  |
| scan | - | PASS | - |  |
| histogram | - | PASS | - |  |
| BlackScholes | - | PASS | - |  |
| fastWalshTransform | - | PASS | - |  |
| transpose | - | PASS | - |  |
| stream_triad | - | PASS | - |  |
| reduce | - | PASS | - |  |
| nbody | - | PASS | - |  |
| blackscholes | - | PASS | - |  |
| mandelbrot | - | PASS | - |  |
| conv2d | - | PASS | - |  |
| sgemm_cublas | - | PASS | - |  |
| fft_cufft | - | PASS | - |  |
| sha256 | - | PASS | - |  |
| memcpy2d | - | PASS | - |  |
| attach_verify | - | FAIL | FAIL | rc=1 6s quiet=1 xid=0 kf3_refusals=12 — FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) |
| stream_default | - | PASS | - |  |
| stream_created | - | PASS | - |  |
| stream_nonblocking | - | PASS | - |  |
| stream_perthread | - | PASS | - |  |
| stream_two | - | PASS | - |  |
| stream_created2nd | - | PASS | - |  |
| gpu_burn | - | PASS | - |  |
| torch_correct | - | PASS | - |  |
| torch_ai_bench | - | PASS | - |  |
| hf_generate | - | PASS | - |  |
| cupy | - | PASS | - |  |
| llama_cpp_gen | - | PASS | - |  |
| llama_bench | - | PASS | - |  |
| vulkaninfo | - | PASS | - |  |
| vkpeak | - | PASS | - |  |
| egl_offscreen | - | PASS | - |  |
| clinfo | - | PASS | - |  |
| clpeak | - | PASS | - |  |
| nvenc_h264 | - | PASS | - |  |
| nvenc_hevc | - | PASS | - |  |
| nvdec_h264 | - | PASS | - |  |
| hashcat | - | PASS | - |  |
| blender_cycles | - | PASS | - |  |
| geekbench_gpu | - | PASS | - |  |

host - / guest FAIL: 4, host - / guest PASS: 66, host - / guest TIMEOUT: 1
