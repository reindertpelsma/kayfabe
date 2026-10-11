APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=3 quiet=3 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi
APPRES side=win app=deviceQuery_demo verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=13 quiet=9 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=matmul_t verdict=PASS rc=0 secs=62 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
