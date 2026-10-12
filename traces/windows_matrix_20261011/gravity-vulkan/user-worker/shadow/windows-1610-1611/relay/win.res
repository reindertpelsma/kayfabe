APPRES side=win app=edge_video_h264 verdict=PASS rc=0 secs=40 quiet=3 note=- boot=g1 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=gravitymark_d3d12 verdict=FAIL rc=1 secs=19 quiet=0 note=rc:1,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image boot=g2 guest_tdr=3 wer=0 gsp_cycles=1 proof=none:pdh:Cuda|Compute|3D|Copy tier=1 cat=d3d
APPRES side=win app=gravitymark_vk verdict=BOOT_FAIL rc=- secs=0 quiet=- note=guest-did-not-start:pre-guest hook failed rc:1 boot=g3 guest_tdr=0 wer=0 gsp_cycles=- proof=- tier=1 cat=vulkan
