APPRES side=win app=hf_generate verdict=PASS rc=0 secs=42 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=ai
APPDIG side=win app=hf_generate OUTSHA hf_generate 0d973108a6251e14
APPRES side=win app=cdp_quicksort verdict=FAIL rc=1 secs=11 quiet=6 note=rc:1,success-string-missing,no-gpu-proof | nvrtc: error: failed to open nvrtc-builtins64_129.dll. boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=none:out,pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-equiv
APPRES side=win app=stream_perthread_t verdict=PASS rc=0 secs=26 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out,pdh:3d tier=1 cat=stream-probe
APPRES side=win app=cupy_sha256 verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_cg verdict=PASS rc=0 secs=9 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_asynccopy verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_check verdict=PASS rc=0 secs=9 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=torch
APPRES side=win app=managed_t verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
