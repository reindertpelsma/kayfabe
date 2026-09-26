APPRES side=guest app=UnifiedMemoryStreams verdict=TIMEOUT rc=137 secs=136 quiet=136 note=- boot=ap_iso1_b1 guest_xid=0 kf3_lines=877 kf3_refusals=6
APPS_WEDGE boot=ap_iso1_b1 after=UnifiedMemoryStreams sanity_vectorAdd=none
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=5 quiet=0 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_iso1_b2 guest_xid=0 kf3_lines=1216 kf3_refusals=4
APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=11 quiet=0 note=Test Summary: Error amount = 1.000000, result = SUCCESS boot=ap_iso1_b3 guest_xid=0 kf3_lines=1304 kf3_refusals=4
APPRES side=guest app=attach_verify verdict=FAIL rc=1 secs=6 quiet=1 note=FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure) boot=ap_iso1_b4 guest_xid=0 kf3_lines=1115 kf3_refusals=4
APPRES side=guest app=gpu_burn verdict=FAIL rc=139 secs=18 quiet=0 note=cuInit returned 0 (no error) boot=ap_iso1_b5 guest_xid=0 kf3_lines=988 kf3_refusals=8
APPRES side=guest app=torch_ai_bench verdict=FAIL rc=1 secs=24 quiet=2 note=RuntimeError: CUDA error: unspecified launch failure boot=ap_iso1_b6 guest_xid=0 kf3_lines=1221 kf3_refusals=4
APPRES side=guest app=clpeak verdict=FAIL rc=0 secs=104 quiet=0 note=clpeak: 4 test groups skipped (clFinish (-36)) boot=ap_iso1_b7 guest_xid=0 kf3_lines=1182 kf3_refusals=4
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=29 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_iso1_b8 guest_xid=0 kf3_lines=1853 kf3_refusals=4
