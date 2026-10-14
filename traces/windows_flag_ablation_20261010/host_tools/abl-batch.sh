#!/bin/bash
# abl-batch.sh — pops "NAME|KF3_A KF3_B" lines from ablate/todo.txt, runs each as the next run number (queued behind other users of the GPU), summarises into ablate/results.txt
W=/var/lib/kf-windows-20261005; A=$W/ablate; BIN=${ABL_BIN:-89721ca4}; HOLD=${ABL_HOLD:-180}
echo $$ > $A/batch.pid
while :; do
  [ -e $A/stop ] && { echo "$(date -Is) stop file; exit" >> $A/batch.log; exit 0; }
  line=$(head -1 $A/todo.txt 2>/dev/null); [ -n "$line" ] || { echo "$(date -Is) todo empty; exit" >> $A/batch.log; exit 0; }
  sed -i 1d $A/todo.txt
  name=${line%%|*}; drop=${line#*|}; [ "$drop" = "$line" ] && drop=""
  N=$(cat $A/nextrun); echo $((N+1)) > $A/nextrun
  echo "$(date -Is) run $N name=$name drop=[$drop]" >> $A/batch.log
  echo "$N $name" >> $A/runs.txt
  bash $W/tdropus/queue.sh $N $BIN $HOLD tdr-run27.sh KF_GUEST_PW=kfsign7 DWM_OVERLAY_OFF=1 SKIP_SHORTS=1 SCROLL_EVERY=3600 RUN_WIN_FLAGS= "ABL_DROP=$drop" > $A/run$N.out 2>&1
  { echo "=== $name"; bash $A/abl-sum.sh $N; } >> $A/results.txt 2>&1
  echo "$(date -Is) run $N done" >> $A/batch.log
done
