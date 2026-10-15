#!/bin/bash
# callers.sh OUTDIR : name the caller of the worker's openat storm: perf record -e syscalls:sys_enter_openat --call-graph dwarf on the two workers for 6 s, then report the user stacks (parent chain).
O=$1; mkdir -p $O
QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2991.state)
tids=$(for t in $(ls /proc/$QP/task); do case "$(cat /proc/$QP/task/$t/comm)" in kf3-worker*) echo -n "$t,";; esac; done | sed 's/,$//')
echo "tids=$tids bin=$(readlink /proc/$QP/exe)" > $O/meta.txt
perf record -e syscalls:sys_enter_openat --call-graph dwarf,32768 -t $tids -o $O/perf.data -- sleep 6 > $O/record.out 2>&1
perf report -i $O/perf.data --stdio --no-children --sort sym -g caller,0.5,callee,function,percent --max-stack 30 2>/dev/null | grep -v "^#" | grep -v "^$" | head -150 > $O/callers.txt
perf script -i $O/perf.data -F tid,ip,sym --max-stack 12 2>/dev/null | head -400 > $O/script_head.txt
rm -f $O/perf.data
echo "end=$(date -Is)" >> $O/meta.txt
