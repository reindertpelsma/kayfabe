#!/usr/bin/env bash
# (The box-side driver the 2026-09-30 runs used: copy to /root/prov on a bench box; usage below.)
# usage: uvmg_run.sh <rev> <tag> <mode> [modes]   — fetch, build kf3 at <rev>, run the lane.
set -uo pipefail
REV=$1; TAG=$2; MODE=$3; MODES=${4:-}
LOG=/root/prov/run_${TAG}.driver.log
exec >"$LOG" 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
echo "START $(date -Is) rev=$REV tag=$TAG mode=$MODE modes=$MODES"
export PATH="$PATH:$HOME/.cargo/bin"
cd /root/kayfabe || exit 3
git fetch -q origin v3-uvm-guest || exit 3
git checkout -q --detach "$REV" || exit 3
echo "HEAD=$(git rev-parse HEAD)"
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /root/prov/build_${TAG}.log 2>&1 || { echo "BUILD FAILED"; tail -20 /root/prov/build_${TAG}.log; exit 4; }
tail -1 /root/prov/build_${TAG}.log
[ -n "$MODES" ] && export UM_PROBE_MODES="$MODES"
[ -n "${DIAG:-}" ] && export KF3_FAULTLOG=1 KF3_MAPLOG=1
MODE=$MODE bash scripts/bench/uvm_guest_lane.sh "$TAG"
echo "LANE_RC=$?"
tail -40 /root/prov/uvmg_${TAG}.log
