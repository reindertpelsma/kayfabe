| app | category | tier | full run (wa1f) | secs | small run (wa1) | note |
|---|---|---:|---|---:|---|---|
| cup2_ladder | cuda-ladder | 1 | PASS | 5 |  |  |
| cup3_ladder | cuda-ladder | 1 | PASS | 5 |  |  |
| deviceQuery_demo | cuda-demo | 1 | PASS | 5 | PASS 5s |  |
| driver_status | probe | 1 | PASS | 5 |  |  |
| dxgi_adapters | probe | 1 | PASS | 5 |  |  |
| nvidia_smi | smi | 1 | PASS | 5 | PASS 5s |  |
| opencl_icd | opencl | 1 | PASS | 5 |  |  |
| vectorAdd_demo | cuda-demo | 1 | PASS | 5 |  |  |
| vulkaninfo | vulkan | 1 | PASS | 7 | PASS 6s |  |
| cuda_callback | cuda-equiv | 1 | PASS | 5 |  |  |
| cuda_occupancy | cuda-equiv | 1 | PASS | 5 |  |  |
| glgears_wgl | opengl | 1 | PASS | 15 | PASS 15s |  |
| nbody_demo_gl | cuda-demo | 1 | FAIL | 30 |  | no-gpu-proof |
| atomics_t | cuda-equiv | 1 | PASS | 16 |  |  |
| bandwidthTest_demo | cuda-demo | 1 | PASS | 5 |  |  |
| blackscholes_t | cuda-equiv | 1 | PASS | 9 |  |  |
| cg_t | cuda-equiv | 1 | PASS | 18 |  |  |
| dxprobe_d3d11 | d3d | 1 | PASS | 5 | PASS 5s |  |
| dxprobe_d3d12 | d3d | 1 | PASS | 7 |  |  |
| fft_t | cuda-equiv | 1 | PASS | 7 |  |  |
| fwt_t | cuda-equiv | 1 | PASS | 6 |  |  |
| graphs_t | cuda-equiv | 1 | PASS | 6 |  |  |
| hist_t | cuda-equiv | 1 | PASS | 6 |  |  |
| mandelbrot_t | cuda-equiv | 1 | PASS | 7 |  |  |
| matmul_t | cuda-equiv | 1 | PASS | 7 | PASS 16s |  |
| memcpy2d_t | cuda-equiv | 1 | PASS | 6 |  |  |
| nbody_demo_bench | cuda-demo | 1 | PASS | 4 |  |  |
| nbody_t | cuda-equiv | 1 | PASS | 7 |  |  |
| oceanFFT_demo | cuda-demo | 1 | PASS | 30 |  |  |
| randomFog_demo | cuda-demo | 1 | PASS | 32 |  |  |
| reduce_t | cuda-equiv | 1 | PASS | 9 |  |  |
| rng_t | cuda-equiv | 1 | PASS | 7 |  |  |
| scan_t | cuda-equiv | 1 | PASS | 17 |  |  |
| sort_t | cuda-equiv | 1 | PASS | 9 |  |  |
| stream_created2nd_t | stream-probe | 1 | PASS | 6 |  |  |
| stream_created_t | stream-probe | 1 | PASS | 6 |  |  |
| stream_default_t | stream-probe | 1 | PASS | 7 |  |  |
| stream_nonblocking_t | stream-probe | 1 | PASS | 6 |  |  |
| stream_perthread_t | stream-probe | 1 | PASS | 6 |  |  |
| stream_two_t | stream-probe | 1 | PASS | 6 |  |  |
| streams_t | cuda-equiv | 1 | PASS | 9 |  |  |
| tc16_t | cuda-equiv | 1 | PASS | 9 |  |  |
| tcbf16_t | cuda-equiv | 1 | PASS | 9 |  |  |
| transpose_t | cuda-equiv | 1 | PASS | 6 |  |  |
| triad_t | cuda-equiv | 1 | PASS | 9 |  |  |
| zerocopy_t | cuda-equiv | 1 | PASS | 7 |  |  |
| overlay_yuv_hide | d3d | 2 | FAIL | 4 |  | rc:5,success-string-missing |
| overlay_yuv_occlude | d3d | 2 | FAIL | 4 |  | rc:5,success-string-missing |
| overlay_yuv_resize | d3d | 2 | FAIL | 6 |  | rc:5,success-string-missing |
| overlay_yuv_move | d3d | 2 | FAIL | 4 |  | rc:5,success-string-missing |
| overlay_yuv_recreate | d3d | 2 | FAIL | 5 |  | rc:5,success-string-missing |
| edge_webgl | browser | 1 | PASS | 22 |  |  |
| edge_webgl_headless | browser | 1 | PASS | 15 |  |  |
| edge_webgpu | browser | 1 | PASS | 21 |  |  |
| overlay_yuv_fullscreen | d3d | 2 | FAIL | 4 |  | rc:5,success-string-missing |
| overlay_yuv_probe | d3d | 1 | FAIL | 4 |  | rc:5,success-string-missing |
| cup8_ladder | cuda-ladder | 1 | PASS | 6 | PASS 5s |  |
| cupy_asynccopy | cuda-equiv | 1 | PASS | 17 |  |  |
| cupy_cg | cuda-equiv | 1 | PASS | 11 |  |  |
| cupy_sha256 | cuda-equiv | 1 | PASS | 7 |  |  |
| d3d12_signal_probe | d3d | 1 | PASS | 9 |  |  |
| dxdiag_report | probe | 1 | PASS | 44 |  |  |
| managed_t | cuda-equiv | 1 | TIMEOUT | 181 |  | timeout |
| torch_burn | stress | 1 | PASS | 78 |  |  |
| ort_directml | d3d | 1 | PASS | 11 |  |  |
| cupy_check | torch | 1 | PASS | 9 |  |  |
| hashcat | crypto | 1 | PASS | 30 |  |  |
| overlay_yuv_soak | d3d | 3 | NOT_RUN (budget) |  |  |  |
| torch_correct | torch | 1 | NOT_RUN (budget) |  | PASS 19s |  |
| cdp_quicksort | cuda-equiv | 1 | NOT_RUN (budget) |  |  |  |
| hf_generate | ai | 1 | NOT_RUN (budget) |  |  |  |
| llama_cuda_gen | llm | 1 | NOT_RUN (budget) |  |  |  |
| vkpeak | vulkan | 1 | NOT_RUN (budget) |  |  |  |
| blender_cycles | render | 1 | NOT_RUN (budget) |  |  |  |
| llama_cuda_bench | llm | 1 | NOT_RUN (budget) |  |  |  |
| llama_vulkan_bench | llm | 1 | NOT_RUN (budget) |  |  |  |
| torch_ai_bench | torch | 1 | NOT_RUN (budget) |  |  |  |
| clpeak_cuda | opencl | 1 | NOT_RUN (budget) |  |  |  |
| clpeak_ocl | opencl | 1 | NOT_RUN (budget) |  |  |  |
