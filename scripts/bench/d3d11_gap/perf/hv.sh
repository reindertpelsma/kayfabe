#!/bin/bash
# hv.sh LABEL API(direct3d11|opengl) [SMI]  : assert environment, then one Heaven in-app benchmark on the running kayfabe VM
LABEL=$1; API=$2; P=/data/claude-gap/perf; EV=/data/claude-gap/ev/heaven/$LABEL; mkdir -p $EV
bash $P/assert_env.sh $EV/assert.txt > /dev/null
echo "HV_START $LABEL $API $(date -Is)" > $EV/startmarker
bash $P/heaven_run.sh $LABEL $API K ${3:-0} > /data/claude-gap/ev/heaven/$LABEL.out 2>&1
echo "HV_EXIT $LABEL rc=$? $(date -Is)" >> $EV/startmarker
grep -a -E "FPS|Score" /data/claude-gap/ev/heaven/$LABEL.out | head -3
