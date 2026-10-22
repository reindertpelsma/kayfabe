APPRES side=win app=driver_status verdict=PASS rc=0 secs=4 quiet=3 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=dxgi_adapters verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=probe
APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi
APPRES side=win app=cup2_ladder verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=cup3_ladder verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-ladder
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=dxprobe_d3d11 verdict=PASS rc=0 secs=5 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=dxprobe_d3d12 verdict=PASS rc=0 secs=7 quiet=2 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=heaven_d3d11 verdict=FAIL rc=0 secs=49 quiet=49 note=no-gpu-proof boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=d3d
APPRES side=win app=heaven_opengl verdict=FAIL rc=0 secs=49 quiet=48 note=no-gpu-proof boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=opengl
APPRES side=win app=valley_d3d11 verdict=FAIL rc=0 secs=50 quiet=49 note=no-gpu-proof boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=d3d
APPRES side=win app=valley_opengl verdict=FAIL rc=0 secs=49 quiet=48 note=no-gpu-proof boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=2 cat=opengl
APPRES side=win app=atomics_t verdict=PASS rc=0 secs=15 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=bandwidthTest_demo verdict=PASS rc=0 secs=5 quiet=0 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=blackscholes_t verdict=PASS rc=0 secs=9 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=blender_cycles verdict=PASS rc=0 secs=31 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=render
APPRES side=win app=cdp_quicksort verdict=NOTRUN rc=- secs=0 quiet=- note=stage-failed:cuda_nvrtc:tar -xf cuda_nvrtc-windows-x86_64-12.9.86-archive.zip failed rc:1 boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=cuda-equiv
APPRES side=win app=cg_t verdict=PASS rc=0 secs=13 quiet=1 note=- boot=desktop guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
