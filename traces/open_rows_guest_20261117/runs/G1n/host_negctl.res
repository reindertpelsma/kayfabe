NEGCTL app=blender_cycles control=CONTROL_OK judged=FAIL why=[app] FAIL: CUDA: BLENDER_OK missing (the row dropped its device proof); OPTIX: BLENDER_OK missing (the row dropped its device proof); a CPU
NEGCTL app=llama_cpp_gen control=CONTROL_OK judged=FAIL why=[app] FAIL: offloaded 0/29 layers: the model ran on the CPU
APPDIG side=host app=llama_cpp_gen OUTSHA llama_cpp bc77046cb68ce1b9
NEGCTL app=llama_bench control=CONTROL_OK judged=FAIL why=[app] FAIL: llama-bench ran on backend 'CUDA' with ngl 0
NEGCTL app=hashcat control=CONTROL_OK judged=FAIL why=[app] FAIL: hashcat used a non-NVIDIA device: #3 pthread-AMD EPYC 7B13 64-Core Processor
NEGCTL app=clpeak control=CONTROL_OK judged=FAIL why=[app] FAIL: no NVIDIA device block holds a bandwidth result (devices: pthread-AMD EPYC 7B13 64-Core )
NEGCTL app=egl_offscreen control=CONTROL_OK judged=FAIL why=[app] FAIL: no `[egl] renderer:` line
NEGCTL app=nvdec_h264 control=CONTROL_OK judged=FAIL why=[app] FAIL: the decoder outputs a software pixel format (pix_fmt: yuv420p): decoded on the CPU
NEGCTL app=nvenc_h264 control=CONTROL_OK judged=FAIL why=[app] FAIL: the stream was not encoded by h264_nvenc
NEGCTL app=nvenc_hevc control=CONTROL_OK judged=FAIL why=[app] FAIL: the stream was not encoded by hevc_nvenc
