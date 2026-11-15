#!/bin/bash
# stage.sh NAME cmd...  : runs a stage with START/EXIT lines in /workspace/relgate/stages.log, own log per stage
source /workspace/relgate/env.sh
n=$1; shift
mkdir -p /workspace/relgate
echo "STAGE $n START $(date -u -Is) pid=$$ cmd=$*" >> /workspace/relgate/stages.log
"$@" > /workspace/relgate/$n.run 2>&1
rc=$?
echo "STAGE $n EXIT rc=$rc $(date -u -Is)" >> /workspace/relgate/stages.log
