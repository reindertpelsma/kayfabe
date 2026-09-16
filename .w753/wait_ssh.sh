#!/usr/bin/env bash
for i in $(seq 1 40); do
  if ./.w753/vh.sh 'echo READY' 2>/dev/null | grep -q READY; then echo "SSH_UP $(date +%T)"; exit 0; fi
  sleep 15
done
echo "SSH_TIMEOUT"; exit 1
