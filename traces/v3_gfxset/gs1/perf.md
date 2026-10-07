| item | value | bare metal | guest | guest/bare | note |
|---|---|---|---|---|---|
| vk_info | vk_ext_count | 251 | 250 | 1.00 |  |
| vkpeak | fp32-scalar | 15061.6 | 15023.7 | 1.00 |  |
| vkpeak | fp32-vec4 | 14892.3 | 14881.1 | 1.00 |  |
| vkpeak | fp16-scalar | 22407.8 | 22403.2 | 1.00 |  |
| vkpeak | fp16-vec4 | 16478.4 | 16473.7 | 1.00 |  |
| vkpeak | fp16-matrix | 89517.6 | 89491.5 | 1.00 |  |
| vkpeak | fp64-scalar | 350.98 | 350.84 | 1.00 |  |
| vkpeak | fp64-vec4 | 351.97 | 351.46 | 1.00 |  |
| vkpeak | int32-scalar | 11231 | 11224.2 | 1.00 |  |
| vkpeak | int32-vec4 | 11125.1 | 11123.2 | 1.00 |  |
| vkpeak | int16-scalar | 8963.58 | 8950.84 | 1.00 |  |
| vkpeak | int16-vec4 | 9186.88 | 9174.25 | 1.00 |  |
| vkpeak | int8-dotprod | 10724.7 | 10719.4 | 1.00 |  |
| vkpeak | int8-matrix | 177313 | 177273 | 1.00 |  |
| vkpeak | bf16-matrix | 44645.2 | 44634.1 | 1.00 |  |
| egl_offscreen | Mtri_s | 2.6 | 2.6 | 1.00 |  |
| gl_micro | gl_decompose.drawcall_kcalls_s | 24281.5 | 11222.6 | 0.46 |  |
| gl_micro | gl_decompose.fill_Gpix_s | 162.684 | 153.802 | 0.95 |  |
| gl_micro | gl_decompose.finish_us | 8.57 | 59.82 | 0.14 |  |
| gl_micro | gl_decompose.mapwrite_GBs | 8.393 | 10.351 | 1.23 | ⚠ FASTER than bare metal — check for early-ended waits |
| gl_micro | gl_decompose.bufsub_GBs | 8.32 | 7.716 | 0.93 |  |
| gl_micro | gl_decompose.texsub_GBs | 9.484 | 9.587 | 1.01 |  |
| gl_micro | gl_decompose.readpix_GBs | 8.874 | 6.641 | 0.75 |  |
| gl_micro | gl_drawrate.drawcall_ns | 46.8 | 130.8 | 0.36 |  |
| gl_micro | gl_drawrate.drawcall_kcalls_s | 21379.8 | 7643 | 0.36 |  |
| gl_micro | gl_drawrate.drawcall_wall_s | 0.009 | 0.026 | 0.35 |  |
| gl_micro | gl_finishrate.finish_us | 9.07 | 60.84 | 0.15 |  |
| gl_micro | gl_finishrate.finish_per_s | 110295 | 16436 | 0.15 |  |
| gl_micro | gl_finishrate.finish_wall_s | 0.181 | 1.217 | 0.15 |  |
| sway_screencap | capture_fps | 62 | 61.2 | 0.99 |  |
| glmark2 | suite_offscreen_score | 41082 | 11014 | 0.27 |  |
| glmark2 | suite_windowed_score | 2378 | 942 | 0.40 |  |
| pv_nvenc_nvdec_fps | nvenc_h264_fps | 198 | 124 | 0.63 |  |
| pv_nvenc_nvdec_fps | nvdec_h264_speed | 17.4 | 10.1 | 0.58 |  |
| geekbench_vulkan | workload_count | 11 | 11 | 1.00 |  |
| blender_opendata | monster (samples/min) | 1069.89 | 972.331 | 0.91 |  |
| blender_opendata | junkshop (samples/min) | 615.914 | 248.173 | 0.40 |  |
| blender_opendata | classroom (samples/min) | 511.695 | 474.778 | 0.93 |  |
| gl_info | gl_ext_count | 397 | 397 | 1.00 |  |
| blender_cycles_cuda | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_cycles_optix | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_eevee | mean | 0.712611 | 0.712611 | 1.00 |  |
| blender_workbench | mean | 0.634848 | 0.634848 | 1.00 |  |
| blender_eevee_vulkan | mean | 0.712611 | 0.712611 | 1.00 |  |
