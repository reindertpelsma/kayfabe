#!/bin/bash
echo "ROOTLESS_PROC id=$(id)"; grep -E '^(NoNewPrivs|CapEff|CapBnd)' /proc/self/status
export BENCH_DIR=/workspace/bench HOME=/home/kfrun KF_DEVICE=kf3 PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export CARGO_TARGET_DIR=/workspace/bench/kf-verify-target
cd /workspace/kf-ci-src
bash scripts/fastguest/fast_suite.sh cirelrl4 180
echo "ROOTLESS_SUITE_RC=$?"
