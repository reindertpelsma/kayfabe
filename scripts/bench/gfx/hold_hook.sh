#!/usr/bin/env bash
# ★ POST_CAPTURE_HOOK that HOLDS the guest up (driver loaded, device opened) for interactive
# diagnosis over gssh_nv, until `touch /workspace/bench/release_<tag>` or HOLD_MAX seconds.
# The boot's evidence (dmesg after, census) is still collected by boot_capture.sh on release.
TAG=${1:?tag}; B=${BENCH_DIR:-/workspace/bench}; MAX=${HOLD_MAX:-3600}
echo "HOLD_START $(date -Is) — release with: touch $B/release_$TAG"
touch "$B/holding_$TAG"
t=0; while [ ! -f "$B/release_$TAG" ] && [ $t -lt "$MAX" ]; do sleep 5; t=$((t+5)); done
rm -f "$B/holding_$TAG" "$B/release_$TAG"
echo "HOLD_END $(date -Is) after ${t}s"
