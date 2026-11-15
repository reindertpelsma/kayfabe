APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=8 quiet=7 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi
APPRES side=win app=deviceQuery_demo verdict=PASS rc=0 secs=10 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-demo
APPRES side=win app=matmul_t verdict=PASS rc=0 secs=33 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=cuda-equiv
APPRES side=win app=cup8_ladder verdict=PASS rc=0 secs=8 quiet=2 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=cuda-cup8-readback tier=1 cat=cuda-ladder
APPRES side=win app=dxprobe_d3d11 verdict=PASS rc=0 secs=7 quiet=1 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
APPRES side=win app=glgears_wgl verdict=PASS rc=0 secs=16 quiet=11 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=opengl
APPRES side=win app=vulkaninfo verdict=PASS rc=0 secs=8 quiet=0 note=warn:WARNING: [Loader Message] Code 0 : windows_read_data_files_in_registry: Registry lookup failed to get layer manifest files. boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan
APPRES side=win app=torch_correct verdict=PASS rc=0 secs=37 quiet=4 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=torch
APPDIG side=win app=torch_correct DIGEST cnn_train_step 763c693a5a53f948
APPRES side=win app=edge_webgl verdict=PASS rc=0 secs=34 quiet=2 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=browser
APPRES side=win app=dxprobe_d3d12 verdict=PASS rc=0 secs=12 quiet=1 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=d3d
