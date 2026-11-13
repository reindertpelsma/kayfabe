#!/bin/bash
# g.sh 'powershell cmd'  — run in guest via PC
exec ssh root@172.22.1.20 "ssh -i /workspace/bench/propimage_20261112/keys/id_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=8 -p 33037 kfimage@127.0.0.1 $(printf '%q' "$1")"
