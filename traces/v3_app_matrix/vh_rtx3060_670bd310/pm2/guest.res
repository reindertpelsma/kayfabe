APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=0 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=66 kf3_refusals=1
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=889 kf3_refusals=0
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=847 kf3_refusals=0
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=838 kf3_refusals=0
APPRES side=guest app=matrixMul verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=851 kf3_refusals=0
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=829 kf3_refusals=0
APPRES side=guest app=bandwidthTest verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=868 kf3_refusals=0
APPRES side=guest app=simpleStreams verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=880 kf3_refusals=0
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=879 kf3_refusals=0
APPRES side=guest app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=850 kf3_refusals=0
APPRES side=guest app=simpleCallback verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=870 kf3_refusals=0
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=892 kf3_refusals=0
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=865 kf3_refusals=0
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=836 kf3_refusals=0
APPRES side=guest app=concurrentKernels verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=879 kf3_refusals=0
APPRES side=guest app=simpleIPC verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=1604 kf3_refusals=0
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=134 secs=27 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:214 code=719(cudaErrorLaunchFailure) "cudaStreamAttachMemAsync(stream[tid + 1], t.data, 0, cudaMemAttachSingle)"  boot=ap_pm2_b1 guest_xid=0 kf3_lines=993 kf3_refusals=0
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=1 quiet=0 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_pm2_b1 guest_xid=0 kf3_lines=952 kf3_refusals=0
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=2 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_pm2_b1 guest_xid=0 kf3_lines=1036 kf3_refusals=0
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=883 kf3_refusals=0
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=12 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=887 kf3_refusals=0
APPRES side=guest app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=853 kf3_refusals=0
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=920 kf3_refusals=0
APPRES side=guest app=graphMemoryNodes verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=853 kf3_refusals=0
APPRES side=guest app=simpleCudaGraphs verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=854 kf3_refusals=0
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=937 kf3_refusals=0
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=853 kf3_refusals=0
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=1 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_pm2_b1 guest_xid=0 kf3_lines=938 kf3_refusals=0
APPRES side=guest app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=1 quiet=0 note=warn:Max absolute error: 0.000000E+00 boot=ap_pm2_b1 guest_xid=0 kf3_lines=853 kf3_refusals=0
APPRES side=guest app=reduction verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=857 kf3_refusals=0
APPRES side=guest app=scan verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=856 kf3_refusals=0
APPRES side=guest app=histogram verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=884 kf3_refusals=0
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=1 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_pm2_b1 guest_xid=0 kf3_lines=852 kf3_refusals=0
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=856 kf3_refusals=0
APPRES side=guest app=transpose verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=881 kf3_refusals=0
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=reduce verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=nbody verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=839 kf3_refusals=0
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=922 kf3_refusals=0
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=839 kf3_refusals=0
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=839 kf3_refusals=0
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=1 quiet=0 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_pm2_b1 guest_xid=0 kf3_lines=860 kf3_refusals=0
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=842 kf3_refusals=0
APPRES side=guest app=stream_created verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=841 kf3_refusals=0
APPRES side=guest app=stream_nonblocking verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=stream_perthread verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=839 kf3_refusals=0
APPRES side=guest app=stream_two verdict=PASS rc=0 secs=0 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=stream_created2nd verdict=PASS rc=0 secs=0 quiet=0 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=839 kf3_refusals=0
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=4 quiet=0 note=cuInit returned 0 (no error) boot=ap_pm2_b1 guest_xid=0 kf3_lines=533 kf3_refusals=0
APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=53 quiet=1 note=- boot=ap_pm2_b1 guest_xid=0 kf3_lines=1543 kf3_refusals=0
APPRES side=guest app=torch_ai_bench verdict=TIMEOUT rc=137 secs=915 quiet=411 note=CHECK torch_matmul_fp32_TFLOPs FAIL boot=ap_pm2_b1 guest_xid=0 kf3_lines=1237 kf3_refusals=1
APPS_WEDGE boot=ap_pm2_b1 after=torch_ai_bench sanity_vectorAdd=0
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=52 quiet=1 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=976 kf3_refusals=0
APPRES side=guest app=cupy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=993 kf3_refusals=0
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=6 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_pm2_b2 guest_xid=0 kf3_lines=1570 kf3_refusals=0
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=1789 kf3_refusals=0
APPRES side=guest app=vulkaninfo verdict=PASS rc=0 secs=2 quiet=1 note=warn:error: XDG_RUNTIME_DIR is invalid or not set in the environment. boot=ap_pm2_b2 guest_xid=0 kf3_lines=972 kf3_refusals=0
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=404 quiet=2 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=19 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=324 kf3_refusals=0
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=995 kf3_refusals=0
APPRES side=guest app=clpeak verdict=FAIL rc=0 secs=101 quiet=0 note=clpeak: 4 test groups skipped (clFinish (-36)) boot=ap_pm2_b2 guest_xid=0 kf3_lines=923 kf3_refusals=0
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=925 kf3_refusals=0
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=924 kf3_refusals=0
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=928 kf3_refusals=0
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=25 quiet=0 note=warn:This means that hashcat cannot use the full parallel power of your device(s). boot=ap_pm2_b2 guest_xid=0 kf3_lines=1579 kf3_refusals=0
APPRES side=guest app=blender_cycles verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_pm2_b2 guest_xid=0 kf3_lines=1726 kf3_refusals=0
APPRES side=guest app=geekbench_gpu verdict=PASS rc=0 secs=66 quiet=0 note=warn: unknown error (internal code 35) boot=ap_pm2_b2 guest_xid=0 kf3_lines=3000 kf3_refusals=0
