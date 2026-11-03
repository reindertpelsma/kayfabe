#!/bin/bash
# Unattended lane: ON A2 (PR65) -> ON A1 (page B) [only if A showed 0x50] -> OFF B1 (overdue-fence probe). Start/exit markers.
. /root/sweep2/common.sh
exec >/root/prov/lane.log 2>&1
echo "LANE_START $(date -u -Is)"
trap 'rc=$?; echo "LANE_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
[ -x "$BIN" ]
arm() { # profile run
  echo "ARM_BEGIN $2 $(date -u -Is)"
  bash /root/sweep2/arm.sh "$1" "$2" "$BIN" "$BASE"; arc=$?
  echo "ARM_RC $2 rc=$arc"
  grep -q 'ARM_EXIT run=' "/root/prov/arm_$2.log" || { echo "ARM_NOT_OBSERVED $2"; return 60; }
  [ ! -e /root/prov/XID_STOP ] || { echo "LANE_STOP_XID after $2"; return 61; }
  bash /root/sweep2/collect_bw.sh "$2" || echo "COLLECT_FAILED $2"
  [ ! -e /root/prov/XID_STOP ] || { echo "LANE_STOP_XID after $2"; return 61; }
  return 0
}

grep -q '"arg1": 80' "$VM/logs/on_a_run.log" && echo A_SHOWED_0x50
bash /root/sweep2/collect_bw.sh on_a || echo "COLLECT_FAILED on_a"
[ ! -e /root/prov/XID_STOP ] || { echo LANE_STOP_XID; exit 61; }
if grep -q '"arg1": 80' "$VM/logs/on_a_run.log" 2>/dev/null; then
  cp /root/sweep2/page_b.txt /root/prov/page_b.txt
  echo 'export KF3_DIAG_CAPS_PAGE_FILE=/root/prov/page_b.txt' >/root/prov/arm_env_on_b.sh
  arm on on_b || exit $?
else echo A_DID_NOT_SHOW_0x50; fi
echo 'export KF3_COMPLETION_PROBE=500' >/root/prov/arm_env_off_c.sh
arm off off_c || exit $?
nvidia-smi -L; dmesg -T | grep -a 'Xid' | tail -5 || true
echo "LANE_DONE $(date -u -Is)"
