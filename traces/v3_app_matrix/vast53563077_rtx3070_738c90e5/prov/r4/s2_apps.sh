#!/bin/bash
# s2_apps.sh <off|on> — the guest app matrix in R3's shapes: batched (all 71 rows in one boot, no PM)
# + every non-PASS app alone; PM in one boot; 100 sequential vectorAdd processes in one boot.
# on = the doorbell fast path (-device kf3-gpu,...,doorbell-ioeventfd=on).
. /root/r4/common.sh
M=${1:?off|on}
case $M in off) export -n KF3_DEV_EXTRA 2>/dev/null; unset KF3_DEV_EXTRA ;; on) export KF3_DEV_EXTRA=$ON ;; *) exit 2 ;; esac
echo "S2_START $(date -Is) rev=$REV mode=$M KF3_DEV_EXTRA=${KF3_DEV_EXTRA:-<unset>} qb=$QB"
alive_qemu && { echo "A QEMU is up before s2"; exit 6; }
export KF_GUEST_IMG=$APPIMG KF3_BIN=$QB
step apps_${M} env APPS_PER_BOOT=100 bash scripts/apps/apps_matrix.sh guest r4${M} all
step apps_${M}pm env APPS_GUEST_PM=1 APPS_PER_BOOT=100 APPS_NO_ISOLATE=1 bash scripts/apps/apps_matrix.sh guest r4${M}pm all
step apps_${M}seq env APPS_PER_BOOT=100 APPS_NO_ISOLATE=1 bash scripts/apps/apps_matrix.sh guest r4${M}seq $(for i in $(seq 1 100); do echo vectorAdd; done)
for r in r4${M} r4${M}pm r4${M}seq; do echo "RESULT $r batched=$(grep -c 'verdict=PASS' /workspace/apps/results/$r/guest.res)/$(grep -c '^APPRES' /workspace/apps/results/$r/guest.res) iso=$(grep -c 'verdict=PASS' /workspace/apps/results/$r/guest_isolated.res 2>/dev/null)/$(grep -c '^APPRES' /workspace/apps/results/$r/guest_isolated.res 2>/dev/null) wedges=$(grep -c APPS_WEDGE /workspace/apps/results/$r/guest.res)"; done
echo "S2_EXIT rc=0 $(date -Is)"
