| item | value | bare metal | guest | guest/bare | note |
|---|---|---|---|---|---|
| vk_info | vk_ext_count | 251 | 251 | 1.00 |  |
| vkpeak | fp32-scalar | 15061.6 | 15010.8 | 1.00 |  |
| vkpeak | fp32-vec4 | 14892.3 | 14851.4 | 1.00 |  |
| vkpeak | fp16-scalar | 22407.8 | 22401.5 | 1.00 |  |
| vkpeak | fp16-vec4 | 16478.4 | 16473.5 | 1.00 |  |
| vkpeak | fp16-matrix | 89517.6 | 89097.4 | 1.00 |  |
| vkpeak | fp64-scalar | 350.98 | 352.04 | 1.00 |  |
| vkpeak | fp64-vec4 | 351.97 | 351.05 | 1.00 |  |
| vkpeak | int32-scalar | 11231 | 11203.3 | 1.00 |  |
| vkpeak | int32-vec4 | 11125.1 | 11123.1 | 1.00 |  |
| vkpeak | int16-scalar | 8963.58 | 8960.43 | 1.00 |  |
| vkpeak | int16-vec4 | 9186.88 | 9104.42 | 0.99 |  |
| vkpeak | int8-dotprod | 10724.7 | 10682 | 1.00 |  |
| vkpeak | int8-matrix | 177313 | 176667 | 1.00 |  |
| vkpeak | bf16-matrix | 44645.2 | 44368.7 | 0.99 |  |
| egl_offscreen | Mtri_s | 2.6 | 2.6 | 1.00 |  |
| gl_micro | gl_decompose.drawcall_kcalls_s | 24281.5 | 9784.4 | 0.40 |  |
| gl_micro | gl_decompose.fill_Gpix_s | 162.684 | 153.981 | 0.95 |  |
| gl_micro | gl_decompose.finish_us | 8.57 | 63.95 | 0.13 |  |
| gl_micro | gl_decompose.mapwrite_GBs | 8.393 | 10.167 | 1.21 | ⚠ FASTER than bare metal — check for early-ended waits |
| gl_micro | gl_decompose.bufsub_GBs | 8.32 | 7.76 | 0.93 |  |
| gl_micro | gl_decompose.texsub_GBs | 9.484 | 9.186 | 0.97 |  |
| gl_micro | gl_decompose.readpix_GBs | 8.874 | 6.296 | 0.71 |  |
| gl_micro | gl_drawrate.drawcall_ns | 46.8 | 152 | 0.31 |  |
| gl_micro | gl_drawrate.drawcall_kcalls_s | 21379.8 | 6577.4 | 0.31 |  |
| gl_micro | gl_drawrate.drawcall_wall_s | 0.009 | 0.03 | 0.30 |  |
| gl_micro | gl_finishrate.finish_us | 9.07 | 63.88 | 0.14 |  |
| gl_micro | gl_finishrate.finish_per_s | 110295 | 15655 | 0.14 |  |
| gl_micro | gl_finishrate.finish_wall_s | 0.181 | 1.278 | 0.14 |  |
| sway_screencap | capture_fps | 62 | 60.5 | 0.98 |  |
| glmark2 | suite_offscreen_score | 41082 | 11123 | 0.27 |  |
| glmark2 | suite_windowed_score | 2378 | 938 | 0.39 |  |
| pv_nvenc_nvdec_fps | nvenc_h264_fps | 198 | 145 | 0.73 |  |
| pv_nvenc_nvdec_fps | nvdec_h264_speed | 17.4 | 10.2 | 0.59 |  |
| geekbench_vulkan | workload_count | 11 | 11 | 1.00 |  |
| blender_opendata | monster (samples/min) | 1069.89 | 856.809 | 0.80 |  |
| blender_opendata | junkshop (samples/min) | 615.914 | 209.145 | 0.34 |  |
| blender_opendata | classroom (samples/min) | 511.695 | 386.974 | 0.76 |  |
| gl_info | gl_ext_count | 397 | 397 | 1.00 |  |
| blender_cycles_cuda | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_cycles_optix | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_eevee | mean | 0.712611 | 0.712611 | 1.00 |  |
| blender_workbench | mean | 0.634848 | 0.634848 | 1.00 |  |
| blender_eevee_vulkan | mean | 0.712611 | 0.712611 | 1.00 |  |
| vk_ofa | good_frac | 0.9859 | 0.9859 | 1.00 |  |
