#!/usr/bin/env bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
export WINVM_ROOT=/workspace/sweep-win-56766467
export WINVM_QEMU=/workspace/bench/kf3-bins/24906bf3/qemu-system-x86_64
export WINVM_RAM_MB=8192 WINVM_SMP=8
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
exec >/root/prov/sweep_windows581_diag.log 2>&1
trap 'rc=$?; echo "WINDOWS581_DIAG_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = 24906bf334665c9000b16b271005593daa3586c7 ]
grep -q '^WINDOWS581_COLD_READY ' /root/prov/sweep_windows581_stage.log
[ ! -e /root/prov/sweep_windows581_diag.started ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ "$(git ls-remote origin refs/heads/integration/prod-candidate-5-20261018 | cut -f1)" = f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e ]
git merge-base --is-ancestor f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e HEAD
date -u -Is >/root/prov/sweep_windows581_diag.started
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep-p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo "H_AD102_581_START $(date -u -Is)"
echo 'FALSIFIER postedB297allocationNV_OK yet same abort/unload point; schedule20801111 remainsREFUSED/NOT_DERIVABLE.'
bash "$W" run --overlay sweep581 --arm B --guest-driver 580.94.02 --once --max-boots 1 --tag sweep581_diag --detach
for i in $(seq 1 90); do
  if timeout 20 bash "$W" ssh 'echo KF_SSH_UP' 2>/dev/null | grep -q KF_SSH_UP; then break; fi
  sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' | grep KF_SSH_UP
python3 /root/sweep_windows_receipt.py sweep581_diag
python3 - <<'PY'
import json
r=json.load(open('/root/prov/sweep581_diag_process_receipt.json'))
assert any('guest-driver=580.94.02' in a for a in r['argv'])
PY
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage581.ps1' >/root/prov/sweep_windows581_pre_enable.txt 2>&1
grep '^STAGE581_READY=1 ' /root/prov/sweep_windows581_pre_enable.txt
# No driver activation before its correct ABI is installed in a fresh QEMU.
if grep -aq 'kf-rm: rpc-trace' "$VM/logs/sweep581_diag_b1_qemu.log"; then
  echo 'PRECONDITION_REFUSED: unexpected GSP traffic while581 device disabled'; exit 7
fi
bash "$W" scp-to /root/sweep-probes/enable581_once.ps1 C:/kf/enable581_once.ps1
timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable581_once.ps1' >/root/prov/sweep_windows581_enable.txt 2>&1
cat /root/prov/sweep_windows581_enable.txt
bash "$W" shot sweep581_diag
python3 scripts/bench/windows/abort_point.py points "$VM/logs/sweep581_diag_b1_qemu.log" >/root/prov/sweep_windows581_abort_point.json
echo "WINDOWS581_DIAG_COLLECTED $(date -u -Is)"
# Leave alive for bounded text screening; controller stops before the next stage.
