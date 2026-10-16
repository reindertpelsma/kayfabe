#!/bin/bash
# kfp.sh TAG MODE JOB.ps1 [timeout_s] : push the job to the guest, start it (service|interactive), wait for done.flag, print C:\kfperf\out\TAG\log.txt (tail), copy it to $EV/guest/TAG.log
TAG=$1; MODE=$2; JOB=$3; TO=${4:-900}; EV=${EV:-/data/claude-gap/ev}; mkdir -p $EV/guest
KFA() { timeout $(( ${2:-60} + 15 )) python3 /data/kfagent_server.py run "$1" ${2:-60}; }
B64=$(base64 -w0 "$JOB")
KFA "New-Item -ItemType Directory -Force C:\\kfperf\\jobs | Out-Null; [IO.File]::WriteAllBytes(\"C:\\kfperf\\jobs\\$TAG.ps1\", [Convert]::FromBase64String(\"$B64\")); \"pushed\"" 60 | tail -1
KFA "& C:\\kfperf\\kfp_launch.ps1 -Tag $TAG -Mode $MODE" 90 | tail -2
t=0; while [ $t -lt $TO ]; do
  r=$(KFA "if (Test-Path C:\\kfperf\\out\\$TAG\\done.flag) { \"DONE\" } else { \"WAIT\" }" 40 2>&1 | tr -d "\r" | tail -1)
  [ "$r" = DONE ] && break; sleep 8; t=$((t+8+3))
done
[ "$r" = DONE ] || echo "TIMEOUT waiting for $TAG after $TO s"
KFA "Get-Content -Raw C:\\kfperf\\out\\$TAG\\log.txt" 60 | tr -d "\r" > $EV/guest/$TAG.log
tail -${TAILN:-25} $EV/guest/$TAG.log
