#!/bin/bash
# put.sh files... : local -> PC -> guest C:\kf\
for f in "$@"; do scp -q $f root@172.22.1.20:/tmp/; done
ssh root@172.22.1.20 "cd /tmp; scp -q -i /workspace/bench/propimage_20261112/keys/id_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 33037 $(for f in "$@"; do basename $f; done | tr '\n' ' ') kfimage@127.0.0.1:C:/kf/" 2>&1 | grep -v Warn
