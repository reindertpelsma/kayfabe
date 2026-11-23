#!/bin/bash
# step.sh NAME CMD... : one long step in the background with a start marker, a log and an exit line (a killed job and a running job look the same).
N=$1; shift; mkdir -p /workspace/steps
echo "START $N $(date -u +%FT%TZ) cmd=$*" > /workspace/steps/$N.start; rm -f /workspace/steps/$N.exit
nohup setsid bash -c "$* > /workspace/steps/$N.log 2>&1; echo \"EXIT rc=\$? \$(date -u +%FT%TZ)\" > /workspace/steps/$N.exit" >/dev/null 2>&1 &
echo "started $N pid=$!"
