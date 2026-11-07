APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=341 kf3_refusals=18
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1236 kf3_refusals=29
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1187 kf3_refusals=22
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1174 kf3_refusals=20
APPRES side=guest app=matrixMul verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1181 kf3_refusals=19
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1164 kf3_refusals=19
APPRES side=guest app=bandwidthTest verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1192 kf3_refusals=19
APPRES side=guest app=simpleStreams verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1205 kf3_refusals=19
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1203 kf3_refusals=19
APPRES side=guest app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1172 kf3_refusals=17
APPRES side=guest app=simpleCallback verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1183 kf3_refusals=17
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1208 kf3_refusals=17
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1184 kf3_refusals=17
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1160 kf3_refusals=17
APPRES side=guest app=concurrentKernels verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1195 kf3_refusals=17
APPRES side=guest app=simpleIPC verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1954 kf3_refusals=17
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=134 secs=23 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1], CUBLAS_OP_N, t.size, t.size, &one, t.data, t.siz boot=ap_r1_b1 guest_xid=0 kf3_lines=1297 kf3_refusals=18
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=2 quiet=1 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_r1_b1 guest_xid=0 kf3_lines=1281 kf3_refusals=17
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=2 quiet=0 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_r1_b1 guest_xid=0 kf3_lines=1330 kf3_refusals=14
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1192 kf3_refusals=14
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=12 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1198 kf3_refusals=14
APPRES side=guest app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1167 kf3_refusals=14
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1215 kf3_refusals=14
APPRES side=guest app=graphMemoryNodes verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=simpleCudaGraphs verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1241 kf3_refusals=14
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=2 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_r1_b1 guest_xid=0 kf3_lines=1239 kf3_refusals=14
APPRES side=guest app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=1 quiet=0 note=warn:Max absolute error: 0.000000E+00 boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=reduction verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=12
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1169 kf3_refusals=14
APPRES side=guest app=scan verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=histogram verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1192 kf3_refusals=14
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=2 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_r1_b1 guest_xid=0 kf3_lines=1168 kf3_refusals=14
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1169 kf3_refusals=14
APPRES side=guest app=transpose verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1192 kf3_refusals=14
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=reduce verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=nbody verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1155 kf3_refusals=14
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1157 kf3_refusals=14
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1155 kf3_refusals=14
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1228 kf3_refusals=14
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1155 kf3_refusals=14
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=1 quiet=0 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_r1_b1 guest_xid=0 kf3_lines=1174 kf3_refusals=14
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=stream_created verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=stream_nonblocking verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1155 kf3_refusals=14
APPRES side=guest app=stream_perthread verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=stream_two verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1156 kf3_refusals=14
APPRES side=guest app=stream_created2nd verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1157 kf3_refusals=14
APPRES side=guest app=gpu_burn verdict=PASS rc=0 secs=94 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1770 kf3_refusals=23
APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=53 quiet=2 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1427 kf3_refusals=14
APPRES side=guest app=torch_ai_bench verdict=PASS rc=0 secs=21 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1415 kf3_refusals=14
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=15 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1241 kf3_refusals=14
APPRES side=guest app=cupy verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1277 kf3_refusals=14
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=4 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_r1_b1 guest_xid=0 kf3_lines=1778 kf3_refusals=14
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1973 kf3_refusals=14
APPRES side=guest app=vulkaninfo verdict=PASS rc=0 secs=2 quiet=0 note=warn:error: XDG_RUNTIME_DIR is invalid or not set in the environment. boot=ap_r1_b1 guest_xid=0 kf3_lines=1368 kf3_refusals=8
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=451 quiet=3 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=4
APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=583 kf3_refusals=6
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1591 kf3_refusals=28
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=120 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1227 kf3_refusals=14
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1309 kf3_refusals=15
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1284 kf3_refusals=14
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=1263 kf3_refusals=14
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=22 quiet=0 note=warn:This means that hashcat cannot use the full parallel power of your device(s). boot=ap_r1_b1 guest_xid=0 kf3_lines=1922 kf3_refusals=17
APPRES side=guest app=blender_cycles verdict=PASS rc=0 secs=10 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=2363 kf3_refusals=28
