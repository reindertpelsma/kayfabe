#!/bin/bash
# usage: u_fast.sh <tag> <budget> [KF_ARMS...] : fast guest as kfrootless under the shared host lock
tag=$1; budget=$2; arms=$3
nohup /data/claude-rootless/bin/sampler.sh "kf-fastguest" /data/claude-rootless/ev/sample_$tag.txt >/dev/null 2>&1 &
flock -o /tmp/kayfabe-fastguest.lock /data/claude-rootless/bin/as_user.sh env KF_LOCK=/home/kfrootless/run.lock BENCH_DIR=/home/kfrootless/bench QEMU_BIN=/data/claude-rootless/inst/bin/qemu-system-x86_64 KF_ARMS="$arms" bash /data/claude-rootless/wt/scripts/fastguest/run_fast_guest.sh $tag $budget
