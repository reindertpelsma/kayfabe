#!/usr/bin/env bash
# ★ 2026-10-20 -- live stack dump on a progress starvation (MEASUREMENT; pairs with KF3_DIAG_PROGRESS).
#
#   usage: bash scripts/bench/progress_gdb.sh <qemu-pid> <log-file> [out-dir]
#
# Waits for a `PROGRESS_STARVED` line in <log-file> (the kf3 diagnostic prints one per starved point, per
# worker rule and per relay block: docs/design/V3_USER_RING_EVENT_LOOP_20261014.md section 11), then runs
#   gdb -p <pid> -batch -ex 'set pagination off' -ex 'thread apply all bt 16' -ex detach
# and saves the output to <out-dir>/progress-gdb-<pid>-<UTC time>.txt (default <out-dir>: the log's directory).
#
# COST AND TIMING (read before using it):
#  * each attach STOPS THE WHOLE VM PROCESS for about a second (every thread, the vCPUs included). The guest sees a
#    1 s pause; a starvation measurement is disturbed AFTER the detection, never before it. Nothing here attaches
#    until the diagnostic itself has printed the starvation, and a pause can end or create a starvation by itself, so
#    a dump taken after the first one is evidence of the state at that instant only.
#  * at most 3 attachments per boot (= per <pid>; the script exits after the third). Lines within COOLDOWN seconds
#    of an attachment belong to the same burst and do not attach again.
#  * needs ptrace rights on <pid> (run as root) and gdb. Without gdb it falls back to /proc/<pid>/task/*/{stat,
#    wchan,syscall,stack} (kernel stacks need root) and says so in the output; that fallback does not pause the VM.
#  * the binary must carry symbols for names in the backtraces (the kf3-bins builds do).
#
# The script does not touch locks, VMs or GPUs; it only reads the log and attaches to the pid it was given.
set -u
PID=${1:?usage: progress_gdb.sh <qemu-pid> <log-file> [out-dir]}
LOG=${2:?usage: progress_gdb.sh <qemu-pid> <log-file> [out-dir]}
OUT=${3:-$(dirname "$LOG")}
MAX=3
COOLDOWN=${PROGRESS_GDB_COOLDOWN:-20}
case "$PID" in '' | *[!0-9]*) echo "progress_gdb: <qemu-pid> must be a number" >&2; exit 2 ;; esac
[ -r "$LOG" ] || { echo "progress_gdb: cannot read $LOG" >&2; exit 2; }
mkdir -p "$OUT" || exit 2
HAVE_GDB=0
command -v gdb >/dev/null 2>&1 && HAVE_GDB=1
echo "PROGRESS_GDB_START $(date -u +%FT%TZ) pid=$PID log=$LOG gdb=$HAVE_GDB max=$MAX out=$OUT"

dump() {
  local f=$1
  {
    echo "# progress_gdb: attach at $(date -u +%FT%TZ) pid=$PID (trigger: $2)"
    if [ "$HAVE_GDB" = 1 ]; then
      echo "# tool: gdb (the VM was paused while attached)"
      timeout 60 gdb -iex 'set debuginfod enabled off' -p "$PID" -batch -ex 'set pagination off' -ex 'thread apply all bt 16' -ex detach 2>&1
    else
      echo "# tool: /proc fallback (gdb not installed): stat, wchan, syscall, kernel stack per thread; no user stacks"
      for t in /proc/"$PID"/task/*; do
        echo "== tid ${t##*/} comm=$(cat "$t/comm" 2>/dev/null)"
        echo "stat:    $(cut -d' ' -f3,14,15 "$t/stat" 2>/dev/null)  (state utime stime)"
        echo "wchan:   $(cat "$t/wchan" 2>/dev/null)"
        echo "syscall: $(cat "$t/syscall" 2>/dev/null)"
        cat "$t/stack" 2>/dev/null | sed 's/^/stack:   /'
      done
    fi
    echo "# progress_gdb: done at $(date -u +%FT%TZ)"
  } >"$f" 2>&1
}

n=0
last=0
# tail -F survives log rotation; -n0: only lines written from now on
tail -n0 -F "$LOG" 2>/dev/null | while IFS= read -r line; do
  kill -0 "$PID" 2>/dev/null || { echo "PROGRESS_GDB_EXIT pid=$PID gone attachments=$n"; break; }
  case "$line" in *PROGRESS_STARVED*) ;; *) continue ;; esac
  now=$(date +%s)
  [ $((now - last)) -ge "$COOLDOWN" ] || continue
  if [ "$n" -ge "$MAX" ]; then
    echo "PROGRESS_GDB_REFUSED pid=$PID: already $n attachments this boot (max $MAX)"
    break
  fi
  n=$((n + 1))
  last=$now
  f="$OUT/progress-gdb-$PID-$(date -u +%Y%m%dT%H%M%SZ).txt"
  echo "PROGRESS_GDB_ATTACH #$n pid=$PID file=$f trigger=$(printf '%s' "$line" | cut -c1-160)"
  dump "$f" "$(printf '%s' "$line" | cut -c1-160)"
  echo "PROGRESS_GDB_DONE #$n file=$f bytes=$(wc -c <"$f")"
  [ "$n" -lt "$MAX" ] || { echo "PROGRESS_GDB_EXIT pid=$PID attachments=$n (max reached)"; break; }
done
# the pipeline's tail may linger until its next write; end it
pkill -P $$ tail 2>/dev/null
exit 0
