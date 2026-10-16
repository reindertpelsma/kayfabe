#!/bin/bash
# profsum.sh LABEL : per-thread summaries from ev/heaven/LABEL/prof/perf.data (kept by perfall.sh). Percentages are RELATIVE to the thread's own samples.
O=/data/claude-gap/ev/heaven/$1/prof; D=$O/perf.data
[ -f $D ] || { echo "no $D"; exit 1; }
awk '{print $2" "$3" "$1" "$4}' $O/threads.txt | head -12 > $O/tids12.txt
: > $O/summary.txt
while read -r tid comm n share; do
  echo "=== thread $tid $comm  samples=$n  share_of_process_samples=$share  (~$(echo "$n" | awk '{printf "%.0f", $1/30/9.99}')% of one CPU over 30 s at 999 Hz)" >> $O/summary.txt
  echo "--- self (top 10)" >> $O/summary.txt
  nice -n 15 perf report -i $D --tid=$tid --percentage relative --no-children --stdio --sort symbol -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -10 | sed -E 's/ +/ /g' | cut -c1-170 >> $O/summary.txt
  echo "--- inclusive (top 12, call-chain frames of kf_/Device/libc/kernel only)" >> $O/summary.txt
  nice -n 15 perf report -i $D --tid=$tid --percentage relative --children --stdio --sort symbol -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | grep -vE "inlined|start_thread|__clone3|call_once|drop<|for_value_raw|size_of_val_raw|thread_start|__rust_begin_short_backtrace|spawn_unchecked" | head -12 | sed -E 's/ +/ /g' | cut -c1-170 >> $O/summary.txt
done < $O/tids12.txt
echo done $1
