#!/bin/bash
echo "ROOTLESS_PROC id=$(id) "; grep -E '^(NoNewPrivs|CapInh|CapPrm|CapEff|CapBnd|CapAmb)' /proc/self/status
export BENCH_DIR=/workspace/bench HOME=/home/kfrun KF_DEVICE=kf3 PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
cd /workspace/kf-ci-src
bash scripts/bench/cuda_ladder.sh "$@"
echo "ROOTLESS_RC=$?"
