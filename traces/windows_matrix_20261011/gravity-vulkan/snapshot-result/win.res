APPRES side=win app=nbody_demo_gl verdict=FAIL rc=0 secs=29 quiet=2 note=no-gpu-proof boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=cuda-demo
APPRES side=win app=gravitymark_vk verdict=TIMEOUT rc=124 secs=603 quiet=1 note=KFWAIT gravitymark_vk STILL_RUNNING_AFTER 600 s boot=g2 guest_tdr=3 wer=0 gsp_cycles=1 proof=- tier=1 cat=vulkan
