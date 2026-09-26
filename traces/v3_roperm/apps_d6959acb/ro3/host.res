APPRES side=host app=torch_correct verdict=PASS rc=0 secs=10 quiet=1 note=-
APPDIG side=host app=torch_correct DIGEST cnn_train_step 763c693a5a53f948
APPRES side=host app=llama_cpp_gen verdict=PASS rc=0 secs=3 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead
APPDIG side=host app=llama_cpp_gen OUTSHA llama_cpp 30451301af8e011d
APPRES side=host app=llama_bench verdict=PASS rc=0 secs=4 quiet=0 note=-
APPRES side=host app=vkpeak verdict=PASS rc=0 secs=360 quiet=0 note=-
APPRES side=host app=nvenc_h264 verdict=PASS rc=0 secs=6 quiet=0 note=-
APPRES side=host app=nvenc_hevc verdict=PASS rc=0 secs=7 quiet=0 note=-
APPRES side=host app=nvdec_h264 verdict=PASS rc=0 secs=8 quiet=0 note=-
APPRES side=host app=vectorAdd verdict=PASS rc=0 secs=2 quiet=0 note=-
APPRES side=host app=simpleAtomicIntrinsics verdict=PASS rc=0 secs=3 quiet=1 note=-
