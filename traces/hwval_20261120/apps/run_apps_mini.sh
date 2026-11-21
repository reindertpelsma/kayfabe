#!/usr/bin/env bash
# run_apps_mini.sh - the CUDA-samples cut of the Linux app matrix on one box (hwval_20261120): the bare-metal baseline, then the
# same rows inside the fat guest (one app per boot, scripts/apps/apps_matrix.sh unchanged). Run ON the box after mini_bundle.sh
# and inject_guest.sh. KF3_BIN pins the kf3 binary of the revision under test (the newest binary on a box can be another revision's).
set -uo pipefail
REPO=/root/kayfabe
export KF_DEVICE=kf3 KF_GUEST_IMG=/workspace/bench/guest-apps.qcow2 KF3_DEV_EXTRA=guest-driver=580.159.04
export KF3_BIN=${KF3_BIN:?KF3_BIN}
export PATH=$PATH:$HOME/.cargo/bin
APPS="nvidia_smi deviceQuery vectorAdd vectorAddDrv matrixMul matrixMulDrv bandwidthTest simpleStreams asyncAPI simpleCallback simpleCooperativeGroups cudaTensorCoreGemm graphMemoryNodes simpleCudaGraphs simpleCUBLAS simpleCUFFT"
echo "START $(date -Is) rev=$(git -C $REPO rev-parse HEAD) kf3=$KF3_BIN"
bash $REPO/scripts/apps/apps_matrix.sh host hwval_apps $APPS > /root/prov/apps_host.log 2>&1; echo "HOST_RC=$? $(date -Is)"
bash $REPO/scripts/apps/apps_matrix.sh guest hwval_apps $APPS > /root/prov/apps_guest.log 2>&1; echo "GUEST_RC=$? $(date -Is)"
echo "EXIT $(date -Is)"
