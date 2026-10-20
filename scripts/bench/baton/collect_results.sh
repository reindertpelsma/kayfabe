#!/bin/bash
# collect_results.sh RUNS_DIR OUT_DIR - small evidence only (no serial logs, no csv): per session the
# report, the session line, the relay loop summaries, the watchdog episodes (parsed) and probe output.
set -eu
R=$1; O=$2; mkdir -p "$O"
HERE="$(cd "$(dirname "$0")" && pwd)"
python3 -I "$HERE/ab_table.py" "$R" > "$O/ab_table.txt"
for d in "$R"/*/; do
  t=$(basename "$d"); [ -d "$d/files" ] || continue
  mkdir -p "$O/$t"
  python3 -I "$HERE/campaign_report.py" --min-gap 1.0 "$d/files" > "$O/$t/report.tsv" 2>&1 || true
  head -1 "$d/session.log" > "$O/$t/session.txt" 2>/dev/null || true
  grep -a "USER_RING_LOOP SUMMARY" "$d"/fast_*_qemu.log 2>/dev/null | cut -c1-600 > "$O/$t/loop_summaries.txt" || true
  for w in "$d"/files/*.wd; do [ -s "$w" ] && python3 -I "$HERE/wd_parse.py" "$w" >> "$O/$t/watchdog.txt"; done || true
  cat "$d"/files/*.probe > "$O/$t/probe.txt" 2>/dev/null || true
  find "$O/$t" -empty -delete
done
