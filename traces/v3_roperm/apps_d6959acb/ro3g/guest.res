APPRES side=guest app=torch_correct verdict=PASS rc=0 secs=51 quiet=1 note=- boot=ap_ro3g_b1 guest_xid=0 kf3_lines=1434 kf3_refusals=4
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=56 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_ro3g_b2 guest_xid=0 kf3_lines=1868 kf3_refusals=4
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=22 quiet=1 note=- boot=ap_ro3g_b3 guest_xid=0 kf3_lines=2075 kf3_refusals=4
APPRES side=guest app=vkpeak verdict=PASS rc=0 secs=377 quiet=4 note=- boot=ap_ro3g_b4 guest_xid=0 kf3_lines=3000 kf3_refusals=0
APPRES side=guest app=nvenc_h264 verdict=PASS rc=0 secs=12 quiet=0 note=- boot=ap_ro3g_b5 guest_xid=0 kf3_lines=1218 kf3_refusals=4
APPRES side=guest app=nvenc_hevc verdict=PASS rc=0 secs=12 quiet=0 note=- boot=ap_ro3g_b6 guest_xid=0 kf3_lines=1213 kf3_refusals=4
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=12 quiet=0 note=- boot=ap_ro3g_b7 guest_xid=0 kf3_lines=1206 kf3_refusals=4
APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=4 quiet=0 note=- boot=ap_ro3g_b8 guest_xid=0 kf3_lines=1103 kf3_refusals=4
APPRES side=guest app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=5 quiet=1 note=- boot=ap_ro3g_b9 guest_xid=0 kf3_lines=1124 kf3_refusals=4
