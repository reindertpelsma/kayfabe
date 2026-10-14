APPRES side=win app=blackscholes_t verdict=TIMEOUT rc=124 secs=132 quiet=130 note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=cuda-equiv
APPRES side=win app=cuda_callback verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cuda_occupancy verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=deviceQuery_demo verdict=PASS rc=0 secs=14 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=glgears_wgl verdict=TIMEOUT rc=124 secs=197 quiet=- note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=- tier=1 cat=opengl
APPRES side=win app=matmul_t verdict=PASS rc=0 secs=74 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
