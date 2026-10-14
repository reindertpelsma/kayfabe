#!/bin/bash
# Start the Linux demo (interactive.sh, kf3 4bc62999) once the GPU lock is free (another agent held it at 20:09).
for i in $(seq 1 720); do
  if ! fuser /tmp/kayfabe-fastguest.lock >/dev/null 2>&1; then
    sleep 20
    fuser /tmp/kayfabe-fastguest.lock >/dev/null 2>&1 && continue
    cd /var/lib/kf-windows-20261005/kayfabe-broker-interactive/scripts/bench/display && KF3_REV=4bc62999 exec bash ./interactive.sh
  fi
  sleep 10
done
echo "gave up waiting for the GPU lock $(date -Is)"
