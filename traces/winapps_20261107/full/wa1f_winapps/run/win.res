APPRES side=win app=cup2_ladder verdict=PASS rc=0 secs=5 quiet=4 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=cup3_ladder verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=deviceQuery_demo verdict=PASS rc=0 secs=5 quiet=4 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=driver_status verdict=PASS rc=0 secs=5 quiet=4 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=dxgi_adapters verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi
APPRES side=win app=opencl_icd verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opencl
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=5 quiet=4 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
APPRES side=win app=vulkaninfo verdict=PASS rc=0 secs=7 quiet=1 note=warn:WARNING: [Loader Message] Code 0 : windows_read_data_files_in_registry: Registry lookup failed to get layer manifest files. boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=cuda_callback verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cuda_occupancy verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=15 quiet=10 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=nbody_demo_gl verdict=FAIL rc=0 secs=30 quiet=1 note=no-gpu-proof boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-demo
APPRES side=win app=atomics_t verdict=PASS rc=0 secs=16 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=bandwidthTest_demo verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=blackscholes_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cg_t verdict=PASS rc=0 secs=18 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=dxprobe_d3d11 verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=dxprobe_d3d12 verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=fft_t verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=fwt_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=graphs_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=hist_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=mandelbrot_t verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=matmul_t verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=memcpy2d_t verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=nbody_demo_bench verdict=PASS rc=0 secs=4 quiet=4 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=nbody_t verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=oceanFFT_demo verdict=PASS rc=0 secs=30 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-oceanfft-readback tier=1 cat=cuda-demo
APPRES side=win app=randomFog_demo verdict=PASS rc=0 secs=32 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-randomfog-readback tier=1 cat=cuda-demo
APPRES side=win app=reduce_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=rng_t verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=scan_t verdict=PASS rc=0 secs=17 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=sort_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=stream_created2nd_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_created_t verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_default_t verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_nonblocking_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_perthread_t verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=stream_two_t verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stream-probe
APPRES side=win app=streams_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=tc16_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=tcbf16_t verdict=PASS rc=0 secs=9 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=transpose_t verdict=PASS rc=0 secs=6 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=triad_t verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=zerocopy_t verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=overlay_yuv_hide verdict=FAIL rc=5 secs=4 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"hide","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DXGI_O boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=overlay_yuv_occlude verdict=FAIL rc=5 secs=4 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"occlude","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DXG boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=overlay_yuv_resize verdict=FAIL rc=5 secs=6 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"resize","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DXGI boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=overlay_yuv_move verdict=FAIL rc=5 secs=4 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"move","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DXGI_O boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=overlay_yuv_recreate verdict=FAIL rc=5 secs=5 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"recreate","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DX boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=edge_webgl verdict=PASS rc=0 secs=22 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=edge_webgl_headless verdict=PASS rc=0 secs=15 quiet=2 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=edge_webgpu verdict=PASS rc=0 secs=21 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=overlay_yuv_fullscreen verdict=FAIL rc=5 secs=4 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"fullscreen","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no  boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=2 cat=d3d
APPRES side=win app=overlay_yuv_probe verdict=FAIL rc=5 secs=4 quiet=1 note=rc:5,success-string-missing | KFOVL JSON {"program":"kf_overlayprobe","version":1,"scenario":"steady","verdict":"FAIL","result":"SETUP","code":5,"reason":"CheckOverlaySupport reports no DXGI boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=cupy_asynccopy verdict=PASS rc=0 secs=17 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_cg verdict=PASS rc=0 secs=11 quiet=5 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_sha256 verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=d3d12_signal_probe verdict=PASS rc=0 secs=9 quiet=0 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=dxdiag_report verdict=PASS rc=0 secs=44 quiet=1 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=managed_t verdict=TIMEOUT rc=124 secs=181 quiet=175 note=timeout boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=cuda-equiv
APPRES side=win app=torch_burn verdict=PASS rc=0 secs=78 quiet=2 note=warn:[burn] t:10s iters:3116 errors:0 boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stress
APPRES side=win app=ort_directml verdict=PASS rc=0 secs=11 quiet=1 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=cupy_check verdict=PASS rc=0 secs=9 quiet=0 note=- boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=torch
APPRES side=win app=hashcat verdict=PASS rc=0 secs=30 quiet=1 note=warn:Watchdog: Temperature abort trigger set to 90c boot=g5 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=crypto
