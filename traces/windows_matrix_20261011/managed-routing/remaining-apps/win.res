APPRES side=win app=valley_opengl verdict=TIMEOUT rc=124 secs=272 quiet=- note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=- tier=2 cat=opengl
APPRES side=win app=vkcube verdict=FAIL rc=0 secs=16 quiet=15 note=no-gpu-proof boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=vulkan
APPRES side=win app=llama_cuda_gen verdict=PASS rc=0 secs=81 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm
APPDIG side=win app=llama_cuda_gen OUTSHA llama_cpp ab35161a94bc34ab
