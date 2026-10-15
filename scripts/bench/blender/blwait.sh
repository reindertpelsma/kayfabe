#!/bin/bash
# blwait.sh <tag> <max_wait_s> : wait until the host file server has <tag>__run.txt containing UPLOADED; print run.txt + result BENCH lines
TAG=$1; MAX=${2:-600}; U=/data/claude-blender/up; t=0
while [ $t -lt $MAX ]; do
  if [ -f $U/${TAG}__run.txt ] && grep -q UPLOADED $U/${TAG}__run.txt; then break; fi
  sleep 10; t=$((t + 10))
done
cat $U/${TAG}__run.txt 2>/dev/null; grep -h "^BENCH\|Error\|error\|out of\|memory" $U/${TAG}__stdout.txt 2>/dev/null | head -${3:-40}
