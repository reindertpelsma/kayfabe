#!/bin/bash
# edid_side.sh <tag> [settle_s] — runs beside broker_lane.sh/lane.sh: waits for the hook's hold, reads the
# EDID the guest's DRM connectors expose (sysfs), the modes, and the relay/fps status, then releases
# the hold. Output: /root/mf/side_<tag>.log (START/EXIT lines) and /root/mf/ev/<tag>/edid_*.
TAG=$1; SETTLE=${2:-30}
G=/root/wt-mf/scripts/bench/gssh_nv
P=/workspace/bench/run_${TAG}_probe.log; Q=/workspace/bench/run_${TAG}_qemu.log
EV=/root/mf/ev/$TAG; mkdir -p $EV
exec > /root/mf/side_$TAG.log 2>&1
echo "SIDE_START $TAG $(date -Is)"
for i in $(seq 900); do grep -aq 'HOLD up to\|DISPLAY_HOLD_FOR_SIDE' $P 2>/dev/null && break; sleep 1; done
echo "SIDE_HOLD_SEEN after ${i}s $(date -Is)"
gq(){ timeout ${2:-60} $G "$1" 2>&1 | tr -d '\r'; }
gq 'for e in /sys/class/drm/card*-*/edid; do s=$(stat -c %s $e); st=$(cat $(dirname $e)/status); echo "EDID_FILE $e size=$s status=$st sha256=$(sha256sum < $e | cut -c1-64)"; done' > $EV/edid_list.txt
cat $EV/edid_list.txt
for e in $(awk '$3 != "size=0" {print $2}' $EV/edid_list.txt); do
  n=$(basename $(dirname $e)); gq "xxd -p $e" > $EV/edid_$n.hex; gq "cat $e" > $EV/edid_$n.bin
  echo "EDID_HEX $n $(tr -d '\n' < $EV/edid_$n.hex | cut -c1-64)... bytes=$(stat -c %s $EV/edid_$n.bin)"
done
gq 'sudo modetest -M nvidia-drm -c 2>&1 | head -60' > $EV/modetest_c.txt
echo "MODES $(grep -E '^ *#[0-9]+ ' $EV/modetest_c.txt | head -12 | awk '{print $2"@"$3}' | tr '\n' ' ')"
gq 'sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority xrandr 2>&1 | head -12' > $EV/xrandr.txt
echo "XRANDR $(grep -E '\*|connected' $EV/xrandr.txt | tr -s ' ' | tr '\n' '|' | cut -c1-300)"
grep -a 'kf3: display: monitor\|EDID fnv' $Q | head -5 | cut -c1-240 | sed 's/^/QLOG_MONITOR /'
sleep $SETTLE
grep -ao 'broker\[[^]]*\]' $Q | tail -1 | sed 's/^/STATUS_BROKER /'
grep -a 'kf3: display fps\[' $Q | tail -3 | cut -c1-400 | sed 's/^/STATUS_FPS /'
touch /workspace/bench/brk/$TAG/release /root/mf/ev/$TAG/release
echo "SIDE_EXIT $(date -Is)"
