#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/ada2build_windows581_recovery.log 2>&1
export WIN_RUN=/root/win/codex-ada2build WIN_SMP=8 WIN_FLAGS='' WIN_KF3_DISPLAY=display=on,gop=on WIN_STDVGA=0
rev=9d6f210b
stop(){ bash /root/ada2build_stop_windows.sh "$WIN_RUN" "$rev" /root/kayfabe; }
cleanup(){
 rc=$?
 if [ -s "$WIN_RUN/qpid" ] && kill -0 "$(cat "$WIN_RUN/qpid")" 2>/dev/null; then stop || true; fi
 echo "WINDOWS581_RECOVERY_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup EXIT
psrun(){ python3 /root/ada2build_ps.py "$1" "$2"; }
start(){
 test -z "$(pgrep -x qemu-system-x86 || true)"
 export QBIN=/root/ada2build-wrappers/$rev/qemu-system-x86_64
 bash scripts/bench/diesweep/win_die.sh start
 python3 /root/ada2build_process_receipt.py "$WIN_RUN" /root/kayfabe "$rev"
 bash scripts/bench/diesweep/win_die.sh wait-qga 240
}
sibling(){
 parent=$1;phase=$2
 test -z "$(pgrep -x qemu-system-x86 || true)"
 if [ -e "$WIN_RUN/windows.qcow2" ]; then mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/completed-$phase.qcow2"; fi
 qemu-img create -q -f qcow2 -F qcow2 -b "$WIN_RUN/$parent.qcow2" "$WIN_RUN/windows.qcow2"
 cp "$WIN_RUN/$parent-VARS.fd" "$WIN_RUN/OVMF_VARS.fd"
}
archive(){
 phase=$1
 cp "$WIN_RUN/flags.txt" "/root/prov/ada2build-$phase-flags.txt"
 cp "$WIN_RUN/qemu.log" "/root/prov/ada2build-$phase-qemu.log"
 cp "$WIN_RUN/process-receipt-$(cat "$WIN_RUN/qpid").json" "/root/prov/ada2build-$phase-process-receipt.json"
}
xids(){ dmesg | grep 'NVRM: Xid' || true; }
accept(){
 phase=$1;build=$2;seconds=$3
 psrun "/root/ada2build_census$build.ps1" 60 >"/root/prov/ada2build-$phase-census.log" 2>&1 || return $?
 grep -q '"problem":0' "/root/prov/ada2build-$phase-census.log" || return 1
 t0=$(date +%s)
 psrun "/root/ada2build_accept$build.ps1" "$seconds" >"/root/prov/ada2build-$phase-accept.log" 2>&1 || return $?
 t1=$(date +%s);echo "HOST_ACCEPT_ELAPSED phase=$phase seconds=$((t1-t0))"
 if [ "$build" = 8151 ]; then test "$((t1-t0))" -ge 420 || return 1; fi
 bash scripts/bench/diesweep/win_die.sh shot "$phase" || return $?
 python3 /root/ada2build_shot_summary.py "$WIN_RUN/shots/$phase.ppm.gz" >"/root/prov/ada2build-$phase-shot-summary.json" || return $?
 xids >"/root/prov/ada2build-$phase-host-xids.after.txt"
 cmp -s "/root/prov/ada2build-$phase-host-xids.before.txt" "/root/prov/ada2build-$phase-host-xids.after.txt" || return 1
}
check_deadline(){ test "$(date +%s)" -lt "$(date -u -d '2026-10-17 00:15:00' +%s)"; }
echo "WINDOWS581_RECOVERY_START $(date -u -Is)"
rev=9d6f210b
export WIN_GUEST_DRIVER=580.94.02 EXPECTED_WINDOWS_DRIVER=32.0.15.8151 WIN_NET_RESTRICT=on
test -z "$(pgrep -x qemu-system-x86 || true)"
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
sibling bound58151 acceptance58151-recovery
xids >/root/prov/ada2build-acceptance58151-recovery-host-xids.before.txt
start
# Previous read-only census exceeded60s; cause unestablished, new bound300s.
set +e
psrun /root/ada2build_census8151.ps1 300 >/root/prov/ada2build-acceptance58151-recovery-census.log 2>&1
census_rc=$?
set -e
result=1
if [ "$census_rc" = 0 ] && grep -q '"problem":0' /root/prov/ada2build-acceptance58151-recovery-census.log; then
 set +e
 psrun /root/ada2build_accept8151.ps1 900 >/root/prov/ada2build-acceptance58151-recovery-accept.log 2>&1
 result=$?
 set -e
 if [ "$result" = 0 ]; then
  bash scripts/bench/diesweep/win_die.sh shot acceptance58151-recovery
  python3 /root/ada2build_shot_summary.py "$WIN_RUN/shots/acceptance58151-recovery.ppm.gz" >/root/prov/ada2build-acceptance58151-recovery-shot-summary.json
  xids >/root/prov/ada2build-acceptance58151-recovery-host-xids.after.txt
  cmp /root/prov/ada2build-acceptance58151-recovery-host-xids.before.txt /root/prov/ada2build-acceptance58151-recovery-host-xids.after.txt
 fi
fi
archive acceptance58151-recovery;stop
cp /root/prov/ada2build_stop_9d6f210b.log /root/prov/ada2build-acceptance58151-recovery-stop.log
echo "WINDOWS_CELL_EXIT phase=acceptance58151-recovery rc=$result census_rc=$census_rc $(date -u -Is)"
echo "WINDOWS581_RECOVERY_COMPLETE $(date -u -Is)"
