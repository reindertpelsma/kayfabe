GSET_RES side=guest item=pv_validate_vk verdict=PASS rc=0 secs=2 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=85 kf3_lines=1024
GSET_RES side=guest item=pv_validate_gl verdict=PASS rc=0 secs=1 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=308
GSET_RES side=guest item=pv_vk_rt_ext verdict=FAIL rc=0 secs=0 note=GSET_FAIL rc=2  boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=34 kf3_lines=333
GSET_RES side=guest item=pv_vk_create_device verdict=PASS rc=0 secs=1 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=78 kf3_lines=910
GSET_RES side=guest item=vk_info verdict=PASS rc=0 secs=2 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=156 kf3_lines=1818
GSET_RES side=guest item=vkpeak verdict=PASS rc=0 secs=464 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=572 kf3_lines=5000
GSET_RES side=guest item=egl_offscreen verdict=PASS rc=0 secs=16 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=24 kf3_lines=321
GSET_RES side=guest item=gl_micro verdict=PASS rc=0 secs=3 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=68 kf3_lines=927
GSET_RES side=guest item=pv_fbo_formats verdict=PASS rc=0 secs=1 note=warn:FBO/ GL_RGBA/GL_UNSIGNED_BYTE texture status=0x8CD5 GL_FRAMEBUFFER_COMPLETE glGetError alloc=0x0 attach=0x0 after=0x0 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=308
GSET_RES side=guest item=pv_egl_dmabuf_export verdict=PASS rc=0 secs=0 note=warn: glGetError=0x0000 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=308
GSET_RES side=guest item=pv_dmabuf_import verdict=PASS rc=0 secs=1 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=88 kf3_lines=1222
GSET_RES side=guest item=pv_xiso_sharing verdict=PASS rc=0 secs=2 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=45 kf3_lines=646
GSET_RES side=guest item=pv_signal_restart_export verdict=PASS rc=0 secs=3 note=warn:PASS signal_restart_export 300/300 dma-buf exports succeeded under 215 SIGALRMs (0 failed) boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=427
GSET_RES side=guest item=pv_gbm_egl_import verdict=PASS rc=0 secs=1 note=warn:[renderD128] FAIL eglCreateImageKHR(NATIVE_PIXMAP from gbm_bo) egl_error=0x300c <-- this is the exact glamor call boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=44 kf3_lines=618
GSET_RES side=guest item=pv_gbmprobe_gbmshot verdict=PASS rc=0 secs=1 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=44 kf3_lines=614
GSET_RES side=guest item=weston_headless verdict=PASS rc=0 secs=14 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=118 kf3_lines=1642
GSET_RES side=guest item=weston_client_diff verdict=PASS rc=0 secs=20 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=73 kf3_lines=856
GSET_RES side=guest item=sway_screencap verdict=PASS rc=0 secs=17 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=80 kf3_lines=1084
GSET_RES side=guest item=glmark2 verdict=PASS rc=0 secs=673 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=371 kf3_lines=2408
GSET_RES side=guest item=pv_nvenc_nvdec_fps verdict=PASS rc=0 secs=10 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=71 kf3_lines=1923
GSET_RES side=guest item=video_nvenc_nvdec verdict=PASS rc=0 secs=29 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=182 kf3_lines=5000
GSET_RES side=guest item=geekbench_vulkan verdict=PASS rc=0 secs=239 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=776 kf3_lines=5000
GSET_RES side=guest item=blender_opendata verdict=PASS rc=0 secs=185 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=116 kf3_lines=5000
GSET_RES side=guest item=vk_compute verdict=PASS rc=0 secs=2 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=86 kf3_lines=931
GSET_RES side=guest item=vk_render verdict=PASS rc=0 secs=1 note=warn:VKR_REF_FAILS=0 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=88 kf3_lines=933
GSET_RES side=guest item=egl_render verdict=PASS rc=0 secs=1 note=warn:EGLR_REF_FAILS=0 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=309
GSET_RES side=guest item=gl_info verdict=PASS rc=0 secs=0 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=306
GSET_RES side=guest item=glx_vgl verdict=PASS rc=0 secs=5 note=warn:GLXR_REF_FAILS=0 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=112 kf3_lines=1584
GSET_RES side=guest item=ff_cuda verdict=PASS rc=0 secs=22 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=39 kf3_lines=5000
GSET_RES side=guest item=ff_vulkan verdict=FAIL rc=0 secs=25 note=GSET_FAIL scale_vulkan rc=187 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=387 kf3_lines=5000
GSET_RES side=guest item=ff_opencl verdict=PASS rc=0 secs=11 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=138 kf3_lines=5000
GSET_RES side=guest item=ff_placebo verdict=FAIL rc=0 secs=5 note=GSET_FAIL libplacebo_scale rc=187 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=385 kf3_lines=5000
GSET_RES side=guest item=blender_cycles_cuda verdict=PASS rc=0 secs=3 note=warn:BLENDER_OK CYCLES CUDA gpu=NVIDIA_GeForce_RTX_3070 backend=none(SystemError) w=640 h=360 mean=0.741335 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=862
GSET_RES side=guest item=blender_cycles_optix verdict=PASS rc=0 secs=4 note=warn:BLENDER_OK CYCLES OPTIX gpu=NVIDIA_GeForce_RTX_3070 backend=none(SystemError) w=640 h=360 mean=0.741335 boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=22 kf3_lines=873
GSET_RES side=guest item=blender_eevee verdict=PASS rc=0 secs=45 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=83 kf3_lines=1038
GSET_RES side=guest item=blender_workbench verdict=PASS rc=0 secs=3 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=73 kf3_lines=972
GSET_RES side=guest item=blender_eevee_vulkan verdict=PASS rc=0 secs=11 note=- boot=gs_gs1_b1 host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 kf3_unserviced=129 kf3_lines=1296
