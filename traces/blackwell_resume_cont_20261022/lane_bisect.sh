#!/bin/bash
# Unattended bisect lane (owner-approved 2026-10-22): control (derived page via the hook), delta-debug over PRECOMP word types,
# then off_c with the overdue-fence probe. Start/exit markers. Intermediate arms are not collected (QMP panic code decides).
. /root/sweep2/common.sh
exec >/root/prov/lane.log 2>&1
echo "LANE_START $(date -u -Is)"
trap 'rc=$?; echo "LANE_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
for i in $(seq 1 400); do
  grep -q '^SWEEP2_BIND_READY' /root/prov/sweep_20261020_provision.log 2>/dev/null && break
  grep -q '^SWEEP_PROVISION_EXIT rc=[1-9]' /root/prov/sweep_20261020_provision.log 2>/dev/null && { echo PROVISION_FAILED; exit 50; }
  sleep 30
done
grep -q '^SWEEP2_BIND_READY' /root/prov/sweep_20261020_provision.log || { echo PROVISION_TIMEOUT; exit 51; }
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
[ -x "$BIN" ]
SW=/root/sweep2
arm() { # profile run
  echo "ARM_BEGIN $2 $(date -u -Is)"
  bash $SW/arm.sh "$1" "$2" "$BIN" "$BASE"; arc=$?
  echo "ARM_RC $2 rc=$arc"
  grep -q 'ARM_EXIT run=' "/root/prov/arm_$2.log" || { echo "ARM_NOT_OBSERVED $2"; return 60; }
  [ ! -e /root/prov/XID_STOP ] || { echo "LANE_STOP_XID after $2"; return 61; }
  return 0
}
verdict() { # run -> PANIC50 | FIXED | OTHER  (QMP panic arg1 80 = 0x50; full init = >=1000 RPCs)
  local run=$1 n
  n=$(wc -l </root/prov/arms/$run/rpc_timing.jsonl 2>/dev/null || echo 0)
  if grep -q '"arg1": 80' "$VM/logs/${run}_run.log" 2>/dev/null; then echo PANIC50; elif [ "$n" -ge 1000 ]; then echo FIXED; else echo OTHER; fi
}
try() { # name types ("-" none) -> sets V
  python3 $SW/page_subset.py $SW/oracle_pr65.tsv $SW/native_page.txt /root/prov/pg_$1.txt "$2" >/dev/null
  echo "export KF3_DIAG_CAPS_PAGE_FILE=/root/prov/pg_$1.txt" >/root/prov/arm_env_$1.sh
  arm on "$1" || exit $?
  V=$(verdict "$1"); echo "VERDICT $1 types=$2 -> $V rpcs=$(wc -l </root/prov/arms/$1/rpc_timing.jsonl)"
}
count=0
try on_c0 -                      # control: derived page through the hook (expect PANIC50)
cur=(0 1 2 3 4 5)               # known to fix (previous lease, page B)
budget=5
while [ ${#cur[@]} -gt 1 ] && [ $budget -gt 0 ]; do
  h=$(( (${#cur[@]}+1)/2 ))
  first=("${cur[@]:0:$h}"); second=("${cur[@]:$h}")
  count=$((count+1)); budget=$((budget-1))
  try on_c$count "$(IFS=,; echo "${first[*]}")"
  if [ "$V" = FIXED ]; then cur=("${first[@]}"); continue; fi
  [ $budget -gt 0 ] || break
  count=$((count+1)); budget=$((budget-1))
  try on_c$count "$(IFS=,; echo "${second[*]}")"
  if [ "$V" = FIXED ]; then cur=("${second[@]}"); continue; fi
  echo "BISECT_NEEDS_BOTH_HALVES cur=${cur[*]}"; break
done
echo "BISECT_RESULT minimal_candidate=${cur[*]} arms_used=$count"
echo 'export KF3_COMPLETION_PROBE=500' >/root/prov/arm_env_off_c.sh
arm off off_c || exit $?
bash $SW/collect_bw.sh off_c || echo "COLLECT_FAILED off_c"
nvidia-smi -L; dmesg -T | grep -a 'Xid' | tail -6 || true
echo "LANE_DONE $(date -u -Is)"
