#!/usr/bin/env bash
# CAUTION: perf sched record writes ~3 MB/s idle (365 MB per 120 s) and more under load: use windows of 60-120 s around a stutter, not the whole session (perf.data is deleted after the summary).
# harvest.sh <qemu_pid> <seconds> [tag]   (run as root on the host; low overhead; read-only on the VM)
# Records for the qemu process (all threads) over <seconds>:
#   perf sched record -p  -> perf sched latency + timehist (only scheduling delays > 1 ms)      [H5 host contention]
#   perf stat per-thread (task-clock, ctx switches, migrations)                                 [H5]
#   cpu frequency/governor samples each second                                                  [H5]
#   nvidia-smi dmon -s um (GPU util/memory), timestamped                                        [GPU load]
#   qemu.log counter lines (PROF, display fps late_max_us, USER_RING_LOOP, REFUSED) with host wall-clock timestamps [H1 H2 H3]
#   thread list (ps -L) of the kayfabe threads
# OFFCPU=1 additionally runs offcputime-bpfcc -p <pid> -f -m 1000 (folded stacks, top 60 lines kept)  [H5 / blocking]
# Output: /data/claude-cursor2/live3/harvest-<tag>-<utc stamp>/ (text only; perf.data is deleted after summarising)
# NOTE: the USER_RING_LOOP SUMMARY lines are printed by qemu at guest shutdown only; the PROF and display lines are periodic.
#       Run `harvest.sh <pid> <s>` DURING the play session; after the session ends, `harvest_summary.sh` is not needed: stop the guest
#       gracefully and `grep 'USER_RING_LOOP SUMMARY' qemu.log`.
set -u
PID=${1:?qemu pid}; DUR=${2:?seconds}; TAG=${3:-run}
RUNDIR=${RUNDIR:-/var/lib/kf-windows-20261005/boundary-kayfabe-2995}; LOG=$RUNDIR/qemu.log
kill -0 "$PID" 2>/dev/null || { echo "pid $PID not alive"; exit 1; }
OUT=${HOUT:-/data/claude-gap/ev}/harvest-$TAG-$(date -u +%Y%m%dT%H%M%S); mkdir -p "$OUT"
echo "pid=$PID dur=$DUR start=$(date -Is) rev=$(readlink /proc/$PID/exe | sed 's#.*/kf3-bins/##')" > "$OUT/meta.txt"
ps -L -p "$PID" -o tid,comm,pcpu --no-headers | awk '$2!~/^qemu-system/ || NR<3' > "$OUT/threads-kayfabe.txt"
ps -L -p "$PID" -o tid,comm,pcpu,psr > "$OUT/threads-all.txt"
OFFSET=$(stat -c %s "$LOG")
# 1. log tail with host timestamps
( timeout $((DUR+2)) tail -c +$((OFFSET+1)) -F "$LOG" 2>/dev/null | grep -a --line-buffered -E 'PROF|display fps|USER_RING_LOOP|REFUSED|late_max|backstop' | while IFS= read -r l; do printf '%s %s\n' "$(date +%T.%3N)" "$l"; done > "$OUT/qemu-counters.txt" ) &
LP=$!
# 2. gpu
timeout $((DUR+2)) nvidia-smi dmon -s um -d 1 -c "$DUR" -o T > "$OUT/dmon.txt" 2>&1 &
SHORT="$!"
# 3. cpu frequency, 1 Hz
( for i in $(seq 1 "$DUR"); do printf '%s ' "$(date +%T)"; cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq | awk '{s+=$1;if(min==""||$1<min)min=$1;if($1>max)max=$1}END{printf "min_khz=%d avg_khz=%d max_khz=%d\n",min,s/NR,max}'; sleep 1; done > "$OUT/cpufreq.txt" ) &
SHORT="$SHORT $!"
# 4. perf stat per thread (cheap)
perf stat -p "$PID" --per-thread -e task-clock,context-switches,cpu-migrations -o "$OUT/perf-stat.txt" -- sleep "$DUR" &
SHORT="$SHORT $!"
# 5. perf sched
perf sched record -p "$PID" -o "$OUT/perf-sched.data" -- sleep "$DUR" > "$OUT/perf-sched.rec.txt" 2>&1 &
SHORT="$SHORT $!"
# 6. offcpu
if [ "${OFFCPU:-0}" = 1 ]; then timeout $((DUR+5)) offcputime-bpfcc -p "$PID" -f -m 1000 "$DUR" > "$OUT/offcpu.folded.full" 2> "$OUT/offcpu.err" & SHORT="$SHORT $!"; fi
wait $SHORT 2>/dev/null
wait "$LP" 2>/dev/null
# summaries
if [ -s "$OUT/perf-sched.data" ]; then
  perf sched latency -i "$OUT/perf-sched.data" -s max 2>/dev/null | head -45 > "$OUT/sched-latency.txt"
  perf sched timehist -i "$OUT/perf-sched.data" 2>/dev/null | awk 'NR<=3 || ($(NF-1)+0)>1.0' | head -300 > "$OUT/sched-delays-gt1ms.txt"
  ls -la "$OUT/perf-sched.data" | awk '{print "perf.data bytes:",$5}' >> "$OUT/meta.txt"; rm -f "$OUT/perf-sched.data" "$OUT/perf-sched.data.old"
fi
[ -s "$OUT/offcpu.folded.full" ] && { awk '{print $NF, $0}' "$OUT/offcpu.folded.full" | sort -rn | head -60 | cut -d' ' -f2- > "$OUT/offcpu.top60.folded"; rm -f "$OUT/offcpu.folded.full"; }
# counters digest: max late_max_us, PROF lines count, any REFUSED
{
  echo "display-fps lines: $(grep -c 'display fps' "$OUT/qemu-counters.txt")  max late_max_us: $(grep -o 'late_max_us=[0-9]*' "$OUT/qemu-counters.txt" | cut -d= -f2 | sort -n | tail -1)"
  echo "PROF lines: $(grep -c ' PROF' "$OUT/qemu-counters.txt")"
  echo "REFUSED lines: $(grep -c REFUSED "$OUT/qemu-counters.txt")"
  echo "xid since start: $(journalctl -k --since "$(sed -n 's/.*start=\([^ ]*\).*/\1/p' "$OUT/meta.txt")" --no-pager 2>/dev/null | grep -c 'Xid')"
} >> "$OUT/meta.txt"
# PROF lines are ~15 KB per 5 s heartbeat: keep them compressed, keep the compact lines readable
grep -a ' PROF' "$OUT/qemu-counters.txt" | gzip > "$OUT/prof.txt.gz"; grep -a -v ' PROF' "$OUT/qemu-counters.txt" > "$OUT/counters-noprof.txt"
grep -a -E ' PROF t=.*bar0_handler| PROF drainer_wake| PROF va ' "$OUT/qemu-counters.txt" > "$OUT/prof-key.txt"; rm -f "$OUT/qemu-counters.txt"
echo "end=$(date -Is)" >> "$OUT/meta.txt"
du -sh "$OUT" | tail -1; echo "OUT=$OUT"
