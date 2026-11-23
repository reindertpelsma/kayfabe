#!/bin/bash
# after host G1 finished: (a) the host control of the vkcts candidates (second host run), (b) the guest repeat, (c) the guest positives, (d) the negative controls (host, guest). vkpeak (7.4 min per side) is left out of the controls.
while pgrep -f "apps_matrix.sh host G1" >/dev/null; do sleep 3; done
ROWS="blender_cycles llama_cpp_gen llama_bench torch_correct hf_generate cupy nvenc_h264 nvenc_hevc nvdec_h264 hashcat clpeak vkpeak egl_offscreen"
NEG="blender_cycles llama_cpp_gen llama_bench hashcat clpeak egl_offscreen nvdec_h264 nvenc_h264 nvenc_hevc"
echo "CHAIN5 host vkcts control $(date -u +%T)"; /workspace/run.sh host V1c vkcts
echo "CHAIN5 guest vkcts repeat $(date -u +%T)"; APPS_APP_TMO=3000 /workspace/run.sh guest V1b vkcts
echo "CHAIN5 guest positives $(date -u +%T)"; APPS_PER_BOOT=100 /workspace/run.sh guest G1 $ROWS
echo "CHAIN5 host negctl $(date -u +%T)"; KF_NEGCTL=1 /workspace/run.sh host G1n $NEG
echo "CHAIN5 guest negctl $(date -u +%T)"; KF_NEGCTL=1 APPS_PER_BOOT=100 APPS_NO_ISOLATE=1 /workspace/run.sh guest G1n $NEG
echo "CHAIN5 done $(date -u +%T)"
