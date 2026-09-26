APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_r1_b1 guest_xid=0 kf3_lines=315 kf3_refusals=7
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b2 guest_xid=0 kf3_lines=1131 kf3_refusals=8
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=7 quiet=2 note=- boot=ap_r1_b3 guest_xid=0 kf3_lines=1107 kf3_refusals=12
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=8 quiet=1 note=- boot=ap_r1_b4 guest_xid=0 kf3_lines=1101 kf3_refusals=13
APPRES side=guest app=matrixMul verdict=FAIL rc=1 secs=4 quiet=0 note=CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b5 guest_xid=0 kf3_lines=1121 kf3_refusals=8
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b6 guest_xid=0 kf3_lines=1095 kf3_refusals=7
APPRES side=guest app=bandwidthTest verdict=FAIL rc=1 secs=7 quiet=1 note=CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()"  boot=ap_r1_b7 guest_xid=0 kf3_lines=1136 kf3_refusals=9
APPRES side=guest app=simpleStreams verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_r1_b8 guest_xid=0 kf3_lines=1149 kf3_refusals=8
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_r1_b9 guest_xid=0 kf3_lines=1145 kf3_refusals=8
APPRES side=guest app=simpleAtomicIntrinsics verdict=TIMEOUT rc=137 secs=75 quiet=75 note=- boot=ap_r1_b10 guest_xid=0 kf3_lines=692 kf3_refusals=46
APPRES side=guest app=simpleCallback verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status"  boot=ap_r1_b11 guest_xid=0 kf3_lines=1143 kf3_refusals=8
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_r1_b12 guest_xid=0 kf3_lines=1159 kf3_refusals=8
APPRES side=guest app=simpleZeroCopy verdict=TIMEOUT rc=137 secs=76 quiet=76 note=- boot=ap_r1_b13 guest_xid=0 kf3_lines=678 kf3_refusals=46
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=6 quiet=1 note=- boot=ap_r1_b14 guest_xid=0 kf3_lines=1103 kf3_refusals=8
APPRES side=guest app=concurrentKernels verdict=FAIL rc=1 secs=5 quiet=1 note=CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_r1_b15 guest_xid=0 kf3_lines=1161 kf3_refusals=8
APPRES side=guest app=simpleIPC verdict=FAIL rc=1 secs=10 quiet=1 note=CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs[id], DATA_SIZE, cudaMemcpyDeviceToHost, stream)"  boot=ap_r1_b16 guest_xid=0 kf3_lines=1875 kf3_refusals=10
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=139 secs=34 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:216 code=719(cudaErrorLaunchFailure) "cudaStreamAttachMemAsync(stream[tid + 1], t.vector, 0, cudaMemAttachSingle)"  boot=ap_r1_b17 guest_xid=0 kf3_lines=1250 kf3_refusals=23
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=5 quiet=1 note=Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_r1_b18 guest_xid=0 kf3_lines=1148 kf3_refusals=7
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=9 quiet=2 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_r1_b19 guest_xid=0 kf3_lines=1291 kf3_refusals=10
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_r1_b20 guest_xid=0 kf3_lines=1147 kf3_refusals=10
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=18 quiet=1 note=- boot=ap_r1_b21 guest_xid=0 kf3_lines=1151 kf3_refusals=14
APPRES side=guest app=globalToShmemAsyncCopy verdict=FAIL rc=1 secs=6 quiet=1 note=CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b22 guest_xid=0 kf3_lines=1120 kf3_refusals=7
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=10 quiet=2 note=- boot=ap_r1_b23 guest_xid=0 kf3_lines=1183 kf3_refusals=10
APPRES side=guest app=graphMemoryNodes verdict=FAIL rc=1 secs=7 quiet=1 note=CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_square, hostArrays->bytes, cudaMemcpyDeviceToHost, boot=ap_r1_b24 guest_xid=0 kf3_lines=1121 kf3_refusals=8
APPRES side=guest app=simpleCudaGraphs verdict=TIMEOUT rc=137 secs=76 quiet=76 note=- boot=ap_r1_b25 guest_xid=0 kf3_lines=692 kf3_refusals=46
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=8 quiet=1 note=- boot=ap_r1_b26 guest_xid=0 kf3_lines=1203 kf3_refusals=10
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=7 quiet=2 note=- boot=ap_r1_b27 guest_xid=0 kf3_lines=1118 kf3_refusals=9
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=8 quiet=3 note=warn:Test Summary: Error amount = 0.000000 boot=ap_r1_b28 guest_xid=0 kf3_lines=1202 kf3_refusals=9
APPRES side=guest app=MersenneTwisterGP11213 verdict=FAIL rc=1 secs=5 quiet=1 note=CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_r1_b29 guest_xid=0 kf3_lines=1121 kf3_refusals=8
APPRES side=guest app=reduction verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_r1_b30 guest_xid=0 kf3_lines=3000 kf3_refusals=2
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=13 quiet=1 note=- boot=ap_r1_b31 guest_xid=0 kf3_lines=1121 kf3_refusals=12
APPRES side=guest app=scan verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_r1_b32 guest_xid=0 kf3_lines=1118 kf3_refusals=9
APPRES side=guest app=histogram verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_r1_b33 guest_xid=0 kf3_lines=1146 kf3_refusals=9
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=7 quiet=1 note=warn:Max absolute error: 1.192093E-05 boot=ap_r1_b34 guest_xid=0 kf3_lines=1118 kf3_refusals=9
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=11 quiet=2 note=- boot=ap_r1_b35 guest_xid=0 kf3_lines=1119 kf3_refusals=10
APPRES side=guest app=transpose verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b36 guest_xid=0 kf3_lines=1145 kf3_refusals=8
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=9 quiet=1 note=- boot=ap_r1_b37 guest_xid=0 kf3_lines=1105 kf3_refusals=10
APPRES side=guest app=reduce verdict=PASS rc=0 secs=11 quiet=1 note=- boot=ap_r1_b38 guest_xid=0 kf3_lines=1106 kf3_refusals=11
APPRES side=guest app=nbody verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b39 guest_xid=0 kf3_lines=1103 kf3_refusals=8
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_r1_b40 guest_xid=0 kf3_lines=1103 kf3_refusals=8
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b41 guest_xid=0 kf3_lines=1103 kf3_refusals=8
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=9 quiet=0 note=- boot=ap_r1_b42 guest_xid=0 kf3_lines=1105 kf3_refusals=10
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=9 quiet=1 note=- boot=ap_r1_b43 guest_xid=0 kf3_lines=1189 kf3_refusals=10
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=7 quiet=1 note=- boot=ap_r1_b44 guest_xid=0 kf3_lines=1104 kf3_refusals=9
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b45 guest_xid=0 kf3_lines=1104 kf3_refusals=9
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_r1_b46 guest_xid=0 kf3_lines=1103 kf3_refusals=8
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=5 quiet=1 note=FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) boot=ap_r1_b47 guest_xid=0 kf3_lines=1113 kf3_refusals=8
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=83 quiet=0 note=cuInit returned 0 (no error) boot=ap_r1_b48 guest_xid=0 kf3_lines=1454 kf3_refusals=51
APPRES side=guest app=torch_correct verdict=FAIL rc=1 secs=21 quiet=1 note=RuntimeError: GET was unable to find an engine to execute this computation boot=ap_r1_b49 guest_xid=0 kf3_lines=1318 kf3_refusals=16
APPRES side=guest app=torch_ai_bench verdict=FAIL rc=1 secs=31 quiet=2 note=RuntimeError: CUDA error: unspecified launch failure boot=ap_r1_b50 guest_xid=0 kf3_lines=1327 kf3_refusals=21
APPRES side=guest app=hf_generate verdict=TIMEOUT rc=137 secs=615 quiet=615 note=- boot=ap_r1_b51 guest_xid=0 kf3_lines=609 kf3_refusals=313
APPRES side=guest app=cupy verdict=PASS rc=0 secs=14 quiet=1 note=- boot=ap_r1_b52 guest_xid=0 kf3_lines=1247 kf3_refusals=12
APPRES side=guest app=llama_cpp_gen verdict=FAIL rc=134 secs=47 quiet=0 note=bash: line 1: 2071 Aborted (core dumped) /opt/apps/bundle/llama/llama-simple -m /opt/apps/data/qwen2.5-1.5b-instruct-q4_k_m.gguf -n 64 -ngl 99 "Explain in three boot=ap_r1_b53 guest_xid=0 kf3_lines=1828 kf3_refusals=4
APPRES side=guest app=llama_bench verdict=FAIL rc=134 secs=49 quiet=0 note=/workspace/apps/srcs/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu:109: CUDA error boot=ap_r1_b54 guest_xid=0 kf3_lines=1857 kf3_refusals=4
APPRES side=guest app=vulkaninfo verdict=TIMEOUT rc=124 secs=62 quiet=62 note=- boot=ap_r1_b55 guest_xid=0 kf3_lines=2136 kf3_refusals=32
APPRES side=guest app=vkpeak verdict=FAIL rc=255 secs=67 quiet=0 note=- boot=ap_r1_b56 guest_xid=0 kf3_lines=2176 kf3_refusals=32
APPRES side=guest app=egl_offscreen verdict=FAIL rc=0 secs=3 quiet=0 note=CHECK egl_gl_Mtri_s FAIL boot=ap_r1_b57 guest_xid=0 kf3_lines=289 kf3_refusals=4
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=10 quiet=3 note=- boot=ap_r1_b58 guest_xid=0 kf3_lines=1500 kf3_refusals=8
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=96 quiet=0 note=- boot=ap_r1_b59 guest_xid=0 kf3_lines=1145 kf3_refusals=4
APPRES side=guest app=nvenc_h264 verdict=FAIL rc=0 secs=7 quiet=0 note=- boot=ap_r1_b60 guest_xid=0 kf3_lines=1099 kf3_refusals=4
APPRES side=guest app=nvenc_hevc verdict=FAIL rc=0 secs=8 quiet=0 note=- boot=ap_r1_b61 guest_xid=0 kf3_lines=1099 kf3_refusals=4
APPRES side=guest app=nvdec_h264 verdict=FAIL rc=3 secs=13 quiet=0 note=- boot=ap_r1_b62 guest_xid=0 kf3_lines=1102 kf3_refusals=4
APPRES side=guest app=hashcat verdict=FAIL rc=0 secs=25 quiet=0 note=Watchdog: Temperature abort trigger set to 90c boot=ap_r1_b63 guest_xid=0 kf3_lines=1838 kf3_refusals=4
APPRES side=guest app=blender_cycles verdict=FAIL rc=0 secs=24 quiet=0 note=- boot=ap_r1_b64 guest_xid=0 kf3_lines=2241 kf3_refusals=8
APPRES side=guest app=geekbench_gpu verdict=FAIL rc=0 secs=20 quiet=0 note=[0926/022831:ERROR:optimizer.cpp(122)] build_patches_padding: optimization failed for size { 32, 1, 1, }: Waiting for OpenCL kernel dispatch (build_patches_padd boot=ap_r1_b65 guest_xid=0 kf3_lines=1748 kf3_refusals=4
