#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/windows_resume.log 2>&1
controller_pgid=$BASHPID
guard_pid=
cleanup_controller(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$guard_pid" ]; then kill "$guard_pid" 2>/dev/null; wait "$guard_pid" 2>/dev/null; fi
 journalctl -k -b -o short-iso-precise --no-pager >/root/prov/resume_final_kernel.log
 echo "WINDOWS_RESUME_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup_controller EXIT
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07 CARGO_BUILD_JOBS=8
[ "$(git rev-parse HEAD)" = 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
[ ! -e /root/prov/XID_STOP ]
[ ! -e /root/prov/windows_resume.started ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
grep '^PRECOMP_TIER3_EXIT rc=0 ' /root/prov/precomp_tier3_exit.txt
python3 - <<'CHECKPOINT'
import json,sys
from pathlib import Path
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
approved=json.loads(Path('/root/prov/native_receipt.json').read_text())['approved_native_xid_lines']
actual=G.N.xid_lines(G.N.kernel_snapshot())
assert len(approved)==5 and tuple(actual)==tuple(approved),'new Xid during PRECOMP read is fatal'
p=Path('/root/prov/native_receipt.json.ready.json')
if p.exists():p.rename(p.with_suffix('.before_resume.json'))
CHECKPOINT
date -u -Is >/root/prov/windows_resume.started
python3 -E -s /root/sweep2/guard/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$controller_pgid" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/resume_guard_watch.log 2>&1 &
guard_pid=$!
for i in $(seq 1 20); do [ -f /root/prov/native_receipt.json.ready.json ] && break; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/native_receipt.json.ready.json ]; kill -0 "$guard_pid"
echo "WINDOWS_RESUME_START native/tier3 already measured; same frozen product $(date -u -Is)"
bash /root/sweep2/start_secondary.sh
bash /root/sweep2/build_b1.sh &
b1_build_pid=$!
bash /root/sweep_windows_install.sh
wait "$b1_build_pid"
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"

# N0 evidence is exported during the standard-VGA installation; no decision wait.
for i in $(seq 1 60); do [ -f /root/prov/N0_EXPORT_ACK ] && break; [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/N0_EXPORT_ACK ]
. /root/sweep2/common.sh
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
# The new phase authorizes one T1 field-derived ON, then B1 only on observed Code0.
run_cell(){
 local run=$1 binary=$2 source=$3 arm_rc=0
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 python3 /root/sweep2/budget_start.py "$run" || { echo "CELL_UNRUN_BUDGET $run"; return 30; }
 echo "VALIDATION_ARM_BEGIN $run $(date -u -Is)"
 bash /root/sweep2/arm.sh on "$run" "$binary" "$source" || arm_rc=$?
 echo "VALIDATION_ARM_RESULT run=$run rc=$arm_rc $(date -u -Is)"
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 [ -z "$(pgrep -x qemu-system-x86 || true)" ]
 if [ "$arm_rc" != 0 ]; then
  case "$arm_rc" in
   124|255) grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" ;;
   *) echo "ARM_HARNESS_REFUSAL_STOP run=$run rc=$arm_rc"; exit "$arm_rc" ;;
  esac
 fi
 # Expected guest failure is collected via standard VGA, never another product boot.
 bash /root/sweep2/collect_bw.sh "$run"
 echo "VALIDATION_ARM_END $run $(date -u -Is)"
}
actual_code0(){
 python3 -E -s - "$1" <<'ACTUAL_CODE0'
from pathlib import Path
import sys
sys.path.insert(0,'/root/sweep2/guard/blackwell')
from init_observer import clean,result
p=Path('/root/prov/arms')/sys.argv[1]
diag=p/'early_code0.txt'
assert clean(result(diag.read_text()),0)
assert 'WINDOWS_INIT_OBSERVER_END code0=1' in (p/'init_observer.log').read_text()
ACTUAL_CODE0
}
run_cell on_precision "$BIN" "$BASE"
if actual_code0 on_precision; then
 [ -f /root/prov/optional_b1.json ]
 mapfile -t b1 < <(python3 - <<'B1_META'
import json,re,hashlib
from pathlib import Path
j=json.loads(Path('/root/prov/optional_b1.json').read_text())
assert j.get('enabled') and j.get('tests_verified_before_arms') is True
assert re.fullmatch('[0-9a-f]{40}',j['source'])
assert re.fullmatch('/workspace/bench/kf3-bins/[a-zA-Z0-9_-]+/qemu-system-x86_64',j['binary'])
assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']
print(j['binary']);print(j['source'])
B1_META
 )
 [ "${#b1[@]}" = 2 ]
 if python3 /root/sweep2/budget_start.py on_b1; then
  run_cell on_b1 "${b1[0]}" "${b1[1]}"
 else echo OPTIONAL_B1_UNRUN_BUDGET; fi
else
 echo OPTIONAL_B1_UNRUN_NO_ACTUAL_CODE0
fi
echo "VALIDATION_FINAL_DONE $(date -u -Is)"
