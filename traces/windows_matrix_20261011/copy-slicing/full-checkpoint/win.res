APPRES side=win app=cup3_ladder verdict=PASS rc=0 secs=4 quiet=3 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=deviceQuery_demo verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=dxgi_adapters verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=3 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi
APPRES side=win app=opencl_icd verdict=PASS rc=0 secs=3 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opencl
APPRES side=win app=cup2_ladder verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=driver_status verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=vectorAdd_demo verdict=FAIL rc=0 secs=3 quiet=2 note=no-gpu-proof boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy,smi tier=1 cat=cuda-demo
APPRES side=win app=vulkaninfo verdict=PASS rc=0 secs=6 quiet=1 note=warn:WARNING: [Loader Message] Code 0 : windows_read_data_files_in_registry: Registry lookup failed to get layer manifest files. boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=hist_t verdict=PASS rc=0 secs=113 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cg_t verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=fft_t verdict=PASS rc=0 secs=3 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=fwt_t verdict=PASS rc=0 secs=3 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=graphs_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=matmul_t verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=nbody_t verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=rng_t verdict=PASS rc=0 secs=75 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=scan_t verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=sort_t verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=stream_created2nd_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_created_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_default_t verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_nonblocking_t verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_perthread_t verdict=FAIL rc=1 secs=45 quiet=41 note=rc:1,success-string-missing | RuntimeError: Failed to find CUDA headers. Please install CUDA toolkit headers (e.g., pip install cupy-cuda12x[ctk]) or specify CUDA_PATH environment variable. boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_two_t verdict=PASS rc=0 secs=70 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=streams_t verdict=PASS rc=0 secs=11 quiet=0 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=tc16_t verdict=PASS rc=0 secs=7 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=tcbf16_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=transpose_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cuda_callback verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cuda_occupancy verdict=PASS rc=0 secs=3 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=dxprobe_d3d11 verdict=PASS rc=0 secs=3 quiet=0 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=dxprobe_d3d12 verdict=PASS rc=0 secs=4 quiet=3 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=furmark_glinfo verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=furmark_vkinfo verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=mandelbrot_t verdict=PASS rc=0 secs=63 quiet=0 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=memcpy2d_t verdict=PASS rc=0 secs=15 quiet=1 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=reduce_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=triad_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=zerocopy_t verdict=PASS rc=0 secs=5 quiet=2 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cup8_ladder verdict=FAIL rc=0 secs=12 quiet=1 note=no-gpu-proof boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy,smi tier=1 cat=cuda-ladder
APPRES side=win app=cupy_sha256 verdict=FAIL rc=1 secs=142 quiet=0 note=rc:1,success-string-missing | RuntimeError: Failed to find CUDA headers. Please install CUDA toolkit headers (e.g., pip install cupy-cuda12x[ctk]) or specify CUDA_PATH environment variable. boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=d3d12_signal_probe verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=cupy_cg verdict=FAIL rc=1 secs=5 quiet=1 note=rc:1,success-string-missing | RuntimeError: Failed to find CUDA headers. Please install CUDA toolkit headers (e.g., pip install cupy-cuda12x[ctk]) or specify CUDA_PATH environment variable. boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=bandwidthTest_demo verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=14 quiet=1 note=- boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=managed_t verdict=TIMEOUT rc=124 secs=181 quiet=178 note=timeout boot=g6 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=cuda-equiv
APPRES side=win app=nbody_demo_bench verdict=PASS rc=0 secs=4 quiet=3 note=- boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=atomics_t verdict=PASS rc=0 secs=70 quiet=2 note=- boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=blackscholes_t verdict=PASS rc=0 secs=10 quiet=1 note=- boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=nvenc_av1 verdict=FAIL rc=-542398533 secs=12 quiet=0 note=rc:-542398533,success-string-missing,no-gpu-proof | [av1_nvenc @ 000002256d1addc0] OpenEncodeSessionEx failed: incompatible client key (21): (no details) boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:VideoEncode,smi tier=1 cat=video
APPRES side=win app=nvenc_h264 verdict=FAIL rc=-542398533 secs=3 quiet=3 note=rc:-542398533,success-string-missing,no-gpu-proof | [h264_nvenc @ 000001e27c52ac00] OpenEncodeSessionEx failed: incompatible client key (21): (no details) boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:VideoEncode,smi tier=1 cat=video
APPRES side=win app=nvenc_hevc verdict=FAIL rc=-542398533 secs=3 quiet=2 note=rc:-542398533,success-string-missing,no-gpu-proof | [hevc_nvenc @ 0000015c1bb8a980] OpenEncodeSessionEx failed: incompatible client key (21): (no details) boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:VideoEncode,smi tier=1 cat=video
APPRES side=win app=ort_directml verdict=PASS rc=0 secs=17 quiet=12 note=- boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=gputest_triangle verdict=PASS rc=0 secs=16 quiet=16 note=- boot=g7 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=cupy_asynccopy verdict=FAIL rc=1 secs=140 quiet=0 note=rc:1,success-string-missing | RuntimeError: Failed to find CUDA headers. Please install CUDA toolkit headers (e.g., pip install cupy-cuda12x[ctk]) or specify CUDA_PATH environment variable. boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=gputest_fur verdict=PASS rc=0 secs=18 quiet=0 note=- boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=gputest_gi verdict=PASS rc=0 secs=17 quiet=17 note=- boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=opengl
APPRES side=win app=gputest_pixmark_piano verdict=PASS rc=0 secs=17 quiet=17 note=- boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=opengl
APPRES side=win app=gputest_plot3d verdict=PASS rc=0 secs=17 quiet=17 note=- boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=opengl
APPRES side=win app=gputest_tess verdict=PASS rc=0 secs=17 quiet=17 note=- boot=g8 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=opengl
APPRES side=win app=nvdec_h264 verdict=PASS rc=0 secs=44 quiet=2 note=- boot=g8 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=furmark_gl_bench verdict=TIMEOUT rc=124 secs=217 quiet=1 note=KFWAIT furmark_gl_bench STILL_RUNNING_AFTER 105 s boot=g9 guest_tdr=3 wer=0 gsp_cycles=1 proof=- tier=1 cat=opengl
APPRES side=win app=furmark_knot_gl verdict=PASS rc=0 secs=25 quiet=25 note=- boot=g10 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=furmark_vk_bench verdict=PASS rc=0 secs=28 quiet=28 note=- boot=g10 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=oceanFFT_demo verdict=FAIL rc=0 secs=28 quiet=0 note=no-gpu-proof boot=g10 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-demo
APPRES side=win app=randomFog_demo verdict=FAIL rc=0 secs=28 quiet=27 note=no-gpu-proof boot=g10 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-demo
APPRES side=win app=cupy_check verdict=FAIL rc=1 secs=126 quiet=66 note=rc:1,success-string-missing | RuntimeError: Failed to find CUDA headers. Please install CUDA toolkit headers (e.g., pip install cupy-cuda12x[ctk]) or specify CUDA_PATH environment variable. boot=g10 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=torch
APPRES side=win app=d3d11va_h264 verdict=PASS rc=0 secs=45 quiet=21 note=- boot=g10 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=edge_webgl verdict=PASS rc=0 secs=20 quiet=2 note=- boot=g11 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=edge_webgl_headless verdict=PASS rc=0 secs=13 quiet=1 note=- boot=g11 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=edge_webgpu verdict=PASS rc=0 secs=18 quiet=0 note=- boot=g11 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=nbody_demo_gl verdict=FAIL rc=0 secs=28 quiet=1 note=no-gpu-proof boot=g11 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-demo
APPRES side=win app=scale_cuda_nvenc verdict=FAIL rc=-22 secs=41 quiet=0 note=rc:-22,no-gpu-proof | [h264_nvenc @ 00000190862922c0] OpenEncodeSessionEx failed: incompatible client key (21): (no details) boot=g11 guest_tdr=3 wer=0 gsp_cycles=1 proof=none:pdh:VideoEncode|VideoDecode,smi tier=2 cat=video
APPRES side=win app=torch_correct verdict=PASS rc=0 secs=102 quiet=2 note=- boot=g12 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=torch
APPDIG side=win app=torch_correct DIGEST cnn_train_step 763c693a5a53f948
APPRES side=win app=hashcat verdict=PASS rc=0 secs=33 quiet=1 note=warn:Watchdog: Temperature abort trigger set to 90c boot=g12 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=crypto
APPRES side=win app=vulkan_video_decode verdict=PASS rc=0 secs=62 quiet=23 note=- boot=g12 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=3 cat=video
APPRES side=win app=edge_video_h264 verdict=PASS rc=0 secs=29 quiet=1 note=- boot=g13 guest_tdr=0 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=edge_video_vp9 verdict=PASS rc=0 secs=50 quiet=3 note=- boot=g14 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=2 cat=video
APPRES side=win app=heaven_d3d11 verdict=FAIL rc=0 secs=47 quiet=47 note=no-gpu-proof boot=g15 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=d3d
APPRES side=win app=heaven_opengl verdict=FAIL rc=-1073740791 secs=44 quiet=2 note=rc:-1073740791,success-string-missing,no-gpu-proof | KFSURVIVE heaven_opengl EXITED_EARLY rc:-1073740791 after:29 s boot=g15 guest_tdr=3 wer=0 gsp_cycles=1 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=opengl
