#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_run_capture.sh TAG [KEYWORDS] [SECONDS]: one Steel Nomad capture on whatever guest the kfagent broker currently serves
# (kayfabe or VFIO; the same script on both sides). Starts ETW+PresentMon (SYSTEM), clicks RUN AGAIN in the interactive session
# (3DMark Demo must show the results page, or its Run button position must be passed by editing sn_ui.ps1 -FX/-FY),
# waits for the test to end, stops, decodes in the guest (tracerpt) and uploads the zip to /data/claude-sngap-20261018/TAG.zip on the host.
set -uo pipefail
# RUN_ARGS="-FX 0.861 -FY 0.389" selects the RUN button of the Steel Nomad start page (default: RUN AGAIN of the results page)
TAG=$1; KW=${2:-0x1}; SEC=${3:-110}; H=${SN_HOST:?set SN_HOST=root@<trusted-host> (the host that runs kfagent_server.py)}
HERE=$(cd "$(dirname "$0")" && pwd)
run() { ssh $H "cd /data && python3 kfagent_server.py run \"\$(cat $1)\" ${2:-120}"; }
cat > /tmp/sn_a_$TAG.ps1 <<PS
& C:\\kf\\sn\\sn_cap_start.ps1 -Tag $TAG -Keywords $KW -Seconds $SEC
Start-Sleep 3
& C:\\kf\\sn\\sn_it.ps1 -ScriptArgs "-Action run ${RUN_ARGS:-}" -Wait 30
PS
scp -q /tmp/sn_a_$TAG.ps1 $H:/tmp/sn_a_$TAG.ps1
run /tmp/sn_a_$TAG.ps1 120
if [ -n "${HOSTCAP:-}" ]; then   # HOSTCAP="OUTDIR PID SECONDS": the owner's host profiler (capture.sh) over the graphics test, ~22 s after the click
  set -- $HOSTCAP; sleep 18
  ssh $H "nohup bash /data/claude-3dmark-20261018/capture.sh $1 $2 $3 > /dev/null 2>&1 < /dev/null &"; echo "host capture started: $HOSTCAP"
fi
echo "waiting $((SEC-5)) s for the run"
T=$(( $(date +%s) + SEC - 5 )); until [ "$(date +%s)" -ge "$T" ]; do sleep 2; done
cat > /tmp/sn_b_$TAG.ps1 <<PS
& C:\\kf\\sn\\sn_cap_stop.ps1 -Tag $TAG
PS
scp -q /tmp/sn_b_$TAG.ps1 $H:/tmp/sn_b_$TAG.ps1
run /tmp/sn_b_$TAG.ps1 900
ssh $H "mkdir -p /data/claude-sngap-20261018/$TAG && cp /data/kfagent/r/sngap_${TAG}_zip.txt /data/claude-sngap-20261018/$TAG/$TAG.zip && cd /data/claude-sngap-20261018/$TAG && unzip -oq $TAG.zip && ls -la"
