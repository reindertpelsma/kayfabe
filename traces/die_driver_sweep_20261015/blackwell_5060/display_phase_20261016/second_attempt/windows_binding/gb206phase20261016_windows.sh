#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/gb206phase_windows.log 2>&1
echo "WINDOWS_PHASE_START source=$(git rev-parse HEAD) $(date -u -Is)"
export WIN_RUN=/root/win/codex-gb206phase WIN_GUEST_DRIVER=580.65.06 WIN_SMP=8
export WIN_FLAGS='KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1'
rev=$(git rev-parse --short=8 HEAD)
export QBIN=/workspace/bench/kf3-bins/$rev/qemu-system-x86_64
stop(){ bash /root/gb206phase_stop_windows.sh "$WIN_RUN" "$rev" /root/kayfabe; }
cleanup(){
 rc=$?
 if [ -s "$WIN_RUN/qpid" ] && kill -0 "$(cat "$WIN_RUN/qpid")" 2>/dev/null; then stop || true; fi
 echo "WINDOWS_PHASE_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup EXIT
psrun(){ python3 /root/gb206phase20261016_ps.py "$1" "$2"; }
start(){
 test -z "$(pgrep -x qemu-system-x86 || true)"
 bash scripts/bench/diesweep/win_die.sh start
 python3 /root/gb206phase_process_receipt.py "$WIN_RUN" /root/kayfabe "$rev"
 bash scripts/bench/diesweep/win_die.sh wait-qga 240
}
save_phase(){
 phase=$1
 # Export public bodies before destroying the VM/image; opaque reply bytes stay here.
 python3 /root/gb206phase_export_rpc.py "$WIN_RUN/qemu.log" "/root/prov/gb206phase-$phase-rpc" >"/root/prov/gb206phase-$phase-export.json"
 cp "$WIN_RUN/flags.txt" "/root/prov/gb206phase-$phase-flags.txt"
 cp "$WIN_RUN/qemu.log" "/root/prov/gb206phase-$phase-qemu.log"
}
if [ "${1:-baseline}" = baseline ]; then
 test ! -e "$WIN_RUN/bound580.qcow2"
 mkdir -p -m0700 /root/win/secrets
 [ -s /root/win/secrets/pw ] || { openssl rand -hex 9 >/root/win/secrets/pw; chmod 0600 /root/win/secrets/pw; }
 python3 - <<'PY'
from pathlib import Path
p=Path('/root/win/secrets/phase1.ps1');p.write_text(Path('scripts/bench/diesweep/phase1.ps1.in').read_text().replace('@PW@',Path('/root/win/secrets/pw').read_text().strip()));p.chmod(0o600)
PY
 start
 psrun /root/win/secrets/phase1.ps1 120
 psrun scripts/bench/diesweep/drvbind.ps1 180
 stop
 mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/bound580.qcow2"
 cp "$WIN_RUN/OVMF_VARS.fd" "$WIN_RUN/bound580-VARS.fd"
 phases=(on off)
elif [ "$1" = ready ]; then
 test -f "$WIN_RUN/bound580.qcow2"
 phases=(on off)
else
 test "$1" = tiles
 test -f "$WIN_RUN/bound580.qcow2"
 phases=(tiles)
fi
for phase in "${phases[@]}"; do
 # Every diagnostic gets a fresh sibling of the SAME STOPPED driver-bound parent.
 if [ -e "$WIN_RUN/windows.qcow2" ]; then mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/prior-$phase.qcow2"; fi
 qemu-img create -q -f qcow2 -F qcow2 -b "$WIN_RUN/bound580.qcow2" "$WIN_RUN/windows.qcow2"
 cp "$WIN_RUN/bound580-VARS.fd" "$WIN_RUN/OVMF_VARS.fd"
 export WIN_KF3_DISPLAY=display=on,gop=on WIN_STDVGA=0
 if [ "$phase" = off ]; then export WIN_KF3_DISPLAY=display=off WIN_STDVGA=1; fi
 echo "DISPLAY_PHASE_START phase=$phase source=$(git rev-parse HEAD) $(date -u -Is)"
 start
 sleep 20
 set +e
 psrun /root/gb206phase20261016_census.ps1 60 >"/root/prov/gb206phase-$phase-census.log" 2>&1
 census_rc=$?
 set -e
 cat "/root/prov/gb206phase-$phase-census.log"
 echo "DISPLAY_CENSUS_EXIT phase=$phase rc=$census_rc"
 save_phase "$phase"
 stop
 cp "/root/prov/gb206phase_stop_$rev.log" "/root/prov/gb206phase-$phase-stop.log"
 echo "DISPLAY_PHASE_COMPLETE phase=$phase $(date -u -Is)"
done
echo "WINDOWS_PHASE_COMPLETE $(date -u -Is)"
