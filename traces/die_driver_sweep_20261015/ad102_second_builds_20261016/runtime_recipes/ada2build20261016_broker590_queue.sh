#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_broker590_queue.log 2>&1
trap 'rc=$?; echo "BROKER590_QUEUE_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "BROKER590_QUEUED $(date -u -Is)"
for _ in $(seq 1800); do
 if test -f /root/prov/ada2build_fat590_v3.log && grep -q '^FAT590_EXIT rc=' /root/prov/ada2build_fat590_v3.log; then break; fi
 if test "$(date +%s)" -ge "$(date -u -d '2026-10-17 00:05:00' +%s)"; then echo BROKER590_NOTRUN=bounded_lease_time;exit 0;fi
 sleep 3
done
grep -q '^FAT590_CUDA_GRAPHICS_COMPLETE ' /root/prov/ada2build_fat590_v3.log
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "BROKER590_START $(date -u -Is)"
timeout --signal=TERM 1500 bash /root/ada2build_broker590.sh
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "BROKER590_COMPLETE $(date -u -Is)"
