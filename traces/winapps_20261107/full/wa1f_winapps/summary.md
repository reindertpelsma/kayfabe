
### /root/prov/wa1f_winapps/run

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| cup2_ladder | - | PASS | - | proof out |
| cup3_ladder | - | PASS | - | proof out |
| deviceQuery_demo | - | PASS | - | proof out |
| driver_status | - | PASS | - | proof out |
| dxgi_adapters | - | PASS | - | proof out |
| nvidia_smi | - | PASS | - | proof out |
| opencl_icd | - | PASS | - | proof out |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| vulkaninfo | - | PASS | - | proof out |
| cuda_callback | - | PASS | - | proof out |
| cuda_occupancy | - | PASS | - | proof out |
| glgears_wgl | - | PASS | - | proof out |
| nbody_demo_gl | - | FAIL | - | rc=0 30s quiet=1 — no-gpu-proof |
| atomics_t | - | PASS | - | proof out |
| bandwidthTest_demo | - | PASS | - | proof out |
| blackscholes_t | - | PASS | - | proof out |
| cg_t | - | PASS | - | proof out |
| dxprobe_d3d11 | - | PASS | - | proof out |
| dxprobe_d3d12 | - | PASS | - | proof out |
| fft_t | - | PASS | - | proof out |
| fwt_t | - | PASS | - | proof out |
| graphs_t | - | PASS | - | proof out |
| hist_t | - | PASS | - | proof out |
| mandelbrot_t | - | PASS | - | proof out |
| matmul_t | - | PASS | - | proof out |
| memcpy2d_t | - | PASS | - | proof out |
| nbody_demo_bench | - | PASS | - | proof out |
| nbody_t | - | PASS | - | proof out |
| oceanFFT_demo | - | PASS | - | proof cuda-oceanfft-readback |
| randomFog_demo | - | PASS | - | proof cuda-randomfog-readback |
| reduce_t | - | PASS | - | proof out |
| rng_t | - | PASS | - | proof out |
| scan_t | - | PASS | - | proof out |
| sort_t | - | PASS | - | proof out |
| stream_created2nd_t | - | PASS | - | proof out |
| stream_created_t | - | PASS | - | proof out |
| stream_default_t | - | PASS | - | proof out |
| stream_nonblocking_t | - | PASS | - | proof out |
| stream_perthread_t | - | PASS | - | proof out |
| stream_two_t | - | PASS | - | proof out |
| streams_t | - | PASS | - | proof out |
| tc16_t | - | PASS | - | proof out |
| tcbf16_t | - | PASS | - | proof out |
| transpose_t | - | PASS | - | proof out |
| triad_t | - | PASS | - | proof out |
| zerocopy_t | - | PASS | - | proof out |
| overlay_yuv_hide | - | FAIL | - | rc=5 4s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"hide","verdict": |
| overlay_yuv_occlude | - | FAIL | - | rc=5 4s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"occlude","verdic |
| overlay_yuv_resize | - | FAIL | - | rc=5 6s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"resize","verdict |
| overlay_yuv_move | - | FAIL | - | rc=5 4s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"move","verdict": |
| overlay_yuv_recreate | - | FAIL | - | rc=5 5s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"recreate","verdi |
| edge_webgl | - | PASS | - | proof out |
| edge_webgl_headless | - | PASS | - | proof out |
| edge_webgpu | - | PASS | - | proof out |
| overlay_yuv_fullscreen | - | FAIL | - | rc=5 4s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"fullscreen","ver |
| overlay_yuv_probe | - | FAIL | - | rc=5 4s quiet=1 — rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"steady","verdict |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |
| cupy_asynccopy | - | PASS | - | proof out |
| cupy_cg | - | PASS | - | proof out |
| cupy_sha256 | - | PASS | - | proof out |
| d3d12_signal_probe | - | PASS | - | proof out |
| dxdiag_report | - | PASS | - | proof out |
| managed_t | - | TIMEOUT | - | rc=124 181s quiet=175 — timeout |
| torch_burn | - | PASS | - | proof out |
| ort_directml | - | PASS | - | proof out |
| cupy_check | - | PASS | - | proof out |
| hashcat | - | PASS | - | proof out |

PASS: 58, FAIL: 8, TIMEOUT: 1  (total 67)
per category: browser: 3/3; crypto: 1/1; cuda-demo: 6/7; cuda-equiv: 26/27; cuda-ladder: 3/3; d3d: 4/11; opencl: 1/1; opengl: 1/1; probe: 3/3; smi: 1/1; stream-probe: 6/6; stress: 1/1; torch: 1/1; vulkan: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
