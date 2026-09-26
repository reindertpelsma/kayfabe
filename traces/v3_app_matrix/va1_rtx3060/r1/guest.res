APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=307 kf3_refusals=7
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b2 guest_xid=0 kf3_lines=1123 kf3_refusals=8
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_r1_b3 guest_xid=0 kf3_lines=1094 kf3_refusals=7
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_r1_b4 guest_xid=0 kf3_lines=1087 kf3_refusals=7
APPRES side=guest app=matrixMul verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b5 guest_xid=0 kf3_lines=1112 kf3_refusals=7
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b6 guest_xid=0 kf3_lines=1088 kf3_refusals=8
APPRES side=guest app=bandwidthTest verdict=FAIL rc=1 secs=8 quiet=0 note=CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()"  boot=ap_r1_b7 guest_xid=0 kf3_lines=1128 kf3_refusals=9
APPRES side=guest app=simpleStreams verdict=FAIL rc=1 secs=7 quiet=1 note=CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_r1_b8 guest_xid=0 kf3_lines=1141 kf3_refusals=8
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b9 guest_xid=0 kf3_lines=1137 kf3_refusals=8
APPRES side=guest app=simpleAtomicIntrinsics verdict=FAIL rc=1 secs=5 quiet=0 note=CUDA error at simpleAtomicIntrinsics.cu:122 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b10 guest_xid=0 kf3_lines=1113 kf3_refusals=8
APPRES side=guest app=simpleCallback verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status"  boot=ap_r1_b11 guest_xid=0 kf3_lines=1139 kf3_refusals=8
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b12 guest_xid=0 kf3_lines=1151 kf3_refusals=8
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_r1_b13 guest_xid=0 kf3_lines=1122 kf3_refusals=7
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b14 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=concurrentKernels verdict=FAIL rc=1 secs=6 quiet=0 note=CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_r1_b15 guest_xid=0 kf3_lines=1153 kf3_refusals=8
APPRES side=guest app=simpleIPC verdict=FAIL rc=1 secs=10 quiet=1 note=CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs[id], DATA_SIZE, cudaMemcpyDeviceToHost, stream)"  boot=ap_r1_b16 guest_xid=0 kf3_lines=1866 kf3_refusals=9
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=134 secs=34 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1], CUBLAS_OP_N, t.size, t.size, &one, t.data, t.siz boot=ap_r1_b17 guest_xid=0 kf3_lines=1241 kf3_refusals=22
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=6 quiet=1 note=Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_r1_b18 guest_xid=0 kf3_lines=1141 kf3_refusals=8
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=11 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_r1_b19 guest_xid=0 kf3_lines=1284 kf3_refusals=11
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_r1_b20 guest_xid=0 kf3_lines=1137 kf3_refusals=8
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=19 quiet=1 note=- boot=ap_r1_b21 guest_xid=0 kf3_lines=1144 kf3_refusals=15
APPRES side=guest app=globalToShmemAsyncCopy verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b22 guest_xid=0 kf3_lines=1113 kf3_refusals=8
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=10 quiet=2 note=- boot=ap_r1_b23 guest_xid=0 kf3_lines=1174 kf3_refusals=9
APPRES side=guest app=graphMemoryNodes verdict=FAIL rc=1 secs=8 quiet=1 note=CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_square, hostArrays->bytes, cudaMemcpyDeviceToHost, boot=ap_r1_b24 guest_xid=0 kf3_lines=1113 kf3_refusals=8
APPRES side=guest app=simpleCudaGraphs verdict=FAIL rc=1 secs=8 quiet=1 note=CUDA error at simpleCudaGraphs.cu:273 code=719(cudaErrorLaunchFailure) "cudaGraphLaunch(graphExec, streamForGraph)"  boot=ap_r1_b25 guest_xid=0 kf3_lines=1114 kf3_refusals=9
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=11 quiet=1 note=- boot=ap_r1_b26 guest_xid=0 kf3_lines=1196 kf3_refusals=11
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b27 guest_xid=0 kf3_lines=1109 kf3_refusals=8
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=7 quiet=1 note=warn:Test Summary: Error amount = 0.000000 boot=ap_r1_b28 guest_xid=0 kf3_lines=1193 kf3_refusals=8
APPRES side=guest app=MersenneTwisterGP11213 verdict=FAIL rc=1 secs=6 quiet=0 note=CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b29 guest_xid=0 kf3_lines=1113 kf3_refusals=8
APPRES side=guest app=reduction verdict=PASS rc=0 secs=8 quiet=1 note=- boot=ap_r1_b30 guest_xid=0 kf3_lines=3000 kf3_refusals=1
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=14 quiet=1 note=- boot=ap_r1_b31 guest_xid=0 kf3_lines=1112 kf3_refusals=11
APPRES side=guest app=scan verdict=PASS rc=0 secs=9 quiet=1 note=- boot=ap_r1_b32 guest_xid=0 kf3_lines=1110 kf3_refusals=9
APPRES side=guest app=histogram verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_r1_b33 guest_xid=0 kf3_lines=1138 kf3_refusals=9
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=9 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_r1_b34 guest_xid=0 kf3_lines=1111 kf3_refusals=10
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=11 quiet=0 note=- boot=ap_r1_b35 guest_xid=0 kf3_lines=1111 kf3_refusals=10
APPRES side=guest app=transpose verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_r1_b36 guest_xid=0 kf3_lines=1137 kf3_refusals=8
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=10 quiet=0 note=- boot=ap_r1_b37 guest_xid=0 kf3_lines=1097 kf3_refusals=10
APPRES side=guest app=reduce verdict=PASS rc=0 secs=15 quiet=1 note=- boot=ap_r1_b38 guest_xid=0 kf3_lines=1099 kf3_refusals=12
APPRES side=guest app=nbody verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b39 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b40 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b41 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=10 quiet=1 note=- boot=ap_r1_b42 guest_xid=0 kf3_lines=1097 kf3_refusals=10
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=13 quiet=1 note=- boot=ap_r1_b43 guest_xid=0 kf3_lines=1182 kf3_refusals=11
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=9 quiet=1 note=- boot=ap_r1_b44 guest_xid=0 kf3_lines=1096 kf3_refusals=9
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=7 quiet=1 note=- boot=ap_r1_b45 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b46 guest_xid=0 kf3_lines=1095 kf3_refusals=8
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=8 quiet=2 note=FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) boot=ap_r1_b47 guest_xid=0 kf3_lines=1106 kf3_refusals=9
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=75 quiet=0 note=cuInit returned 0 (no error) boot=ap_r1_b48 guest_xid=0 kf3_lines=1442 kf3_refusals=47
APPRES side=guest app=torch_correct verdict=FAIL rc=1 secs=27 quiet=1 note=RuntimeError: GET was unable to find an engine to execute this computation boot=ap_r1_b49 guest_xid=0 kf3_lines=1312 kf3_refusals=18
APPRES side=guest app=torch_ai_bench verdict=FAIL rc=1 secs=23 quiet=2 note=RuntimeError: CUDA error: unspecified launch failure boot=ap_r1_b50 guest_xid=0 kf3_lines=1205 kf3_refusals=17
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=56 quiet=2 note=- boot=ap_r1_b51 guest_xid=0 kf3_lines=1211 kf3_refusals=33
APPRES side=guest app=cupy verdict=FAIL rc=1 secs=13 quiet=1 note=cupy_backends.cuda.libs.nvrtc.NVRTCError: NVRTC_ERROR_COMPILATION (6) boot=ap_r1_b52 guest_xid=0 kf3_lines=1210 kf3_refusals=11
APPRES side=guest app=llama_cpp_gen verdict=TIMEOUT rc=124 secs=601 quiet=601 note=- boot=ap_r1_b53 guest_xid=0 kf3_lines=1071 kf3_refusals=307
APPRES side=guest app=llama_bench verdict=FAIL rc=134 secs=51 quiet=0 note=/workspace/apps/srcs/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu:109: CUDA error boot=ap_r1_b54 guest_xid=0 kf3_lines=1850 kf3_refusals=31
APPRES side=guest app=vulkaninfo verdict=TIMEOUT rc=124 secs=66 quiet=66 note=- boot=ap_r1_b55 guest_xid=0 kf3_lines=2166 kf3_refusals=65
APPRES side=guest app=vkpeak verdict=FAIL rc=255 secs=66 quiet=0 note=- boot=ap_r1_b56 guest_xid=0 kf3_lines=2167 kf3_refusals=66
APPRES side=guest app=egl_offscreen verdict=FAIL rc=0 secs=3 quiet=1 note=CHECK egl_gl_Mtri_s FAIL boot=ap_r1_b57 guest_xid=0 kf3_lines=281 kf3_refusals=7
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=10 quiet=4 note=- boot=ap_r1_b58 guest_xid=0 kf3_lines=1492 kf3_refusals=14
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=99 quiet=0 note=- boot=ap_r1_b59 guest_xid=0 kf3_lines=1139 kf3_refusals=55
APPRES side=guest app=nvenc_h264 verdict=FAIL rc=0 secs=8 quiet=0 note=- boot=ap_r1_b60 guest_xid=0 kf3_lines=1091 kf3_refusals=9
APPRES side=guest app=nvenc_hevc verdict=FAIL rc=0 secs=8 quiet=0 note=- boot=ap_r1_b61 guest_xid=0 kf3_lines=1091 kf3_refusals=4
APPRES side=guest app=nvdec_h264 verdict=FAIL rc=3 secs=13 quiet=0 note=- boot=ap_r1_b62 guest_xid=0 kf3_lines=1094 kf3_refusals=4
APPRES side=guest app=hashcat verdict=FAIL rc=0 secs=28 quiet=1 note=Watchdog: Temperature abort trigger set to 90c boot=ap_r1_b63 guest_xid=0 kf3_lines=1831 kf3_refusals=4
APPRES side=guest app=blender_cycles verdict=TIMEOUT rc=124 secs=900 quiet=884 note=- boot=ap_r1_b64 guest_xid=0 kf3_lines=2176 kf3_refusals=11
APPRES side=guest app=geekbench_gpu verdict=FAIL rc=0 secs=21 quiet=0 note=[0926/022702:ERROR:optimizer.cpp(122)] build_patches_padding: optimization failed for size { 32, 1, 1, }: Waiting for OpenCL kernel dispatch (build_patches_padd boot=ap_r1_b65 guest_xid=0 kf3_lines=1741 kf3_refusals=4
