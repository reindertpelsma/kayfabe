#!/bin/bash
# order A B B A A (A = candidate worktree wt, B = base 0a746117 worktree wt-base). Tags cand1 base1 base2 cand2 cand3.
D=/data/claude-candhw; H=$D/wt/scripts/bench/candidate_hw; EV=$D/ev/l2
echo "CHAIN_START $(date -Is)" > $EV/chain.log
for spec in "wt cand1" "wt-base base1" "wt-base base2" "wt cand2" "wt cand3"; do
  set -- $spec
  s=$(date '+%Y-%m-%d %H:%M:%S')
  # memory gate: wait while pressure is high
  for i in $(seq 60); do f=$(awk '/^full/{split($2,a,"=");print a[2]}' /proc/pressure/memory); m=$(awk '/MemAvailable/{print int($2/1048576)}' /proc/meminfo); [ "${f%.*}" -lt 8 ] && [ "$m" -ge 4 ] && break; sleep 10; done
  bash $H/l2_suite.sh $D/$1 $2 180 > $EV/chain_$2.out 2>&1
  bash $H/xid_map.sh $EV $2 "$s" "$(date '+%Y-%m-%d %H:%M:%S')" > $EV/xidmap_$2.txt 2>&1
  echo "CHAIN_DONE $2 $(date -Is)" >> $EV/chain.log
done
echo "CHAIN_ALL_DONE $(date -Is)" >> $EV/chain.log
