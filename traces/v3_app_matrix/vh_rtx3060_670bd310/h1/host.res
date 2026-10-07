APPRES side=host app=nvidia_smi verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=deviceQuery verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=vectorAdd verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=vectorAddDrv verdict=PASS rc=0 secs=2 quiet=1 note=-
APPRES side=host app=matrixMul verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=matrixMulDrv verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=bandwidthTest verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=simpleStreams verdict=PASS rc=0 secs=4 quiet=1 note=-
APPRES side=host app=asyncAPI verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleCallback verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=1 note=-
APPRES side=host app=simpleZeroCopy verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleCooperativeGroups verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=concurrentKernels verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=simpleIPC verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=UnifiedMemoryStreams verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=UnifiedMemoryPerf verdict=PASS rc=0 secs=18 quiet=1 note=-
APPRES side=host app=conjugateGradientUM verdict=PASS rc=0 secs=2 quiet=0 note=warn:Test Summary: Error amount = 0.000000, result = SUCCESS
APPRES side=host app=cudaTensorCoreGemm verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=bf16TensorCoreGemm verdict=PASS rc=0 secs=7 quiet=0 note=-
APPRES side=host app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=cdpSimpleQuicksort verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=graphMemoryNodes verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=simpleCudaGraphs verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=simpleCUBLAS verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleCUFFT verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=conjugateGradient verdict=PASS rc=0 secs=3 quiet=1 note=warn:Test Summary: Error amount = 0.000000
APPRES side=host app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=2 quiet=1 note=warn:Max absolute error: 0.000000E+00
APPRES side=host app=reduction verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=sortingNetworks verdict=PASS rc=0 secs=9 quiet=0 note=-
APPRES side=host app=scan verdict=PASS rc=0 secs=4 quiet=1 note=-
APPRES side=host app=histogram verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=BlackScholes verdict=PASS rc=0 secs=3 quiet=0 note=warn:Max absolute error: 1.192093E-05
APPRES side=host app=fastWalshTransform verdict=PASS rc=0 secs=6 quiet=0 note=-
APPRES side=host app=transpose verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=stream_triad verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=reduce verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=nbody verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=blackscholes verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=mandelbrot verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=conv2d verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=sgemm_cublas verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=fft_cufft verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=sha256 verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=memcpy2d verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=attach_verify verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=stream_default verdict=PASS rc=0 secs=2 quiet=1 note=-
APPRES side=host app=stream_created verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=stream_nonblocking verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=stream_perthread verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=stream_two verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=stream_created2nd verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=gpu_burn verdict=PASS rc=0 secs=101 quiet=0 note=warn:cuInit returned 0 (no error)
APPRES side=host app=torch_correct verdict=PASS rc=0 secs=10 quiet=2 note=-
APPDIG side=host app=torch_correct DIGEST cnn_train_step 763c693a5a53f948
APPRES side=host app=torch_ai_bench verdict=PASS rc=0 secs=39 quiet=2 note=-
APPRES side=host app=hf_generate verdict=PASS rc=0 secs=9 quiet=2 note=-
APPDIG side=host app=hf_generate OUTSHA hf_generate 0d973108a6251e14
APPRES side=host app=cupy verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=llama_cpp_gen verdict=PASS rc=0 secs=4 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead
APPDIG side=host app=llama_cpp_gen OUTSHA llama_cpp a60151fa54b13227
APPRES side=host app=llama_bench verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=vulkaninfo verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=vkpeak verdict=PASS rc=0 secs=412 quiet=1 note=-
APPRES side=host app=egl_offscreen verdict=PASS rc=0 secs=24 quiet=0 note=-
APPRES side=host app=clinfo verdict=PASS rc=0 secs=6 quiet=1 note=-
APPRES side=host app=clpeak verdict=PASS rc=0 secs=118 quiet=1 note=-
APPRES side=host app=nvenc_h264 verdict=PASS rc=0 secs=5 quiet=0 note=-
APPRES side=host app=nvenc_hevc verdict=PASS rc=0 secs=5 quiet=0 note=-
APPRES side=host app=nvdec_h264 verdict=PASS rc=0 secs=7 quiet=0 note=-
APPRES side=host app=hashcat verdict=PASS rc=0 secs=18 quiet=0 note=warn:[33mThis means that hashcat cannot use the full parallel power of your device(s).[0m
APPRES side=host app=blender_cycles verdict=PASS rc=0 secs=10 quiet=0 note=-
APPRES side=host app=geekbench_gpu verdict=PASS rc=0 secs=64 quiet=0 note=warn: unknown error (internal code 35)
