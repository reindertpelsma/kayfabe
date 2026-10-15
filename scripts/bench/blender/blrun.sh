#!/bin/bash
# blrun.sh <tag> <timeout_s> <priority> "<blender args>" : start bl_run.ps1 detached in the guest (SYSTEM, hidden). Poll with blwait.sh.
# '#d' inside the args is replaced by the guest output dir C:\kf\apps\blender\out\<tag>.
TAG=$1; TO=$2; PRI=$3; ARGS=$4
CMD="\$a = '$ARGS'; Start-Process powershell -WindowStyle Hidden -ArgumentList ('-NoProfile -ExecutionPolicy Bypass -File C:\\kf\\apps\\blender\\bl_run.ps1 -Tag $TAG -TimeoutSec $TO -Priority $PRI -BlArgs \"' + \$a + '\"'); 'launched $TAG'"
timeout 40 python3 /data/kfagent_server.py run "$CMD" 30
