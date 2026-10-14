#!/bin/bash
# marks2.sh <run-dir> <out> : every 5 s, host time + qemu.log and trace.log line counts
R=$1; O=$2; echo "MARKS_START $(date -Is) pid=$$" >> $O
while kill -0 $(pgrep -f "name boundary-kayfabe-${R##*-} " | head -1) 2>/dev/null; do
  echo "$(date +%H:%M:%S.%N | cut -c1-12) lines=$(wc -l < $R/qemu.log) trace=$(wc -l < $R/trace.log 2>/dev/null)" >> $O; sleep 5; done
echo "MARKS_EXIT $(date -Is)" >> $O
