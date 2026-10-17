#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/gb206phase_cmc_windows.log 2>&1
echo "CMC_WINDOWS_START source=$(git rev-parse HEAD) $(date -u -Is)"
export WIN_RUN=/root/win/codex-gb206phase WIN_GUEST_DRIVER=580.65.06 WIN_SMP=8
rev=$(git rev-parse --short=8 HEAD)
export QBIN=/workspace/bench/kf3-bins/$rev/qemu-system-x86_64
stop(){ bash /root/gb206phase_stop_windows.sh "$WIN_RUN" "$rev" /root/kayfabe; }
cleanup(){
 rc=$?
 if [ -s "$WIN_RUN/qpid" ] && kill -0 "$(cat "$WIN_RUN/qpid")" 2>/dev/null; then stop || true; fi
 echo "CMC_WINDOWS_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup EXIT
test -f "$WIN_RUN/bound580.qcow2"
for phase in cmc-off cmc-on; do
 test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:50:00Z' +%s)"
 test -z "$(pgrep -x qemu-system-x86 || true)"
 export WIN_FLAGS='KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1'
 if [ "$phase" = cmc-on ]; then export WIN_FLAGS="$WIN_FLAGS KF3_CMC_LOG_BUFFER_PROBE=1"; fi
 export WIN_KF3_DISPLAY=display=on,gop=on WIN_STDVGA=0
 if [ -e "$WIN_RUN/windows.qcow2" ]; then mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/prior-$phase.qcow2"; fi
 qemu-img create -q -f qcow2 -F qcow2 -b "$WIN_RUN/bound580.qcow2" "$WIN_RUN/windows.qcow2"
 cp "$WIN_RUN/bound580-VARS.fd" "$WIN_RUN/OVMF_VARS.fd"
 echo "CMC_PHASE_START phase=$phase $(date -u -Is)"
 echo 'FALSIFIER_BEFORE_RUN=With a89 actually answered NV_OK, unchanged first teardown boundary rejects rank2; no CCU probe unless it moves.'
 bash scripts/bench/diesweep/win_die.sh start
 python3 /root/gb206phase_cmc_process_receipt.py "$WIN_RUN" /root/kayfabe "$rev"
 bash scripts/bench/diesweep/win_die.sh wait-qga 240
 sleep 20
 set +e
 python3 /root/gb206phase20261016_ps.py /root/gb206phase20261016_census.ps1 60 >"/root/prov/gb206phase-$phase-census.log" 2>&1
 census_rc=$?
 set -e
 echo "CMC_CENSUS_EXIT phase=$phase rc=$census_rc"
 python3 /root/gb206phase_export_rpc.py "$WIN_RUN/qemu.log" "/root/prov/gb206phase-$phase-rpc" >"/root/prov/gb206phase-$phase-export.json"
 python3 /root/gb206phase_cmc_posted_status.py "$WIN_RUN/qemu.log" >"/root/prov/gb206phase-$phase-posted-status.json"
 cp "$WIN_RUN/flags.txt" "/root/prov/gb206phase-$phase-flags.txt"
 cp "$WIN_RUN/qemu.log" "/root/prov/gb206phase-$phase-qemu.log"
 cp "$WIN_RUN/process-receipt-$(cat "$WIN_RUN/qpid").json" "/root/prov/gb206phase-$phase-process-receipt.json"
 stop
 cp "/root/prov/gb206phase_stop_$rev.log" "/root/prov/gb206phase-$phase-stop.log"
 echo "CMC_PHASE_COMPLETE phase=$phase $(date -u -Is)"
done
dmesg | grep 'NVRM: Xid' > /root/prov/gb206phase_cmc_host_xids.log || true
echo "CMC_WINDOWS_COMPLETE $(date -u -Is)"
