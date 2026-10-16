#!/bin/bash
# cyc.sh OUTDIR : arm/release cycle rate of the two workers: perf stat of openat/mmap/munmap/ioctl syscalls for 10 s
O=$1; mkdir -p $O
QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2995.state)
tids=$(for t in $(ls /proc/$QP/task); do case "$(cat /proc/$QP/task/$t/comm)" in kf3-worker*) echo -n "$t,";; esac; done | sed 's/,$//')
perf stat -e syscalls:sys_enter_openat,syscalls:sys_enter_mmap,syscalls:sys_enter_munmap,syscalls:sys_enter_ioctl,syscalls:sys_enter_epoll_wait,syscalls:sys_enter_futex -t $tids -o $O/syscall_rates.txt -- sleep 10 > /dev/null 2>&1
python3 /data/claude-gap/perf/threadcpu.py snap $QP $O/t0.json; sleep 10; python3 /data/claude-gap/perf/threadcpu.py snap $QP $O/t1.json; python3 /data/claude-gap/perf/threadcpu.py diff $O/t0.json $O/t1.json 6 > $O/threads_10s.txt
