#!/bin/bash
# usage: run_leg.sh LEG "payload args before sha/nonce"
LEG=$1; shift
cd /workspace/kf-ci-src
SHA=9b3d165192e03b5cd27a7eca1a0f7f494cbc058a
NONCE=$(python3 -c "import secrets;print(secrets.token_hex(16))")
SOFT=$(( ($(date +%s)+${SOFT_S:-11000})*1000 ))
echo "START leg=$LEG $(date -u +%FT%TZ) sha=$SHA soft_deadline_ms=$SOFT args=$*" > /workspace/payload_start_$LEG.marker
KF_SOFT_DEADLINE_MS=$SOFT python3 -I scripts/ci/vast_payload.py --instruction /workspace/instruction.json "$@" $SHA $NONCE > /workspace/payload_stdout_$LEG.log 2>&1
echo "EXIT rc=$? $(date -u +%FT%TZ)" > /workspace/payload_exit_$LEG.line
