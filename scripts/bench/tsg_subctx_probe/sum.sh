#!/bin/bash
# usage: sum.sh dir pattern
cd $1
for f in $2; do
  b=${f%.log}
  echo "=== $b  [$(sed -n 's/^# //p;1q' /data/claude-tsgprobe-20261018/scripts/$b.txt | cut -c1-150)]"
  grep -hE "^RES" $f | sed -E 's/^RES t=[0-9.]+ e=[0-9.]+ //' | grep -E "ctx |chan |obj |bind|sched|token .*status=0x[^0]|work|bench|timeslice|settle|NOT|line [0-9]+ ret|freech|freectx|unknown|no object" | sed -E 's/ vas=none//; s/n0\{info32=0x0 info16=0x0 status=0x0\}//' | cut -c1-260
  echo "   [meta] $(cat $b.meta | sed -E 's/script=[^ ]+ //; s/start=[^ ]+ end=[^ ]+ //; s/xid_total_before=[0-9]+ xid_total_after=[0-9]+ //')"
  grep -hE "Xid|GspRm|Assert" $b.journal | sed -E 's/^[^ ]+ claude kernel: //' | cut -c1-220 | head -5
done
