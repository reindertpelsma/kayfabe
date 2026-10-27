#!/bin/busybox sh
# Packaged as /bin/kftool beside the unchanged cup3 and cup8bench instruments.
set -eu
exec 8</dev/nvidia0
start=${KF_Z1_START:-0}
echo "Z1_READY epoch=$(date +%s)"
while [ "$(date +%s)" -lt "$start" ]; do sleep 1; done
echo "Z1_WORK_BEGIN epoch=$(date +%s)"
/bin/cup3
export BENCH_SIZES=1024 BENCH_ITERS=4000 BENCH_BATCH=10 BENCH_VERIFY=1
/bin/cup8bench
echo "Z1_WORK_END epoch=$(date +%s)"
