#!/bin/bash
# perfall.sh LABEL [SECONDS=30] : owner request 2026-10-16. perf record -g -F 999 on ALL threads of the kayfabe QEMU for SECONDS in the middle of a Heaven benchmark,
# then 20 s of off-CPU time (bcc offcputime, folded stacks, per thread name) for the same process. Text reports only; perf.data is deleted.
# Output under ev/heaven/LABEL/prof/: threads.txt (tid, comm, samples, share), self_<comm>_<tid>.txt, incl_<comm>_<tid>.txt, offcpu_by_thread.txt, offcpu_<comm>.txt
L=$1; S=${2:-30}; O=/data/claude-gap/ev/heaven/$L/prof; mkdir -p $O
QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2995.state)
D=$O/perf.data
perf record -F 999 -g -p $QP -o $D -- sleep $S > $O/record.log 2>&1
perf report -i $D --no-children --stdio --sort tid,comm -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -40 > $O/threads.txt
# per-thread reports for the 14 busiest threads: self symbols and inclusive (children) symbols
for t in $(awk '{print $1}' $O/threads.txt | sed -n 2,200p | head -0); do :; done
awk 'NR>0 && $2 ~ /^[0-9]+:/ {print $2}' $O/threads.txt | head -14 > $O/tids.txt
[ -s $O/tids.txt ] || grep -oE "[0-9]+:[A-Za-z0-9_ -]+" $O/threads.txt | head -14 > $O/tids.txt
while read -r tc; do
  tid=${tc%%:*}; comm=$(echo "${tc#*:}" | tr ' /' '__')
  perf report -i $D --tid=$tid --no-children --stdio --sort symbol -g none --percent-limit 0.8 2>/dev/null | grep -v "^#" | grep -v "^$" | head -14 > $O/self_${comm}_$tid.txt
  perf report -i $D --tid=$tid --children --stdio --sort symbol -g none --percent-limit 3 2>/dev/null | grep -v "^#" | grep -v "^$" | head -16 > $O/incl_${comm}_$tid.txt
done < $O/tids.txt
perf report -i $D --no-children --stdio --sort comm,dso -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -30 > $O/comm_dso.txt
# call-graph depth check (frame pointers): the fraction of samples whose callchain has one frame only
perf script -i $D -F tid,ip,sym --no-inline 2>/dev/null | awk 'BEGIN{n=0;one=0;fr=0} /^ *[0-9]+ *$/ {next} /^[ \t]*[0-9a-f]+ / {fr++; next} {if(n>0 && fr<=1) one++; n++; fr=0}END{print "samples="n" single_frame_chains="one}' > $O/callchain_depth.txt 2>&1
rm -f $D
# off-CPU, 20 s, folded, kernel+user stacks; summarised by thread name
offcputime-bpfcc -p $QP -f -m 50 20 > $O/offcpu_folded.txt 2> $O/offcpu.err
python3 - "$O" <<'PY'
import sys, collections, re
O = sys.argv[1]; tot = collections.Counter(); leaf = collections.defaultdict(collections.Counter)
for l in open(O + "/offcpu_folded.txt", errors="replace"):
    l = l.rstrip("\n")
    if not l or " " not in l: continue
    st, v = l.rsplit(" ", 1)
    try: v = int(v)
    except ValueError: continue
    fr = st.split(";"); comm = fr[0]
    tot[comm] += v
    # the blocking kernel frame is the LAST frame in the folded order (leaf), the user caller the first non-[unknown]
    kern = [f for f in fr[1:] if re.search(r"(futex|epoll|poll|ioctl|read|nanosleep|sched|wait)", f)]
    leaf[comm][(kern[-1] if kern else fr[-1])[:60]] += v
with open(O + "/offcpu_by_thread.txt", "w") as o:
    o.write("# off-CPU microseconds in the 20 s window, by thread name (a thread blocked the whole time shows ~20,000,000)\n")
    for c, v in tot.most_common(20):
        o.write("%-22s %12d us  (%.1f%% of 20 s)\n" % (c, v, v / 200000.0))
        for k, x in leaf[c].most_common(5): o.write("      %-60s %12d us\n" % (k, x))
PY
