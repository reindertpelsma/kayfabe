#!/bin/bash
# chain.sh REV N1:FLOOD1 N2:FLOOD2 ... — serial boots (FLOOD "-" = flood off), each under the fastguest flock
REV=$1; shift
for a in "$@"; do
  n=${a%%:*}; f=${a#*:}
  [ "$f" = "-" ] && f=""
  bash /var/lib/kf-windows-20261005/irqflood-launch2.sh "$n" "$REV" "$f"
  sleep 20
done
