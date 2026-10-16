#!/bin/bash
for spec in "r1 root" "u2 user" "r2 root"; do set -- $spec; bash /data/claude-rootless/bin/win_heaven.sh $1 $2 > /data/claude-rootless/ev/win_$1.out 2>&1; done
echo CHAIN_DONE > /data/claude-rootless/ev/chain_win.done
