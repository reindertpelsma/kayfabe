#!/bin/bash
# run_setobj.sh OUTDIR CASE:REP [CASE:REP ...]
# Runs each (case, rep) as a fresh process (= fresh RM client/TSG/channel) and brackets it with a host-journal window.
# MUST be started under:  flock -o /tmp/kayfabe-fastguest.lock bash run_setobj.sh ...   (one GPU job, < 5 min).
# Writes OUTDIR/<case>_<rep>.log (probe output), .journal (NVRM lines in the window), .meta (start/end epoch, pid, rc).
set -u
OUT=$1; shift
BIN=${SETOBJ_BIN:-/data/claude-setobj/bin/setobj_probe}
mkdir -p "$OUT"
echo "JOB_START $(date -Is) pid=$$ n=$# bin_md5=$(md5sum "$BIN" | cut -c1-12)"
for cr in "$@"; do
  c=${cr%%:*}; r=${cr##*:}
  s=$(date +%s.%N)
  timeout 60 "$BIN" "$c" "$r" > "$OUT/${c}_${r}.log" 2>&1 &
  p=$!
  wait $p; rc=$?
  e=$(date +%s.%N)
  sleep 2
  # the probe's own pid is in its START line; the journal window is padded by 1 s before and 3 s after
  ppid_=$(grep -m1 -o 'pid=[0-9]*' "$OUT/${c}_${r}.log" | head -1 | cut -d= -f2)
  journalctl -k --no-pager -o short-iso --since "@$(printf '%.0f' "$(echo "$s - 1" | bc)")" --until "@$(printf '%.0f' "$(echo "$e + 4" | bc)")" 2>/dev/null | grep -E 'NVRM|Xid' > "$OUT/${c}_${r}.journal"
  echo "case=$c rep=$r pid=${ppid_:-?} rc=$rc start=$s end=$e" > "$OUT/${c}_${r}.meta"
  nx=$(grep -c "pid=${ppid_:-NONE}," "$OUT/${c}_${r}.journal")
  echo "DONE case=$c rep=$r pid=${ppid_:-?} rc=$rc xid_lines_for_pid=$nx"
  if ! nvidia-smi -L > /dev/null 2>&1; then echo "ABORT: nvidia-smi failed after $c:$r"; break; fi
done
echo "JOB_END $(date -Is)"
