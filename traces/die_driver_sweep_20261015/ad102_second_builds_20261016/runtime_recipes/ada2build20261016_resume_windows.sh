#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/ada2build_resume_windows.log 2>&1
export WIN_RUN=/root/win/codex-ada2build WIN_SMP=8 WIN_FLAGS='' WIN_KF3_DISPLAY=display=on,gop=on WIN_STDVGA=0
rev=9d6f210b
stop(){ bash /root/ada2build_stop_windows.sh "$WIN_RUN" "$rev" /root/kayfabe; }
cleanup(){
 rc=$?
 if [ -s "$WIN_RUN/qpid" ] && kill -0 "$(cat "$WIN_RUN/qpid")" 2>/dev/null; then stop || true; fi
 echo "RESUME_WINDOWS_EXIT rc=$rc $(date -u -Is)"
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
echo "RESUME_WINDOWS_START $(date -u -Is)"
# Staging is exactly once per chunk. A failed/uncertain put stops this controller.
while kill -0 100983 2>/dev/null; do
 grep -q 'NVIDIA_STAGE_EXIT rc=' /root/prov/ada2build_stage_nvidia.log && break
 sleep 2
done
grep -q 'NVIDIA_STAGE_EXIT rc=0' /root/prov/ada2build_stage_nvidia.log
psrun /root/ada2build_verify_probes.ps1 60 >/root/prov/ada2build_after_stage_probes.log 2>&1
! grep -q PROBE_FILE_MISSING /root/prov/ada2build_after_stage_probes.log
stop
test ! -e "$WIN_RUN/bound580-payload.qcow2"
mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/bound580-payload.qcow2"
cp "$WIN_RUN/OVMF_VARS.fd" "$WIN_RUN/bound580-payload-VARS.fd"
# All four row tests use fresh siblings of exactly this stopped parent, P19 and no CD.
export WIN_GUEST_DRIVER=580.65.06 EXPECTED_WINDOWS_DRIVER=32.0.15.8088 WIN_NET_RESTRICT=on
for entry in reference580:9d6f210b ablate-caps00c:a52494d4 ablate-lpwr2801:a7509bf3 ablate-lpwr2806:003ad0e7; do
 check_deadline
 phase=${entry%:*};rev=${entry#*:}
 git checkout --detach "$rev"
 sibling bound580-payload "$phase"
 xids >"/root/prov/ada2build-$phase-host-xids.before.txt"
 start
 set +e
 accept "$phase" 8088 300
 result=$?
 set -e
 archive "$phase";stop
 cp "/root/prov/ada2build_stop_$rev.log" "/root/prov/ada2build-$phase-stop.log"
 echo "WINDOWS_CELL_EXIT phase=$phase rc=$result $(date -u -Is)"
done
check_deadline
rev=9d6f210b;git checkout --detach "$rev"
sibling bound580-payload install58151-qga
export WIN_NET_RESTRICT=off
start
set +e
psrun /root/ada2build_install58151_qga.ps1 900 >/root/prov/ada2build_install58151_qga.log 2>&1
install_rc=$?
set -e
echo "INSTALL_58151_QGA_EXIT rc=$install_rc $(date -u -Is)"
if [ "$install_rc" -ne 0 ]; then
 psrun /root/ada2build_install_outcome.ps1 30 >/root/prov/ada2build_install_outcome_qga.log 2>&1
 echo INSTALL_COMPLETION_RECOVERED_WITHOUT_REPLAY=1
fi
psrun /root/ada2build_set_gsp8151.ps1 60 >/root/prov/ada2build_set_gsp8151.log 2>&1
archive install58151-qga;stop
mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/bound58151.qcow2";cp "$WIN_RUN/OVMF_VARS.fd" "$WIN_RUN/bound58151-VARS.fd"
sibling bound58151 acceptance58151-qga
export WIN_NET_RESTRICT=on WIN_GUEST_DRIVER=580.94.02 EXPECTED_WINDOWS_DRIVER=32.0.15.8151
xids >/root/prov/ada2build-acceptance58151-qga-host-xids.before.txt
start
set +e
accept acceptance58151-qga 8151 900
new_rc=$?
set -e
archive acceptance58151-qga;stop
cp "/root/prov/ada2build_stop_$rev.log" /root/prov/ada2build-acceptance58151-qga-stop.log
echo "WINDOWS_CELL_EXIT phase=acceptance58151-qga rc=$new_rc $(date -u -Is)"
echo "RESUME_WINDOWS_COMPLETE $(date -u -Is)"
