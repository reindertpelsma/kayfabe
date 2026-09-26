#!/usr/bin/env bash
L=/workspace/bench/w390c4.log
echo "C3_START $(date -Is) pid=$$" > $L
cd /root/kayfabe && git fetch -q origin && git checkout -q master && git pull -q --ff-only
echo "HEAD=$(git rev-parse --short HEAD)" >> $L
( env KAYFABE_MMU_INVAL=on KAYFABE_REPO=/root/kayfabe \
   CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290 KAYFABE_TAG=w390c4 \
   POST_CAPTURE_HOOK=/root/kayfabe/scripts/bench/cup3_hook.sh GQ_TIMEOUT=900 \
   scripts/bench/w290p_run.sh drain )
echo "C3_EXIT rc=$? $(date -Is)" >> $L
