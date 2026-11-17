APPRES side=host app=blender_cycles verdict=PASS rc=0 secs=5 quiet=0 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=llama_cpp_gen verdict=PASS rc=0 secs=1 quiet=0 note=warn:done_getting_tensors: tensor 'token_embd.weight' (q4_K) (and 0 others) cannot be used with preferred buffer type CUDA_Host, using CPU instead gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPDIG side=host app=llama_cpp_gen OUTSHA llama_cpp 30451301af8e011d
APPRES side=host app=llama_bench verdict=PASS rc=0 secs=1 quiet=0 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=torch_correct verdict=PASS rc=0 secs=5 quiet=1 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPDIG side=host app=torch_correct DIGEST cnn_train_step 763c693a5a53f948
APPRES side=host app=hf_generate verdict=PASS rc=0 secs=5 quiet=0 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPDIG side=host app=hf_generate OUTSHA hf_generate 0d973108a6251e14
APPRES side=host app=cupy verdict=PASS rc=0 secs=2 quiet=0 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=nvenc_h264 verdict=FAIL rc=0 secs=2 quiet=0 note=gpu-use:FAIL:[app] FAIL: the stream was not encoded by h264_nvenc gpu_use=FAIL gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=nvenc_hevc verdict=FAIL rc=0 secs=3 quiet=0 note=gpu-use:FAIL:[app] FAIL: the stream was not encoded by hevc_nvenc gpu_use=FAIL gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=nvdec_h264 verdict=FAIL rc=0 secs=4 quiet=0 note=gpu-use:FAIL:[app] FAIL: the decoder outputs a software pixel format (pix_fmt: yuv420p): decoded on the CPU gpu_use=FAIL gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=hashcat verdict=PASS rc=0 secs=15 quiet=0 note=warn:[33mThis means that hashcat cannot use the full parallel power of your device(s).[0m gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=clpeak verdict=PASS rc=0 secs=51 quiet=0 note=warn: Build Log: error: unknown target CPU 'generic' gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=vkpeak verdict=PASS rc=0 secs=348 quiet=1 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
APPRES side=host app=egl_offscreen verdict=PASS rc=0 secs=13 quiet=0 note=- gpu_use=PASS gpu_auth=guest gpu_subs=- gpu_births=-
