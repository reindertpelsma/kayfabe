#!/bin/bash
# run.sh SIDE RUN ROW... : apps_matrix.sh with the apps image and the kf3 device (the PR commands, plus the guest image the apps stage exports); caller sets APPS_APP_TMO, KF_NEGCTL, ...
cd /workspace/kf-ci-src
export KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2 KF_DEVICE=kf3
exec bash scripts/apps/apps_matrix.sh "$@"
