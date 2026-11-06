#!/bin/bash
# run_probe.sh OUTDIR SCRIPT [SCRIPT ...]
# Runs each probe script as a fresh process (= fresh RM client / TSG) as the UNPRIVILEGED user `ubuntu` (uid 1000,
# not in any privileged RM group; /dev/nvidia* are 0666) and brackets it with a host-journal window.
# MUST be started under:  flock /tmp/kayfabe-fastguest.lock bash run_probe.sh ...   (one GPU job at a time),
# with the owner's kayfabe VM STOPPED when the script can fault a channel.
# Per script: OUTDIR/<name>.log (probe output), .journal (NVRM/Xid lines in the window), .meta (times, pid, rc, Xid counts).
set -u
OUT=$1; shift
BIN=${PROBE_BIN:-/data/claude-tsgprobe-20261018/bin/tsg_subctx_probe}
USER_RUN=${PROBE_USER:-ubuntu}
mkdir -p "$OUT"; chmod 0777 "$OUT"
echo "JOB_START $(date -Is) pid=$$ n=$# bin_md5=$(md5sum "$BIN" | cut -c1-12) user=$USER_RUN"
xid_total() { journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid'; }
for scr in "$@"; do
  name=$(basename "$scr" .txt)
  x0=$(xid_total)
  s=$(date +%s.%N)
  runuser -u "$USER_RUN" -- timeout 90 "$BIN" "$scr" > "$OUT/$name.log" 2>&1 &
  p=$!
  wait $p; rc=$?
  e=$(date +%s.%N)
  sleep 3
  pid_=$(grep -m1 -o 'pid=[0-9]*' "$OUT/$name.log" | head -1 | cut -d= -f2)
  journalctl -k --no-pager -o short-iso --since "@$(printf '%.0f' "$(echo "$s - 1" | bc)")" --until "@$(printf '%.0f' "$(echo "$e + 5" | bc)")" 2>/dev/null | grep -E 'NVRM|Xid' > "$OUT/$name.journal"
  x1=$(xid_total)
  nx=$(grep -c "pid=${pid_:-NONE}," "$OUT/$name.journal")
  echo "script=$scr rc=$rc pid=${pid_:-?} start=$s end=$e xid_total_before=$x0 xid_total_after=$x1 xid_delta=$((x1-x0)) xid_lines_for_pid=$nx" > "$OUT/$name.meta"
  echo "DONE $name rc=$rc pid=${pid_:-?} xid_delta=$((x1-x0)) xid_lines_for_pid=$nx"
  if ! nvidia-smi -L > /dev/null 2>&1; then echo "ABORT: nvidia-smi failed after $name"; break; fi
done
echo "JOB_END $(date -Is)"
