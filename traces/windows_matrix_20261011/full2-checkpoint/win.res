APPRES side=win app=atomics_t verdict=PASS rc=0 secs=95 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=bandwidthTest_demo verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=blackscholes_t verdict=FAIL rc=-1073741819 secs=108 quiet=107 note=rc:-1073741819,success-string-missing,no-gpu-proof boot=g1 guest_tdr=3 wer=0 gsp_cycles=1 proof=none:out,pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-equiv
APPRES side=win app=blender_cycles verdict=PASS rc=0 secs=14 quiet=4 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=render
APPRES side=win app=cg_t verdict=PASS rc=0 secs=86 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=clpeak_cuda verdict=TIMEOUT rc=124 secs=1501 quiet=1488 note=timeout boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=opencl
