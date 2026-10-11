APPRES side=win app=d3d11va_h264 verdict=PASS rc=0 secs=27 quiet=17 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode,smi tier=1 cat=video
APPRES side=win app=vulkan_video_decode verdict=PASS rc=0 secs=40 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode,smi tier=3 cat=video
APPRES side=win app=edge_video_h264 verdict=PASS rc=0 secs=29 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode tier=1 cat=video
APPRES side=win app=edge_video_vp9 verdict=PASS rc=0 secs=28 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode tier=2 cat=video
APPRES side=win app=dxva2_h264 verdict=PASS rc=0 secs=59 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode,smi tier=2 cat=video
