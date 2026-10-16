#!/bin/bash
# perfhook.sh LABEL [SECONDS=30] : G3 fallback. perf record -g on (a) the two kf3 workers (the USER ring relay), (b) the vCPU threads, (c) everything else of the qemu pid,
# during a Heaven benchmark; text reports only (perf.data deleted). Output: ev/heaven/LABEL/perf_*.txt
L=$1; S=${2:-30}; O=/data/claude-gap/ev/heaven/$L; mkdir -p $O
QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2995.state)
W=""; V=""
for t in $(ls /proc/$QP/task); do c=$(cat /proc/$QP/task/$t/comm 2>/dev/null); case "$c" in kf3-worker*) W="$W,$t";; "CPU "*|*KVM*) V="$V,$t";; esac; done
W=${W#,}; V=${V#,}
echo "workers=$W vcpus=$V" > $O/perf_threads.txt
for g in W V; do
  eval T=\$$g; [ -n "$T" ] || continue
  perf record -F 1999 -g -t "$T" -o $O/perf_$g.data -- sleep $S > $O/perf_${g}_record.log 2>&1
  perf report -i $O/perf_$g.data --no-children --stdio --sort symbol -g none --percent-limit 0.7 2>/dev/null | head -90 > $O/perf_${g}_flat.txt
  perf report -i $O/perf_$g.data --children --stdio --sort symbol -g none --percent-limit 2 2>/dev/null | head -70 > $O/perf_${g}_children.txt
  perf report -i $O/perf_$g.data --no-children --stdio --sort comm,dso -g none 2>/dev/null | head -30 > $O/perf_${g}_dso.txt
  rm -f $O/perf_$g.data
done
