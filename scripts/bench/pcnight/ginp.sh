#!/bin/bash
# ginp.sh 'm 10 10; c'
echo "$1" > /workspace/bench/pcnight_20261113/tools/ops.txt
/workspace/bench/pcnight_20261113/tools/put.sh /workspace/bench/pcnight_20261113/tools/ops.txt
/tmp/claude-0/gi.sh inp_$RANDOM '& C:\\kf\\inp.ps1 -File C:\\kf\\ops.txt' | tail -2
