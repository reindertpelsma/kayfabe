#!/bin/bash
# perfall.sh LABEL [SECONDS=30] : owner request 2026-10-16. perf record -g -F 999 on ALL threads of the kayfabe QEMU for SECONDS in the middle of a Heaven benchmark,
# then 20 s of off-CPU time (bcc offcputime, folded stacks, per thread name) for the same process. Text reports only; perf.data is deleted.
# Output under ev/heaven/LABEL/prof/: threads.txt (tid, comm, samples, share), self_<comm>_<tid>.txt, incl_<comm>_<tid>.txt, offcpu_by_thread.txt, offcpu_<comm>.txt
L=$1; S=${2:-30}; O=/data/claude-gap/ev/heaven/$L/prof; mkdir -p $O
QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2995.state)
D=$O/perf.data
perf record -F 999 -g -p $QP -o $D -- sleep $S > $O/record.log 2>&1
# off-CPU / blocking, 20 s, bpftrace: (1) time each kf3-worker thread spends INSIDE each syscall (epoll_wait / futex = waiting, ioctl/read/write = work or BAR access),
# (2) run-queue delay (wakeup -> on CPU) of the worker threads, (3) the number of blocks > 20 us. Per worker thread name.
cat > $O/offcpu.bt <<BT
tracepoint:raw_syscalls:sys_enter /pid == $QP && (comm == "kf3-worker0" || comm == "kf3-worker1")/ { @st[tid] = nsecs; @id[tid] = args->id; }
tracepoint:raw_syscalls:sys_exit /@st[tid]/ { \$d = nsecs - @st[tid]; @sys_us[comm, @id[tid]] = sum(\$d / 1000); @sys_n[comm, @id[tid]] = count(); if (\$d > 20000) { @blk20_n[comm, @id[tid]] = count(); @blk20_us[comm, @id[tid]] = sum(\$d / 1000); } delete(@st[tid]); delete(@id[tid]); }
tracepoint:sched:sched_wakeup /args->comm == "kf3-worker0" || args->comm == "kf3-worker1"/ { @w[args->pid] = nsecs; }
tracepoint:sched:sched_switch /@w[args->next_pid]/ { @rq_us[args->next_comm] = sum((nsecs - @w[args->next_pid]) / 1000); @rq_n[args->next_comm] = count(); delete(@w[args->next_pid]); }
tracepoint:sched:sched_switch /args->prev_comm == "kf3-worker0" || args->prev_comm == "kf3-worker1"/ { @csw[args->prev_comm, args->prev_state == 0 ? "preempted" : "blocked"] = count(); }
interval:s:20 { exit(); }
END { clear(@st); clear(@id); clear(@w); }
BT
timeout 40 bpftrace $O/offcpu.bt > $O/offcpu_bpftrace.txt 2> $O/offcpu.err
python3 - "$O" <<'PY'
import sys, re
O = sys.argv[1]
names = {0:"read",1:"write",3:"close",7:"poll",9:"mmap",11:"munmap",16:"ioctl",17:"pread64",18:"pwrite64",24:"sched_yield",35:"nanosleep",202:"futex",228:"clock_gettime",230:"clock_nanosleep",232:"epoll_wait",257:"openat",270:"pselect6",271:"ppoll",281:"epoll_pwait",318:"getrandom",334:"rseq"}
d = {}
for l in open(O + "/offcpu_bpftrace.txt", errors="replace"):
    m = re.match(r"@(\w+)\[(.*?)\]: (\d+)", l.strip())
    if m: d.setdefault(m.group(1), {})[m.group(2)] = int(m.group(3))
with open(O + "/offcpu_by_thread.txt", "w") as o:
    o.write("# 20 s of bpftrace on the two kf3 workers. us = microseconds inside the syscall; n = calls; blk20 = calls longer than 20 us\n")
    for comm in ("kf3-worker0", "kf3-worker1"):
        o.write("== %s\n" % comm)
        rows = []
        for k, us in d.get("sys_us", {}).items():
            c, sid = k.rsplit(", ", 1)
            if c != comm: continue
            sid = int(sid); n = d["sys_n"].get(k, 0); b = d.get("blk20_n", {}).get(k, 0); bu = d.get("blk20_us", {}).get(k, 0)
            rows.append((us, names.get(sid, "sys%d" % sid), n, b, bu))
        for us, nm, n, b, bu in sorted(rows, reverse=True)[:8]:
            o.write("   %-16s us=%10d (%.1f%% of 20 s) calls=%9d blk20_calls=%8d blk20_us=%10d\n" % (nm, us, us / 200000.0, n, b, bu))
        o.write("   runqueue wait after wakeup: us=%s n=%s\n" % (d.get("rq_us", {}).get(comm), d.get("rq_n", {}).get(comm)))
        for k, v in d.get("csw", {}).items():
            if k.startswith(comm): o.write("   switches %s = %d\n" % (k, v))
PY
# --- reports AFTER the 20 s bpftrace window (perf report is slow; the benchmark must still be running while both samplers run) ---
perf script -i $D -F comm,tid -G 2>/dev/null | awk 'NF>=2 {t=$NF; $NF=""; sub(/ +$/,""); gsub(/ /,"_"); c[t" "$0]++; n++} END {for (k in c) printf "%d %s %.1f%%\n", c[k], k, 100*c[k]/n}' | sort -rn | head -40 > $O/threads.txt   # samples tid comm share
awk '{print $2":"$3}' $O/threads.txt | head -14 > $O/tids.txt
while read -r tc; do
  tid=${tc%%:*}; comm=$(echo "${tc#*:}" | tr ' /' '__')
  perf report -i $D --tid=$tid --no-children --stdio --sort symbol -g none --percent-limit 0.8 2>/dev/null | grep -v "^#" | grep -v "^$" | head -14 > $O/self_${comm}_$tid.txt
  perf report -i $D --tid=$tid --children --stdio --sort symbol -g none --percent-limit 3 2>/dev/null | grep -v "^#" | grep -v "^$" | head -16 > $O/incl_${comm}_$tid.txt
done < $O/tids.txt
perf report -i $D --no-children --stdio --sort comm,dso -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -30 > $O/comm_dso.txt
# call-graph depth check (frame pointers): the fraction of samples whose callchain has one frame only
perf script -i $D -F tid,ip,sym --no-inline 2>/dev/null | awk 'BEGIN{n=0;one=0;fr=0} /^ *[0-9]+ *$/ {next} /^[ \t]*[0-9a-f]+ / {fr++; next} {if(n>0 && fr<=1) one++; n++; fr=0}END{print "samples="n" single_frame_chains="one}' > $O/callchain_depth.txt 2>&1
# perf.data kept until the arm is analysed (deleted by the coordinator script)
