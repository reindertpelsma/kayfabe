### /workspace/gfxset/results/gs2

| item | bare metal | guest | digests vs bare metal | verdict | detail |
|---|---|---|---|---|---|
| pv_validate_vk | PASS | PASS (batched) | 2/2 match | **PASS** | kf3_refusals=4 (see triage) |
| pv_validate_gl | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| pv_vk_rt_ext | PASS | PASS (batched) | 1/1 match | **PASS** | kf3_refusals=4 (see triage) |
| pv_vk_create_device | PASS | PASS (batched) | 0/0 match | **PASS** |  |
| vk_info | PASS | PASS (batched) | 6/6 match | **PASS** |  |
| vkpeak | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| egl_offscreen | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| gl_micro | PASS | PASS (batched) | 0/0 match | **PASS** |  |
| pv_fbo_formats | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| pv_egl_dmabuf_export | PASS | PASS (batched) | 0/0 match | **PASS** |  |
| pv_dmabuf_import | PASS | PASS (batched) | 4/4 match | **PASS** |  |
| pv_xiso_sharing | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| pv_signal_restart_export | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| pv_gbm_egl_import | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| pv_gbmprobe_gbmshot | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| weston_headless | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| weston_client_diff | PASS | PASS (batched) | 0/0 match | **PASS** |  |
| sway_screencap | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| glmark2 | PASS | PASS (batched) | 3/3 match | **PASS** |  |
| pv_nvenc_nvdec_fps | PASS | PASS (batched) | 1/1 match | **PASS** | kf3_refusals=1 (see triage) |
| video_nvenc_nvdec | PASS | PASS (batched) | 13/13 match | **PASS** |  |
| geekbench_vulkan | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| blender_opendata | PASS | PASS (batched) | 0/0 match | **PASS** |  |
| vk_compute | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| vk_render | PASS | PASS (batched) | 3/3 match | **PASS** |  |
| egl_render | PASS | PASS (batched) | 3/3 match | **PASS** |  |
| gl_info | PASS | PASS (batched) | 3/3 match | **PASS** |  |
| glx_vgl | PASS | PASS (batched) | 3/3 match | **PASS** |  |
| ff_cuda | PASS | PASS (batched) | 6/6 match | **PASS** |  |
| ff_vulkan | PASS | PASS (batched) | 9/9 match | **PASS** |  |
| ff_opencl | PASS | PASS (batched) | 8/8 match | **PASS** |  |
| ff_placebo | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| blender_cycles_cuda | PASS | PASS (batched) | 0/0 match, 1 nondet, png by PSNR 96.015316dB vs floor 99.325248dB: DIFF DIFF:png(psnr) | **FAIL(DIFF)** | rc=0 4s host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 — warn:BLENDER_OK CYCLES CUDA |
| blender_cycles_optix | PASS | PASS (batched) | 0/0 match, 1 nondet, png by PSNR 99.994716dB vs floor 100.786528dB: MATCH | **PASS** |  |
| blender_eevee | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| blender_workbench | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| blender_eevee_vulkan | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| vk_ofa | PASS | PASS (batched) | 2/2 match | **PASS** |  |

FAIL(DIFF): 1, PASS: 37
GSET_SUITE_SUMMARY pass=37/38 notrun_host=0
