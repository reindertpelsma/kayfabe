#!/bin/bash
# queue.sh TAG "case:rep case:rep ..." [more groups...]   (runs on the host; each group is a separate lock-holding job)
cd /data/claude-setobj/src
tag=$1; shift
n=0
for grp in "$@"; do
  n=$((n+1))
  flock -o /tmp/kayfabe-fastguest.lock bash run_setobj.sh ../out/${tag}_$n $grp > ../out/${tag}_$n.job 2>&1
done
echo QUEUE_DONE > ../out/${tag}.done
