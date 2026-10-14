#!/bin/bash
# hardstop.sh — owner deadline 2026-10-10 03:41 CEST: stop the interactive guest cleanly, then by TERM, release the GPU.
W=/var/lib/kf-windows-20261005
L(){ echo "HARDSTOP $(date -Is) $*" >> $W/winprod/hardstop.log; }
L "deadline reached"
touch /tmp/kf-stop-winprod
for i in $(seq 40); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 10; done
if pgrep -x qemu-system-x86 >/dev/null; then
  L "qemu still alive after 400 s; TERM the runners"
  for f in $W/winprod/run*/runner.pid; do p=$(cat $f 2>/dev/null); [ -n "$p" ] && kill -0 $p 2>/dev/null && kill -TERM $p; done
  sleep 120
fi
pgrep -x qemu-system-x86 >/dev/null && L "qemu STILL alive: left for the owner" || L "no qemu running"
cat /sys/kernel/iommu_groups/11/type >> $W/winprod/hardstop.log
