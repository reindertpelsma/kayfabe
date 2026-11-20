#!/bin/bash
set -u
cd /workspace/kf-ci-src
nohup bash -c 'dmesg -w --time-format iso >> /workspace/kernel.log 2>&1' >/dev/null 2>&1 &
echo $! > /workspace/kernel_log.pid
nohup bash -c 'while :; do echo "$(date -u +%FT%TZ) bytes=$(stat -c%s /workspace/kernel.log) xid=$(grep -c Xid /workspace/kernel.log)" >> /workspace/kernel_offsets.log; sleep 30; done' >/dev/null 2>&1 &
SOFT=$(( ($(date +%s) + 300*60) * 1000 ))
echo "START $(date -u +%FT%TZ) sha=84dff90b6a4b088f2817e5d32c0b5136303148d6 soft_deadline_ms=$SOFT instruction_sha256=$(sha256sum /workspace/instruction.json | cut -d' ' -f1)" > /workspace/payload_start.marker
nohup setsid bash -c "KF_SOFT_DEADLINE_MS=$SOFT python3 -I scripts/ci/vast_payload.py --instruction /workspace/instruction.json 84dff90b6a4b088f2817e5d32c0b5136303148d6 743d0ffb3e3a27f4af3bfd0370fbc5b5 > /workspace/payload_stdout.log 2>&1; echo \"EXIT rc=\$? \$(date -u +%FT%TZ)\" > /workspace/payload_exit.line" >/dev/null 2>&1 &
echo $! > /workspace/payload.pid
sleep 3; cat /workspace/payload_start.marker; pgrep -af vast_payload | head -3
