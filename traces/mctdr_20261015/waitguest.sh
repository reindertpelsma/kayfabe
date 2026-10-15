#!/bin/bash
# waitguest.sh : wait until the kfagent answers and the interactive session has explorer, then 45 s settle
for i in $(seq 1 60); do
  o=$(timeout 25 python3 /data/kfagent_server.py run '(Get-Process explorer -ErrorAction SilentlyContinue | measure).Count; (Get-Date).ToString("HH:mm:ss")' 20 2>/dev/null | head -1 | tr -d '\r')
  [ "$o" = 1 ] && break; sleep 5
done
echo "guest ready after ~$((i*5+0)) s: explorer=$o at $(date +%T)"; sleep 45
