#!/bin/bash
# video_hook.sh <tag> — POST_CAPTURE_HOOK for boot_capture.sh: run video_lane.sh INSIDE the guest
# with the SAME static ffmpeg the bare-metal baseline used, and pull its results into the probe log.
#   env: VIDEO_FF (host path of the static ffmpeg, default /workspace/video/ff/bin/ffmpeg)
#        VIDEO_TIMEOUT (guest-side bound, seconds, default 1500)
# ⊘ The lane runs DETACHED in the guest under its own timeout (never an outer SIGKILL mid-ioctl);
#   the hook polls for the terminator line and reports a missing one as its own state.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
FF=${VIDEO_FF:-/workspace/video/ff/bin/ffmpeg}
TO=${VIDEO_TIMEOUT:-1500}
[ -x "$FF" ] || { echo "★ video hook: no ffmpeg at $FF"; echo HOOK_RC=2; exit 2; }
echo "=== video hook: ffmpeg sha256 $(sha256sum "$FF" | cut -c1-16)  lane md5 $(md5sum < "$SRC_DIR/video_lane.sh" | cut -c1-8)"
$G 'mkdir -p /var/tmp/vid && cat > /var/tmp/vid/ffmpeg && chmod +x /var/tmp/vid/ffmpeg' < "$FF" || { echo "push ffmpeg failed"; echo HOOK_RC=2; exit 2; }
$G 'cat > /var/tmp/vid/video_lane.sh' < "$SRC_DIR/video_lane.sh"
$G 'ls /usr/lib/x86_64-linux-gnu/ | grep -E "libnvidia-encode.so.1|libnvcuvid.so.1|libcuda.so.1" | tr "\n" " "; echo'
# ★ Optional ioctl differential (VIDEO_SHIM = the nvdiff LD_PRELOAD recorder built on the host):
# the same small encode and decode the host trace was taken with, recorded in the guest.
if [ -n "${VIDEO_SHIM:-}" ] && [ -f "$VIDEO_SHIM" ]; then
  $G 'cat > /var/tmp/vid/nvdiff_shim.so' < "$VIDEO_SHIM"
  for w in enc dec; do
    if [ $w = enc ]; then args='-f lavfi -i testsrc2=size=320x240:rate=30 -frames:v 10 -c:v h264_nvenc -preset p4 -rc constqp -qp 23 -bf 0 -f h264 /var/tmp/vid/t.h264'
    else args='-hwaccel cuda -hwaccel_output_format cuda -i /var/tmp/vid/x.h264 -vf hwdownload,format=nv12 -f null -'
         $G "/var/tmp/vid/ffmpeg -hide_banner -y -f lavfi -i testsrc2=size=320x240:rate=30 -frames:v 10 -c:v libx264 -threads 1 -f h264 /var/tmp/vid/x.h264 >/dev/null 2>&1"; fi
    $G "rm -f /var/tmp/vid/$w.jsonl; NVDIFF_OUT=/var/tmp/vid/$w.jsonl LD_PRELOAD=/var/tmp/vid/nvdiff_shim.so timeout -s INT 120 /var/tmp/vid/ffmpeg -hide_banner -y $args > /var/tmp/vid/trace_$w.log 2>&1; echo TRACE_${w}_RC=\$?; tail -4 /var/tmp/vid/trace_$w.log"
    $G "cat /var/tmp/vid/$w.jsonl" > "/workspace/bench/run_${1}_$w.jsonl"
    echo "TRACE_$w records=$(wc -l < /workspace/bench/run_${1}_$w.jsonl)"
  done
fi
$G "rm -rf /var/tmp/vid/out /var/tmp/vid/lane.rc; nohup setsid bash -c 'timeout -s INT $TO bash /var/tmp/vid/video_lane.sh /var/tmp/vid/ffmpeg /var/tmp/vid/out; echo LANE_EXIT=\$? > /var/tmp/vid/lane.rc' >/dev/null 2>&1 < /dev/null &"
t0=$(date +%s)
while :; do
  sleep 5
  if $G 'test -s /var/tmp/vid/lane.rc'; then break; fi
  if [ $(( $(date +%s) - t0 )) -gt $(( TO + 60 )) ]; then echo "★ video hook: no lane.rc after $((TO+60))s — the guest lane did not terminate"; break; fi
  if ! $G true 2>/dev/null; then echo "★ video hook: guest unreachable"; break; fi
done
echo "=== guest video lane results ==="
$G 'cat /var/tmp/vid/out/results.txt 2>/dev/null; cat /var/tmp/vid/lane.rc 2>/dev/null' | sed 's/^/GUEST_VIDEO /'
echo "=== failing step logs (tail) ==="
$G 'cd /var/tmp/vid/out 2>/dev/null && for f in $(grep -l "rc=[1-9]" results.txt >/dev/null 2>&1; grep -o "^[a-z0-9_]* rc=[1-9][0-9]*" results.txt | cut -d" " -f1); do echo "--- $f.log"; tail -15 $f.log; done'
echo "=== guest dmesg tail ==="
$G 'sudo dmesg | tail -40'
echo HOOK_RC=0
