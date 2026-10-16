#!/bin/bash
tag=$1; budget=${2:-180}
echo "R_SUITE_START $(date -Is) tag=$tag"
flock -o /tmp/kayfabe-fastguest.lock env KF_LOCK=/data/claude-rootless/root.lock BENCH_DIR=/data/claude-rootless/bench QEMU_BIN=/data/claude-rootless/inst/bin/qemu-system-x86_64 bash /data/claude-rootless/wt/scripts/fastguest/fast_suite.sh $tag $budget
echo "R_SUITE_EXIT rc=$? $(date -Is)"
