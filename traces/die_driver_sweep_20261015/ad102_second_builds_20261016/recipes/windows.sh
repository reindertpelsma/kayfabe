#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/ada2build_windows.log 2>&1
export WIN_RUN=/root/win/codex-ada2build WIN_SMP=8 WIN_FLAGS='' WIN_KF3_DISPLAY=display=on,gop=on WIN_STDVGA=0
rev=9d6f210b
stop(){ bash /root/ada2build_stop_windows.sh "$WIN_RUN" "$rev" /root/kayfabe; }
cleanup(){
 rc=$?
 if [ -s "$WIN_RUN/qpid" ] && kill -0 "$(cat "$WIN_RUN/qpid")" 2>/dev/null; then stop || true; fi
 echo "WINDOWS_EXIT rc=$rc $(date -u -Is)"
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
archive_phase(){
 phase=$1
 cp "$WIN_RUN/flags.txt" "/root/prov/ada2build-$phase-flags.txt"
 cp "$WIN_RUN/qemu.log" "/root/prov/ada2build-$phase-qemu.log"
 cp "$WIN_RUN/process-receipt-$(cat "$WIN_RUN/qpid").json" "/root/prov/ada2build-$phase-process-receipt.json"
}
accept(){
 phase=$1;build=$2;seconds=$3
 psrun "/root/ada2build_census$build.ps1" 60 >"/root/prov/ada2build-$phase-census.log" 2>&1 || return $?
 grep -q '"problem":0' "/root/prov/ada2build-$phase-census.log" || { echo "WINDOWS_CELL_FAIL phase=$phase adapter_problem";return 1; }
 psrun /root/ada2build_stage_probes.ps1 90 >"/root/prov/ada2build-$phase-stage-probes.log" 2>&1 || return $?
 t0=$(date +%s)
 psrun "/root/ada2build_accept$build.ps1" "$seconds" >"/root/prov/ada2build-$phase-accept.log" 2>&1 || return $?
 t1=$(date +%s);echo "HOST_ACCEPT_ELAPSED phase=$phase seconds=$((t1-t0))"
 if [ "$build" = 8151 ]; then test "$((t1-t0))" -ge 420 || return 1; fi
 bash scripts/bench/diesweep/win_die.sh shot "$phase" || return $?
 python3 /root/ada2build_shot_summary.py "$WIN_RUN/shots/$phase.ppm.gz" >"/root/prov/ada2build-$phase-shot-summary.json" || return $?
}
echo "WINDOWS_START source=$(git rev-parse HEAD) $(date -u -Is)"
export WIN_GUEST_DRIVER=580.65.06 EXPECTED_WINDOWS_DRIVER=32.0.15.8088
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
test ! -e "$WIN_RUN/bound580.qcow2"
mkdir -p -m0700 /root/win/secrets
openssl rand -hex 9 >/root/win/secrets/pw;chmod 600 /root/win/secrets/pw
python3 - <<'PY'
from pathlib import Path
p=Path('/root/win/secrets/phase1.ps1');p.write_text(Path('scripts/bench/diesweep/phase1.ps1.in').read_text().replace('@PW@',Path('/root/win/secrets/pw').read_text().strip()));p.chmod(0o600)
PY
start
psrun /root/win/secrets/phase1.ps1 120 >/root/prov/ada2build_phase1_PRIVATE.log 2>&1
psrun /root/ada2build_bind580.ps1 240 >/root/prov/ada2build_bind580.log 2>&1
archive_phase binding580
stop
mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/bound580.qcow2";cp "$WIN_RUN/OVMF_VARS.fd" "$WIN_RUN/bound580-VARS.fd"
sibling bound580 baseline580
start
set +e
accept baseline580 8088 300
baseline_rc=$?
set -e
archive_phase baseline580;stop
cp "/root/prov/ada2build_stop_$rev.log" /root/prov/ada2build-baseline580-stop.log
echo "WINDOWS_CELL_EXIT phase=baseline580 rc=$baseline_rc $(date -u -Is)"
# Upgrade gets its own sibling; no baseline parent is mutated, and no installer replay.
sibling bound580 install58151
export WIN_NET_RESTRICT=off
start
set +e
psrun /root/ada2build_install58151.ps1 900 >/root/prov/ada2build_install58151.log 2>&1
install_rc=$?
set -e
echo "INSTALL_58151_EXIT rc=$install_rc $(date -u -Is)"
if [ "$install_rc" -ne 0 ]; then
 psrun /root/ada2build_install_outcome.ps1 30 >/root/prov/ada2build_install_outcome.log 2>&1
 echo INSTALL_COMPLETION_RECOVERED_WITHOUT_REPLAY=1
fi
# Direct binding only: do not rerun the installer after an uncertain timeout.
psrun /root/ada2build_set_gsp8151.ps1 60 >/root/prov/ada2build_set_gsp8151.log 2>&1
archive_phase install58151
stop
mv "$WIN_RUN/windows.qcow2" "$WIN_RUN/bound58151.qcow2";cp "$WIN_RUN/OVMF_VARS.fd" "$WIN_RUN/bound58151-VARS.fd"
sibling bound58151 acceptance58151
export WIN_NET_RESTRICT=on WIN_GUEST_DRIVER=580.94.02 EXPECTED_WINDOWS_DRIVER=32.0.15.8151
start
set +e
accept acceptance58151 8151 900
new_rc=$?
set -e
archive_phase acceptance58151;stop
cp "/root/prov/ada2build_stop_$rev.log" /root/prov/ada2build-acceptance58151-stop.log
echo "WINDOWS_CELL_EXIT phase=acceptance58151 rc=$new_rc $(date -u -Is)"
echo "WINDOWS_BASE_BUILDS_COMPLETE baseline_rc=$baseline_rc second_rc=$new_rc $(date -u -Is)"
