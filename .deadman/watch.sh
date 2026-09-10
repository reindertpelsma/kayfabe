#!/usr/bin/env bash
# ★★★ DEADMAN SWITCH for the rented box. Destroys it only if the heartbeat goes stale, so a
# session that ends without cleanup does not leave it billing — and a session still working
# keeps it alive by touching the file.
#
# ⊘ The window is generous on purpose: a boot takes 5 minutes and an LLM run 20+, so a short
# window would kill the box mid-measurement, which is worse than an hour of billing.
ID=${1:?instance id}
BEAT=${2:?heartbeat file}
WINDOW=${WINDOW:-10800}   # 3 hours
while true; do
  sleep 300
  [ -f "$BEAT" ] || continue
  age=$(( $(date +%s) - $(stat -c %Y "$BEAT") ))
  if [ "$age" -gt "$WINDOW" ]; then
    echo "$(date -Is) heartbeat ${age}s old (> $WINDOW) — destroying $ID" >> "$BEAT.log"
    vastai destroy instance "$ID" -y >> "$BEAT.log" 2>&1
    exit 0
  fi
done
