APPRES side=host app=nvidia_smi verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=deviceQuery verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=vectorAdd verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=vectorAddDrv verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=matrixMulDrv verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=asyncAPI verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleOccupancy verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleZeroCopy verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=simpleCooperativeGroups verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=cudaTensorCoreGemm verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=bf16TensorCoreGemm verdict=PASS rc=0 secs=8 quiet=1 note=-
APPRES side=host app=cdpSimpleQuicksort verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleCUBLAS verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=simpleCUFFT verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=conjugateGradient verdict=PASS rc=0 secs=2 quiet=0 note=warn:Test Summary: Error amount = 0.000000
APPRES side=host app=reduction verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=sortingNetworks verdict=PASS rc=0 secs=9 quiet=1 note=-
APPRES side=host app=scan verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=histogram verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=BlackScholes verdict=PASS rc=0 secs=3 quiet=0 note=warn:Max absolute error: 1.192093E-05
APPRES side=host app=fastWalshTransform verdict=PASS rc=0 secs=6 quiet=1 note=-
APPRES side=host app=transpose verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=stream_triad verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=reduce verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=nbody verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=blackscholes verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=mandelbrot verdict=PASS rc=0 secs=3 quiet=1 note=-
APPRES side=host app=conv2d verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=sgemm_cublas verdict=PASS rc=0 secs=4 quiet=1 note=-
APPRES side=host app=fft_cufft verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=sha256 verdict=PASS rc=0 secs=4 quiet=1 note=-
APPRES side=host app=memcpy2d verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=hf_generate verdict=PASS rc=0 secs=11 quiet=1 note=-
APPDIG side=host app=hf_generate OUTSHA hf_generate 0d973108a6251e14
APPRES side=host app=cupy verdict=PASS rc=0 secs=4 quiet=1 note=-
APPRES side=host app=clinfo verdict=PASS rc=0 secs=6 quiet=2 note=-
APPRES side=host app=clpeak verdict=PASS rc=0 secs=118 quiet=1 note=-
APPRES side=host app=stream_default verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=conjugateGradientUM verdict=PASS rc=0 secs=3 quiet=0 note=warn:Test Summary: Error amount = 0.000000, result = SUCCESS
APPRES side=host app=UnifiedMemoryStreams verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=UnifiedMemoryPerf verdict=PASS rc=0 secs=18 quiet=0 note=-
APPRES side=host app=attach_verify verdict=PASS rc=0 secs=3 quiet=0 note=-
APPRES side=host app=torch_ai_bench verdict=PASS rc=0 secs=41 quiet=1 note=-
APPRES side=host app=gpu_burn verdict=PASS rc=0 secs=102 quiet=0 note=warn:cuInit returned 0 (no error)
