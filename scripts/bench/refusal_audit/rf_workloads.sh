#!/usr/bin/env bash
# rf_workloads.sh <side> <outdir> <workload|all>... — run the V3 refusal-audit workloads ON THIS
# MACHINE (the bench host = bare-metal baseline, or inside the kf3 fat guest) under the nvdiff
# LD_PRELOAD recorder, one process tree per workload. Identical on both sides.
#
#   <outdir>/<workload>/<side>_r1.jsonl   every /dev/nvidia* ioctl, params before+after (nvdiff_shim)
#   <outdir>/<workload>/<side>.log        the workload's own output + an exit line
#   RFRES side= w= rc= secs= recs= verdict=   one line per workload on stdout
#
# env: RF_SHIM (default /opt/rf/nvdiff_shim.so), RF_BIN (/opt/rf: cup2/3/8, blocksync, torch_train.py)
# ⊘ A zero-record capture is UNMEASURED, never "no ioctls": it is flagged recs=0 verdict=NOCAPTURE.
set -uo pipefail
SIDE=${1:?side}; OUT=${2:?outdir}; shift 2
A=${APPS_ROOT:-/opt/apps}; B=$A/bundle; D=$A/data; PY=$A/venv/bin/python; S=$B/samples
L=${RF_BIN:-/opt/rf}; SHIM=${RF_SHIM:-$L/nvdiff_shim.so}
export LD_LIBRARY_PATH=$B/lib:$B/llama${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export HF_HOME=$D/hf HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1
# name|timeout|command   (the pass regex is advisory here: the audit reads statuses, not verdicts)
W=$(cat <<'EOT'
cup2|120|CE rv=0xabcd1234|$L/cup2
cup3|120|KERNEL rv=43 want=43 -> PASS|$L/cup3
cup8|300|CUP8 VERDICT: PASS|$L/cup8
blocksync|120|BLOCKSYNC_DONE ok|$L/blocksync
torch_correct|300|TORCH_CORRECT_DONE|$PY $B/share/torch_correct.py
torch_train|300|TORCH_TRAIN_DONE|$PY $L/torch_train.py
llama_gen|300|main: decoded|$B/llama/llama-simple -m $D/qwen2.5-1.5b-instruct-q4_k_m.gguf -n 64 -ngl 99 "Explain in three sentences why the sky is blue."
clpeak|900|Global memory bandwidth|$B/bin/clpeak
vulkaninfo|60|NVIDIA|vulkaninfo --summary
vkpeak|900|fp32-scalar|$B/bin/vkpeak 0
egl_offscreen|90|CHECK|$B/bin/egl_offscreen
nvenc_h264|180|frame=|ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=10 -c:v h264_nvenc -preset p4 $V/nvenc_h264.mp4 2>&1 | tail -5; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $V/nvenc_h264.mp4 | sed 's/^/frame= /'
nvenc_hevc|180|frame=|ffmpeg -y -hide_banner -nostats -f lavfi -i testsrc=size=1280x720:rate=30:duration=10 -c:v hevc_nvenc -preset p4 $V/nvenc_hevc.mp4 2>&1 | tail -5; ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 $V/nvenc_hevc.mp4 | sed 's/^/frame= /'
nvdec_h264|180|frame=|[ -s $V/x264.mp4 ] || ffmpeg -y -hide_banner -f lavfi -i testsrc=size=1280x720:rate=30:duration=10 -c:v libx264 -pix_fmt yuv420p $V/x264.mp4 >/dev/null 2>&1; ffmpeg -hide_banner -hwaccel cuda -hwaccel_output_format cuda -i $V/x264.mp4 -f null - 2>&1 | tr "\r" "\n" | grep -E "frame=|rror|hwaccel|cuvid" | tail -4
nvidia_smi|60|Driver Version|nvidia-smi; nvidia-smi -q >/dev/null; timeout -s INT 6 nvidia-smi -l 1 >/dev/null; echo NVSMI_LOOP_RC=$?
gpu_burn|120|GPU 0: OK|cd $B/gpu-burn && ./gpu_burn 10
simpleStreams|120|4 streams:|$S/simpleStreams
simpleCudaGraphs|90|final reduced sum|$S/simpleCudaGraphs
graphMemoryNodes|90|PASSED|$S/graphMemoryNodes
simpleIPC|120|Process 0 complete|$S/simpleIPC
asyncAPI|90|CPU executed|$S/asyncAPI
simpleCallback|90|Success|$S/simpleCallback
EOT
)
[ "${1:-}" = list ] && { echo "$W" | cut -d'|' -f1 | tr '\n' ' '; echo; exit 0; }
[ -f "$SHIM" ] || { echo "RFRES side=$SIDE w=- verdict=NOSHIM shim=$SHIM"; exit 2; }
V=$OUT/_video; mkdir -p "$V"
[ "${1:-}" = all ] && set -- $(echo "$W" | cut -d'|' -f1)
for w in "$@"; do
  row=$(echo "$W" | awk -F'|' -v a="$w" '$1==a' | head -1)
  [ -n "$row" ] || { echo "RFRES side=$SIDE w=$w verdict=UNKNOWN"; continue; }
  IFS='|' read -r name tmo rx cmd <<<"$row"
  cmd=${cmd//\$S/$S}; cmd=${cmd//\$B/$B}; cmd=${cmd//\$D/$D}; cmd=${cmd//\$L/$L}; cmd=${cmd//\$V/$V}; cmd=${cmd//\$PY/$PY}
  d=$OUT/$name; mkdir -p "$d"; cap=$d/${SIDE}_r1.jsonl; log=$d/$SIDE.log
  rm -f "$cap"
  echo "=== $name side=$SIDE start=$(date -Is) cmd: $cmd" > "$log"
  t0=$(date +%s)
  # ⊘ setsid + timeout on the whole group; the recorder rides LD_PRELOAD into every child
  NVDIFF_OUT=$cap NVDIFF_MAXBUF=65536 LD_PRELOAD=$SHIM timeout -k 15 "$tmo" setsid bash -c "$cmd" >> "$log" 2>&1 < /dev/null
  rc=$?; t1=$(date +%s)
  echo "=== end rc=$rc secs=$((t1-t0)) $(date -Is)" >> "$log"
  n=$(cat "$cap" 2>/dev/null | wc -l)
  if [ "$n" -eq 0 ]; then v=NOCAPTURE
  elif [ $rc -eq 124 ] || [ $rc -eq 137 ]; then v=TIMEOUT
  elif [ $rc -eq 0 ] && grep -qaE "$rx" "$log" && ! grep -qaE '^CHECK .*FAIL|BLOCKSYNC.*FAIL' "$log"; then v=PASS
  else v=FAIL; fi
  echo "RFRES side=$SIDE w=$name rc=$rc secs=$((t1-t0)) recs=$n verdict=$v"
done
