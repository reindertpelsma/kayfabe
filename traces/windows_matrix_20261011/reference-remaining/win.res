APPRES side=win app=vkcube verdict=PASS rc=0 secs=37 quiet=36 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d tier=2 cat=vulkan
APPRES side=win app=llama_cuda_gen verdict=PASS rc=0 secs=60 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=llm
APPDIG side=win app=llama_cuda_gen OUTSHA llama_cpp 39d8f2786e8757a1
APPRES side=win app=dxdiag_report verdict=PASS rc=0 secs=29 quiet=29 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=probe
APPRES side=win app=llama_cuda_bench verdict=PASS rc=0 secs=6 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=llm
APPRES side=win app=llama_vulkan_bench verdict=PASS rc=0 secs=31 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out,pdh:3d tier=1 cat=llm
APPRES side=win app=blender_cycles verdict=PASS rc=0 secs=20 quiet=6 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=render
APPRES side=win app=vkpeak verdict=PASS rc=0 secs=127 quiet=123 note=warn:System.Management.Automation.RemoteException boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out,pdh:3d tier=1 cat=vulkan
APPRES side=win app=gravitymark_d3d12_raster verdict=PASS rc=0 secs=187 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d tier=2 cat=d3d
APPRES side=win app=gravitymark_d3d12_rt verdict=PASS rc=0 secs=181 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d+copy tier=3 cat=d3d
APPRES side=win app=torch_ai_bench verdict=PASS rc=0 secs=35 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=torch
APPRES side=win app=gravitymark_d3d12 verdict=PASS rc=0 secs=179 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d+copy tier=1 cat=d3d
APPRES side=win app=gravitymark_gl verdict=PASS rc=0 secs=191 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d tier=2 cat=opengl
APPRES side=win app=gravitymark_d3d11 verdict=PASS rc=0 secs=205 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:3d+copy tier=1 cat=d3d
APPRES side=win app=clpeak_ocl verdict=PASS rc=0 secs=775 quiet=0 note=warn:Failed to read file: C:\WINDOWS\TEMP\dep-e5d6ca.d boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out,pdh:3d+copy tier=1 cat=opencl
APPRES side=win app=torch_burn verdict=PASS rc=0 secs=67 quiet=2 note=warn:[burn] t:10s iters:3601 errors:0 boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out,smi tier=1 cat=stress
