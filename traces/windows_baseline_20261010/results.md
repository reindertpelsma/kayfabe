83 apps: FAIL 4, PASS 78, TIMEOUT 1

| category | PASS | FAIL | TIMEOUT | NOTRUN |
|---|---|---|---|---|
| browser | 0 | 3 | 0 | 0 |
| crypto | 1 | 0 | 0 | 0 |
| cuda-demo | 7 | 0 | 0 | 0 |
| cuda-equiv | 25 | 0 | 0 | 0 |
| cuda-ladder | 3 | 0 | 0 | 0 |
| d3d | 6 | 0 | 0 | 0 |
| llm | 3 | 0 | 0 | 0 |
| opencl | 3 | 0 | 0 | 0 |
| opengl | 6 | 0 | 0 | 0 |
| probe | 3 | 0 | 0 | 0 |
| render | 1 | 0 | 0 | 0 |
| smi | 1 | 0 | 0 | 0 |
| stream-probe | 6 | 0 | 0 | 0 |
| stress | 1 | 0 | 0 | 0 |
| torch | 3 | 0 | 0 | 0 |
| video | 5 | 1 | 0 | 0 |
| vulkan | 4 | 0 | 1 | 0 |

| reasoned difficulty for kayfabe | PASS | FAIL | TIMEOUT | NOTRUN |
|---|---|---|---|---|
| low | 34 | 0 | 0 | 0 |
| medium | 15 | 1 | 0 | 0 |
| high | 29 | 3 | 1 | 0 |

| app | cat | tier | diff | verdict | secs | proof | GPU engines (peak %) | nvlddmkm/153 | note |
|---|---|---|---|---|---|---|---|---|---|
| atomics_t | cuda-equiv | 1 | low | PASS | 18 | out |  | 0 |  |
| bandwidthTest_demo | cuda-demo | 1 | medium | PASS | 4 | out |  | 0 |  |
| blackscholes_t | cuda-equiv | 1 | low | PASS | 18 | out |  | 0 |  |
| blender_cycles | render | 1 | high | PASS | 34 | out |  | 0 |  |
| cg_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| clpeak_cuda | opencl | 1 | high | PASS | 1176 | out,pdh:3d+copy | 3d=3251,copy=1 | 3 | warn:LiteRT: LiteRT library (libLiteRt) not found |
| clpeak_ocl | opencl | 1 | high | PASS | 1037 | out,pdh:3d+copy | 3d=148,copy=1 | 0 | warn:LiteRT: LiteRT library (libLiteRt) not found |
| cup2_ladder | cuda-ladder | 1 | low | PASS | 4 | out |  | 0 |  |
| cup3_ladder | cuda-ladder | 1 | low | PASS | 3 | out |  | 0 |  |
| cup8_ladder | cuda-ladder | 1 | medium | PASS | 4 | smi |  | 0 |  |
| cupy_asynccopy | cuda-equiv | 1 | low | PASS | 24 | out |  | 0 |  |
| cupy_cg | cuda-equiv | 1 | low | PASS | 11 | out |  | 0 |  |
| cupy_check | torch | 1 | medium | PASS | 9 | out |  | 0 |  |
| cupy_sha256 | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| d3d11va_h264 | video | 1 | high | PASS | 30 | pdh:videodecode,smi | copy=1,videodecode=79 | 0 |  |
| d3d12_signal_probe | d3d | 1 | medium | PASS | 7 | out |  | 0 |  |
| deviceQuery_demo | cuda-demo | 1 | low | PASS | 3 | out |  | 0 |  |
| driver_status | probe | 1 | low | PASS | 4 | out |  | 0 |  |
| dxdiag_report | probe | 1 | medium | PASS | 88 | out |  | 0 |  |
| dxgi_adapters | probe | 1 | low | PASS | 3 | out |  | 0 |  |
| dxprobe_d3d11 | d3d | 1 | high | PASS | 5 | out |  | 0 |  |
| dxprobe_d3d12 | d3d | 1 | high | PASS | 4 | out |  | 0 |  |
| edge_video_h264 | video | 1 | high | FAIL | 85 | none:pdh:VideoDecode |  | 0 | rc=1,success-string-missing,no-gpu-proof |
| edge_webgl | browser | 1 | high | FAIL | 78 | none:out,pdh:3D|Compute|Copy |  | 0 | rc=1,success-string-missing,no-gpu-proof |
| edge_webgl_headless | browser | 1 | medium | FAIL | 75 | none:out,pdh:3D|Compute|Copy |  | 0 | rc=1,success-string-missing,no-gpu-proof |
| edge_webgpu | browser | 1 | high | FAIL | 78 | none:out,pdh:Compute|3D|Copy |  | 0 | rc=1,success-string-missing,no-gpu-proof |
| fft_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| furmark_gl_bench | opengl | 1 | high | PASS | 27 | out,pdh:3d | 3d=52 | 0 |  |
| furmark_glinfo | opengl | 1 | medium | PASS | 7 | out |  | 0 |  |
| furmark_knot_gl | opengl | 1 | high | PASS | 27 | out,pdh:3d | 3d=55 | 0 |  |
| furmark_vk_bench | vulkan | 1 | high | PASS | 27 | out,pdh:3d | 3d=98 | 0 |  |
| furmark_vkinfo | vulkan | 1 | medium | PASS | 4 | out |  | 0 |  |
| fwt_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| glgears_wgl | opengl | 1 | high | PASS | 14 | out,pdh:3d | 3d=2 | 0 |  |
| gputest_fur | opengl | 1 | high | PASS | 21 | out,pdh:3d | 3d=49 | 0 |  |
| gputest_triangle | opengl | 1 | high | PASS | 17 | out,pdh:3d | 3d=1 | 0 |  |
| graphs_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| gravitymark_d3d11 | d3d | 1 | high | PASS | 204 | pdh:3d+copy | 3d=51,copy=5 | 0 |  |
| gravitymark_d3d12 | d3d | 1 | high | PASS | 184 | pdh:3d | 3d=100 | 0 |  |
| gravitymark_vk | vulkan | 1 | high | TIMEOUT | 604 | - | 3d=48,copy=14 | 0 | KFWAIT gravitymark_vk STILL_RUNNING_AFTER 600 s |
| hashcat | crypto | 1 | medium | PASS | 22 | out |  | 0 | warn:Watchdog: Temperature abort trigger set to 90c |
| hist_t | cuda-equiv | 1 | low | PASS | 6 | out |  | 0 |  |
| llama_cuda_bench | llm | 1 | high | PASS | 81 | out |  | 0 |  |
| llama_cuda_gen | llm | 1 | high | PASS | 13 | out |  | 0 |  |
| llama_vulkan_bench | llm | 1 | high | PASS | 37 | out,pdh:3d+copy | 3d=706,copy=2 | 0 |  |
| managed_t | cuda-equiv | 1 | high | PASS | 9 | out |  | 0 |  |
| mandelbrot_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| matmul_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| memcpy2d_t | cuda-equiv | 1 | medium | PASS | 7 | out |  | 0 |  |
| nbody_demo_bench | cuda-demo | 1 | medium | PASS | 4 | out |  | 0 |  |
| nbody_demo_gl | cuda-demo | 1 | high | PASS | 30 | pdh:3d | 3d=34 | 0 |  |
| nbody_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| nvdec_h264 | video | 1 | high | PASS | 25 | pdh:videodecode,smi | videodecode=87 | 0 |  |
| nvenc_av1 | video | 1 | high | PASS | 7 | pdh:videoencode,smi | videoencode=33 | 0 |  |
| nvenc_h264 | video | 1 | high | PASS | 7 | pdh:videoencode,smi | videoencode=19 | 0 |  |
| nvenc_hevc | video | 1 | high | PASS | 7 | pdh:videoencode,smi | videoencode=26 | 0 |  |
| nvidia_smi | smi | 1 | low | PASS | 3 | out |  | 0 |  |
| oceanFFT_demo | cuda-demo | 1 | high | PASS | 29 | pdh:3d | 3d=24 | 0 |  |
| opencl_icd | opencl | 1 | low | PASS | 3 | out |  | 0 |  |
| ort_directml | d3d | 1 | high | PASS | 9 | out |  | 0 |  |
| randomFog_demo | cuda-demo | 1 | high | PASS | 29 | pdh:3d | 3d=24 | 0 |  |
| reduce_t | cuda-equiv | 1 | medium | PASS | 7 | out |  | 0 |  |
| rng_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| scan_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| sort_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_created2nd_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_created_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_default_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_nonblocking_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_perthread_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| stream_two_t | stream-probe | 1 | low | PASS | 7 | out |  | 0 |  |
| streams_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| tc16_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| tcbf16_t | cuda-equiv | 1 | low | PASS | 7 | out |  | 0 |  |
| torch_ai_bench | torch | 1 | high | PASS | 51 | out,pdh:3d | 3d=1046 | 0 |  |
| torch_burn | stress | 1 | high | PASS | 67 | out,pdh:3d,smi | 3d=156 | 0 | warn:[burn] t=10s iters=805 errors=0 |
| torch_correct | torch | 1 | medium | PASS | 11 | out |  | 0 |  |
| transpose_t | cuda-equiv | 1 | medium | PASS | 7 | out |  | 0 |  |
| triad_t | cuda-equiv | 1 | medium | PASS | 6 | out |  | 0 |  |
| vectorAdd_demo | cuda-demo | 1 | low | PASS | 4 | smi |  | 0 |  |
| vkpeak | vulkan | 1 | high | PASS | 136 | out,pdh:3d | 3d=255 | 0 | warn:System.Management.Automation.RemoteException |
| vulkaninfo | vulkan | 1 | low | PASS | 4 | out |  | 0 | warn:WARNING: [Loader Message] Code 0 : windows_read_data_files_in_registry: Registry look |
| zerocopy_t | cuda-equiv | 1 | medium | PASS | 7 | out |  | 0 |  |
