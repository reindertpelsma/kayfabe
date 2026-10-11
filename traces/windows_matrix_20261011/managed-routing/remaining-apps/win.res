APPRES side=win app=valley_opengl verdict=TIMEOUT rc=124 secs=272 quiet=- note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=- tier=2 cat=opengl
APPRES side=win app=vkcube verdict=FAIL rc=0 secs=16 quiet=15 note=no-gpu-proof boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=vulkan
APPRES side=win app=llama_cuda_gen verdict=PASS rc=0 secs=81 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm
APPDIG side=win app=llama_cuda_gen OUTSHA llama_cpp ab35161a94bc34ab
APPRES side=win app=furmark_gl_stress verdict=FAIL rc=0 secs=65 quiet=64 note=no-gpu-proof boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:out,pdh:Cuda|Compute|3D|Copy tier=2 cat=stress
APPRES side=win app=torch_burn verdict=PASS rc=0 secs=117 quiet=2 note=warn:[burn] t:10s iters:1445 errors:0 boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stress
