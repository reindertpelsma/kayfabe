#!/bin/bash
# usage: wait_marker.sh ID 'remote grep file' 'pattern' max_minutes   -- exits 0 when found, 1 on timeout. Checks every 3 min.
id=$1; file=$2; pat=$3; max=$4
for i in $(seq 1 $((max/3))); do
  if timeout 25 ssh -F /workspace/bw-run/ssh.conf bwlane-$id "grep -aq '$pat' $file" 2>/dev/null; then echo "FOUND $pat $(date -u -Is)"; exit 0; fi
  sleep 180
done
echo "TIMEOUT $pat $(date -u -Is)"; exit 1
