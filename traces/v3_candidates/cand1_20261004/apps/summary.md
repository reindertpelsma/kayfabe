
### /workspace/apps/results/cand1b

| app | host | guest (batched) | guest (alone) | guest detail |
|---|---|---|---|---|
| nvidia_smi | PASS | PASS | - |  |
| deviceQuery | PASS | PASS | - |  |
| vectorAdd | PASS | PASS | - |  |
| vectorAddDrv | PASS | PASS | - |  |
| matrixMul | PASS | PASS | - |  |
| matrixMulDrv | PASS | PASS | - |  |
| bandwidthTest | PASS | PASS | - |  |
| simpleStreams | PASS | PASS | - |  |
| asyncAPI | PASS | PASS | - |  |
| simpleAtomicIntrinsics | PASS | PASS | - |  |
| simpleCallback | PASS | PASS | - |  |
| simpleOccupancy | PASS | PASS | - |  |
| simpleZeroCopy | PASS | PASS | - |  |
| simpleCooperativeGroups | PASS | PASS | - |  |
| concurrentKernels | PASS | PASS | - |  |
| simpleIPC | PASS | PASS | - |  |
| UnifiedMemoryStreams | PASS | FAIL | FAIL | rc=134 10s quiet=0 xid=0 kf3_refusals=12 — CUDA error at UnifiedMemoryStreams.cu:214 |
| UnifiedMemoryPerf | PASS | FAIL | FAIL | rc=1 5s quiet=1 xid=0 kf3_refusals=12 — Running ...CUDA error at matrixMultiplyPerf.cu:435 |
| conjugateGradientUM | PASS | FAIL | FAIL | rc=0 9s quiet=1 xid=0 kf3_refusals=12 — Test Summary: Error amount = 1.000000, result = SUCCESS |
| cudaTensorCoreGemm | PASS | PASS | - |  |
| bf16TensorCoreGemm | PASS | PASS | - |  |
| globalToShmemAsyncCopy | PASS | PASS | - |  |
| cdpSimpleQuicksort | PASS | PASS | - |  |
| graphMemoryNodes | PASS | PASS | - |  |
| simpleCudaGraphs | PASS | PASS | - |  |
| simpleCUBLAS | PASS | PASS | - |  |
| simpleCUFFT | PASS | PASS | - |  |
| conjugateGradient | PASS | PASS | - |  |
| MersenneTwisterGP11213 | PASS | PASS | - |  |
| reduction | PASS | PASS | - |  |
| sortingNetworks | PASS | PASS | - |  |
| scan | PASS | PASS | - |  |
| histogram | PASS | PASS | - |  |
| BlackScholes | PASS | PASS | - |  |
| fastWalshTransform | PASS | PASS | - |  |
| transpose | PASS | PASS | - |  |
| stream_triad | PASS | PASS | - |  |
| reduce | PASS | PASS | - |  |
| nbody | PASS | PASS | - |  |
| blackscholes | PASS | PASS | - |  |
| mandelbrot | PASS | PASS | - |  |
| conv2d | PASS | PASS | - |  |
| sgemm_cublas | PASS | PASS | - |  |
| fft_cufft | PASS | PASS | - |  |
| sha256 | PASS | PASS | - |  |
| memcpy2d | PASS | PASS | - |  |
| attach_verify | PASS | FAIL | FAIL | rc=1 5s quiet=1 xid=0 kf3_refusals=12 — FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) |
| stream_default | PASS | PASS | - |  |
| stream_created | PASS | PASS | - |  |
| stream_nonblocking | PASS | PASS | - |  |
| stream_perthread | PASS | PASS | - |  |
| stream_two | PASS | PASS | - |  |
| stream_created2nd | PASS | PASS | - |  |
| gpu_burn | PASS | PASS | - |  |
| torch_correct | PASS | PASS | - | cnn_train_step digest == host |
| torch_ai_bench | PASS | PASS | - |  |
| hf_generate | PASS | PASS | - | hf_generate digest == host |
| cupy | PASS | PASS | - |  |
| llama_cpp_gen | PASS | PASS | - | llama_cpp digest == host |
| llama_bench | PASS | PASS | - |  |
| vulkaninfo | PASS | PASS | - |  |
| vkpeak | PASS | PASS | - |  |
| egl_offscreen | PASS | PASS | - |  |
| clinfo | PASS | PASS | - |  |
| clpeak | PASS | PASS | - |  |
| nvenc_h264 | PASS | PASS | - |  |
| nvenc_hevc | PASS | PASS | - |  |
| nvdec_h264 | PASS | PASS | - |  |
| hashcat | PASS | PASS | - |  |
| blender_cycles | PASS | PASS | - |  |
| geekbench_gpu | PASS | PASS | - |  |

host PASS / guest FAIL: 4, host PASS / guest PASS: 67
