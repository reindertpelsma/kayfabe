APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=10 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_nb2_b1 guest_xid=0 kf3_lines=1303 kf3_refusals=4
APPRES side=guest app=cudaTensorCoreGemm verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1123 kf3_refusals=4
APPRES side=guest app=bf16TensorCoreGemm verdict=PASS rc=0 secs=13 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1126 kf3_refusals=4
APPRES side=guest app=globalToShmemAsyncCopy verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1092 kf3_refusals=4
APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1158 kf3_refusals=4
APPRES side=guest app=graphMemoryNodes verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1093 kf3_refusals=4
APPRES side=guest app=simpleCudaGraphs verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1092 kf3_refusals=4
APPRES side=guest app=simpleCUBLAS verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1176 kf3_refusals=4
APPRES side=guest app=simpleCUFFT verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1091 kf3_refusals=3
APPRES side=guest app=conjugateGradient verdict=PASS rc=0 secs=1 quiet=0 note=warn:Test Summary: Error amount = 0.000000 boot=ap_nb2_b1 guest_xid=0 kf3_lines=1174 kf3_refusals=2
APPRES side=guest app=MersenneTwisterGP11213 verdict=PASS rc=0 secs=2 quiet=1 note=warn:Max absolute error: 0.000000E+00 boot=ap_nb2_b1 guest_xid=0 kf3_lines=1090 kf3_refusals=2
APPRES side=guest app=reduction verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=sortingNetworks verdict=PASS rc=0 secs=10 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1094 kf3_refusals=2
APPRES side=guest app=scan verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1090 kf3_refusals=2
APPRES side=guest app=histogram verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1119 kf3_refusals=2
APPRES side=guest app=BlackScholes verdict=PASS rc=0 secs=3 quiet=0 note=warn:Max absolute error: 1.192093E-05 boot=ap_nb2_b1 guest_xid=0 kf3_lines=1090 kf3_refusals=2
APPRES side=guest app=fastWalshTransform verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1093 kf3_refusals=2
APPRES side=guest app=transpose verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1115 kf3_refusals=2
APPRES side=guest app=stream_triad verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=reduce verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=nbody verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=blackscholes verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=mandelbrot verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=conv2d verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=sgemm_cublas verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1160 kf3_refusals=2
APPRES side=guest app=fft_cufft verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=sha256 verdict=PASS rc=0 secs=3 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=memcpy2d verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=2 quiet=1 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_nb2_b1 guest_xid=0 kf3_lines=1094 kf3_refusals=2
APPRES side=guest app=stream_default verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1075 kf3_refusals=2
APPRES side=guest app=stream_created verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1077 kf3_refusals=2
APPRES side=guest app=stream_nonblocking verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_perthread verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_two verdict=PASS rc=0 secs=1 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=stream_created2nd verdict=PASS rc=0 secs=2 quiet=0 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1076 kf3_refusals=2
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=63 quiet=0 note=cuInit returned 0 (no error) boot=ap_nb2_b1 guest_xid=0 kf3_lines=979 kf3_refusals=4
APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=77 quiet=1 note=- boot=ap_nb2_b1 guest_xid=0 kf3_lines=1403 kf3_refusals=2
APPRES side=guest app=torch_ai_bench verdict=FAIL rc=1 secs=3 quiet=0 note=/opt/apps/venv/lib/python3.12/site-packages/torch/cuda/__init__.py:129: UserWarning: CUDA initialization: CUDA driver initialization failed, you might not have  boot=ap_nb2_b1 guest_xid=0 kf3_lines=387 kf3_refusals=3
APPS_WEDGE boot=ap_nb2_b1 after=torch_ai_bench sanity_vectorAdd=0
APPRES side=guest app=hf_generate verdict=PASS rc=0 secs=52 quiet=1 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1219 kf3_refusals=4
APPRES side=guest app=cupy verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1226 kf3_refusals=4
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=8 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_nb2_b2 guest_xid=0 kf3_lines=1808 kf3_refusals=4
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=4 quiet=1 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=2031 kf3_refusals=4
APPRES side=guest app=vulkaninfo verdict=PASS rc=0 secs=2 quiet=0 note=warn:error: XDG_RUNTIME_DIR is invalid or not set in the environment. boot=ap_nb2_b2 guest_xid=0 kf3_lines=1212 kf3_refusals=4
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=418 quiet=2 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=20 quiet=0 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=562 kf3_refusals=4
APPRES side=guest app=clinfo verdict=PASS rc=0 secs=5 quiet=3 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1468 kf3_refusals=8
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=96 quiet=1 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1155 kf3_refusals=3
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=8 quiet=0 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1164 kf3_refusals=2
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=7 quiet=0 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1163 kf3_refusals=2
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=6 quiet=0 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=1162 kf3_refusals=2
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=27 quiet=0 note=warn:This means that hashcat cannot use the full parallel power of your device(s). boot=ap_nb2_b2 guest_xid=0 kf3_lines=1816 kf3_refusals=2
APPRES side=guest app=blender_cycles verdict=PASS rc=0 secs=11 quiet=0 note=- boot=ap_nb2_b2 guest_xid=0 kf3_lines=2201 kf3_refusals=4
APPRES side=guest app=geekbench_gpu verdict=PASS rc=0 secs=69 quiet=0 note=warn: unknown error (internal code 35) boot=ap_nb2_b2 guest_xid=0 kf3_lines=3000 kf3_refusals=0
