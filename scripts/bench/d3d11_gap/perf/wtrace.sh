#!/bin/bash
# wtrace.sh OUTDIR : attribute the SYSTEM time of kf3-worker0/1: perf trace -s (syscall classes, 40 s), perf record -g kernel stacks (25 s), ioctl command census (8 s). Root, host only.
O=$1; mkdir -p $O
QP=${QP:-$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2995.state)}
T0=$(ls /proc/$QP/task | while read t; do [ "$(cat /proc/$QP/task/$t/comm)" = kf3-worker0 ] && echo $t; done)
T1=$(ls /proc/$QP/task | while read t; do [ "$(cat /proc/$QP/task/$t/comm)" = kf3-worker1 ] && echo $t; done)
echo "qpid=$QP worker0=$T0 worker1=$T1 start=$(date -Is)" > $O/meta.txt
timeout 70 perf trace -s -t $T0,$T1 -o $O/trace_summary.txt -- sleep 40 > /dev/null 2>&1
echo "trace_s done $(date +%T)" >> $O/meta.txt
perf record -g -t $T0,$T1 -o $O/perf.data -- sleep 25 > $O/record.out 2>&1
perf report -i $O/perf.data --stdio --no-children --sort sym -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -45 > $O/top_symbols.txt
perf report -i $O/perf.data --stdio --no-children --sort comm,sym -g caller,0.5,callee --max-stack 14 2>/dev/null | grep -v "^#" | head -120 > $O/callgraph_top.txt
rm -f $O/perf.data $O/perf.data.old
echo "record done $(date +%T)" >> $O/meta.txt
timeout 30 perf trace -e ioctl -t $T0,$T1 -o $O/ioctl_raw.txt -- sleep 8 > /dev/null 2>&1
awk '{ if (match($0, /cmd: [A-Za-z0-9_|x]+/)) c=substr($0, RSTART, RLENGTH); else c="?"; if (match($0, /ioctl\(fd: [0-9]+/)) f=substr($0, RSTART+11, RLENGTH-11); else f="?"; n[f" "c]++; tot++ } END { for (k in n) print n[k], k; print tot, "TOTAL" }' $O/ioctl_raw.txt | sort -rn | head -30 > $O/ioctl_census.txt
wc -l $O/ioctl_raw.txt >> $O/meta.txt; head -3 $O/ioctl_raw.txt >> $O/meta.txt
ls -l /proc/$QP/task/$T0/fd 2>/dev/null | awk '{print $9, $10, $11}' | grep -E "nvidia|dri|uvm|memfd|dma" | head -30 > $O/worker0_fds.txt
echo "end=$(date -Is)" >> $O/meta.txt
