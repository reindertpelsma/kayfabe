#!/bin/bash
source /workspace/relgate/env.sh
cd $R
st=/workspace/relgate/stage.sh
last() { tail -1 /workspace/relgate/stages.log | grep -q 'EXIT rc=0 '; }
$st cuda_guest bash $R/scripts/bench/cuda_ladder.sh guest $TAG 1 cup2,cup3,cup8,cup8bench; last || exit 1
[ "$(grep -c '^CL_ROW mode=guest .*verdict=PASS' /workspace/bench/cl_${TAG}_guest.out)" = 4 ] || { echo "CHAIN stop cuda_guest rows" >> /workspace/relgate/stages.log; exit 1; }
$st apps env KF_APPS_PROVISION=1 bash $R/scripts/bench/box/apps_gate.sh $TAG $P middle; last || exit 1
$st windows_gate bash $R/scripts/bench/box/windows_gate.sh $TAG $P; last || exit 1
$st windows_apps env KF_WINAPPS_PROFILE=mid bash $R/scripts/bench/box/windows_apps.sh $TAG $P; last || exit 1
$st windows_gate_2 bash $R/scripts/bench/box/windows_gate.sh ${TAG}w2 $P; last || exit 1
echo "CHAIN done $(date -u -Is)" >> /workspace/relgate/stages.log
