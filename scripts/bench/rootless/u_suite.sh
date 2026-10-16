#!/bin/bash
# fast suite as kfrootless under the shared host lock; start marker + exit line
tag=$1; budget=${2:-180}
echo "U_SUITE_START $(date -Is) tag=$tag budget=$budget"
rm -f /data/claude-rootless/ev/STOP_WATCH
nohup /data/claude-rootless/bin/watch_caps.sh /data/claude-rootless/ev/caps_$tag.txt >/dev/null 2>&1 &
W=$!
flock -o /tmp/kayfabe-fastguest.lock /data/claude-rootless/bin/as_user.sh env KF_LOCK=/home/kfrootless/run.lock BENCH_DIR=/home/kfrootless/bench QEMU_BIN=/data/claude-rootless/inst/bin/qemu-system-x86_64 bash /data/claude-rootless/wt/scripts/fastguest/fast_suite.sh $tag $budget
rc=$?
touch /data/claude-rootless/ev/STOP_WATCH
echo "U_SUITE_EXIT rc=$rc $(date -Is)"
