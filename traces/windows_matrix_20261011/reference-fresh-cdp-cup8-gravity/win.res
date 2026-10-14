APPRES side=win app=cdp_quicksort verdict=PASS rc=0 secs=17 quiet=1 note=warn:CDP_CHILD_LAUNCHES 490 CUDA_ERROR 0 boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=3 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=gravitymark_vk verdict=TIMEOUT rc=124 secs=603 quiet=603 note=KFWAIT gravitymark_vk STILL_RUNNING_AFTER 600 s boot=g1 guest_tdr=0 wer=0 gsp_cycles=- proof=- tier=1 cat=vulkan
