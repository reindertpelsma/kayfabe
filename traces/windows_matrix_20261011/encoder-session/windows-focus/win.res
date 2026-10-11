APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=14 quiet=9 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=nvenc_h264 verdict=PASS rc=0 secs=37 quiet=27 note=- boot=g1 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videoencode tier=1 cat=video
APPRES side=win app=nvenc_hevc verdict=PASS rc=0 secs=29 quiet=0 note=- boot=g2 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videoencode tier=1 cat=video
APPRES side=win app=nvenc_av1 verdict=PASS rc=0 secs=40 quiet=30 note=- boot=g3 guest_tdr=3 wer=0 gsp_cycles=1 proof=pdh:videoencode tier=1 cat=video
