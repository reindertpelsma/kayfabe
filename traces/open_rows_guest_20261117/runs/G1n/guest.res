APPRES side=guest app=blender_cycles verdict=FAIL rc=0 secs=25 quiet=0 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=433 kf3_refusals=28
APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=19 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 338 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead boot=ap_G1n_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=12
APPRES side=guest app=llama_bench verdict=PASS rc=0 secs=8 quiet=1 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=3000 kf3_refusals=12
APPRES side=guest app=hashcat verdict=PASS rc=0 secs=42 quiet=0 note=warn:Watchdog: Temperature abort trigger set to 90c boot=ap_G1n_b1 guest_xid=0 kf3_lines=425 kf3_refusals=19
APPRES side=guest app=clpeak verdict=PASS rc=0 secs=43 quiet=0 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=405 kf3_refusals=19
APPRES side=guest app=egl_offscreen verdict=FAIL rc=0 secs=0 quiet=0 note=libEGL warning: egl: failed to create dri2 screen boot=ap_G1n_b1 guest_xid=0 kf3_lines=3 kf3_refusals=0
APPRES side=guest app=nvdec_h264 verdict=PASS rc=0 secs=3 quiet=0 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=5 kf3_refusals=0
APPRES side=guest app=nvenc_h264 verdict=FAIL rc=0 secs=0 quiet=0 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=4 kf3_refusals=0
APPRES side=guest app=nvenc_hevc verdict=FAIL rc=0 secs=0 quiet=0 note=- boot=ap_G1n_b1 guest_xid=0 kf3_lines=4 kf3_refusals=0
