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
# ⊘ NEGATIVE CONTROL of the GPU-use assertion (docs/design/V3_GPU_USE_ASSERTION.md): KF_NEGCTL=1 runs the CPU form of every row whose application CAN fall
#   back to the CPU (Cycles CPU, llama.cpp -ngl 0, hashcat -D 1, clpeak on the pocl platform, vkpeak/EGL on a software device, ffmpeg software decode and
#   encode, the GPU hidden from torch/CuPy). scripts/apps/gpu_use_rows.py judges it and must say FAIL. Unset: every command below is what it always was.
if [ "${KF_NEGCTL:-0}" = 1 ]; then
  export KF_BLENDER_DEVS=CPU KF_NGL=0 KF_HASHCAT_ARGS="-D 1" KF_CLPEAK_ARGS="-p 1" KF_VKPEAK_DEV=1 KF_DEC_ARGS="-hwaccel none" KF_ENC=libx264
  export KF_EGL_ENV="__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json" KF_GPU_ENV="CUDA_VISIBLE_DEVICES="
fi

# ⊘ llama_cpp_gen's digest covers ONLY the generated text: this llama.cpp rev logs VRAM size and
#   timings between the tokens (measured 670bd310: 3 digests for one token-identical output).
# name|timeout|pass-regex|command   (pass = rc 0 AND regex matches AND no "CHECK .* FAIL")
# ⊘ nvcodec_matrix and nvml_dcgm are the open DISCOVERY rows (docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md): not in a gate list. Each prints its result
#   files between @@FILE markers (--emit) so a guest's single log carries them; `open_rows/compare_logs.py` compares a host log with a guest log.
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
torch_correct|300|TORCH_CORRECT_DONE|${KF_GPU_ENV:+env $KF_GPU_ENV }$PY $B/share/torch_correct.py
torch_ai_bench|900|CHECK bert_infer_seqs ok|${KF_GPU_ENV:+env $KF_GPU_ENV }$PY $B/share/ai_bench.py
hf_generate|600|OUTSHA|HF_MODEL=Qwen/Qwen2-0.5B-Instruct ${KF_GPU_ENV:+env $KF_GPU_ENV }$PY $B/share/hf_generate.py
cupy|300|CUPY_DONE|CUDA_PATH=$B/cuda ${KF_GPU_ENV:+env $KF_GPU_ENV }$PY $B/share/cupy_check.py
llama_cpp_gen|600|OUTSHA|$B/llama/llama-simple -m $D/qwen2.5-1.5b-instruct-q4_k_m.gguf -n 64 -ngl ${KF_NGL:-99} "Explain in three sentences why the sky is blue." > $O/llama_gen.txt 2>&1; rc=$?; cat $O/llama_gen.txt; echo "OUTSHA llama_cpp $(sed -n '/sky is blue\./,/main: decoded/p' $O/llama_gen.txt | grep -v 'main: decoded' | sed -E 's/CUDA Graph id [0-9]+ reused//g' | grep -v -E '^[a-z_]+: ' | tr -d '\n' | sha256sum | cut -c1-16)"; exit $rc
llama_bench|900|tg64|$B/llama/llama-bench -m $D/qwen2.5-1.5b-instruct-q4_k_m.gguf -ngl ${KF_NGL:-99} -p 512 -n 64 -r 2
vulkaninfo|60|NVIDIA|vulkaninfo --summary
vkpeak|900|fp32-scalar|$B/bin/vkpeak ${KF_VKPEAK_DEV:-0}
egl_offscreen|90|CHECK|${KF_EGL_ENV:+env $KF_EGL_ENV }$B/bin/egl_offscreen
clinfo|60|NVIDIA CUDA|clinfo -l; clinfo | grep -m3 -E 'Platform Name|Device Name'
clpeak|600|Global memory bandwidth|$B/bin/clpeak ${KF_CLPEAK_ARGS:-}
nvenc_h264|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v ${KF_ENC:-h264_nvenc} -preset p4 $O/nvenc_h264.mp4 2>&1 | tee $O/nvenc_h264.ffmpeg.txt | grep -v "^  " | tail -25; grep -a -m1 "^  Stream #.* -> #" $O/nvenc_h264.ffmpeg.txt; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $O/nvenc_h264.mp4 | sed 's/^/frame= /;s/$/ /'; echo "ENCODER_TAG $(ffprobe -v error -select_streams v:0 -show_entries stream_tags=encoder -of default=nw=1:nk=1 $O/nvenc_h264.mp4)"
nvenc_hevc|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v ${KF_ENC:-hevc_nvenc} -preset p4 $O/nvenc_hevc.mp4 2>&1 | tee $O/nvenc_hevc.ffmpeg.txt | grep -v "^  " | tail -25; grep -a -m1 "^  Stream #.* -> #" $O/nvenc_hevc.ffmpeg.txt; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $O/nvenc_hevc.mp4 | sed 's/^/frame= /;s/$/ /'; echo "ENCODER_TAG $(ffprobe -v error -select_streams v:0 -show_entries stream_tags=encoder -of default=nw=1:nk=1 $O/nvenc_hevc.mp4)"
nvdec_h264|180|frame= *600 |ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=20 -c:v libx264 -pix_fmt yuv420p $O/x264.mp4 >/dev/null 2>&1; ffmpeg -hide_banner -v verbose ${KF_DEC_ARGS:--hwaccel cuda -hwaccel_output_format cuda} -i $O/x264.mp4 -f null - > $O/nvdec.txt 2>&1; rc=$?; tr "\r" "\n" < $O/nvdec.txt | grep -E "frame=|rror|hwaccel|cuvid" | tail -4; grep -a "Reinit context" $O/nvdec.txt | tail -3; grep -q "Failed setup for format cuda" $O/nvdec.txt && { echo "NVDEC_NOT_USED (software fallback)"; exit 3; }; exit $rc
nvcodec_matrix|3000|NVCODEC_MATRIX_SUMMARY pass=[0-9]+ not_supported=[0-9]+ fail=0 mismatch=0|$PY $B/share/open_rows/nvcodec_matrix.py run --out $O/nvcodec_matrix --bundle $B --emit
nvml_dcgm|3300|NVML_DCGM_SUMMARY nvml_fields_ok=|$PY $B/share/open_rows/dcgm_nvml.py run --out $O/nvml_dcgm --bundle $B --diag-timeout 1500 --emit
hashcat|300|e4726719b68b205913167f0975d977ee:kayfab|hashcat --potfile-disable -O -m 0 -a 3 ${KF_HASHCAT_ARGS:-} e4726719b68b205913167f0975d977ee '?l?l?l?l?l?l' 2>&1 | tee $O/hashcat_full.txt | tail -30; grep -a -E '^(\* Device #|CUDA API|OpenCL API|HIP API)' $O/hashcat_full.txt || true
blender_cycles|900|BLENDER_OK OPTIX|for dev in ${KF_BLENDER_DEVS:-CUDA OPTIX}; do $D/blender/blender -b --factory-startup --python $B/share/blender_render.py -- $dev $O/blender_$dev.png 2>&1 | grep -E "BLENDER_|Error|error|Fra:1 .*Finished|[Ff]alling back" | tail -6; done
geekbench_gpu|1200|Uploading results|cd $D/geekbench && ./geekbench6 --gpu OpenCL 2>&1 | tail -60
EOF
)
# DISCOVERY rows report and never gate: they are in `list` (so the names are known) but not in `listall` / `all`, and in no gate list.
# vkcts: the open Vulkan CTS subset (docs/design/V3_VULKAN_CTS_ROW.md). Needs `vkcts.py build` once ($B/vkcts) and the tool dir $B/vkcts_tool.
# vllm, tensorrt, jax: the open ML rows (docs/design/V3_OPEN_ML_ROWS.md). Each needs `mlrows.py setup` once ($B/mlrows: one venv per row from a hashed lock) and the tool dir $B/mlrows_tool.
DISCOVERY=$(cat <<'EOF'
vkcts|2700|VKCTS_SUMMARY|python3 $B/vkcts_tool/vkcts.py run --side $SIDE --artifact $B/vkcts --out $O/vkcts
vllm|1800|VLLM_SUMMARY status=PASS|python3 -B -I $B/mlrows_tool/mlrows.py run vllm --root $B/mlrows --out $O/vllm
tensorrt|1500|TENSORRT_SUMMARY status=PASS|python3 -B -I $B/mlrows_tool/mlrows.py run tensorrt --root $B/mlrows --out $O/tensorrt
jax|1200|JAX_SUMMARY status=PASS|python3 -B -I $B/mlrows_tool/mlrows.py run jax --root $B/mlrows --out $O/jax
EOF
)
[ "${1:-}" = list ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut -d'|' -f1 | tr '\n' ' '; echo; exit 0; }
[ "${1:-}" = listall ] && { echo "$APPS" | cut -d'|' -f1 | tr '\n' ' '; echo; exit 0; }
[ "${1:-}" = all ] && set -- $(echo "$APPS" | cut -d'|' -f1)

for app in "$@"; do
  row=$({ echo "$APPS"; echo "$DISCOVERY"; } | awk -F'|' -v a="$app" '$1==a' | head -1)
  [ -n "$row" ] || { echo "APPRES side=$SIDE app=$app verdict=NOTRUN rc=- secs=0 quiet=- note=unknown-app"; continue; }
  IFS='|' read -r name tmo rx cmd <<<"$row"
  cmd=${cmd//\$SIDE/$SIDE}; cmd=${cmd//\$S/$S}; cmd=${cmd//\$B/$B}; cmd=${cmd//\$D/$D}; cmd=${cmd//\$O/$O}; cmd=${cmd//\$PY/$PY}
  log=$O/$name.log
  { echo "=== $name side=$SIDE start=$(date -Is) host=$(hostname) cmd: $cmd"; } > "$log"
  t0=$(date +%s)
  # setsid + timeout on the whole group so a spawned child (simpleIPC, ffmpeg) cannot outlive it
  timeout -k 15 "$tmo" setsid bash -c "$cmd" >> "$log" 2>&1 < /dev/null
  rc=$?; t1=$(date +%s)
  quiet=$(( t1 - $(stat -c %Y "$log") ))
  echo "=== end rc=$rc secs=$((t1-t0)) $(date -Is)" >> "$log"
  # ⊘ the header and end lines (`=== ...`) are dropped BEFORE the first match is taken: the header carries the row's own command (`--diag-timeout`, `grep -E "Error|error"`, `ffprobe -v error`),
  #   and `grep -m1 ... | grep -v` took that line first and then dropped it, so the note of such a row was always `-` (measured 2026-11-20: nvml_dcgm on an image without the venv)
  note=$(grep -a -v -E '^=== |errors: 0|CHECK .* ok' "$log" | grep -a -m1 -iE 'error|fail|illegal|cannot|unable|Xid|timeout|abort|segmentation|core dumped|no CUDA|not found|No such' | tr -s ' \t' ' ' | cut -c1-160 | tr '|' '/')
  if [ $rc -eq 124 ] || [ $rc -eq 137 ]; then v=TIMEOUT
  elif grep -qaE 'command not found|No such file or directory' "$log" && ! grep -qaE "$rx" "$log"; then v=NOTRUN
  elif [ $rc -eq 0 ] && grep -qaE "$rx" "$log" && ! grep -qaE '^CHECK .*FAIL' "$log"; then v=PASS; note=${note:+warn:$note}
  else v=FAIL; fi
  [ "$rx" = CHECK ] && [ $v = PASS ] && ! grep -qaE '^CHECK .*ok' "$log" && v=FAIL
  # ⊘ measured 670bd310 (vh nb2): clpeak printed its bandwidth line (the old predicate) while its GPU
  # integer / transfer / launch-latency tests aborted with `clFinish (-36)` after a host Xid 31 —
  # a false PASS. clpeak reports a failed test as "Tests skipped"; the host baseline has none.
  [ "$name" = clpeak ] && [ $v = PASS ] && grep -qaE 'clFinish \(-|Tests skipped' "$log" && { v=FAIL; note="clpeak: $(grep -aE -c 'Tests skipped' "$log") test groups skipped ($(grep -aoE -m1 'cl[A-Za-z]+ \(-[0-9]+\)' "$log"))"; }
  # an app may state its own note in one line, `APPNOTE <text>` (case names in its log would match the generic grep above)
  appnote=$(grep -a -m1 '^APPNOTE ' "$log" | cut -c9-160 | tr '|' '/'); [ -n "$appnote" ] && note=$appnote
  # an app may say it was NOT RUN for a named reason (the hardware or its own configuration cannot take it): `APPNOTRUN <reason>` with exit 0; never a result about the GPU
  appnr=$(grep -a -m1 '^APPNOTRUN ' "$log" | cut -c11-160 | tr '|' '/'); [ -n "$appnr" ] && [ "$rc" -eq 0 ] && { v=NOTRUN; note="not-run:$appnr"; }
  echo "APPRES side=$SIDE app=$name verdict=$v rc=$rc secs=$((t1-t0)) quiet=$quiet note=${note:--}"
  grep -a -E '^(OUTSHA|DIGEST) ' "$log" | sed "s/^/APPDIG side=$SIDE app=$name /"
done
