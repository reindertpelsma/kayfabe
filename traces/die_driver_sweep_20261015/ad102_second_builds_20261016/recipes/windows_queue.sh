#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_windows_queue.log 2>&1
trap 'rc=$?; echo "WINDOWS_QUEUE_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WINDOWS_QUEUE_START $(date -u -Is)"
while kill -0 93115 2>/dev/null; do
 test "$(date +%s)" -lt "$(date -u -d '2026-10-16T22:45:00Z' +%s)"
 sleep 3
done
grep -q '^LINUX595_RAW30_COMPLETE ' /root/prov/ada2build_raw.log
test -z "$(pgrep -x qemu-system-x86 || true)"
grep -q '^ADA2BUILD_PAYLOAD_READY ' /root/prov/ada2build_payload.log
test "$(git -C /root/kayfabe rev-parse --short=9 HEAD)" = 9d6f210ba
bash /root/ada2build_windows.sh
echo "WINDOWS_QUEUE_COMPLETE $(date -u -Is)"
