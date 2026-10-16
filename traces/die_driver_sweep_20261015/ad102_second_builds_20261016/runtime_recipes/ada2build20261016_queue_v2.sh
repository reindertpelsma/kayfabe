#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_queue_v2.log 2>&1
trap 'rc=$?; echo "QUEUE_V2_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "QUEUE_V2_START $(date -u -Is)"
for _ in $(seq 200); do
 if ! ps -p 101374 -o stat= | grep -qv '^[[:space:]]*Z'; then break; fi
 sleep 2
done
if ps -p 101374 -o stat= | grep -qv '^[[:space:]]*Z'; then echo OLD_CONTROLLER_STILL_ALIVE;exit 1;fi
test -z "$(pgrep -x qemu-system-x86 || true)"
bash /root/ada2build_resume_windows_v2.sh
