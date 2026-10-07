APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=311 kf3_refusals=5
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=7 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1146 kf3_refusals=11
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1124 kf3_refusals=4
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1112 kf3_refusals=4
APPRES side=guest app=matrixMul verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1119 kf3_refusals=4
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1106 kf3_refusals=4
APPRES side=guest app=bandwidthTest verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1130 kf3_refusals=4
APPRES side=guest app=simpleStreams verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1137 kf3_refusals=4
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1131 kf3_refusals=4
APPRES side=guest app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1111 kf3_refusals=2
APPRES side=guest app=simpleCallback verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1119 kf3_refusals=2
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1138 kf3_refusals=2
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1120 kf3_refusals=2
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=concurrentKernels verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1129 kf3_refusals=2
APPRES side=guest app=simpleIPC verdict=PASS rc=0 secs=7 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1864 kf3_refusals=2
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=139 secs=6 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1], CUBLAS_OP_N, t.size, t.size, &one, t.data, t.siz boot=ap_cand2_b1 guest_xid=0 kf3_lines=1204 kf3_refusals=3
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=3 quiet=0 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_cand2_b1 guest_xid=0 kf3_lines=1183 kf3_refusals=2
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=4 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_cand2_b1 guest_xid=0 kf3_lines=1237 kf3_refusals=2
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1130 kf3_refusals=2
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=16 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1136 kf3_refusals=2
APPRES side=guest app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1110 kf3_refusals=2
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1145 kf3_refusals=2
APPRES side=guest app=graphMemoryNodes verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1113 kf3_refusals=2
APPRES side=guest app=simpleCudaGraphs verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1110 kf3_refusals=2
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1165 kf3_refusals=2
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1111 kf3_refusals=2
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=3 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_cand2_b1 guest_xid=0 kf3_lines=1165 kf3_refusals=2
APPRES side=guest app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=3 quiet=1 note=warn:Max absolute error: 0.000000E+00 boot=ap_cand2_b1 guest_xid=0 kf3_lines=1111 kf3_refusals=2
APPRES side=guest app=reduction verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=2947 kf3_refusals=2
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=11 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1116 kf3_refusals=2
APPRES side=guest app=scan verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1111 kf3_refusals=2
APPRES side=guest app=histogram verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1131 kf3_refusals=2
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=3 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_cand2_b1 guest_xid=0 kf3_lines=1111 kf3_refusals=2
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1113 kf3_refusals=2
APPRES side=guest app=transpose verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1129 kf3_refusals=2
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=reduce verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=2
APPRES side=guest app=nbody verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=2
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1103 kf3_refusals=2
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=3 quiet=0 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_cand2_b1 guest_xid=0 kf3_lines=1120 kf3_refusals=2
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=stream_created verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=stream_nonblocking verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=stream_perthread verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=stream_two verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=stream_created2nd verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1102 kf3_refusals=2
APPRES side=guest app=gpu_burn verdict=PASS rc=0 secs=99 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1578 kf3_refusals=7
APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=41 quiet=2 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1308 kf3_refusals=2
APPRES side=guest app=torch_ai_bench verdict=PASS rc=0 secs=49 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1315 kf3_refusals=2
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=26 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1175 kf3_refusals=2
APPRES side=guest app=cupy verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1192 kf3_refusals=2
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=10 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_cand2_b1 guest_xid=0 kf3_lines=1571 kf3_refusals=2
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1714 kf3_refusals=2
APPRES side=guest app=vulkaninfo verdict=PASS rc=0 secs=4 quiet=0 note=warn:error: XDG_RUNTIME_DIR is invalid or not set in the environment. boot=ap_cand2_b1 guest_xid=0 kf3_lines=1345 kf3_refusals=4
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=421 quiet=5 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=21 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=579 kf3_refusals=2
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=6 quiet=2 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1509 kf3_refusals=4
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=155 quiet=1 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1182 kf3_refusals=2
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=10 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1199 kf3_refusals=3
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=11 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1198 kf3_refusals=2
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=10 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=1196 kf3_refusals=2
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=19 quiet=0 note=warn:This means that hashcat cannot use the full parallel power of your device(s). boot=ap_cand2_b1 guest_xid=0 kf3_lines=1862 kf3_refusals=3
APPRES side=guest app=blender_cycles verdict=PASS rc=0 secs=18 quiet=0 note=- boot=ap_cand2_b1 guest_xid=0 kf3_lines=2269 kf3_refusals=4
APPRES side=guest app=geekbench_gpu verdict=PASS rc=0 secs=79 quiet=0 note=warn: unknown error (internal code 35) boot=ap_cand2_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
