#!/bin/bash
# exits when the lane ended (LANE_EXIT) or the provision failed; prints one status line per change
id=$1; last=""
for i in $(seq 1 90); do
  out=$(timeout 40 ssh -F /workspace/bw-run/ssh.conf bwlane-$id 'tail -n1 /root/prov/resume.log 2>/dev/null | cut -c1-120; grep -a "ARM_BEGIN\|ARM_RC\|LANE_" /root/prov/lane.log 2>/dev/null | tail -n2' 2>&1 | tr '\n' '|')
  [ "$out" = "$last" ] || { echo "$(date -u +%H:%M:%S) $out"; last="$out"; }
  echo "$out" | grep -q "LANE_EXIT\|RESUME_EXIT rc=[1-9]" && exit 0
  sleep 120
done
echo WATCH_TIMEOUT
