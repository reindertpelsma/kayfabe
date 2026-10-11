APPRES side=win app=managed_t verdict=TIMEOUT rc=124 secs=180 quiet=36 note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=cuda-equiv
APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=14 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=5 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
