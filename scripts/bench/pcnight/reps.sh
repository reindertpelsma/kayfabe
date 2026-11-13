#!/bin/bash
A=$1; D=/workspace/bench/pcnight_20261113
for R in 1 2; do /tmp/claude-0/runbench.sh $A $R; ssh root@172.22.1.20 "cd $D; bash summ.sh $A $R" > $D/local_$A/summ_$R.txt 2>&1; done
echo ALLDONE
