APPRES side=win app=valley_opengl verdict=TIMEOUT rc=124 secs=272 quiet=- note=timeout boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=- tier=2 cat=opengl
APPRES side=win app=vkcube verdict=FAIL rc=0 secs=16 quiet=15 note=no-gpu-proof boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=vulkan
APPRES side=win app=llama_cuda_gen verdict=PASS rc=0 secs=81 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm
APPDIG side=win app=llama_cuda_gen OUTSHA llama_cpp ab35161a94bc34ab
APPRES side=win app=furmark_gl_stress verdict=FAIL rc=0 secs=65 quiet=64 note=no-gpu-proof boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:out,pdh:Cuda|Compute|3D|Copy tier=2 cat=stress
APPRES side=win app=torch_burn verdict=PASS rc=0 secs=117 quiet=2 note=warn:[burn] t:10s iters:1445 errors:0 boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=stress
APPRES side=win app=dxva2_h264 verdict=PASS rc=0 secs=39 quiet=29 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=pdh:videodecode tier=2 cat=video
APPRES side=win app=dxdiag_report verdict=PASS rc=0 secs=22 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=llama_cuda_bench verdict=PASS rc=0 secs=77 quiet=1 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm
APPRES side=win app=llama_vulkan_bench verdict=PASS rc=0 secs=26 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm
APPRES side=win app=blender_cycles verdict=PASS rc=0 secs=14 quiet=4 note=- boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=render
APPRES side=win app=vkpeak verdict=PASS rc=0 secs=117 quiet=1 note=warn:System.Management.Automation.RemoteException boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=gravitymark_d3d12_raster verdict=FAIL rc=0 secs=188 quiet=188 note=no-gpu-proof boot=g4 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=d3d
APPRES side=win app=gravitymark_d3d12_rt verdict=FAIL rc=1 secs=33 quiet=0 note=rc:1,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image boot=g4 guest_tdr=3 wer=0 gsp_cycles=1 proof=none:pdh:Cuda|Compute|3D|Copy tier=3 cat=d3d
