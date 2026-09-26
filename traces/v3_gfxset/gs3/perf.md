| item | value | bare metal | guest | guest/bare | note |
|---|---|---|---|---|---|
| vk_info | vk_ext_count | 251 | 251 | 1.00 |  |
| vkpeak | fp32-scalar | 15061.6 | 15044.9 | 1.00 |  |
| vkpeak | fp32-vec4 | 14892.3 | 14886.8 | 1.00 |  |
| vkpeak | fp16-scalar | 22407.8 | 22350.4 | 1.00 |  |
| vkpeak | fp16-vec4 | 16478.4 | 16436.8 | 1.00 |  |
| vkpeak | fp16-matrix | 89517.6 | 89011.7 | 0.99 |  |
| vkpeak | fp64-scalar | 350.98 | 350.87 | 1.00 |  |
| vkpeak | fp64-vec4 | 351.97 | 351.9 | 1.00 |  |
| vkpeak | int32-scalar | 11231 | 11228.2 | 1.00 |  |
| vkpeak | int32-vec4 | 11125.1 | 11123.2 | 1.00 |  |
| vkpeak | int16-scalar | 8963.58 | 8954.33 | 1.00 |  |
| vkpeak | int16-vec4 | 9186.88 | 9147.64 | 1.00 |  |
| vkpeak | int8-dotprod | 10724.7 | 10720.2 | 1.00 |  |
| vkpeak | int8-matrix | 177313 | 175813 | 0.99 |  |
| vkpeak | bf16-matrix | 44645.2 | 44471.8 | 1.00 |  |
| egl_offscreen | Mtri_s | 2.6 | 2.6 | 1.00 |  |
| gl_micro | gl_decompose.drawcall_kcalls_s | 24281.5 | 10144.7 | 0.42 |  |
| gl_micro | gl_decompose.fill_Gpix_s | 162.684 | 153.975 | 0.95 |  |
| gl_micro | gl_decompose.finish_us | 8.57 | 61.05 | 0.14 |  |
| gl_micro | gl_decompose.mapwrite_GBs | 8.393 | 9.983 | 1.19 | ⚠ FASTER than bare metal — check for early-ended waits |
| gl_micro | gl_decompose.bufsub_GBs | 8.32 | 7.589 | 0.91 |  |
| gl_micro | gl_decompose.texsub_GBs | 9.484 | 8.788 | 0.93 |  |
| gl_micro | gl_decompose.readpix_GBs | 8.874 | 6.103 | 0.69 |  |
| gl_micro | gl_drawrate.drawcall_ns | 46.8 | 222.9 | 0.21 |  |
| gl_micro | gl_drawrate.drawcall_kcalls_s | 21379.8 | 4485.9 | 0.21 |  |
| gl_micro | gl_drawrate.drawcall_wall_s | 0.009 | 0.045 | 0.20 |  |
| gl_micro | gl_finishrate.finish_us | 9.07 | 62.08 | 0.15 |  |
| gl_micro | gl_finishrate.finish_per_s | 110295 | 16107 | 0.15 |  |
| gl_micro | gl_finishrate.finish_wall_s | 0.181 | 1.242 | 0.15 |  |
| sway_screencap | capture_fps | 62 | 61.2 | 0.99 |  |
| glmark2 | suite_offscreen_score | 41082 | 10720 | 0.26 |  |
| glmark2 | suite_windowed_score | 2378 | 937 | 0.39 |  |
| pv_nvenc_nvdec_fps | nvenc_h264_fps | 198 | 142 | 0.72 |  |
| pv_nvenc_nvdec_fps | nvdec_h264_speed | 17.4 | 10.1 | 0.58 |  |
| geekbench_vulkan | workload_count | 11 | 11 | 1.00 |  |
| blender_opendata | monster (samples/min) | 1069.89 | 972.053 | 0.91 |  |
| blender_opendata | junkshop (samples/min) | 615.914 | 251.899 | 0.41 |  |
| blender_opendata | classroom (samples/min) | 511.695 | 473.969 | 0.93 |  |
| gl_info | gl_ext_count | 397 | 397 | 1.00 |  |
| blender_cycles_cuda | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_cycles_optix | mean | 0.741335 | 0.741335 | 1.00 |  |
| blender_eevee | mean | 0.712611 | 0.712611 | 1.00 |  |
| blender_workbench | mean | 0.634848 | 0.634848 | 1.00 |  |
| blender_eevee_vulkan | mean | 0.712611 | 0.712611 | 1.00 |  |
| vk_ofa | good_frac | 0.9859 | 0.9859 | 1.00 |  |
