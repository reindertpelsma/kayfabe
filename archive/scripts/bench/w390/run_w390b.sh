#!/usr/bin/env bash
# ★ START marker + EXIT line: "file exists but has no terminator" must be detectable.
L=/workspace/bench/w390b2.log
echo "W390B_START $(date -Is) pid=$$" > $L
cd /root/kayfabe
KAYFABE_REPO=/root/kayfabe \
CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290 \
KAYFABE_TAG=w390b2 \
POST_CAPTURE_HOOK=/root/kayfabe/scripts/bench/cup3_hook.sh \
GQ_TIMEOUT=900 \
  scripts/bench/w290p_run.sh drain >> $L 2>&1
echo "W390B_EXIT rc=$? $(date -Is)" >> $L
