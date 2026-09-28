APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=307 kf3_refusals=5
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1127 kf3_refusals=11
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1097 kf3_refusals=4
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1083 kf3_refusals=4
APPRES side=guest app=matrixMul verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1093 kf3_refusals=4
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1082 kf3_refusals=4
APPRES side=guest app=bandwidthTest verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=4
APPRES side=guest app=simpleStreams verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1112 kf3_refusals=4
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1109 kf3_refusals=4
APPRES side=guest app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1085 kf3_refusals=2
APPRES side=guest app=simpleCallback verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1094 kf3_refusals=2
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1112 kf3_refusals=2
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1094 kf3_refusals=2
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=concurrentKernels verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=2
APPRES side=guest app=simpleIPC verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1818 kf3_refusals=2
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=134 secs=30 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:214 code=719(cudaErrorLaunchFailure) "cudaStreamAttachMemAsync(stream[tid + 1], t.data, 0, cudaMemAttachSingle)"  boot=ap_m20_b1 guest_xid=0 kf3_lines=1190 kf3_refusals=3
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=2 quiet=1 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_m20_b1 guest_xid=0 kf3_lines=1157 kf3_refusals=2
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=3 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_m20_b1 guest_xid=0 kf3_lines=1211 kf3_refusals=2
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1104 kf3_refusals=2
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=14 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1109 kf3_refusals=2
APPRES side=guest app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1085 kf3_refusals=2
APPRES side=guest app=cdpSimpleQuicksort verdict=TIMEOUT rc=124 secs=61 quiet=59 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1208 kf3_refusals=2
APPRES side=guest app=graphMemoryNodes verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1086 kf3_refusals=2
APPRES side=guest app=simpleCudaGraphs verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1085 kf3_refusals=2
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1139 kf3_refusals=2
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1084 kf3_refusals=2
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=2 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_m20_b1 guest_xid=0 kf3_lines=1140 kf3_refusals=2
APPRES side=guest app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=2 quiet=0 note=warn:Max absolute error: 0.000000E+00 boot=ap_m20_b1 guest_xid=0 kf3_lines=1085 kf3_refusals=2
APPRES side=guest app=reduction verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=2921 kf3_refusals=2
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=9 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1088 kf3_refusals=2
APPRES side=guest app=scan verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1086 kf3_refusals=2
APPRES side=guest app=histogram verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=2
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=3 quiet=1 note=warn:Max absolute error: 1.192093E-05 boot=ap_m20_b1 guest_xid=0 kf3_lines=1086 kf3_refusals=2
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=7 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1087 kf3_refusals=2
APPRES side=guest app=transpose verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=2
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=reduce verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=nbody verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1130 kf3_refusals=2
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=1 quiet=0 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_m20_b1 guest_xid=0 kf3_lines=1094 kf3_refusals=2
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_created verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_nonblocking verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_perthread verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_two verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_created2nd verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=gpu_burn verdict=PASS rc=0 secs=96 quiet=0 note=warn:cuInit returned 0 (no error) boot=ap_m20_b1 guest_xid=0 kf3_lines=1547 kf3_refusals=7
APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=55 quiet=2 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1289 kf3_refusals=2
APPRES side=guest app=torch_ai_bench verdict=PASS rc=0 secs=44 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1289 kf3_refusals=2
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=14 quiet=2 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1143 kf3_refusals=2
APPRES side=guest app=cupy verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1167 kf3_refusals=2
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=4 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_m20_b1 guest_xid=0 kf3_lines=1545 kf3_refusals=2
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1689 kf3_refusals=2
APPRES side=guest app=vulkaninfo verdict=PASS rc=0 secs=3 quiet=0 note=warn:error: XDG_RUNTIME_DIR is invalid or not set in the environment. boot=ap_m20_b1 guest_xid=0 kf3_lines=1324 kf3_refusals=4
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=412 quiet=3 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=20 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=570 kf3_refusals=2
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=4 quiet=2 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1475 kf3_refusals=4
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=128 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1148 kf3_refusals=2
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1174 kf3_refusals=3
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1172 kf3_refusals=2
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=1172 kf3_refusals=2
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=27 quiet=0 note=warn:This means that hashcat cannot use the full parallel power of your device(s). boot=ap_m20_b1 guest_xid=0 kf3_lines=1827 kf3_refusals=3
APPRES side=guest app=blender_cycles verdict=PASS rc=0 secs=11 quiet=0 note=- boot=ap_m20_b1 guest_xid=0 kf3_lines=2221 kf3_refusals=4
APPRES side=guest app=geekbench_gpu verdict=PASS rc=0 secs=70 quiet=0 note=warn: unknown error (internal code 35) boot=ap_m20_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
