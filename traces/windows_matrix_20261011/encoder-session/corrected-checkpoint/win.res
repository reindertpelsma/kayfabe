APPRES side=win app=oceanFFT_demo verdict=PASS rc=0 secs=37 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-oceanfft-readback tier=1 cat=cuda-demo
APPRES side=win app=randomFog_demo verdict=PASS rc=0 secs=31 quiet=25 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-randomfog-readback tier=1 cat=cuda-demo
APPRES side=win app=hf_generate verdict=PASS rc=0 secs=112 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=ai
APPDIG side=win app=hf_generate OUTSHA hf_generate 0d973108a6251e14
APPRES side=win app=cdp_quicksort verdict=PASS rc=0 secs=8 quiet=0 note=warn:CDP_CHILD_LAUNCHES 490 CUDA_ERROR 0 boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=vectorAdd_demo verdict=PASS rc=0 secs=4 quiet=3 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-vectoradd-readback tier=1 cat=cuda-demo
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=stream_perthread_t verdict=TIMEOUT rc=124 secs=121 quiet=46 note=timeout boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=- tier=1 cat=stream-probe
APPRES side=win app=cupy_sha256 verdict=PASS rc=0 secs=149 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_cg verdict=PASS rc=0 secs=7 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_asynccopy verdict=PASS rc=0 secs=3 quiet=0 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cupy_check verdict=PASS rc=0 secs=9 quiet=2 note=- boot=g3 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=torch
