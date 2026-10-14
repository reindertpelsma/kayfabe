APPRES side=win app=nvenc_av1 verdict=PASS rc=0 secs=29 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videoencode,smi tier=1 cat=video
APPRES side=win app=nvenc_h264 verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videoencode,smi tier=1 cat=video
APPRES side=win app=nvenc_hevc verdict=PASS rc=0 secs=9 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videoencode,smi tier=1 cat=video
APPRES side=win app=nvdec_h264 verdict=PASS rc=0 secs=22 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode,smi tier=1 cat=video
APPRES side=win app=scale_cuda_nvenc verdict=PASS rc=0 secs=22 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=pdh:videodecode+videoencode,smi tier=2 cat=video
APPRES side=win app=managed_t verdict=PASS rc=0 secs=16 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
