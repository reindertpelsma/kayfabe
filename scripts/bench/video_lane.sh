#!/bin/bash
# video_lane.sh <ffmpeg> <outdir> — the NVENC / NVDEC recipe, run IDENTICALLY on bare metal and in a
# kayfabe guest, so the two results files compare line for line (V3_VIDEO_ENGINES.md §5).
#
# ★ The same static ffmpeg binary on both sides (BtbN n8.1 linux64-gpl, loads libnvidia-encode /
#   libnvcuvid / libcuda at run time) — a distro ffmpeg differs by version and default encoder
#   parameters, which would make a bitstream compare meaningless.
# ★ Every artefact is content-checked: md5 of every encoded stream and every decoded frame set,
#   plus the software decode of the NVENC output and its PSNR against the source. NVDEC output of
#   a conformant H.264/HEVC stream must be BIT-EXACT to libavcodec's software decode.
# ⊘ Start marker + per-step rc + terminator: an empty or truncated results file is its own state.
set -u
FF=${1:?ffmpeg path}; OUT=${2:?outdir}
mkdir -p "$OUT"; cd "$OUT" || exit 2
R=results.txt; : > "$R"
say() { echo "$*" | tee -a "$R"; }
md5() { [ -s "$1" ] && md5sum "$1" | cut -d' ' -f1 || echo EMPTY; }
say "VIDEO_LANE_START $(date -Is) host=$(hostname) ff=$(sha256sum "$FF" | cut -c1-16) compact=${VIDEO_COMPACT:-0}"
say "driver=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1 | grep -o '[0-9]*\.[0-9]*\.[0-9]*' | head -1)"
W=1280; H=720; N=90
T="timeout -s INT 600"
# ⊘ VIDEO_COMPACT=1 (guest diagnosis, OFF by default and never on bare metal): drop the page cache and
#   compact memory before each step, so each process's pinned sysmem is physically contiguous.
#   Exists to TEST the hypothesis that the kf3 walk's per-space run cap (16 384) is reached by a later
#   process's fragmented sysmem (V3_VIDEO_ENGINES.md §6) — a result with it is not the default one.
compact() { [ "${VIDEO_COMPACT:-0}" = 1 ] && { sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null; echo 1 | sudo tee /proc/sys/vm/compact_memory >/dev/null; }; return 0; }
run() { # name, cmd...
  local n=$1; shift
  compact
  local t0=$(date +%s.%N)
  "$@" > "$n.log" 2>&1; local rc=$?
  local t1=$(date +%s.%N)
  say "$n rc=$rc secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.2f", b-a}')"
  return $rc
}
YUV="-f rawvideo -pix_fmt yuv420p -s ${W}x${H} -r 30"
run src $T "$FF" -hide_banner -y -f lavfi -i testsrc2=size=${W}x${H}:rate=30 -frames:v $N -pix_fmt yuv420p -f rawvideo src.yuv
say "src.yuv md5=$(md5 src.yuv) bytes=$(stat -c %s src.yuv 2>/dev/null)"

# ---- NVENC: fixed QP, no B-frames, fixed GOP — deterministic rate control --------------------------
for c in h264 hevc; do
  run enc_$c $T "$FF" -hide_banner -y $YUV -i src.yuv -c:v ${c}_nvenc -preset p4 -rc constqp -qp 23 -g 30 -bf 0 -f $c enc_$c.bit
  say "enc_$c.bit md5=$(md5 enc_$c.bit) bytes=$(stat -c %s enc_$c.bit 2>/dev/null)"
  # software decode of the NVENC stream: frame content + PSNR against the source
  run swdec_$c $T "$FF" -hide_banner -y -i enc_$c.bit -f rawvideo -pix_fmt yuv420p swdec_$c.yuv
  say "swdec_$c.yuv md5=$(md5 swdec_$c.yuv) frames=$(( $(stat -c %s swdec_$c.yuv 2>/dev/null || echo 0) / (W*H*3/2) ))"
  "$FF" -hide_banner $YUV -i swdec_$c.yuv $YUV -i src.yuv -lavfi psnr -f null - 2>&1 | grep -o 'PSNR y:[^ ]* u:[^ ]* v:[^ ]* average:[^ ]*' | sed "s/^/psnr_$c /" | tee -a "$R"
done

# ---- NVDEC: hwaccel cuda and the cuvid decoders, on the NVENC stream AND on a libx264 stream -------
# ⊘ -threads 1: libx264's output depends on its thread count (= the CPU count), which differs
#   between the host and the guest; single-threaded it is a deterministic function of the input.
run enc_x264 $T "$FF" -hide_banner -y $YUV -i src.yuv -c:v libx264 -threads 1 -qp 23 -g 30 -bf 2 -f h264 enc_x264.bit
say "enc_x264.bit md5=$(md5 enc_x264.bit)"
run swdec_x264 $T "$FF" -hide_banner -y -i enc_x264.bit -f rawvideo -pix_fmt yuv420p swdec_x264.yuv
say "swdec_x264.yuv md5=$(md5 swdec_x264.yuv)"
for s in h264 hevc x264; do
  # ⊘ -hwaccel_output_format cuda + hwdownload: a plain `-hwaccel cuda` FALLS BACK to software
  #   decode when NVDEC is unusable and still exits 0 — measured, guest vid1. Frames that must be
  #   downloaded from device memory cannot come from the software decoder.
  run hwdec_$s $T "$FF" -hide_banner -y -hwaccel cuda -hwaccel_output_format cuda -i enc_$s.bit -vf hwdownload,format=nv12 -f rawvideo -pix_fmt yuv420p hwdec_$s.yuv
  say "hwdec_$s.yuv md5=$(md5 hwdec_$s.yuv) bitexact_vs_sw=$([ "$(md5 hwdec_$s.yuv)" = "$(md5 swdec_$s.yuv)" ] && echo YES || echo NO)"
  dec=h264_cuvid; [ $s = hevc ] && dec=hevc_cuvid
  run cuvid_$s $T "$FF" -hide_banner -y -c:v $dec -i enc_$s.bit -f rawvideo -pix_fmt yuv420p cuvid_$s.yuv
  say "cuvid_$s.yuv md5=$(md5 cuvid_$s.yuv) bitexact_vs_sw=$([ "$(md5 cuvid_$s.yuv)" = "$(md5 swdec_$s.yuv)" ] && echo YES || echo NO)"
done
# ---- throughput (not graded, recorded): 1080p NVENC h264, 300 frames from lavfi --------------------
run perf_nvenc_1080p $T "$FF" -hide_banner -y -benchmark -f lavfi -i testsrc2=size=1920x1080:rate=60 -frames:v 300 -c:v h264_nvenc -preset p1 -f null -
grep -o 'fps=[ 0-9.]*' perf_nvenc_1080p.log | tail -1 | sed 's/^/perf_nvenc_1080p /' | tee -a "$R"
say "VIDEO_LANE_END rc=0 $(date -Is)"
