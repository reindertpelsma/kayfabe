#!/usr/bin/env bash
# (The box-side driver the 2026-09-30 runs used: copy to /root/prov on a bench box; usage below.)
# usage: uvmapps_run.sh <rev> <tag> <mode>  — fetch, build kf3 at <rev>, build the apps, host baseline, guest lane.
set -uo pipefail
REV=$1; TAG=$2; MODE=$3
LOG=/root/prov/run_${TAG}.driver.log
exec >"$LOG" 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
echo "START $(date -Is) rev=$REV tag=$TAG mode=$MODE"
export PATH="$PATH:$HOME/.cargo/bin"
cd /root/kayfabe || exit 3
git fetch -q origin v3-uvm-guest || exit 3
git checkout -q --detach "$REV" || exit 3
echo "HEAD=$(git rev-parse HEAD)"
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /root/prov/build_${TAG}.log 2>&1 || { echo "BUILD FAILED"; tail -20 /root/prov/build_${TAG}.log; exit 4; }
tail -1 /root/prov/build_${TAG}.log
bash scripts/bench/uvm_apps_build.sh
if [ "${HOST_BASELINE:-1}" = 1 ]; then
  echo "=== HOST (bare-metal) baseline, patched module loaded, not opted in"
  rmmod nvidia_uvm 2>/dev/null; insmod /root/efs/patched/nvidia-uvm.ko uvm_efs_enable=1 uvm_efs_timeout_ms=30000
  for a in attach_verify UnifiedMemoryStreams conjugateGradientUM UnifiedMemoryPerf; do
    out=$(cd /workspace/bench/uvmapps && LD_LIBRARY_PATH=/workspace/bench/uvmapps/lib timeout -k 5 300 ./$a 2>&1; echo RC=$?)
    echo "HOSTAPP app=$a rc=$(echo "$out" | sed -n 's/^RC=//p') last=$(echo "$out" | grep -v '^RC=' | tail -2 | tr '\n' '|' | cut -c1-200)"
  done
  dmesg | grep 'NVRM: Xid' | tail -3
fi
[ -n "${DIAG:-}" ] && export KF3_FAULTLOG=1 KF3_MAPLOG=1
HOOK=uvm_apps_hook.sh MODE=$MODE bash scripts/bench/uvm_guest_lane.sh "$TAG"
echo "LANE_RC=$?"
tail -30 /root/prov/uvmg_${TAG}.log
