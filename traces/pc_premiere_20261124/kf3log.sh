#!/usr/bin/env bash
# kf3log.sh RUN: counts of the kf3 log lines that mark a walk cut, a ceiling, a dead channel, a slow VA path or a refusal, plus those lines
set -euo pipefail
L=/workspace/bench/pc_premiere_20261124/runs/$1/qemu.log
echo "# kf3 log of run $1: $(wc -l < $L) lines"
for p in "walk per-space CEILING" "walk report CUT" "DEAD" "VA_SLOW" "refuse_mask=0x400" "refuse_mask=0x1000" "CLEARED OVER ABSENCE" "Xid"; do printf "%-24s %s\n" "$p" "$(grep -c -- "$p" $L || true)"; done
echo "# matching lines (hex runs masked, first 40)"
grep -E "walk per-space CEILING|walk report CUT|DEAD|VA_SLOW|refuse_mask|CLEARED OVER ABSENCE|Xid" $L | sed -E "s/GPU-[0-9a-fA-F-]+/GPU-<uuid>/g" | cut -c1-260 | head -40 || true
