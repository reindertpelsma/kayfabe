APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=13 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=4 quiet=3 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=torch_ai_bench verdict=FAIL rc=- secs=411 quiet=- note=guest-rebooted (bugcheck?) boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=- tier=1 cat=torch
