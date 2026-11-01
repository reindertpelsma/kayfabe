#!/bin/bash
set -euo pipefail
exec >/root/prov/sweep_preparation_continuation.log 2>&1
root=$BASHPID;guard_pid=
cleanup(){ rc=$?;trap - EXIT;set +e;[ -z "$guard_pid" ] || { kill "$guard_pid";wait "$guard_pid"; };echo "PREPARATION_CONTINUATION_EXIT rc=$rc $(date -u -Is)"; }
trap cleanup EXIT
cd /root/kayfabe
. /root/sweep2/common.sh
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ ! -e /root/prov/XID_STOP ]
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
python3 -E -s /root/sweep2/guard/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$root" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/guard_watch_continued.log 2>&1 &
guard_pid=$!
for i in $(seq 1 30);do [ -f /root/prov/native_receipt.json.ready.json ] && break;kill -0 "$guard_pid";sleep 1;done
python3 - "$root" <<'VERIFY_READY'
import json,sys
from pathlib import Path
assert json.loads(Path('/root/prov/native_receipt.json.ready.json').read_text())['owned_pgid']==int(sys.argv[1])
VERIFY_READY
kill -0 "$guard_pid"
echo "PREPARATION_CONTINUATION_START $(date -u -Is)"
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"

# N0 evidence is exported during the standard-VGA installation; no decision wait.
for i in $(seq 1 60); do [ -f /root/prov/N0_EXPORT_ACK ] && break; [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/N0_EXPORT_ACK ]
. /root/sweep2/common.sh
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
for cell in 'on on_precision' 'off off_c'; do
 set -- $cell
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 if ! python3 /root/sweep2/budget_start.py "$2"; then echo "CELL_UNRUN_BUDGET $2"; continue; fi
 echo "VALIDATION_ARM_BEGIN $2 $(date -u -Is)"
 bash /root/sweep2/arm.sh "$1" "$2" "$BIN" "$BASE"
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 bash /root/sweep2/collect_bw.sh "$2"
 echo "VALIDATION_ARM_END $2 $(date -u -Is)"
 # Pause for export/push before another arm, bounded by 5 min.
 for i in $(seq 1 300); do [ -f /root/prov/EXPORT_ACK_$2 ] && break; sleep 1; done
 [ -f /root/prov/EXPORT_ACK_$2 ]
done
echo "VALIDATION_DONE $(date -u -Is)"

# Optional B1 is sealed before ARM_GO and uses a separately pinned tested binary.
# VALIDATION_DONE above means the mandatory pair; FINAL_DONE closes the lane.
if [ -f /root/prov/optional_b1.json ]; then
 mapfile -t b1 < <(python3 - <<'B1_META'
import json,re
from pathlib import Path
j=json.loads(Path('/root/prov/optional_b1.json').read_text())
if j.get('enabled'):
 assert j.get('tests_verified_before_arms') is True
 assert re.fullmatch('[0-9a-f]{40}',j['source'])
 assert re.fullmatch('/workspace/bench/kf3-bins/[a-zA-Z0-9_-]+/qemu-system-x86_64',j['binary'])
 import hashlib
 assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']
 print(j['binary']);print(j['source'])
B1_META
 )
 if [ "${#b1[@]}" = 2 ]; then
  [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
  if ! python3 /root/sweep2/budget_start.py on_b1; then echo OPTIONAL_B1_FIRST_NEXT_LEASE; echo VALIDATION_FINAL_DONE; exit 0; fi
  bash /root/sweep2/arm.sh on on_b1 "${b1[0]}" "${b1[1]}"
  [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
  bash /root/sweep2/collect_bw.sh on_b1
  for i in $(seq 1 300); do [ -f /root/prov/EXPORT_ACK_on_b1 ] && break; sleep 1; done
  [ -f /root/prov/EXPORT_ACK_on_b1 ]
 else
  echo OPTIONAL_B1_NOT_READY
 fi
fi
echo "VALIDATION_FINAL_DONE $(date -u -Is)"
