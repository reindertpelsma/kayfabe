#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_run_capture_late.sh TAG KEYWORDS DELAY SECONDS: like sn_run_capture.sh, but the click comes first and ETW+PresentMon start DELAY s later (for the
# high-volume Profiler keyword: 0x3 = Base + Profiler would fill the 400 MiB cap during the 20 s load phase), then run SECONDS (inside the graphics test).
set -uo pipefail
TAG=$1; KW=${2:-0x3}; DELAY=${3:-28}; SEC=${4:-45}; H=${SN_HOST:?set SN_HOST=root@<trusted-host>}
run() { ssh $H "cd /data && python3 kfagent_server.py run \"\$(cat $1)\" ${2:-120}"; }
cat > /tmp/sn_a_$TAG.ps1 <<PS
& C:\\kf\\sn\\sn_it.ps1 -ScriptArgs "-Action run ${RUN_ARGS:-}" -Wait 30
PS
scp -q /tmp/sn_a_$TAG.ps1 $H:/tmp/sn_a_$TAG.ps1
run /tmp/sn_a_$TAG.ps1 120
T=$(( $(date +%s) + DELAY )); until [ "$(date +%s)" -ge "$T" ]; do sleep 1; done
cat > /tmp/sn_c_$TAG.ps1 <<PS
& C:\\kf\\sn\\sn_cap_start.ps1 -Tag $TAG -Keywords $KW -Seconds $SEC
PS
scp -q /tmp/sn_c_$TAG.ps1 $H:/tmp/sn_c_$TAG.ps1
run /tmp/sn_c_$TAG.ps1 60
T=$(( $(date +%s) + SEC )); until [ "$(date +%s)" -ge "$T" ]; do sleep 1; done
cat > /tmp/sn_b_$TAG.ps1 <<PS
& C:\\kf\\sn\\sn_cap_stop.ps1 -Tag $TAG
PS
scp -q /tmp/sn_b_$TAG.ps1 $H:/tmp/sn_b_$TAG.ps1
run /tmp/sn_b_$TAG.ps1 900
ssh $H "mkdir -p /data/claude-sngap-20261018/$TAG && cp /data/kfagent/r/sngap_${TAG}_zip.txt /data/claude-sngap-20261018/$TAG/$TAG.zip && cd /data/claude-sngap-20261018/$TAG && unzip -oq $TAG.zip && ls -la"
