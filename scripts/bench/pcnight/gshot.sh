#!/bin/bash
N=$1; mkdir -p /workspace/bench/pcnight_20261113/shots
/tmp/claude-0/gi.sh sh_$N "& C:\\kf\\shot.ps1 -Out C:\\kf\\t\\$N.png" | tail -1
ssh root@172.22.1.20 "scp -q -i /workspace/bench/propimage_20261112/keys/id_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 33037 kfimage@127.0.0.1:C:/kf/t/$N.png /tmp/$N.png" 2>&1 | grep -v Warning
scp -q root@172.22.1.20:/tmp/$N.png /workspace/bench/pcnight_20261113/shots/$N.png && echo got $N
