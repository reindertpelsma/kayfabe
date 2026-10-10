#!/bin/bash
# Every 5 min: is there a picture, and is the capture still being refreshed?
# A stale file still holding a picture means frames STOPPED -- that is the bug.
D=/var/lib/docker/volumes/nvkvm-steamos-state/_data
OUT=/root/gs-test/black-watch.log
echo "=== started $(date -Is) : fix branch, GAMESCOPE_COMPOSITE_FORCE=1 ===" >> "$OUT"
while true; do
  age=$(( $(date +%s) - $(stat -c %Y "$D/frame.ppm" 2>/dev/null || echo 0) ))
  read -r mean verdict <<<"$(python3 - "$D/frame.ppm" <<'PY' 2>/dev/null
import sys
try:
    f=open(sys.argv[1],'rb'); f.readline(); f.readline(); f.readline()
    d=f.read(); m=sum(d[::53])/len(d[::53])
    print(f"{m:.2f}", "BLACK" if m<2 else "PICTURE")
except Exception: print("NA","NOFILE")
PY
)"
  ids=$(cd /root/gs-test/repo && timeout 25 ./steamos-ssh 'for i in 1 2 3; do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null | tr -d "\t "; sleep 0.5; done | tr "\n" " "' 2>/dev/null)
  echo "$(date +%H:%M:%S) $verdict mean=$mean capture_age=${age}s fb=[$ids]" >> "$OUT"
  sleep 300
done
