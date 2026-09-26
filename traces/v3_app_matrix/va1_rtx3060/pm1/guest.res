APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=0 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=65 kf3_refusals=1
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=881 kf3_refusals=0
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=vectorAddDrv verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=829 kf3_refusals=0
APPRES side=guest app=matrixMul verdict=FAIL rc=1 secs=1 quiet=1 note=CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=854 kf3_refusals=0
APPRES side=guest app=matrixMulDrv verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=830 kf3_refusals=0
APPRES side=guest app=bandwidthTest verdict=FAIL rc=1 secs=3 quiet=1 note=CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=870 kf3_refusals=0
APPRES side=guest app=simpleStreams verdict=FAIL rc=1 secs=2 quiet=0 note=CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=882 kf3_refusals=0
APPRES side=guest app=asyncAPI verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=878 kf3_refusals=0
APPRES side=guest app=simpleAtomicIntrinsics verdict=FAIL rc=1 secs=1 quiet=0 note=CUDA error at simpleAtomicIntrinsics.cu:122 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=854 kf3_refusals=0
APPRES side=guest app=simpleCallback verdict=FAIL rc=1 secs=1 quiet=0 note=CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=881 kf3_refusals=0
APPRES side=guest app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=893 kf3_refusals=0
APPRES side=guest app=simpleZeroCopy verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=864 kf3_refusals=0
APPRES side=guest app=simpleCooperativeGroups verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=concurrentKernels verdict=FAIL rc=1 secs=0 quiet=0 note=CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=892 kf3_refusals=0
APPRES side=guest app=simpleIPC verdict=FAIL rc=1 secs=4 quiet=0 note=CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs[id], DATA_SIZE, cudaMemcpyDeviceToHost, stream)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=1608 kf3_refusals=0
APPRES side=guest app=UnifiedMemoryStreams verdict=FAIL rc=134 secs=4 quiet=0 note=CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1], CUBLAS_OP_N, t.size, t.size, &one, t.data, t.siz boot=ap_pm1_b1 guest_xid=0 kf3_lines=974 kf3_refusals=0
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=1 quiet=0 note=Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=881 kf3_refusals=0
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=1 quiet=0 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_pm1_b1 guest_xid=0 kf3_lines=1023 kf3_refusals=0
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=880 kf3_refusals=0
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=12 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=884 kf3_refusals=0
APPRES side=guest app=globalToShmemAsyncCopy verdict=FAIL rc=1 secs=1 quiet=0 note=CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=854 kf3_refusals=0
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=917 kf3_refusals=0
APPRES side=guest app=graphMemoryNodes verdict=FAIL rc=1 secs=2 quiet=1 note=CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_square, hostArrays->bytes, cudaMemcpyDeviceToHost, boot=ap_pm1_b1 guest_xid=0 kf3_lines=855 kf3_refusals=0
APPRES side=guest app=simpleCudaGraphs verdict=FAIL rc=1 secs=1 quiet=0 note=CUDA error at simpleCudaGraphs.cu:273 code=719(cudaErrorLaunchFailure) "cudaGraphLaunch(graphExec, streamForGraph)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=854 kf3_refusals=0
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=935 kf3_refusals=0
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=1 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=850 kf3_refusals=0
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=1 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_pm1_b1 guest_xid=0 kf3_lines=935 kf3_refusals=0
APPRES side=guest app=MersenneTwisterGP11213 verdict=FAIL rc=1 secs=1 quiet=0 note=CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)"  boot=ap_pm1_b1 guest_xid=0 kf3_lines=855 kf3_refusals=0
APPRES side=guest app=reduction verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=855 kf3_refusals=0
APPRES side=guest app=scan verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=851 kf3_refusals=0
APPRES side=guest app=histogram verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=879 kf3_refusals=0
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=2 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_pm1_b1 guest_xid=0 kf3_lines=851 kf3_refusals=0
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=5 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=852 kf3_refusals=0
APPRES side=guest app=transpose verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=879 kf3_refusals=0
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=reduce verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=nbody verdict=PASS rc=0 secs=0 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=836 kf3_refusals=0
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=836 kf3_refusals=0
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=921 kf3_refusals=0
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=836 kf3_refusals=0
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=1 quiet=0 note=FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=846 kf3_refusals=0
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=837 kf3_refusals=0
APPRES side=guest app=stream_created verdict=FAIL rc=1 secs=1 quiet=0 note=CHECK created FAIL sync=719(unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=stream_nonblocking verdict=FAIL rc=1 secs=1 quiet=0 note=CHECK nonblocking FAIL sync=719(unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=841 kf3_refusals=0
APPRES side=guest app=stream_perthread verdict=FAIL rc=1 secs=1 quiet=1 note=CHECK perthread FAIL sync=719(unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=stream_two verdict=FAIL rc=1 secs=0 quiet=0 note=CHECK two_a FAIL sync=719(unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=840 kf3_refusals=0
APPRES side=guest app=stream_created2nd verdict=FAIL rc=1 secs=0 quiet=0 note=CHECK c2_created FAIL sync=719(unspecified launch failure) boot=ap_pm1_b1 guest_xid=0 kf3_lines=841 kf3_refusals=0
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=5 quiet=0 note=cuInit returned 0 (no error) boot=ap_pm1_b1 guest_xid=0 kf3_lines=533 kf3_refusals=0
APPRES side=guest app=torch_correct verdict=FAIL rc=1 secs=9 quiet=1 note=RuntimeError: GET was unable to find an engine to execute this computation boot=ap_pm1_b1 guest_xid=0 kf3_lines=1436 kf3_refusals=0
APPRES side=guest app=torch_ai_bench verdict=FAIL rc=1 secs=7 quiet=1 note=RuntimeError: CUDA error: unspecified launch failure boot=ap_pm1_b1 guest_xid=0 kf3_lines=941 kf3_refusals=0
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=37 quiet=1 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=945 kf3_refusals=0
APPRES side=guest app=cupy verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=978 kf3_refusals=0
APPRES side=guest app=llama_cpp_gen verdict=FAIL rc=134 secs=26 quiet=0 note=bash: line 1: 18442 Aborted (core dumped) /opt/apps/bundle/llama/llama-simple -m /opt/apps/data/qwen2.5-1.5b-instruct-q4_k_m.gguf -n 64 -ngl 99 "Explain in thre boot=ap_pm1_b1 guest_xid=0 kf3_lines=1553 kf3_refusals=0
APPRES side=guest app=llama_bench verdict=TIMEOUT rc=137 secs=915 quiet=914 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=2596 kf3_refusals=1
APPRES side=guest app=vulkaninfo verdict=TIMEOUT rc=137 secs=75 quiet=75 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=45 kf3_refusals=0
APPRES side=guest app=vkpeak verdict=TIMEOUT rc=137 secs=915 quiet=915 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=507 kf3_refusals=0
APPRES side=guest app=egl_offscreen verdict=TIMEOUT rc=137 secs=105 quiet=105 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=67 kf3_refusals=0
APPRES side=guest app=clinfo verdict=TIMEOUT rc=124 secs=60 quiet=60 note=- boot=ap_pm1_b1 guest_xid=0 kf3_lines=33 kf3_refusals=0
