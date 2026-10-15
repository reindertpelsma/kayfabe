APPRES side=win app=edge_video_h264 verdict=PASS rc=0 secs=31 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=gravitymark_d3d12 verdict=FAIL rc=0 secs=183 quiet=1 note=no-gpu-proof boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=d3d
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=5 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
APPRES side=win app=gravitymark_vk verdict=PASS rc=0 secs=180 quiet=180 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=pdh:copy tier=1 cat=vulkan
