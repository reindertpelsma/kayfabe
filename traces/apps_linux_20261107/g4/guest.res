APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_g4_b1 guest_xid=0 kf3_lines=1222 kf3_refusals=29
APPRES side=guest app=UnifiedMemoryPerf verdict=FAIL rc=1 secs=2 quiet=1 note=Running ...CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(streamToRunOn)"  boot=ap_g4_b1 guest_xid=0 kf3_lines=1314 kf3_refusals=23
APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=2 quiet=1 note=- boot=ap_g4_b1 guest_xid=0 kf3_lines=1194 kf3_refusals=20
