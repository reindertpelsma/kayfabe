#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
export WINVM_ROOT=/workspace/sweep-win-56766467
export WINVM_QEMU=/workspace/bench/kf3-bins/24906bf3/qemu-system-x86_64
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
exec >/root/prov/sweep_windows581_stage.log 2>&1
trap 'rc=$?; echo "WINDOWS581_STAGE_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = 24906bf334665c9000b16b271005593daa3586c7 ]
grep -q '^WINDOWS580_SUITE_HOLD_COMPLETE ' /root/prov/sweep_windows580_resume.log
[ -f /root/prov/sweep_windows580_evidence_exported ]
[ ! -e /root/prov/sweep_windows581_stage.started ]
date -u -Is >/root/prov/sweep_windows581_stage.started
echo "WINDOWS581_STAGE_START single upgrade after580PASS $(date -u -Is)"
echo 'FALSIFIER allocation accepted but same abort/unload point; scope AD102/581.51/wire580.94.02 only; scheduling20801111 stays refused.'
echo 'cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a  /workspace/sweep-win-media581/driver58151.exe' | sha256sum -c -
timeout 60 bash "$W" ssh 'New-Item -ItemType Directory -Force C:\kf\nv581 | Out-Null'
bash "$W" scp-to /workspace/sweep-win-media581/extracted/Display.Driver C:/kf/nv581/
bash "$W" scp-to /root/sweep-probes/verify_stage581.ps1 C:/kf/verify_stage581.ps1
# Preserve the disabled device through staging: no new581 driver runs against
# the580.65.06 QEMU. The new process uses580.94.02 before its single enable.
timeout 900 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\gsp_on.ps1 -Inf C:\kf\nv581\Display.Driver\nv_dispui.inf -NoEnable' >/root/prov/sweep_windows581_stage_driver.txt 2>&1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage581.ps1' >/root/prov/sweep_windows581_stage_ready.txt 2>&1
grep '^STAGE581_READY=1 ' /root/prov/sweep_windows581_stage_ready.txt
timeout 300 bash "$W" stop
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "$VM/disk/sweep581.qcow2" ]
# Only our stopped test overlay is renamed; the sealed public-media base is kept.
mv -n "$VM/disk/sweep580.qcow2" "$VM/disk/sweep581.qcow2"
echo "WINDOWS581_COLD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
