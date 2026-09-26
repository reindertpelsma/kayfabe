#!/usr/bin/env bash
# run_apps.sh <side> <app>... — run app-matrix workloads ON THIS MACHINE (the bench host as the
# bare-metal baseline, or inside the kf3 fat guest), one process per app, as root.
#   side: host | guest (a label only — the commands are identical on both sides)
#   app : a name from `run_apps.sh list`, or `all`
# Layout (same on both sides): /opt/apps/{bundle,venv,data}; logs → /opt/apps/out/<side>/<app>.log
# Emits one line per app:
#   APPRES side=<s> app=<a> verdict=PASS|FAIL|TIMEOUT|NOTRUN rc=<rc> secs=<n> quiet=<s> note=<...>
# ⊘ TIMEOUT carries `quiet=` = seconds since the log last grew before the kill, so a SLOW app
#   (still printing) is distinguishable from a HUNG one (silent for most of its budget).
# ⊘ NOTRUN = a precondition was missing (binary/model absent) — never a result about the GPU.
set -uo pipefail
SIDE=${1:?side}; shift
A=${APPS_ROOT:-/opt/apps}; B=$A/bundle; D=$A/data; PY=$A/venv/bin/python
O=$A/out/$SIDE; mkdir -p "$O"
export LD_LIBRARY_PATH=$B/lib:$B/llama${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export HF_HOME=$D/hf HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1
S=$B/samples

# name|timeout|pass-regex|command   (pass = rc 0 AND regex matches AND no "CHECK .* FAIL")
# ⊘ many cuda-samples verify on the CPU and signal only through their EXIT CODE (no PASS line);
#   their regex is then a completion marker from the last verified step, and rc 0 is the verdict.
APPS=$(cat <<'EOF'
nvidia_smi|60|RTX|nvidia-smi
deviceQuery|60|Result = PASS|$S/deviceQuery
vectorAdd|60|Test PASSED|$S/vectorAdd
vectorAddDrv|60|Result = PASS|cd $S && ./vectorAddDrv
matrixMul|90|Result = PASS|$S/matrixMul
matrixMulDrv|90|Result = PASS|cd $S && ./matrixMulDrv
bandwidthTest|120|Result = PASS|$S/bandwidthTest
simpleStreams|90|4 streams:|$S/simpleStreams
asyncAPI|60|CPU executed [0-9]+ iterations|$S/asyncAPI
simpleAtomicIntrinsics|60|completed, returned OK|$S/simpleAtomicIntrinsics
simpleCallback|60|Success|$S/simpleCallback
simpleOccupancy|60|Test PASSED|$S/simpleOccupancy
simpleZeroCopy|60|Releasing CPU memory|$S/simpleZeroCopy
simpleCooperativeGroups|60|\.\.\.Done\.|$S/simpleCooperativeGroups
concurrentKernels|60|Test passed|$S/concurrentKernels
simpleIPC|90|Process 0 complete|$S/simpleIPC
UnifiedMemoryStreams|120|All Done|$S/UnifiedMemoryStreams
UnifiedMemoryPerf|300|^16384|$S/UnifiedMemoryPerf
conjugateGradientUM|120|Test Summary:  Error amount = 0|$S/conjugateGradientUM
cudaTensorCoreGemm|120|TFLOPS|$S/cudaTensorCoreGemm
bf16TensorCoreGemm|120|TFLOPS|$S/bf16TensorCoreGemm
globalToShmemAsyncCopy|120|Result = PASS|$S/globalToShmemAsyncCopy
cdpSimpleQuicksort|60|Validating results: OK|$S/cdpSimpleQuicksort
graphMemoryNodes|60|PASSED|$S/graphMemoryNodes
simpleCudaGraphs|60|final reduced sum|$S/simpleCudaGraphs
simpleCUBLAS|60|test passed|$S/simpleCUBLAS
simpleCUFFT|60|Transforming signal back|$S/simpleCUFFT
conjugateGradient|60|Test Summary:  Error amount = 0|$S/conjugateGradient
MersenneTwisterGP11213|60|L1 norm: 0\.0+E\+00|$S/MersenneTwisterGP11213
reduction|120|Test passed|$S/reduction
sortingNetworks|120|keys and values array: OK|$S/sortingNetworks
scan|120|Results Match|$S/scan
histogram|120|Test passed|$S/histogram
BlackScholes|60|Test passed|$S/BlackScholes
fastWalshTransform|60|Test passed|$S/fastWalshTransform
transpose|120|Test passed|$S/transpose
stream_triad|90|CHECK|$B/bin/stream_triad
reduce|90|CHECK|$B/bin/reduce
nbody|90|CHECK|$B/bin/nbody
blackscholes|90|CHECK|$B/bin/blackscholes
mandelbrot|90|CHECK|$B/bin/mandelbrot
conv2d|90|CHECK|$B/bin/conv2d
sgemm_cublas|90|CHECK|$B/bin/sgemm_cublas
fft_cufft|90|CHECK|$B/bin/fft_cufft
sha256|90|CHECK|$B/bin/sha256
memcpy2d|90|CHECK|$B/bin/memcpy2d
attach_verify|90|RESULT: CORRECT|$B/bin/attach_verify
stream_default|60|STREAM_PROBE_DONE default rc=0|$B/bin/stream_probe default
stream_created|60|STREAM_PROBE_DONE created rc=0|$B/bin/stream_probe created
stream_nonblocking|60|STREAM_PROBE_DONE nonblocking rc=0|$B/bin/stream_probe nonblocking
stream_perthread|60|STREAM_PROBE_DONE perthread rc=0|$B/bin/stream_probe perthread
stream_two|60|STREAM_PROBE_DONE two rc=0|$B/bin/stream_probe two
stream_created2nd|60|STREAM_PROBE_DONE created2nd rc=0|$B/bin/stream_probe created2nd
gpu_burn|180|GPU 0: OK|cd $B/gpu-burn && ./gpu_burn 60
torch_correct|300|TORCH_CORRECT_DONE|$PY $B/share/torch_correct.py
torch_ai_bench|900|CHECK bert_infer_seqs ok|$PY $B/share/ai_bench.py
hf_generate|600|OUTSHA|HF_MODEL=Qwen/Qwen2-0.5B-Instruct $PY $B/share/hf_generate.py
cupy|300|CUPY_DONE|CUDA_PATH=$B/cuda $PY $B/share/cupy_check.py
llama_cpp_gen|600|OUTSHA|$B/llama/llama-simple -m $D/qwen2.5-1.5b-instruct-q4_k_m.gguf -n 64 -ngl 99 "Explain in three sentences why the sky is blue." > $O/llama_gen.txt 2>&1; rc=$?; cat $O/llama_gen.txt; echo "OUTSHA llama_cpp $(grep -v -E '^(llama_|load|print_info|main:|ggml_|common_|\.|system_info|sampler|generate|init|build|graph|decode|\s*$)' $O/llama_gen.txt | sha256sum | cut -c1-16)"; exit $rc
llama_bench|900|tg64|$B/llama/llama-bench -m $D/qwen2.5-1.5b-instruct-q4_k_m.gguf -ngl 99 -p 512 -n 64 -r 2
vulkaninfo|60|NVIDIA|vulkaninfo --summary
vkpeak|900|fp32-scalar|$B/bin/vkpeak 0
egl_offscreen|90|CHECK|$B/bin/egl_offscreen
clinfo|60|NVIDIA CUDA|clinfo -l; clinfo | grep -m3 -E 'Platform Name|Device Name'
clpeak|600|Global memory bandwidth|$B/bin/clpeak
nvenc_h264|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v h264_nvenc -preset p4 $O/nvenc_h264.mp4 2>&1 | grep -v "^  " | tail -25; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $O/nvenc_h264.mp4 | sed 's/^/frame= /;s/$/ /'
nvenc_hevc|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v hevc_nvenc -preset p4 $O/nvenc_hevc.mp4 2>&1 | grep -v "^  " | tail -25; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $O/nvenc_hevc.mp4 | sed 's/^/frame= /;s/$/ /'
nvdec_h264|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v libx264 -pix_fmt yuv420p $O/x264.mp4 >/dev/null 2>&1; ffmpeg -hide_banner -hwaccel cuda -hwaccel_output_format cuda -i $O/x264.mp4 -f null - > $O/nvdec.txt 2>&1; rc=$?; tr "\r" "\n" < $O/nvdec.txt | grep -E "frame=|rror|hwaccel|cuvid" | tail -4; grep -q "Failed setup for format cuda" $O/nvdec.txt && { echo "NVDEC_NOT_USED (software fallback)"; exit 3; }; exit $rc
hashcat|300|e4726719b68b205913167f0975d977ee:kayfab|hashcat --potfile-disable -O -m 0 -a 3 e4726719b68b205913167f0975d977ee '?l?l?l?l?l?l' 2>&1 | tail -30
blender_cycles|900|BLENDER_OK OPTIX|for dev in CUDA OPTIX; do $D/blender/blender -b --factory-startup --python $B/share/blender_render.py -- $dev $O/blender_$dev.png 2>&1 | grep -E "BLENDER_|Error|error|Fra:1 .*Finished" | tail -6; done
geekbench_gpu|1200|Uploading results|cd $D/geekbench && ./geekbench6 --gpu OpenCL 2>&1 | tail -60
EOF
)
[ "${1:-}" = list ] && { echo "$APPS" | cut -d'|' -f1 | tr '\n' ' '; echo; exit 0; }
[ "${1:-}" = all ] && set -- $(echo "$APPS" | cut -d'|' -f1)

for app in "$@"; do
  row=$(echo "$APPS" | awk -F'|' -v a="$app" '$1==a' | head -1)
  [ -n "$row" ] || { echo "APPRES side=$SIDE app=$app verdict=NOTRUN rc=- secs=0 quiet=- note=unknown-app"; continue; }
  IFS='|' read -r name tmo rx cmd <<<"$row"
  cmd=${cmd//\$S/$S}; cmd=${cmd//\$B/$B}; cmd=${cmd//\$D/$D}; cmd=${cmd//\$O/$O}; cmd=${cmd//\$PY/$PY}
  log=$O/$name.log
  { echo "=== $name side=$SIDE start=$(date -Is) host=$(hostname) cmd: $cmd"; } > "$log"
  t0=$(date +%s)
  # setsid + timeout on the whole group so a spawned child (simpleIPC, ffmpeg) cannot outlive it
  timeout -k 15 "$tmo" setsid bash -c "$cmd" >> "$log" 2>&1 < /dev/null
  rc=$?; t1=$(date +%s)
  quiet=$(( t1 - $(stat -c %Y "$log") ))
  echo "=== end rc=$rc secs=$((t1-t0)) $(date -Is)" >> "$log"
  note=$(grep -a -m1 -iE 'error|fail|illegal|cannot|unable|Xid|timeout|abort|segmentation|core dumped|no CUDA|not found|No such' "$log" | grep -v -E '^=== |errors: 0|CHECK .* ok' | head -1 | tr -s ' \t' ' ' | cut -c1-160 | tr '|' '/')
  if [ $rc -eq 124 ] || [ $rc -eq 137 ]; then v=TIMEOUT
  elif grep -qaE 'command not found|No such file or directory' "$log" && ! grep -qaE "$rx" "$log"; then v=NOTRUN
  elif [ $rc -eq 0 ] && grep -qaE "$rx" "$log" && ! grep -qaE '^CHECK .*FAIL' "$log"; then v=PASS; note=${note:+warn:$note}
  else v=FAIL; fi
  [ "$rx" = CHECK ] && [ $v = PASS ] && ! grep -qaE '^CHECK .*ok' "$log" && v=FAIL
  echo "APPRES side=$SIDE app=$name verdict=$v rc=$rc secs=$((t1-t0)) quiet=$quiet note=${note:--}"
  grep -a -E '^(OUTSHA|DIGEST) ' "$log" | sed "s/^/APPDIG side=$SIDE app=$name /"
done
