#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_finish_queue.log 2>&1
trap 'rc=$?; echo "FINISH_QUEUE_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "FINISH_QUEUE_START $(date -u -Is)"
for _ in $(seq 800); do
 grep -q '^LINUX590_EXIT rc=' /root/prov/ada2build_linux590.log && break
 test "$(date +%s)" -lt "$(date -u -d '2026-10-16 23:45:00' +%s)";sleep 3
done
grep -q '^LINUX590_RAW30_COMPLETE ' /root/prov/ada2build_linux590.log
test -z "$(pgrep -x qemu-system-x86 || true)"
bash /root/ada2build_windows581_recovery.sh
test -z "$(pgrep -x qemu-system-x86 || true)"
if test "$(date +%s)" -lt "$(date -u -d '2026-10-16 23:35:00' +%s)"; then
 bash /root/ada2build_fat590_v3.sh
else
 echo 'FAT590_NOTRUN=insufficient bounded lease time after Windows census recovery'
fi
echo "FINISH_QUEUE_COMPLETE $(date -u -Is)"
