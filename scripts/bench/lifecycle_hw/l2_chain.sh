#!/bin/bash
# order B B A (A = e1e9947c worktree, B = 0a746117 worktree); lc1b (A) already ran first.
D=/data/claude-life; H=$D/wt-ev/scripts/bench/lifecycle_hw
for spec in "wt-base base1" "wt-base base2" "wt-p1b lc1b2"; do
  set -- $spec
  bash $H/l2_suite.sh $D/$1 $2 180 > $D/ev/l2/chain_$2.out 2>&1
  s=$(grep SUITE_START $D/ev/l2/run_$2.txt | awk '{print $3,$4}' | sed 's/T/ /; s/+.*//' ); 
  bash $H/xid_map.sh $D/ev/l2 $2 "$(grep -o '20[0-9-]*T[0-9:]*' $D/ev/l2/run_$2.txt | head -1 | sed 's/T/ /')" "$(date '+%Y-%m-%d %H:%M:%S')" > $D/ev/l2/xidmap_$2.txt 2>&1
  echo "CHAIN_DONE $2 $(date -Is)" >> $D/ev/l2/chain.log
done
echo "CHAIN_ALL_DONE $(date -Is)" >> $D/ev/l2/chain.log
