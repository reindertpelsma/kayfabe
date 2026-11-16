#!/bin/bash
set -u
cd /workspace/kf-ci-src
nohup bash -c 'dmesg -w --time-format iso >> /workspace/kernel.log 2>&1' >/dev/null 2>&1 &
echo $! > /workspace/kernel_log.pid
nohup python3 -I /workspace/offsets.py >/dev/null 2>&1 &
SOFT=$(( ($(date +%s) + 255*60) * 1000 ))
echo "START $(date -u +%FT%TZ) sha=9ddb897e7281ecff4baa9d35acb863bbd265644a soft_deadline_ms=$SOFT instruction_sha256=$(sha256sum /workspace/instruction.json | cut -d' ' -f1)" > /workspace/payload_start.marker
nohup setsid bash -c "KF_SOFT_DEADLINE_MS=$SOFT python3 -I scripts/ci/vast_payload.py --instruction /workspace/instruction.json --only provision --only cuda_host --only merge_bar --only cuda_guest --only windows_gate 9ddb897e7281ecff4baa9d35acb863bbd265644a 42865825151a12e7c78bdcd9cd0f79b9 > /workspace/payload_stdout.log 2>&1; echo \"EXIT rc=\$? \$(date -u +%FT%TZ)\" > /workspace/payload_exit.line" >/dev/null 2>&1 &
echo $! > /workspace/payload.pid
sleep 3; cat /workspace/payload_start.marker; pgrep -af vast_payload | head -3
